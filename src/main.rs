#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::similar_names
)]

use std::{fmt::Display, str::FromStr, sync::OnceLock};

use axum::{http::header::HeaderValue, response::Response, response::Html, routing::get, Json};
use bytes::Bytes;
use deadpool_redis::redis::AsyncCommands;
use serde_json::to_value;
use sha2::Digest;
use small_fixed_array::{FixedString, ValidLength};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod polly;
mod translation;
mod wadiwayan;

mod test;

type Result<T, E = anyhow::Error> = std::result::Result<T, E>;
type ResponseResult<T> = std::result::Result<T, Error>;

#[must_use]
pub fn check_mp3_length(audio: &[u8], max_length: u64) -> bool {
    use bytes::Buf;
    mp3_duration::from_read(&mut audio.reader()).map_or(true, |d| d.as_secs() < max_length)
}

#[derive(serde::Deserialize)]
struct GetVoices {
    mode: TTSMode,
    #[serde(default)]
    raw: bool,
}

async fn get_voices(
    axum::extract::Query(payload): axum::extract::Query<GetVoices>,
) -> ResponseResult<impl axum::response::IntoResponse> {
    let GetVoices { mode, raw } = payload;
    let state = STATE.get().unwrap();

    Ok(axum::Json(if raw {
        match mode {
            TTSMode::Polly => to_value(polly::get_raw_voices(&state.polly).await?),
        }?
    } else {
        to_value(match mode {
            TTSMode::Polly => polly::get_voices(&state.polly).await?,
        })?
    }))
}

async fn get_translation_languages() -> ResponseResult<Json<Vec<(FixedString, FixedString)>>> {
    let state = STATE.get().unwrap();
    let Some(token) = &state.translation_key else {
        return Ok(Json(Vec::new()));
    };

    match translation::get_languages(&state.reqwest, token).await {
        Ok(languages) => Ok(Json(languages)),
        Err(err) => Err(Error::Unknown(err)),
    }
}

#[derive(serde::Deserialize)]
struct GetTTS {
    text: FixedString,
    mode: TTSMode,
    #[serde(rename = "lang")]
    voice: FixedString<u8>,
    #[serde(default)]
    speaking_rate: Option<f32>,
    max_length: Option<u64>,
    #[serde(default)]
    preferred_format: Option<FixedString<u8>>,
    #[serde(default)]
    translation_lang: Option<FixedString<u8>>,
}

async fn get_tts(
    axum::extract::Query(payload): axum::extract::Query<GetTTS>,
    headers: axum::http::HeaderMap,
) -> ResponseResult<Response<axum::body::Body>> {
    let state = STATE.get().unwrap();
    if let Some(auth_key) = state.auth_key.as_deref() {
        let auth_header = headers.get("Authorization");
        if auth_header.map(HeaderValue::to_str).transpose()? != Some(auth_key) {
            return Err(Error::Unauthorized);
        }
    }

    let translation_lang = payload.translation_lang;
    let preferred_format = payload.preferred_format;
    let speaking_rate = payload.speaking_rate;
    let mut text = payload.text;
    let voice = payload.voice;
    let mode = payload.mode;

    mode.check_speaking_rate(speaking_rate)?;
    mode.check_voice(state, &voice).await?;

    let mut cache_key = format!("{text} {voice} {mode} {}", speaking_rate.unwrap_or(0.0));

    if let Some(preferred_format) = &preferred_format {
        cache_key.push(' ');
        cache_key.push_str(preferred_format);
    }

    if let Some(translation_lang) = &translation_lang {
        cache_key.push(' ');
        cache_key.push_str(translation_lang);
    }

    tracing::debug!("Recieved request to TTS: {cache_key}");

    let redis_info = if let Some(redis_state) = &state.redis {
        let cache_hash = sha2::Sha256::digest(&cache_key);

        let mut conn = redis_state.client.get().await?;
        let cached_audio = conn
            .get::<_, Option<String>>(&*cache_hash)
            .await?
            .map(|enc| redis_state.key.decrypt(&enc))
            .transpose()?;

        if let Some(cached_audio) = cached_audio {
            mode.check_length(&cached_audio, payload.max_length)?;

            tracing::debug!("Used cached TTS for {cache_key}");
            return mode.into_response(cached_audio.into(), None);
        }

        Some((conn, &redis_state.key, cache_hash))
    } else {
        None
    };

    if let Some(language) = translation_lang {
        let Some(token) = &state.translation_key else {
            return Err(Error::TranslationDisabled);
        };

        if let Some(translated) = translation::run(&state.reqwest, token, &text, &language).await? {
            text = translated;
        }
    };

    let (audio, content_type) = match mode {
        TTSMode::Polly => {
            polly::get_tts(
                &state.polly,
                text,
                &voice,
                speaking_rate.map(|r| r as u8),
                preferred_format.as_deref(),
            )
            .await?
        }
    };

    tracing::debug!("Generated TTS from {cache_key}");
    if let Some((mut redis_conn, key, cache_hash)) = redis_info {
        if let Err(err) = redis_conn
            .set::<_, _, ()>(&*cache_hash, key.encrypt(&audio))
            .await
        {
            tracing::error!("Failed to cache: {err}");
        } else {
            tracing::debug!("Cached TTS from {cache_key}");
        };
    };

    mode.check_length(&audio, payload.max_length)?;
    mode.into_response(audio, content_type)
}

async fn home() -> Html<&'static str> {
    Html("<h1>Hello from tts-service!</h1>")
}

#[derive(serde::Deserialize, Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
enum TTSMode {
    Polly,
}

impl TTSMode {
    #[allow(clippy::unused_self)]
    fn into_response(
        self,
        data: Bytes,
        _: Option<reqwest::header::HeaderValue>,
    ) -> ResponseResult<Response> {
        Response::builder()
            // TODO: Re-add when reqwest updates http to 1.0
            // .header(axum::http::header::CONTENT_TYPE, content_type.unwrap_or_else(|| HeaderValue::from_static(match self {
            //     #[cfg(feature="gtts")]    Self::gTTS    => "audio/mpeg",
            //     #[cfg(feature="espeak")]  Self::eSpeak  => "audio/wav",
            //     #[cfg(feature="gcloud")]  Self::gCloud  => "audio/opus",
            //     #[cfg(feature="polly")]   Self::Polly   => "audio/ogg",
            // })))
            .body(axum::body::Body::from(data))
            .map_err(Into::into)
    }

    async fn check_voice(self, state: &State, voice: &str) -> ResponseResult<()> {
        if match self {
            Self::Polly => polly::check_voice(&state.polly, voice).await?,
        } {
            Ok(())
        } else {
            Err(Error::UnknownVoice(
                format!("Unknown voice: {voice}").into_boxed_str(),
            ))
        }
    }

    fn check_length(self, audio: &[u8], max_length: Option<u64>) -> ResponseResult<()> {
        if max_length.map_or(true, |max_length| match self {
            Self::Polly => true,
        }) {
            Ok(())
        } else {
            Err(Error::AudioTooLong)
        }
    }

    fn check_speaking_rate(self, speaking_rate: Option<f32>) -> ResponseResult<()> {
        if let Some(speaking_rate) = speaking_rate {
            if let Some(max) = self.max_speaking_rate() {
                if speaking_rate > max {
                    return Err(Error::InvalidSpeakingRate(speaking_rate));
                }
            }
        }

        Ok(())
    }

    const fn max_speaking_rate(self) -> Option<f32> {
        match self {
            Self::Polly => Some(500.0),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Polly => "Polly",
        }
    }
}

impl Display for TTSMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for TTSMode {
    fn serialize<S>(&self, serializer: S) -> std::prelude::v1::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

struct RedisCache {
    client: deadpool_redis::Pool,
    key: fernet::Fernet,
}

struct State {
    auth_key: Option<FixedString<u8>>,
    translation_key: Option<FixedString<u8>>,
    reqwest: reqwest::Client,

    redis: Option<RedisCache>,
    polly: polly::State,
}

static STATE: OnceLock<State> = OnceLock::new();

fn str_to_fixedstring<LenT: ValidLength>(str: String) -> FixedString<LenT> {
    FixedString::try_from(str.into_boxed_str()).expect("string should be less than 256 chars long")
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_env = std::env::var("LOG_LEVEL");

    let fmt_layer = tracing_subscriber::fmt::layer();
    let filter = LevelFilter::from_str(log_env.as_deref().unwrap_or("INFO"))?;

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(filter)
        .init();
    let client = reqwest::Client::new();
    let redis_uri = std::env::var("REDIS_URI").ok();
    let has_redis_uri = redis_uri.is_some();
    let result = STATE.set(State {
        reqwest: client.clone(),
        polly: polly::State::new(&aws_config::load_from_env().await),

        auth_key: std::env::var("AUTH_KEY").ok().map(str_to_fixedstring),
        translation_key: std::env::var("DEEPL_KEY").ok().map(str_to_fixedstring),
        redis: redis_uri.map(|uri| {
            let key = std::env::var("CACHE_KEY").expect("CACHE_KEY not set!");
            RedisCache {
                client: deadpool_redis::Config::from_url(uri)
                    .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                    .unwrap(),
                key: fernet::Fernet::new(&key).unwrap(),
            }
        }),
    });
    if result.is_err() {
        unreachable!()
    }

    let app = axum::Router::new()
        .route("/", get(home))
        .route("/tts", get(get_tts))
        .route("/voices", get(get_voices))
        .route("/translation_languages", get(get_translation_languages))
        .route(
            "/modes",
            get(|| async {
                axum::Json([
                    TTSMode::Polly,
                ])
            }),
        );

    let env_addr = std::env::var("BIND_ADDR");
    let bind_to = env_addr.as_deref().unwrap_or("0.0.0.0:3000");

    tracing::info!(
        "Binding to {bind_to} {} redis enabled!",
        if has_redis_uri { "with" } else { "without" }
    );

    let listener = tokio::net::TcpListener::bind(bind_to).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

#[derive(Debug)]
enum Error {
    Unauthorized,
    TranslationDisabled,
    UnknownVoice(Box<str>),
    AudioTooLong,
    InvalidSpeakingRate(f32),

    Unknown(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Self::Unknown(e.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSpeakingRate(rate) => write!(f, "Invalid speaking rate: {rate}"),
            Self::AudioTooLong => f.write_str("Max length exceeded!"),
            Self::UnknownVoice(msg) => f.write_str(msg),
            Self::Unauthorized => write!(f, "Unauthorized request"),
            Self::TranslationDisabled => {
                write!(f, "Translation requested but no key has been provided")
            }
            Self::Unknown(e) => write!(f, "Unknown error: {e}"),
        }
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> Response {
        if let Error::Unknown(inner) = &self {
            tracing::error!("{inner:?}");
        };

        let json_err = serde_json::json!({
            "display": self.to_string(),
            "code": match self {
                Self::TranslationDisabled => 5,
                Self::Unauthorized => 4,
                Self::InvalidSpeakingRate(_) => 3,
                Self::AudioTooLong => 2,
                Self::UnknownVoice(_) => 1,
                Self::Unknown(_) => 0_u8,
            },
        });

        let status = match self {
            Self::AudioTooLong | Self::InvalidSpeakingRate(_) | Self::TranslationDisabled => {
                axum::http::StatusCode::BAD_REQUEST
            }
            Self::Unknown(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::UnknownVoice(_) => axum::http::StatusCode::BAD_REQUEST,
            Self::Unauthorized => axum::http::StatusCode::FORBIDDEN,
        };

        (status, axum::Json(json_err)).into_response()
    }
}

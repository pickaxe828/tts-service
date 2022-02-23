use tokio::sync::RwLock;
use rand::Rng;

use crate::Error;


#[derive(Clone)]
pub(crate) struct State {
    ip: std::net::IpAddr,
    http: reqwest::Client
}

impl State {
    pub async fn new() -> Result<RwLock<Self>, Error> {
        Ok(RwLock::new({
            let (ip, http) = get_random_ipv6().await?;
            Self {ip, http}
        }))
    }
}



fn parse_url(text: &str, lang: &str) -> reqwest::Url {
    let mut url = reqwest::Url::parse("https://translate.google.com/translate_tts?ie=UTF-8&total=1&idx=0&client=tw-ob").unwrap();
    url.query_pairs_mut()
        .append_pair("tl", lang)
        .append_pair("q", text)
        .append_pair("textlen", &text.len().to_string())
        .finish();
    url
}

async fn get_random_ipv6() -> Result<(std::net::IpAddr, reqwest::Client), Error> {
    let ip_block = std::env::var("IPV6_BLOCK")
        .expect("IPV6_BLOCK not set!").parse()
        .expect("Invalid IPV6 Block!");

    loop {
        let name: String = rand::thread_rng()
            .sample_iter::<char, _>(rand::distributions::Standard)
            .take(16)
            .collect();
    
        tracing::debug!("Generated random name: {:?}", name.as_bytes());
        let ip = ipgen::ip(&name, ip_block).unwrap();

        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(500))
            .local_address(Some(ip))
            .build()?;

        match client.get(parse_url("Hello", "en")).send().await {
            Ok(_) => {
                tracing::warn!("Generated random IP: {}", ip);
                break Ok((ip, client))
            },
            Err(err) if err.is_timeout() => {
                tracing::warn!("Generated IP {} timed out!", ip);
                continue
            },
            Err(err) => break Err(Error::Reqwest(err))
        }
    }
}


pub(crate) async fn get_tts(state: &RwLock<State>, text: &str, lang: &str) -> Result<reqwest::Response, Error> {
    loop {
        let (ip, resp) = {
            let State{ip, http} = state.read().await.clone();
            (ip, http.get(parse_url(text, lang)).send().await?)
        };
    
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            // Generate a new client, with an new IP, and try again
            tracing::warn!("IP {} has been blocked!", ip);
    
            let (new_ip, new_http) = get_random_ipv6().await?;
            let mut state = state.write().await;
            state.http = new_http;
            state.ip = new_ip;
        } else {
            break Ok(resp)
        }
    }
}

pub(crate) fn get_voices() -> Vec<String> {
    serde_json::from_str(include_str!("data/voices-gtts.json")).unwrap()
}
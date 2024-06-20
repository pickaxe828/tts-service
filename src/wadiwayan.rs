use regex::Regex;

pub fn wadiwayan_to_ipa(content: String) -> String {
    content
    // Unorthodox spellings
    .replace("dy", "d͡ʒ")
    .replace("ty", "t͡ʃ")
    // Official spellings
    .replace("j", "d͡ʒ")
    .replace("c", "t͡ʃ")
    .replace("ng", "ŋ")
    .replace("sh", "ʃ")
    .replace("ph", "pʰ")
    .replace("th", "tʰ")
    .replace("kh", "kʰ")
    // Post processing (Official spellings)
    .replace("y", "j")
    .replace("'", "ʔ")
}

pub fn process(content: String) -> String {
    let re = Regex::new(r"w\/`(.|[\n\r])*?`").unwrap();
    let matches: Vec<_> = re.find_iter(&content).collect();
    let mut res: Vec<String> = vec![];

    for i in 0..matches.len()+1 {
        let start = if let Some(res) = matches.get((i as isize -1) as usize) { res.end() } else { 0 };
        let end = if let Some(res) = matches.get(i) { res.start() } else { content.len() as usize };
        
        res.push(content[start..end].to_string().clone());
        if i < matches.len() {
            let wadiwayan_ipa = 
            wadiwayan_to_ipa(
                matches[i].as_str().to_string()
                .replace("w/`", "")
                .replace("`", "")
            );

            res.push(format!("<phoneme alphabet=\"ipa\" ph=\"{wadiwayan_ipa}\"></phoneme>"));
        }
    }

    res.join("")
}
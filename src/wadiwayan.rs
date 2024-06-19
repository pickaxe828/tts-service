pub fn wadiwayan_to_ipa(content: String) -> String {
    content
    // Unorthodox spellings
    .replace("dy", "d͡ʒ")
    .replace("ty", "t͡ʃ")
    // Official spellings
    .replace("ng", "ŋ")
    .replace("c", "t͡s")
    .replace("sh", "ʃ")
    .replace("ph", "pʰ")
    .replace("th", "tʰ")
    .replace("kh", "kʰ")
    // Post processing (Official spellings)
    .replace("y", "j")
    .replace("'", "ʔ")
}
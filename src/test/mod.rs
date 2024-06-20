#[cfg(test)]
mod tests {
    use crate::wadiwayan;

    #[test]
    fn test_process() {
        let content = "This is a message containing w/`Wadiwayan` script by w/`Ayuyuwana`
but how about this w/`multiline
Wadiwayan D:
`omg it works";
        println!("{:?}", wadiwayan::process(content.to_string()));
        assert_eq!(4, 4);
    }
}
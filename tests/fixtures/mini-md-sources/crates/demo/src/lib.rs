pub fn parse(text: &str) -> Result<(), String> {
    if text.is_empty() { Err("empty".into()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits() {
        assert!(super::parse("x").is_ok());
    }
}

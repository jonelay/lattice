// fn loads_old_profile() was renamed; only the comment remembers it.
#[test]
fn loads_profile() {
    assert!(demo::parse("profile").is_ok());
}

// Fixture test file in the Rust dialect. Never compiled — the adapter reads
// it as text.

#[test]
fn early_uncited() {}

// Requirement: alpha/Alpha parses input
#[test]
fn parses_a() {}

#[test]
#[ignore]
fn parses_b() {}

// Requirement: alpha/Alpha reports errors
#[test]
fn reports() {}

fn helper_not_a_test() {}

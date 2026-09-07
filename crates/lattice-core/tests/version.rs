//! Version-changelog agreement: the shipped version has a matching entry.
//!
//! Ported from the Python suite's `test_version.py`, whose subject moved from
//! `pyproject.toml` to the Cargo workspace when the core did.

use std::path::PathBuf;

/// The repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

// Requirement: Version-changelog agreement

#[test]
fn crate_version_has_a_changelog_heading() {
    let version = env!("CARGO_PKG_VERSION");
    let changelog = std::fs::read_to_string(repo_root().join("CHANGELOG.md"))
        .expect("CHANGELOG.md is readable");
    let heading = format!("## [{version}]");

    assert!(
        changelog.lines().any(|l| l.starts_with(&heading)),
        "version {version} has no matching `{heading}` heading in CHANGELOG.md"
    );
}

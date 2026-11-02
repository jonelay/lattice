//! Version-changelog agreement: the shipped version has a matching entry, and
//! candidate entries exist only while their release is in progress.
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

// Requirement: versioning/Version-changelog agreement

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

// Requirement: versioning/Release-candidate identity agrees across release metadata

/// A candidate heading names the release in progress: once `X.Y.0` is cut its
/// candidates fold into the `X.Y.0` section, so a shipped changelog has none.
#[test]
fn candidate_headings_name_only_the_release_in_progress() {
    let version = env!("CARGO_PKG_VERSION");
    let in_progress = version.split_once('-').map(|(release, _)| release);
    let changelog = std::fs::read_to_string(repo_root().join("CHANGELOG.md"))
        .expect("CHANGELOG.md is readable");
    let stale: Vec<&str> = changelog
        .lines()
        .filter_map(|l| l.strip_prefix("## ["))
        .filter_map(|l| l.split_once(']').map(|(v, _)| v))
        .filter(|v| {
            v.split_once('-')
                .is_some_and(|(release, _)| Some(release) != in_progress)
        })
        .collect();
    assert!(
        stale.is_empty(),
        "workspace {version}: fold these candidate sections into their release: {stale:?}"
    );
}

/// `pyproject.toml`'s version, as written. The first `version =` line is the
/// `[project]` table's: the file has no other.
fn pyproject_version() -> String {
    let text = std::fs::read_to_string(repo_root().join("pyproject.toml"))
        .expect("pyproject.toml is readable");
    let line = text
        .lines()
        .find(|l| l.starts_with("version = "))
        .expect("pyproject.toml declares a version");
    line.trim_start_matches("version = ")
        .trim_matches('"')
        .to_owned()
}

/// A Cargo pre-release version in PEP 440's normalized spelling: `0.8.0-rc.8`
/// is `0.8.0rc8`. A version with no pre-release is the same in both.
fn pep440(cargo: &str) -> String {
    let Some((release, pre)) = cargo.split_once('-') else {
        return cargo.to_owned();
    };
    let (label, number) = pre.split_once('.').unwrap_or((pre, ""));
    let label = match label {
        "alpha" => "a",
        "beta" => "b",
        other => other,
    };
    format!("{release}{label}{number}")
}

// Requirement: versioning/Version-changelog agreement

#[test]
fn python_package_version_matches_the_workspace() {
    let cargo = env!("CARGO_PKG_VERSION");
    assert_eq!(
        pyproject_version(),
        pep440(cargo),
        "pyproject.toml's version must be the workspace version {cargo} in PEP 440 form"
    );
}

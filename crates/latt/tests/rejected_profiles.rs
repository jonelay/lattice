//! The rejected-profile corpus: one case per load refusal, each pinning the exit
//! code and the message the binary prints.
//!
//! Cases live under `tests/fixtures/rejected-profiles/<case>/` as `profile.yaml`
//! (run through `latt show`) or `fuse.yaml` (run through `latt fuse`
//! with a generated manifest), beside `expected.json`. The directory is walked
//! when the test runs, so a case added without a code change is still checked,
//! and every mismatch is collected with its case name before the test fails.
//! Set `LATTICE_REGEN_BASELINES=1` to rewrite `expected.json`; the diff is the
//! review (development-verification spec, profile load refusals are pinned).

mod common;

#[path = "common/regen.rs"]
mod regen;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rejected-profiles")
}

/// Run one case through the binary: `(exit, stdout, stderr)` with every path
/// into the case or the scratch directory replaced by `<case>`.
fn run_case(dir: &Path) -> (i32, String, String) {
    let profile = dir.join("profile.yaml");
    let fuse = dir.join("fuse.yaml");
    let mut scratch_path = None;
    let output = if profile.is_file() {
        common::lattice()
            .args(["show", "--profile"])
            .arg(&profile)
            .output()
    } else if fuse.is_file() {
        // The fuse profile is loaded before any source runs, so the manifest
        // needs only a well-formed source list that is never reached. Manifest
        // paths are relative, so the case's file is copied beside it.
        let scratch = common::scratch();
        fs::copy(&fuse, scratch.join("fuse.yaml")).unwrap();
        let manifest = scratch.join("manifest.yaml");
        fs::write(
            &manifest,
            "manifest_version: '1.0.0'\nname: corpus\nversion: '1.0.0'\n\
             fuse_profile: fuse.yaml\nsources:\n  - {name: s, profile: none.yaml, adapter: none, target: .}\n",
        )
        .unwrap();
        scratch_path = Some(scratch.clone());
        common::lattice()
            .args(["fuse", "--format", "plain", "--manifest"])
            .arg(&manifest)
            .output()
    } else {
        panic!("{}: a case holds profile.yaml or fuse.yaml", dir.display());
    }
    .expect("the lattice binary runs");
    let normalize = |bytes: &[u8]| {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        if let Some(scratch) = &scratch_path {
            text = text.replace(&scratch.display().to_string(), "<case>");
        }
        text.replace(&dir.display().to_string(), "<case>")
    };
    (
        common::code(&output),
        normalize(&output.stdout),
        normalize(&output.stderr),
    )
}

// Requirement: development-verification/Profile load refusals are pinned by a corpus
#[test]
fn every_case_refuses_as_pinned() {
    let regen = regen::regen_baselines();
    let mut cases: Vec<PathBuf> = fs::read_dir(corpus())
        .expect("the corpus directory exists")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "the rejected-profile corpus is empty");

    let mut mismatches = Vec::new();
    for dir in &cases {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let (exit, stdout, stderr) = run_case(dir);
        let actual = json!({"exit": exit, "stdout": stdout, "stderr": stderr});
        let expected_path = dir.join("expected.json");
        if regen {
            fs::write(
                &expected_path,
                serde_json::to_string_pretty(&actual).unwrap() + "\n",
            )
            .unwrap();
            continue;
        }
        let expected: Value = match fs::read_to_string(&expected_path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                panic!("{name}: expected.json is not JSON: {e}");
            }),
            Err(_) => {
                mismatches.push(format!(
                    "{name}: no expected.json; run with LATTICE_REGEN_BASELINES=1 to record\n\
                     actual: {actual:#}"
                ));
                continue;
            }
        };
        if expected != actual {
            mismatches.push(format!(
                "{name}:\nexpected: {expected:#}\nactual:   {actual:#}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} cases differ:\n\n{}",
        mismatches.len(),
        cases.len(),
        mismatches.join("\n\n")
    );
}

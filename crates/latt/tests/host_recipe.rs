//! The host recipe in `docs/embedding.md`, on supported items only, answers what
//! `latt validate` prints for the same profile and target.
//!
//! The recipe runs the profile's native TOML adapter, so the suite needs it built in.
#![cfg(feature = "native-adapters")]

mod common;

use std::path::{Path, PathBuf};

use common::{code, stderr, stdout};
use latt_core::document::{resolve_adapter, run_adapter_spec};
use latt_core::output::{Format, output_result};
use latt_core::profile::{load_profile, resolved_document};
use latt_core::service::{InputContext, Request, View, execute};
use latt_core::validate::{apply_messages, apply_suppressions};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// Requirement: graph-service/The supported embedding surface is service-first
#[test]
fn a_host_validating_through_the_service_matches_latt_validate() {
    let profile_path = repo().join("profiles/toml.yaml");
    let target = repo().join("tests/fixtures/mini-toml");

    let profile = load_profile(&profile_path).unwrap();
    let resolved = resolved_document(&profile).unwrap();
    let spec = resolve_adapter(None, &profile).unwrap();
    let graph = run_adapter_spec(&spec, &resolved, &target, profile.adapter_timeout()).unwrap();
    let context = InputContext::default();
    let mut answer = execute(
        View {
            graph: &graph,
            profile: &profile,
            context: &context,
        },
        Request::Validate { strict: false },
    )
    .unwrap();
    apply_suppressions(
        answer.diagnostics.iter_mut().map(|d| &mut d.issue),
        &profile,
    );
    apply_messages(
        answer.diagnostics.iter_mut().map(|d| &mut d.issue),
        &profile,
        &graph,
    );
    let hosted = output_result(&answer.payload, Format::Json);

    let cli = common::lattice()
        .args(["validate", "--format", "json", "--profile"])
        .arg(&profile_path)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("the latt binary runs");
    assert!(code(&cli) < 2, "{}", stderr(&cli));
    assert_eq!(hosted.trim_end(), stdout(&cli).trim_end());
    let findings: serde_json::Value = serde_json::from_str(&hosted).unwrap();
    assert!(
        findings["findings"]
            .as_array()
            .is_some_and(|f| !f.is_empty()),
        "the comparison must cover findings: {hosted}"
    );
}

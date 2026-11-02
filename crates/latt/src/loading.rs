//! Loading the profile and graph for a command, and gating on unreadable input.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use latt_core::document::{resolve_adapter, run_adapter_spec_for_profile};
use latt_core::graph::LatticeGraph;
use latt_core::output::{Format, output_result};
use latt_core::profile::{Profile, load_profile, resolved_document};
use latt_core::suggest::render_suggestions;
use latt_core::types::Issue;

use super::console::{Stream, echo};
use super::{Common, Outcome, describe};

/// Load the profile and run the adapter, or report why neither could happen.
///
/// Every failure here is operational rather than a finding about the register,
/// so each is exit 2 with a message instead of an `Issue`.
pub(super) fn load(common: &Common) -> Result<(Profile, LatticeGraph), String> {
    let profile = load_profile_from(common)?;
    let graph = load_graph(common, &profile)?;
    Ok((profile, graph))
}

/// Load the profile and hold it to `--profile-version` when one is given.
///
/// Every command that reads a profile through `Common` comes here, so the pin
/// cannot be honoured by one command and skipped by another.
pub(super) fn load_profile_from(common: &Common) -> Result<Profile, String> {
    load_profile(&common.profile)
        .and_then(|profile| {
            if let Some(exact) = &common.profile_version {
                profile.require_version(exact)?;
            }
            Ok(profile)
        })
        .map_err(|e| format!("Profile error: {}", describe(&e)))
}

/// The adapter `--adapter` or the profile names, as `describe` reports it: a
/// native adapter's name, or the external program as given.
pub(super) fn adapter_label(common: &Common, profile: &Profile) -> Result<String, String> {
    let spec = resolve_adapter(common.adapter.as_deref().map(|p| p.as_os_str()), profile)
        .map_err(|e| e.message().to_owned())?;
    Ok(match spec {
        latt_core::document::AdapterSpec::External(program) => program.display().to_string(),
        latt_core::document::AdapterSpec::Native(native) => native.name().to_owned(),
        // `AdapterSpec` is non-exhaustive; a variant this binary predates is
        // shown as the core debug-prints it rather than refused.
        other => format!("{other:?}"),
    })
}

/// Run the adapter against an already loaded profile.
pub(super) fn load_graph(common: &Common, profile: &Profile) -> Result<LatticeGraph, String> {
    let resolved =
        resolved_document(profile).map_err(|e| format!("Profile error: {}", describe(&e)))?;
    let spec = resolve_adapter(common.adapter.as_deref().map(|p| p.as_os_str()), profile)
        .map_err(|e| e.message().to_owned())?;
    run_adapter_spec_for_profile(
        &spec,
        &resolved,
        &common.target,
        profile.adapter_timeout(),
        Some(&common.profile),
    )
    .map_err(|e| e.message().to_owned())
}

/// Render every named suggestion document as hint findings, in document order.
///
/// A document the core cannot use (unreadable, unparseable, or a version it
/// does not support) is a broken setup rather than a finding about the
/// register, so it travels as an `Err` the caller turns into exit 2.
pub(super) fn overlay(paths: &[PathBuf], graph: &LatticeGraph) -> Result<Vec<Issue>, String> {
    let mut issues = Vec::new();
    for path in paths {
        let where_ = format!("suggestions '{}'", path.display());
        let text = std::fs::read_to_string(path).map_err(|e| format!("{where_}: {e}"))?;
        let document: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("{where_}: could not parse as JSON: {e}"))?;
        issues.extend(render_suggestions(&document, graph).map_err(|e| format!("{where_}: {e}"))?);
    }
    Ok(issues)
}

/// True when the register could not be read at all: the adapter declared no
/// node and reported an error that gates. A query about it cannot be posed, and
/// answering anyway ("No orphans.", a table of zeros) reads as a clean register.
/// A register that is empty but readable carries no such error and is answered.
pub(super) fn unreadable(graph: &LatticeGraph, adapter_issues: &[Issue]) -> bool {
    graph.iter_nodes().next().is_none() && adapter_issues.iter().any(Issue::gates)
}

/// Apply the profile's SUPPRESS entries to the service's envelope diagnostics.
///
/// The envelope carries them unsuppressed by contract (input attribution for a
/// host); the CLI is a consumer with the profile's policy, so a suppressed
/// adapter issue neither prints in `plain` nor gates, as it would not under
/// `validate`. Called after `unreadable`: a register the adapter read nothing
/// from is refused on what the adapter said, whatever the profile suppresses.
pub(super) fn suppress_adapter_issues(
    adapter_issues: &mut [Issue],
    profile: &Profile,
    graph: &LatticeGraph,
) {
    latt_core::validate::apply_suppressions(adapter_issues.iter_mut(), profile);
    latt_core::validate::apply_messages(adapter_issues.iter_mut(), profile, graph);
}

/// True when the adapter issues have something to print on stderr: json keeps a
/// suppressed record (marked), plain and rich omit it, so an all-suppressed list
/// prints nothing rather than a blank line or "No findings.".
pub(super) fn reportable(adapter_issues: &[Issue], format: Format) -> bool {
    if format == Format::Json {
        !adapter_issues.is_empty()
    } else {
        adapter_issues.iter().any(|i| !i.suppressed)
    }
}

/// Report why a query was not answered, with the adapter's findings, and exit 2.
pub(super) fn refuse_unreadable(adapter_issues: &[Issue], format: Format) -> ExitCode {
    echo(
        &output_result(adapter_issues, format),
        format,
        Stream::Stderr,
    );
    let _ = writeln!(
        std::io::stderr(),
        "Error: no node could be read from the register, so the question was not answered"
    );
    Outcome::CouldNotRun.into()
}

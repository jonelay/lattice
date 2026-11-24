//! Loading the profile and graph for a command, and gating on unreadable input.

use std::path::PathBuf;
use std::process::ExitCode;

use latt_core::document::{resolve_adapter, run_adapter_spec_for_profile};
use latt_core::graph::LatticeGraph;
use latt_core::output::{Format, output_result, sanitize_text};
use latt_core::profile::{Profile, load_profile, resolved_document};
use latt_core::service::ServiceError;
use latt_core::suggest::render_suggestions;
use latt_core::types::{FuseReport, FuseStage, Issue};

use super::console::{Stream, echo, write_stderr};
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

/// The line that says input went unread when the profile suppressed every
/// trace of it, or `None` when no `PARSE_ERROR` was suppressed.
///
/// `SUPPRESS` hides the finding and keeps the exit code, but an answer over
/// partial input must not read as complete, so this line cannot be suppressed.
/// `json` carries the suppressed records themselves and gets no line.
pub(super) fn partial_input_notice<'a>(
    issues: impl IntoIterator<Item = &'a Issue>,
    format: Format,
) -> Option<String> {
    if format == Format::Json {
        return None;
    }
    let count = issues
        .into_iter()
        .filter(|issue| issue.suppressed && issue.code == latt_adapter::codes::PARSE_ERROR)
        .count();
    let pronoun = match count {
        0 => return None,
        1 => "it",
        _ => "them",
    };
    Some(format!(
        "Partial input: the profile suppresses {count} PARSE_ERROR; --format json lists {pronoun}"
    ))
}

/// Write [`partial_input_notice`] to stderr when there is one.
pub(super) fn note_partial_input<'a>(issues: impl IntoIterator<Item = &'a Issue>, format: Format) {
    if let Some(notice) = partial_input_notice(issues, format) {
        write_stderr(&notice);
    }
}

/// What to do next about a fuse failure at `stage`; `sourced` is whether a
/// source failed. The one unsourced `source_run` is `fuse` failing to find its
/// own executable, which no source's profile can fix.
fn fuse_next_action(sourced: bool, stage: FuseStage) -> &'static str {
    const REPORT_A_BUG: &str = "report this as a lattice bug, with the manifest and its profiles";
    match stage {
        FuseStage::Timeout => {
            "raise timeout_seconds in that source's profile, or run `latt trace` on the source alone"
        }
        FuseStage::SourceRun if !sourced => {
            "check that the running `latt` binary still exists and can be executed, and reinstall it if not"
        }
        FuseStage::SourceRun | FuseStage::SourceOutput | FuseStage::SourceProfile => {
            "run `latt trace` with that source's profile, adapter and target from the manifest and read its stderr"
        }
        FuseStage::Manifest | FuseStage::FuseProfile => "fix the file or entry the message names",
        FuseStage::Composition => REPORT_A_BUG,
        // A stage this build does not name yet.
        _ => REPORT_A_BUG,
    }
}

/// One fuse failure as one line: `source 'X' failed at <stage>: <message>.
/// Next: <action>` for a source's failure, `<stage> failed: <message>. Next:
/// <action>` for the composition's own (the message names the file). Without
/// a stage, the line is `source 'X' failed: <message>` or the message alone.
/// What [`sanitize_text`] escapes (a newline included, a tab not) is written as
/// a visible escape, in the source name as in the message.
pub(super) fn fuse_failure_text(
    source: Option<&str>,
    stage: Option<FuseStage>,
    message: &str,
) -> String {
    let text = match (source, stage) {
        (Some(source), Some(stage)) => format!(
            "source '{source}' failed at {}: {}. Next: {}",
            stage.as_str(),
            message.strip_suffix('.').unwrap_or(message),
            fuse_next_action(true, stage)
        ),
        (None, Some(stage)) => format!(
            "{} failed: {}. Next: {}",
            stage.as_str(),
            message.strip_suffix('.').unwrap_or(message),
            fuse_next_action(false, stage)
        ),
        (Some(source), None) => format!("source '{source}' failed: {message}"),
        (None, None) => message.to_owned(),
    };
    sanitize_text(&text).into_owned()
}

/// Name on stderr why a `fuse` run could not run, after its report on stdout:
/// one `fuse:` line per `SOURCE_FAILURE` or `FUSE_ERROR` in `plain` and `rich`,
/// one findings object in the `validate` shape in `json`. A run that could run
/// writes nothing here.
pub(super) fn note_fuse_failures(report: &FuseReport, format: Format) {
    if report.could_run {
        return;
    }
    let issues: Vec<Issue> = report
        .findings
        .iter()
        .filter(|f| {
            f.issue.code == latt_core::fuse::SOURCE_FAILURE
                || f.issue.code == latt_core::fuse::FUSE_ERROR
        })
        .map(|f| {
            let mut issue = f.issue.clone();
            issue.message = fuse_failure_text(f.source.as_deref(), f.stage, &f.issue.message);
            issue
        })
        .collect();
    if issues.is_empty() {
        return;
    }
    if format == Format::Json {
        echo(&output_result(&issues, format), format, Stream::Stderr);
    } else {
        for issue in &issues {
            write_stderr(&format!("fuse: {}", issue.message));
        }
    }
}

/// Report retained adapter diagnostics before the failed service operation.
pub(super) fn report_service_error(
    error: ServiceError,
    profile: &Profile,
    graph: &LatticeGraph,
    format: Format,
) -> ExitCode {
    let message = describe(&error);
    let mut issues: Vec<_> = error.diagnostics.into_iter().map(|d| d.issue).collect();
    suppress_adapter_issues(&mut issues, profile, graph);
    if reportable(&issues, format) {
        echo(&output_result(&issues, format), format, Stream::Stderr);
    }
    note_partial_input(&issues, format);
    write_stderr(&format!("Error: {message}"));
    Outcome::CouldNotRun.into()
}

/// Report why a query was not answered, with the adapter's findings, and exit 2.
pub(super) fn refuse_unreadable(adapter_issues: &[Issue], format: Format) -> ExitCode {
    echo(
        &output_result(adapter_issues, format),
        format,
        Stream::Stderr,
    );
    write_stderr(
        "Error: no node could be read from the register, so the question was not answered",
    );
    Outcome::CouldNotRun.into()
}

#[cfg(test)]
mod tests {
    use super::{FuseStage, fuse_failure_text};

    #[test]
    fn a_fuse_failure_escapes_control_characters_and_keeps_a_tab() {
        assert_eq!(
            fuse_failure_text(Some("specs"), None, "trace exited 2:\r\nError:\tbad\u{7}"),
            r"source 'specs' failed: trace exited 2:\r\nError:".to_owned() + "\t" + r"bad\u{7}"
        );
        assert_eq!(fuse_failure_text(None, None, "a\n\nb"), r"a\n\nb");
    }

    #[test]
    fn a_fuse_failure_escapes_control_characters_in_the_source_name() {
        assert_eq!(
            fuse_failure_text(Some("evil\nFORGED\u{1b}[31m"), None, "bad\n"),
            r"source 'evil\nFORGED\u{1b}[31m' failed: bad\n"
        );
    }

    #[test]
    fn a_staged_fuse_failure_names_its_stage_and_next_action() {
        assert_eq!(
            fuse_failure_text(
                Some("s\u{1b}"),
                Some(FuseStage::Timeout),
                "trace exceeded 1 s and was stopped"
            ),
            r"source 's\u{1b}' failed at timeout: trace exceeded 1 s and was stopped. Next: raise timeout_seconds in that source's profile, or run `latt trace` on the source alone"
        );
        assert_eq!(
            fuse_failure_text(None, Some(FuseStage::Manifest), "bad manifest.\n."),
            r"manifest failed: bad manifest.\n. Next: fix the file or entry the message names"
        );
    }

    #[test]
    fn fuse_unable_to_find_itself_is_not_sent_to_a_source() {
        assert_eq!(
            fuse_failure_text(
                None,
                Some(FuseStage::SourceRun),
                "No such file or directory"
            ),
            "source_run failed: No such file or directory. Next: check that the running `latt` binary still exists and can be executed, and reinstall it if not"
        );
    }

    #[test]
    fn a_composition_failure_is_reported_as_a_bug() {
        assert_eq!(
            fuse_failure_text(None, Some(FuseStage::Composition), "node 'z:A' repeats"),
            "composition failed: node 'z:A' repeats. Next: report this as a lattice bug, with the manifest and its profiles"
        );
    }
}

//! Compose source traces with explicit, source-qualified cross-register joins.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod compose;
mod fuse_profile;
mod manifest;
mod trace;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use latt_adapter::process::{BoundedError, spawn_bounded};

use serde::Deserialize;

use crate::graph::{DuplicateNode, PathwayError};
use crate::profile::ProfileError;
use crate::types::{FuseFinding, FuseReport, FuseStage, Issue, Origin, Severity};
use crate::validate::promote;

pub use compose::assemble;
pub use fuse_profile::{FuseProfile, load_fuse_profile};
pub use manifest::load_manifest;
pub use trace::{SourceTrace, parse_trace};

/// The highest `manifest_version` major this core reads.
const SUPPORTED_MANIFEST_MAJOR: u64 = 1;

/// A source's trace run failed or could not be read.
pub const SOURCE_FAILURE: &str = "SOURCE_FAILURE";
/// A raw ID occurs in more than one source.
pub const CROSS_SOURCE_DUPLICATE_ID: &str = "CROSS_SOURCE_DUPLICATE_ID";
/// A cross-source edge matches more than one allowed target.
pub const AMBIGUOUS_CROSS_REF: &str = "AMBIGUOUS_CROSS_REF";
/// The run could not start: the manifest or fuse profile did not load.
pub const FUSE_ERROR: &str = "FUSE_ERROR";
/// Every code composition emits.
///
/// Each defaults to error.
pub const CODES: &[&str] = &[
    SOURCE_FAILURE,
    CROSS_SOURCE_DUPLICATE_ID,
    AMBIGUOUS_CROSS_REF,
    FUSE_ERROR,
];

/// True when every finding of a composition code carries a `node_id`: the
/// two that report on a node, not the two that report on a source or the run.
pub(crate) fn carries_node(code: &str) -> bool {
    code == CROSS_SOURCE_DUPLICATE_ID || code == AMBIGUOUS_CROSS_REF
}

/// The fuse report envelope's version, written to its JSON header. It moves
/// when the report's top-level keys or the shape of a node, edge or finding do.
pub(crate) const FUSE_VERSION: &str = "1";

/// Set on the `latt trace --format json` fuse runs per source: that child
/// writes its stderr notices unescaped, because fuse quotes them in a json
/// finding message (the lossless channel) and escapes them itself in `plain`
/// and `rich`. Internal to the parent and child, not a supported interface;
/// `latt`'s `main.rs` reads the same name.
const FUSE_CHILD_ENV: &str = "LATTICE_FUSE_CHILD";

/// One register the manifest composes.
///
/// Its paths are written relative to the manifest file and arrive here already
/// resolved by [`load_manifest`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Source {
    /// The name that qualifies the source's IDs and kinds, `name:id`.
    pub name: String,
    /// The source's lattice profile.
    pub profile: PathBuf,
    /// The adapter that reads the source's register: a compiled-in native
    /// adapter name as written, or a program path.
    pub adapter: PathBuf,
    /// The repository the adapter reads.
    pub target: PathBuf,
}

/// A fuse manifest as loaded: every path resolved against the manifest's own
/// directory, so a caller never joins them again.
///
/// Only [`load_manifest`] builds one, which is what guarantees unique,
/// colon-free source names.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Manifest {
    /// The manifest format's version, as written.
    pub manifest_version: String,
    /// The composition's name, copied into the report header.
    pub name: String,
    /// The composition's version, copied into the report header.
    pub version: String,
    /// The fuse profile declaring the cross-source edge kinds.
    pub fuse_profile: PathBuf,
    /// The registers composed, in manifest order.
    pub sources: Vec<Source>,
}

/// Why a manifest, fuse profile, or composition cannot be used.
///
/// Source failures are not errors: they become `SOURCE_FAILURE` findings in the
/// report.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FuseError {
    /// A manifest or fuse profile file that cannot be read. `Display` names only
    /// the file; the I/O error is its `source()`.
    #[error("failed to read {}", path.display())]
    #[non_exhaustive]
    Read {
        /// The file.
        path: PathBuf,
        /// Why it cannot be read.
        #[source]
        source: std::io::Error,
    },
    /// A manifest or fuse profile file that is not valid YAML. `Display` names
    /// only the file; the parser's error, with its line and column, is its
    /// `source()`.
    #[error("failed to parse {}", path.display())]
    #[non_exhaustive]
    Yaml {
        /// The file.
        path: PathBuf,
        /// The YAML parser's error.
        #[source]
        source: serde_norway::Error,
    },
    /// The manifest violates its contract.
    #[error("{0}")]
    Manifest(String),
    /// The fuse profile violates its contract.
    #[error("{0}")]
    FuseProfile(String),
    /// The core profile loader refuses the composed profile. `Display` says so;
    /// the loader's error is its `source()`.
    #[error("the composed fuse profile was refused")]
    Profile(#[from] ProfileError),
    /// One source's trace cannot be composed; `name` is the manifest source.
    #[error("source '{name}' emitted invalid trace JSON: {message}")]
    #[non_exhaustive]
    Trace {
        /// The manifest source.
        name: String,
        /// What is wrong with its trace.
        message: String,
    },
    /// A composed node ID repeats. Every ID is qualified by its source name and
    /// [`load_manifest`], the only builder of a [`Manifest`], refuses a repeated
    /// source name, so two sources sharing a raw ID become a
    /// `CROSS_SOURCE_DUPLICATE_ID` finding, not this error. This is the defensive
    /// re-check at composition.
    #[error("{0}")]
    Duplicate(#[from] DuplicateNode),
    /// The graph refuses a qualified pathway. [`parse_trace`] already checks each
    /// source pathway, so this is the defensive re-check at composition.
    #[error("{0}")]
    Pathway(#[from] PathwayError),
}

impl FuseError {
    /// Return the stage of a `fuse` run this error stopped at, as
    /// `findings[].stage` names it.
    ///
    /// A file that cannot be read or parsed is the manifest's when it is
    /// `manifest` (the path the run was given), else the fuse profile's. A
    /// refused composed profile is the fuse profile's: each source profile has
    /// loaded alone, so what the loader refuses is a fuse profile entry. A
    /// trace that cannot be read is the source's output, as `fuse` stages it.
    /// The composition's own checks re-check what earlier stages passed.
    #[doc(hidden)]
    #[must_use]
    pub fn stage(&self, manifest: &Path) -> FuseStage {
        let of_file = |path: &Path| {
            if std::fs::canonicalize(manifest).is_ok_and(|resolved| resolved == path) {
                FuseStage::Manifest
            } else {
                FuseStage::FuseProfile
            }
        };
        match self {
            Self::Read { path, .. } | Self::Yaml { path, .. } => of_file(path),
            Self::Manifest(_) => FuseStage::Manifest,
            Self::FuseProfile(_) | Self::Profile(_) => FuseStage::FuseProfile,
            Self::Trace { .. } => FuseStage::SourceOutput,
            Self::Duplicate(_) | Self::Pathway(_) => FuseStage::Composition,
        }
    }
}

fn empty_report(manifest: &Manifest) -> FuseReport {
    FuseReport {
        header: BTreeMap::from([
            ("name".into(), manifest.name.clone()),
            ("version".into(), manifest.version.clone()),
            ("manifest_version".into(), manifest.manifest_version.clone()),
            ("fuse_version".into(), FUSE_VERSION.into()),
        ]),
        could_run: true,
        ..Default::default()
    }
}
fn promote_report(report: &mut FuseReport, strict: bool) {
    promote(report.findings.iter_mut().map(|f| &mut f.issue), strict);
}
fn source_findings(SourceTrace(trace): SourceTrace, source: &str) -> Vec<FuseFinding> {
    trace
        .entries
        .into_iter()
        .flat_map(|node| {
            node.findings
                .into_iter()
                .map(move |f| f.attributed(source, Some(&node.id)))
        })
        .chain(
            trace
                .unattachable_findings
                .into_iter()
                .map(|f| f.attributed(source, None)),
        )
        .collect()
}

/// Join `text`'s non-blank lines with `; `, so a child's stderr stays on the one
/// line a finding is printed on.
fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Say how a child ended: `exited 2`, or the signal that stopped it.
fn exit_description(status: std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exited {code}");
    }
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return format!("was killed by signal {signal}");
    }
    status.to_string()
}

fn source_failure(source: &str, stage: FuseStage, message: String) -> FuseFinding {
    FuseFinding {
        issue: Issue::new(
            Severity::Error,
            SOURCE_FAILURE,
            one_line(&message),
            Origin::FuseSource,
            None,
        ),
        source: Some(source.into()),
        locations: Vec::new(),
        stage: Some(stage),
    }
}
/// Build the report of a composition that could not run: no graph, only the
/// healthy sources' own findings and then every failure.
///
/// `runs` holds one slot per manifest source, `None` where that source failed.
fn failed_report(
    manifest: &Manifest,
    runs: Vec<Option<SourceTrace>>,
    failures: Vec<FuseFinding>,
    strict: bool,
) -> FuseReport {
    let mut report = empty_report(manifest);
    report.could_run = false;
    for (source, trace) in manifest.sources.iter().zip(runs) {
        if let Some(trace) = trace {
            report.findings.extend(source_findings(trace, &source.name));
        }
    }
    report.findings.extend(failures);
    promote_report(&mut report, strict);
    report
}

/// What a source's trace run may take beyond its adapter's bound: the trace's
/// own reading and validation around the adapter run.
const SOURCE_RUN_MARGIN: Duration = Duration::from_secs(30);

/// Run the same lattice executable, accepting trace exit 1 as usable evidence.
///
/// Each source's run is bounded by its profile's adapter timeout plus a margin
/// for the trace's own work; a run past it is killed. Source failures are
/// collected across every source, but never yield a partial graph.
///
/// # Errors
///
/// As [`load_manifest`], [`load_fuse_profile`] and [`assemble`]. A source that
/// fails to run, runs past its bound or emits an unusable trace is not an
/// error: it becomes a `SOURCE_FAILURE` finding in a report that could not run.
pub fn fuse(path: &Path, binary: &Path, strict: bool) -> Result<FuseReport, FuseError> {
    let limit = crate::document::adapter_output_limit().map_err(FuseError::Manifest)?;
    fuse_within(path, binary, strict, SOURCE_RUN_MARGIN, limit)
}

/// [`fuse`], with each source run bounded by its adapter timeout plus `margin`
/// and by `output_limit` bytes of trace on stdout.
fn fuse_within(
    path: &Path,
    binary: &Path,
    strict: bool,
    margin: Duration,
    output_limit: usize,
) -> Result<FuseReport, FuseError> {
    let manifest = load_manifest(path)?;
    let profile = load_fuse_profile(&manifest.fuse_profile)?;
    // Before any source runs: a misnamed endpoint is refused in a moment, not after
    // every adapter has read its register. `assemble` checks again for its callers.
    let loaded: std::collections::HashMap<_, _> = manifest
        .sources
        .iter()
        .filter_map(|source| {
            crate::profile::load_profile(&source.profile)
                .ok()
                .map(|loaded| (source.name.clone(), loaded))
        })
        .collect();
    compose::check_endpoints(&manifest, &profile, &loaded)?;
    let mut runs = Vec::new();
    let mut failures = Vec::new();
    for source in &manifest.sources {
        // A profile that does not load fails its trace at once; the default
        // bound only has to outlast that.
        let adapter_bound = loaded.get(&source.name).map_or(
            Duration::from_secs(crate::profile::DEFAULT_ADAPTER_TIMEOUT_SECONDS),
            crate::profile::Profile::adapter_timeout,
        );
        let mut command = Command::new(binary);
        command
            .args(["trace", "--format", "json", "--profile"])
            .arg(&source.profile)
            .arg("--adapter")
            .arg(&source.adapter)
            .arg("--target")
            .arg(&source.target)
            .env(FUSE_CHILD_ENV, "1")
            .stdin(Stdio::null());
        let result = spawn_bounded(command, adapter_bound.saturating_add(margin))
            .map_err(BoundedError::Io)
            .and_then(|child| {
                child
                    .output_limit(output_limit)
                    .stderr_tail(crate::document::ADAPTER_STDERR_TAIL_BYTES)
                    .wait_output()
            })
            .map_err(|e| match e {
                BoundedError::Io(e) if e.kind() == std::io::ErrorKind::FileTooLarge => (
                    FuseStage::SourceOutput,
                    format!(
                        "trace wrote more than {} to stdout and was stopped",
                        crate::document::describe_byte_count(output_limit)
                    ),
                ),
                BoundedError::TimedOut { bound, .. } => (
                    FuseStage::Timeout,
                    format!("trace exceeded {} s and was stopped", bound.as_secs()),
                ),
                other => (FuseStage::SourceRun, other.to_string()),
            })
            .and_then(|output| {
                if !matches!(output.status.code(), Some(0 | 1)) {
                    return Err((
                        FuseStage::SourceRun,
                        format!(
                            "trace {}: {}",
                            exit_description(output.status),
                            one_line(&String::from_utf8_lossy(&output.stderr))
                        ),
                    ));
                }
                let read = |message: String| (FuseStage::SourceOutput, message);
                let text = std::str::from_utf8(&output.stdout).map_err(|e| read(e.to_string()))?;
                parse_trace(text, &source.name).map_err(|e| read(e.to_string()))
            });
        match result {
            Ok(trace) => runs.push(Some(trace)),
            Err((stage, message)) => {
                failures.push(source_failure(&source.name, stage, message));
                runs.push(None);
            }
        }
    }
    if !failures.is_empty() {
        return Ok(failed_report(&manifest, runs, failures, strict));
    }
    assemble(
        &manifest,
        &profile,
        runs.into_iter().flatten().collect(),
        strict,
    )
}

#[cfg(all(test, unix))]
#[path = "../../../latt/tests/common/stub.rs"]
mod stub;

// Requirement: program-composition/Run each source through latt trace
#[cfg(all(test, unix))]
mod bounded_source_runs {
    use std::time::{Duration, Instant};

    use super::{FuseStage, SOURCE_FAILURE, fuse_within};
    use crate::types::FuseFinding;

    /// The one finding of a fuse whose one source runs the `latt` stub `script`,
    /// in a scratch directory named for `test`.
    fn one_failure(test: &str, script: &str) -> FuseFinding {
        one_failure_within(test, script, crate::document::DEFAULT_ADAPTER_OUTPUT_BYTES)
    }

    /// [`one_failure`] with the trace stdout cap set to `limit` bytes.
    fn one_failure_within(test: &str, script: &str, limit: usize) -> FuseFinding {
        let dir = std::env::temp_dir().join(format!("lattice-fuse-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        write(
            "fuse.yaml",
            "manifest_version: '1.0.0'\nname: n\nversion: '1'\nfuse_profile: fp.yaml\n\
             sources:\n  - {name: s, profile: s.yaml, adapter: ./a, target: .}\n",
        );
        write("fp.yaml", "edge_kinds: {}\n");
        write(
            "s.yaml",
            "name: s\nprofile_version: '1.0.0'\nnode_kinds: {}\nedge_kinds: {}\n",
        );
        let binary = dir.join("stub-latt");
        super::stub::write_stub(&binary, script);
        let report = fuse_within(
            &dir.join("fuse.yaml"),
            &binary,
            false,
            Duration::ZERO,
            limit,
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!report.could_run);
        let [finding] = report.findings.as_slice() else {
            panic!("one finding: {:?}", report.findings);
        };
        assert_eq!(finding.issue.code, SOURCE_FAILURE);
        finding.clone()
    }

    // Requirement: adapter-contract/Adapter output is bounded
    #[test]
    fn a_source_that_floods_trace_output_is_a_source_failure() {
        let started = Instant::now();
        let finding = one_failure_within("flood", "#!/bin/sh\nexec yes\n", 1 << 20);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(finding.stage, Some(FuseStage::SourceOutput));
        assert_eq!(
            finding.issue.message,
            "trace wrote more than 1 MiB to stdout and was stopped"
        );
    }

    #[test]
    fn bad_trace_output_has_stage_source_output() {
        let finding = one_failure("output", "#!/bin/sh\necho not json\n");
        assert_eq!(finding.stage, Some(FuseStage::SourceOutput));
        assert!(
            finding
                .issue
                .message
                .starts_with("source 's' emitted invalid trace JSON"),
            "{}",
            finding.issue.message
        );
    }

    #[test]
    fn nonzero_exit_has_stage_source_run() {
        let finding = one_failure("exit", "#!/bin/sh\necho broke >&2\nexit 3\n");
        assert_eq!(finding.stage, Some(FuseStage::SourceRun));
        assert_eq!(finding.issue.message, "trace exited 3: broke");
    }

    #[test]
    fn a_source_run_past_its_bound_has_stage_timeout() {
        let dir = std::env::temp_dir().join(format!("lattice-fuse-bound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        write(
            "fuse.yaml",
            "manifest_version: '1.0.0'\nname: n\nversion: '1'\nfuse_profile: fp.yaml\n\
             sources:\n  - {name: s, profile: s.yaml, adapter: ./a, target: .}\n",
        );
        write("fp.yaml", "edge_kinds: {}\n");
        write(
            "s.yaml",
            "name: s\nprofile_version: '1.0.0'\nadapter:\n  timeout_seconds: 1\n\
             node_kinds: {}\nedge_kinds: {}\n",
        );
        let binary = dir.join("slow-latt");
        super::stub::write_stub(&binary, "#!/bin/sh\nexec sleep 30\n");

        let started = Instant::now();
        let report = fuse_within(
            &dir.join("fuse.yaml"),
            &binary,
            false,
            Duration::ZERO,
            crate::document::DEFAULT_ADAPTER_OUTPUT_BYTES,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!report.could_run);
        let [finding] = report.findings.as_slice() else {
            panic!("one finding: {:?}", report.findings);
        };
        assert_eq!(finding.issue.code, SOURCE_FAILURE);
        assert_eq!(finding.issue.message, "trace exceeded 1 s and was stopped");
        assert_eq!(finding.stage, Some(FuseStage::Timeout));
        let json: serde_json::Value = serde_json::from_str(&crate::output::output_result(
            &report,
            crate::output::Format::Json,
        ))
        .unwrap();
        assert_eq!(json["findings"][0]["stage"], "timeout", "{json}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

//! Compose source traces with explicit, source-qualified cross-register joins.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod compose;
mod fuse_profile;
mod manifest;
mod trace;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::graph::{DuplicateNode, PathwayError};
use crate::profile::ProfileError;
use crate::types::{FuseFinding, FuseReport, Issue, Origin, Severity};
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
    /// A manifest or fuse profile file that cannot be read.
    #[error("{}: {source}", path.display())]
    #[non_exhaustive]
    Read {
        /// The file.
        path: PathBuf,
        /// Why it cannot be read.
        #[source]
        source: std::io::Error,
    },
    /// A manifest or fuse profile file that is not valid YAML.
    #[error("{}: {source}", path.display())]
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
    /// The core profile loader refuses the composed profile.
    #[error("fuse profile: {0}")]
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

fn source_failure(source: &str, message: String) -> FuseFinding {
    FuseFinding {
        issue: Issue::new(
            Severity::Error,
            SOURCE_FAILURE,
            message,
            Origin::FuseSource,
            None,
        ),
        source: Some(source.into()),
        locations: Vec::new(),
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

/// Run the same lattice executable, accepting trace exit 1 as usable evidence.
///
/// Source failures are collected across every source, but never yield a partial
/// graph.
///
/// # Errors
///
/// As [`load_manifest`], [`load_fuse_profile`] and [`assemble`]. A source that
/// fails to run or emits an unusable trace is not an error: it becomes a
/// `SOURCE_FAILURE` finding in a report that could not run.
pub fn fuse(path: &Path, binary: &Path, strict: bool) -> Result<FuseReport, FuseError> {
    let manifest = load_manifest(path)?;
    let profile = load_fuse_profile(&manifest.fuse_profile)?;
    // Before any source runs: a misnamed endpoint is refused in a moment, not after
    // every adapter has read its register. `assemble` checks again for its callers.
    let loaded = manifest
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
        let result = Command::new(binary)
            .args(["trace", "--format", "json", "--profile"])
            .arg(&source.profile)
            .arg("--adapter")
            .arg(&source.adapter)
            .arg("--target")
            .arg(&source.target)
            .output()
            .map_err(|e| e.to_string())
            .and_then(|output| {
                if !matches!(output.status.code(), Some(0 | 1)) {
                    return Err(format!(
                        "trace exited {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                let text = std::str::from_utf8(&output.stdout).map_err(|e| e.to_string())?;
                parse_trace(text, &source.name).map_err(|e| e.to_string())
            });
        match result {
            Ok(trace) => runs.push(Some(trace)),
            Err(message) => {
                failures.push(source_failure(&source.name, message));
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

//! The source trace wire format and its parser.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::graph::check_pathway;
use crate::types::{
    CivilDate, FuseFinding, FuseProvenance, Issue, Note, Origin, Provenance, Severity, Until,
};

use super::FuseError;
use super::manifest::nonempty;

#[derive(Debug, Deserialize)]
pub(super) struct TraceLocation {
    file: String,
    line: u32,
}
impl TraceLocation {
    pub(super) fn attributed(self, source: &str) -> FuseProvenance {
        FuseProvenance {
            source: source.into(),
            location: Provenance::new(self.file, self.line),
        }
    }
}
#[derive(Debug, Deserialize)]
pub(super) struct TraceFinding {
    code: String,
    severity: String,
    message: String,
    file: String,
    line: u32,
    node_id: Option<String>,
    state: Option<String>,
    /// Decided by the source's own profile in its trace run; carried through
    /// the merge as-is, never re-derived from the fuse profile.
    #[serde(default)]
    suppressed: bool,
    /// Attached by the source's own notes file; fuse attaches none of its own.
    #[serde(default)]
    note: Option<TraceNote>,
}
/// A note as a source's trace JSON writes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceNote {
    text: String,
    #[serde(default)]
    until: Option<TraceUntil>,
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    expired: bool,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TraceUntil {
    Date(String),
    Pathway { pathway: String, position: String },
}
impl TraceNote {
    /// Rebuild the note, refusing a date that is not on the calendar.
    fn to_note(&self) -> Result<Note, String> {
        let date = |value: &str| {
            CivilDate::parse(value)
                .ok_or_else(|| format!("note date '{value}' is not a YYYY-MM-DD calendar date"))
        };
        let mut note = Note::new(self.text.clone());
        note.until = match &self.until {
            None => None,
            Some(TraceUntil::Date(value)) => Some(Until::Date(date(value)?)),
            Some(TraceUntil::Pathway { pathway, position }) => Some(Until::Pathway {
                pathway: pathway.clone(),
                position: position.clone(),
            }),
        };
        note.by.clone_from(&self.by);
        note.at = self.at.as_deref().map(date).transpose()?;
        note.expired = self.expired;
        Ok(note)
    }
}
impl TraceFinding {
    pub(super) fn attributed(self, source: &str, node: Option<&str>) -> FuseFinding {
        let mut issue = Issue::new(
            Severity::parse(&self.severity).expect("validated severity"),
            self.code,
            self.message,
            Origin::from_wire(self.file, self.line),
            self.node_id
                .as_deref()
                .or(node)
                .map(|id| qualify(source, id)),
        );
        issue.state = self.state;
        issue.suppressed = self.suppressed;
        issue.note = self
            .note
            .as_ref()
            .map(|note| note.to_note().expect("validated note"));
        FuseFinding {
            issue,
            source: Some(source.into()),
            locations: Vec::new(),
        }
    }
}
#[derive(Debug, Deserialize)]
pub(super) struct TraceEdge {
    pub(super) tgt: String,
    pub(super) kind: String,
    #[serde(default)]
    pub(super) attrs: Map<String, Value>,
    pub(super) provenance: TraceLocation,
}
#[derive(Debug, Deserialize)]
pub(super) struct TraceNode {
    pub(super) id: String,
    pub(super) kind: String,
    #[serde(default)]
    pub(super) attrs: Map<String, Value>,
    pub(super) provenance: TraceLocation,
    #[serde(default)]
    pub(super) edges: Vec<TraceEdge>,
    #[serde(default)]
    pub(super) findings: Vec<TraceFinding>,
}
#[derive(Debug, Deserialize)]
pub(super) struct TracePathway {
    pub(super) name: String,
    pub(super) order: Vec<String>,
    pub(super) current: String,
}
/// One source's trace as [`assemble`](super::assemble) consumes it.
///
/// [`parse_trace`] is the only way to obtain one, so every trace composition
/// sees has passed its checks: an unchecked severity or note is refused there,
/// naming the source, and never reaches composition.
///
/// # Examples
///
/// ```
/// use latt_core::fuse::parse_trace;
///
/// let text = r#"{"header": {"trace_version": "2"}, "entries": [],
///   "unattachable_findings": [{"code": "X", "severity": "fatal", "message": "m",
///                              "file": "f", "line": 1}]}"#;
/// let error = parse_trace(text, "billing").unwrap_err().to_string();
/// assert!(error.contains("billing"), "{error}");
/// assert!(error.contains("invalid finding severity 'fatal'"), "{error}");
/// ```
#[derive(Debug)]
pub struct SourceTrace(pub(super) TraceWire);

/// The trace JSON as deserialized, before [`parse_trace`] has checked it.
#[derive(Debug, Deserialize)]
pub(super) struct TraceWire {
    header: BTreeMap<String, String>,
    pub(super) entries: Vec<TraceNode>,
    #[serde(default)]
    pub(super) unattachable_findings: Vec<TraceFinding>,
    #[serde(default)]
    pub(super) pathways: Vec<TracePathway>,
}

/// Check one source's `trace --format json` output before it is composed.
///
/// Accepts trace versions 1 and 2; a repeated entry, an unknown severity or an
/// invalid pathway is the source's fault and the error names the source.
///
/// # Errors
///
/// [`FuseError::Trace`], naming `source`, for text that is not trace JSON (a
/// `line` that is negative or above `u32::MAX` among it), an unsupported or
/// missing `trace_version`, an empty ID, kind or edge target, a repeated entry or
/// pathway, an unknown finding severity, or an invalid pathway.
pub fn parse_trace(text: &str, source: &str) -> Result<SourceTrace, FuseError> {
    let check = || -> Result<SourceTrace, String> {
        let trace: TraceWire = serde_json::from_str(text).map_err(|e| e.to_string())?;
        match trace.header.get("trace_version").map(String::as_str) {
            Some("1" | "2") => (),
            _ => return Err("unsupported or missing trace_version".into()),
        }
        let mut ids = BTreeSet::new();
        for node in &trace.entries {
            nonempty(&node.id, "node id")?;
            nonempty(&node.kind, "node kind")?;
            if !ids.insert(&node.id) {
                return Err(format!("repeated trace entry '{}'", node.id));
            }
            for edge in &node.edges {
                nonempty(&edge.tgt, "edge target")?;
                nonempty(&edge.kind, "edge kind")?;
            }
        }
        for finding in trace
            .entries
            .iter()
            .flat_map(|n| &n.findings)
            .chain(&trace.unattachable_findings)
        {
            if Severity::parse(&finding.severity).is_none() {
                return Err(format!("invalid finding severity '{}'", finding.severity));
            }
            if let Some(note) = &finding.note {
                note.to_note()?;
            }
        }
        let mut names = BTreeSet::new();
        for pathway in &trace.pathways {
            nonempty(&pathway.name, "pathway name")?;
            if !names.insert(pathway.name.as_str()) {
                return Err(format!("repeated pathway '{}'", pathway.name));
            }
            check_pathway(&pathway.name, &pathway.order, &pathway.current)
                .map_err(|e| e.to_string())?;
        }
        Ok(SourceTrace(trace))
    };
    check().map_err(|message| FuseError::Trace {
        name: source.into(),
        message,
    })
}

pub(super) fn qualify(source: &str, raw: &str) -> String {
    format!("{source}:{raw}")
}

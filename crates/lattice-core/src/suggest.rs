//! The suggestion overlay: a producer's ranked candidate edges become hints.
//!
//! Nothing here ranks anything. A producer — an embedding sidecar, later a panel
//! of them — writes a suggestion document, and the core renders it. Ranking, and
//! anything that would need a model or a network, stays outside.
//!
//! The document is scratch by contract: the core reads it and never writes one.
//! An accepted suggestion becomes durable only when a human edits the register.

use serde_json::Value;

use crate::document::{ContractError, entries, err, require, require_str};
use crate::graph::LatticeGraph;
use crate::types::{Issue, Provenance};
use crate::validate::{FindingCode, default_severity};

/// The suggestion-document version this core prefers.
pub const SUGGESTION_VERSION: &str = "1.0";

/// Every suggestion-document version this core can read. A document outside this
/// set is exit 2, not a finding: reading it on a guess would put candidates the
/// core has misunderstood in front of a reviewer as if they were ranked advice.
pub const SUPPORTED_SUGGESTION_VERSIONS: &[&str] = &[SUGGESTION_VERSION];

/// A rendered suggestion, before it becomes an `Issue`.
struct Suggestion {
    src: String,
    tgt: String,
    kind: String,
    score: f64,
    basis: String,
}

fn check_version(document: &Value) -> Result<(), ContractError> {
    let version = require_str(document, "suggestion_version", "suggestion document")?;
    if !SUPPORTED_SUGGESTION_VERSIONS.contains(&version) {
        let supported = SUPPORTED_SUGGESTION_VERSIONS.join(", ");
        return err(format!(
            "unsupported suggestion version '{version}'; this lattice supports {supported}"
        ));
    }
    Ok(())
}

fn parse_suggestion(entry: &Value, where_: &str) -> Result<Suggestion, ContractError> {
    let score = require(entry, "score", where_)?;
    let Some(score) = score.as_f64() else {
        return err(format!(
            "{where_}: 'score' must be a number, got {}",
            crate::document::type_name(score)
        ));
    };
    Ok(Suggestion {
        src: require_str(entry, "src", where_)?.to_string(),
        tgt: require_str(entry, "tgt", where_)?.to_string(),
        kind: require_str(entry, "kind", where_)?.to_string(),
        score,
        basis: require_str(entry, "basis", where_)?.to_string(),
    })
}

/// Read a suggestion document and render its entries as hint findings.
///
/// Entries are rendered in document order: a producer's tie-break may encode
/// something its scores do not, so re-sorting here would discard it. An entry
/// naming an ID the graph does not declare becomes `SUGGESTION_UNRESOLVED`
/// rather than vanishing — a document goes stale as soon as the register moves
/// under it, and silence would read as "this ranker had nothing to say".
pub fn render_suggestions(
    document: &Value,
    graph: &LatticeGraph,
) -> Result<Vec<Issue>, ContractError> {
    check_version(document)?;
    let producer = require_str(document, "producer", "suggestion document")?;

    let mut issues = Vec::new();
    for (index, entry) in entries(document, "suggestions")?.iter().enumerate() {
        let where_ = format!("suggestion {index}");
        let s = parse_suggestion(entry, &where_)?;

        let unresolved: Vec<&str> = [s.src.as_str(), s.tgt.as_str()]
            .into_iter()
            .filter(|id| graph.node(id).is_none())
            .collect();
        if !unresolved.is_empty() {
            let code = FindingCode::SuggestionUnresolved;
            issues.push(Issue::new(
                default_severity(code),
                code.as_str(),
                format!(
                    "suggestion from '{}' proposes '{}' {} -> {}: {} does not resolve",
                    producer,
                    s.kind,
                    s.src,
                    s.tgt,
                    unresolved.join(" and ")
                ),
                Provenance::new("<suggestions>", 0),
                None,
            ));
            continue;
        }

        // Provenance is the source node's, so acting on a suggestion sends the
        // reviewer to the file and line they would edit.
        let provenance = graph.node(&s.src).map_or_else(
            || Provenance::new("<suggestions>", 0),
            |n| n.provenance.clone(),
        );
        let code = FindingCode::SuggestedEdge;
        issues.push(Issue::new(
            default_severity(code),
            code.as_str(),
            format!(
                "'{}' {} '{}' (score {:.4}, {}): {}",
                s.src, s.kind, s.tgt, s.score, producer, s.basis
            ),
            provenance,
            Some(s.src),
        ));
    }
    Ok(issues)
}

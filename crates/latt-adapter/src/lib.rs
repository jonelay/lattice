//! The interface document Rust adapters emit, and the bounded subprocess runner
//! they share with the core.
//!
//! A [`Document`] serializes to interface version [`INTERFACE_VERSION`]; the
//! core's ingest is the contract it must satisfy. Unreadable input becomes a
//! finding in the document, never a dropped row:
//!
//! ```
//! use latt_adapter::{Document, Edge, Node, Provenance};
//!
//! let mut document = Document::new();
//! let attrs = [("status".to_owned(), "draft".into())].into_iter().collect();
//! let index = document.nodes.len();
//! document
//!     .nodes
//!     .push(Node::new("REQ-1", "requirement", Provenance::new("r.md", 3)).with_attrs(attrs));
//! document.edges.push(
//!     Edge::new("REQ-1", "REQ-0", "derives", Provenance::new("r.md", 3)).declared_by(index),
//! );
//! document.parse_error("row 4 has two cells, header has three", "r.md", 4);
//! document.node_parse_error("bad status", "r.md", 3, "REQ-1");
//!
//! let json = serde_json::to_value(&document)?;
//! assert_eq!(json["interface_version"], "1.4");
//! assert_eq!(json["nodes"][0]["kind"], "requirement");
//! assert_eq!(json["nodes"][0]["attrs"]["status"], "draft");
//! assert_eq!(json["nodes"][0]["provenance"]["line"], 3);
//! assert_eq!(json["edges"][0], serde_json::json!({
//!     "src": "REQ-1", "tgt": "REQ-0", "kind": "derives",
//!     "provenance": {"file": "r.md", "line": 3}, "declared_by": 0,
//! }));
//! assert_eq!(json["findings"][0]["code"], "PARSE_ERROR");
//! assert_eq!(json["findings"][0]["severity"], "error");
//! assert_eq!(json["findings"][0]["node_id"], serde_json::Value::Null);
//! assert_eq!(json["findings"][1]["node_id"], "REQ-1");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod codes;
pub mod fence;
mod files;
#[doc(hidden)]
pub mod process;
mod profile;
mod sorted_map;
mod sources;

#[doc(hidden)]
pub use files::{FileMatcher, absolute_path, glob_regex, is_file_or_dangling, selected_files};
#[doc(hidden)]
pub use profile::{DEFAULT_COMMAND_TIMEOUT, load_resolved_profile, resolved_profile_from_slice};
/// The compact map behind a node's attrs and attr locations; the core reads into it too.
#[doc(hidden)]
pub use sorted_map::SortedMap;
pub use sources::{check_sources, validate_id_names};

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::Serialize;
use serde_json::Value;

/// Decode a register file's bytes as UTF-8 text.
///
/// A leading byte order mark is encoding, not content: left in place it glues
/// itself to the first heading or table row, which then fails to parse in silence.
///
/// # Errors
///
/// The bytes are not valid UTF-8.
///
/// ```
/// assert_eq!(latt_adapter::decode_utf8(b"\xef\xbb\xbf# Title").unwrap(), "# Title");
/// assert!(latt_adapter::decode_utf8(b"\xff").is_err());
/// ```
pub fn decode_utf8(bytes: &[u8]) -> Result<&str, std::str::Utf8Error> {
    std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
}

/// A line holding bytes that are not UTF-8, as [`decode_lines`] found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidLine {
    /// One-based line number.
    pub line: u32,
    /// One-based character column of the first byte that is not UTF-8.
    pub column: u32,
    /// The line as written: its valid text kept, each other byte as `\xNN`.
    pub as_written: String,
}

/// Decode a register file's bytes line by line, keeping every line that is valid.
///
/// Return the text, with each invalid UTF-8 sequence replaced by one U+FFFD so every
/// line keeps its number and its structure, and the lines that held such sequences.
/// A sequence may be several bytes, so count bytes in `as_written`, which shows each
/// one, never in the replacement characters.
/// A caller reads none of those lines as content: the replacement character is
/// not what the register wrote, so the line is reported where it is, as written,
/// and withholds only itself. A leading byte order mark is dropped, as in
/// [`decode_utf8`].
///
/// ```
/// let (text, invalid) = latt_adapter::decode_lines(b"| A | ok |\n| B | caf\xE9 |\n");
/// assert_eq!(text, "| A | ok |\n| B | caf\u{fffd} |\n");
/// assert_eq!(invalid.len(), 1);
/// assert_eq!((invalid[0].line, invalid[0].column), (2, 10));
/// assert_eq!(invalid[0].as_written, r"| B | caf\xE9 |");
///
/// // A truncated three-byte sequence: two bytes, one replacement character.
/// let (text, invalid) = latt_adapter::decode_lines(b"| C | \xE2\x82 |");
/// assert_eq!(text, "| C | \u{fffd} |");
/// assert_eq!(invalid[0].as_written, r"| C | \xE2\x82 |");
/// ```
pub fn decode_lines(bytes: &[u8]) -> (String, Vec<InvalidLine>) {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut text = String::with_capacity(bytes.len());
    let mut invalid = Vec::new();
    for (number, line) in (1u32..).zip(bytes.split(|byte| *byte == b'\n')) {
        if number > 1 {
            text.push('\n');
        }
        if let Ok(valid) = std::str::from_utf8(line) {
            text.push_str(valid);
            continue;
        }
        text.push_str(&String::from_utf8_lossy(line));
        let mut as_written = String::new();
        let mut column = None;
        for chunk in line.utf8_chunks() {
            as_written.push_str(chunk.valid());
            if !chunk.invalid().is_empty() {
                column.get_or_insert(as_written.chars().count() as u32 + 1);
            }
            for byte in chunk.invalid() {
                as_written.push_str(&format!("\\x{byte:02X}"));
            }
        }
        invalid.push(InvalidLine {
            line: number,
            column: column.unwrap_or(1),
            as_written: as_written.trim_end_matches('\r').to_owned(),
        });
    }
    (text, invalid)
}

/// Current Lattice adapter interface version emitted by [`Document`].
pub const INTERFACE_VERSION: &str = "1.4";

/// Serializable interface document produced by an adapter.
///
/// Build one with [`Document::new`] and push to its lists; a field added later
/// starts empty, so this type can grow in a minor release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Document {
    /// Interface version understood by the core.
    pub interface_version: &'static str,
    /// Declared nodes in source order.
    pub nodes: Vec<Node>,
    /// Declared edges in source order.
    pub edges: Vec<Edge>,
    /// Ordering pathways read from the source.
    pub pathways: Vec<Pathway>,
    /// Non-fatal source issues reported by the adapter.
    pub findings: Vec<Issue>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            interface_version: INTERFACE_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
            pathways: Vec::new(),
            findings: Vec::new(),
        }
    }
}

/// One node declaration in an adapter document.
///
/// The attrs and their locations are private: set them with
/// [`with_attrs`](Self::with_attrs) and
/// [`with_attr_provenance`](Self::with_attr_provenance), and read them with
/// [`attrs`](Self::attrs) and [`attr_provenance`](Self::attr_provenance). Each is
/// one compact map per node, so the representation can change without a break.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Node {
    /// Stable node identifier.
    pub id: String,
    /// Profile-declared node kind.
    pub kind: String,
    attrs: SortedMap<String, Value>,
    /// Source location of the declaration.
    pub provenance: Provenance,
    #[serde(skip_serializing_if = "SortedMap::is_empty")]
    attr_provenance: SortedMap<String, Provenance>,
}

impl Node {
    /// Construct a node with no attributes.
    #[must_use]
    pub fn new(id: impl Into<String>, kind: impl Into<String>, provenance: Provenance) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            attrs: SortedMap::new(),
            provenance,
            attr_provenance: SortedMap::new(),
        }
    }

    /// Record where one of the node's attrs was written, when that is not the
    /// node's own line (interface 1.4). The core reports a finding about that attr
    /// there. A second location for the same attr replaces the first.
    ///
    /// The builder does not check `attr` against the node's attrs. Core
    /// ingestion refuses a document that records a location for an attr the
    /// node does not hold, so record locations only for attrs in the final set.
    ///
    /// ```
    /// use latt_adapter::{Node, Provenance};
    ///
    /// let attrs = [("scope".to_owned(), 3.into())].into_iter().collect();
    /// let node = Node::new("language/python", "language", Provenance::new("languages.toml", 1171))
    ///     .with_attrs(attrs)
    ///     .with_attr_provenance("scope", Provenance::new("languages.toml", 1173));
    /// assert_eq!(node.attr_location("scope").and_then(|at| at.line()).map(u32::from), Some(1173));
    /// let json = serde_json::to_value(&node)?;
    /// assert_eq!(json["attr_provenance"]["scope"]["line"], 1173);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn with_attr_provenance(mut self, attr: impl Into<String>, provenance: Provenance) -> Self {
        self.attr_provenance.insert(attr.into(), provenance);
        self
    }

    /// Replace the node's profile-declared and adapter-specific attributes.
    ///
    /// Locations recorded by [`with_attr_provenance`](Self::with_attr_provenance)
    /// are kept, so each must still name an attr in the new set.
    ///
    /// ```
    /// use latt_adapter::{Node, Provenance};
    ///
    /// let attrs = [("title".to_owned(), "t".into()), ("status".to_owned(), "draft".into())];
    /// let node = Node::new("REQ-1", "requirement", Provenance::new("r.md", 3))
    ///     .with_attrs(attrs.into_iter().collect());
    /// assert_eq!(node.attr("status"), Some(&"draft".into()));
    /// let names: Vec<&str> = node.attrs().map(|(name, _)| name).collect();
    /// assert_eq!(names, ["status", "title"]);
    /// ```
    #[must_use]
    pub fn with_attrs(mut self, attrs: BTreeMap<String, Value>) -> Self {
        self.attrs = attrs.into();
        self
    }

    /// Return the value of attr `name`.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&Value> {
        self.attrs.get(name)
    }

    /// Iterate the node's attributes in name order, the order the document writes.
    pub fn attrs(&self) -> impl DoubleEndedIterator<Item = (&str, &Value)> + ExactSizeIterator {
        self.attrs
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }

    /// Return where attr `name` was written, when
    /// [`with_attr_provenance`](Self::with_attr_provenance) recorded it.
    #[must_use]
    pub fn attr_location(&self, name: &str) -> Option<&Provenance> {
        self.attr_provenance.get(name)
    }

    /// Iterate the recorded attr locations in attr-name order, the order the
    /// document writes.
    pub fn attr_provenance(
        &self,
    ) -> impl DoubleEndedIterator<Item = (&str, &Provenance)> + ExactSizeIterator {
        self.attr_provenance
            .iter()
            .map(|(name, provenance)| (name.as_str(), provenance))
    }
}

/// One directed edge declaration in an adapter document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Edge {
    /// Source node identifier.
    pub src: String,
    /// Target node identifier.
    pub tgt: String,
    /// Profile-declared edge kind.
    pub kind: String,
    /// Profile-declared and adapter-specific edge attributes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, Value>,
    /// Source location of the declaration.
    pub provenance: Provenance,
    /// Index in [`Document::nodes`] of the node entry whose row declared this
    /// edge, which need not be the entry for `src`.
    ///
    /// When that entry repeats an earlier ID the core does not read it, and with
    /// this set it does not read the edge either. `None` attaches the edge to
    /// whichever node holds `src`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_by: Option<usize>,
}

impl Edge {
    /// Construct an edge with no attributes.
    #[must_use]
    pub fn new(
        src: impl Into<String>,
        tgt: impl Into<String>,
        kind: impl Into<String>,
        provenance: Provenance,
    ) -> Self {
        Self {
            src: src.into(),
            tgt: tgt.into(),
            kind: kind.into(),
            attrs: BTreeMap::new(),
            provenance,
            declared_by: None,
        }
    }

    /// Replace the edge's attributes.
    #[must_use]
    pub fn with_attrs(mut self, attrs: BTreeMap<String, Value>) -> Self {
        self.attrs = attrs;
        self
    }

    /// Name the node entry, by its index in [`Document::nodes`], whose row
    /// declared this edge. Take the index as `document.nodes.len()` before
    /// pushing that node.
    #[must_use]
    pub fn declared_by(mut self, index: usize) -> Self {
        self.declared_by = Some(index);
        self
    }
}

/// One named ordering pathway and its current position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Pathway {
    /// Profile-declared pathway name.
    pub name: String,
    /// Valid positions in declared order.
    pub order: Vec<String>,
    /// Current position on the pathway.
    pub current: String,
}

impl Pathway {
    /// Construct a pathway at `current` among `order`. Whether `current` is one
    /// of the positions is the core's check, not this constructor's.
    #[must_use]
    pub fn new(name: impl Into<String>, order: Vec<String>, current: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            order,
            current: current.into(),
        }
    }
}

/// How much a finding matters, as the interface spells it.
///
/// The four spellings are the interface version's, so a fifth would be a new
/// [`INTERFACE_VERSION`], and a `match` on this enum needs no wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Gates the run: `error`.
    Error,
    /// Gates only under `--strict`: `warning`.
    Warning,
    /// Reported, never gates: `info`.
    Info,
    /// A suggestion, never gates: `hint`.
    Hint,
}

/// A source issue detected while adapting a register.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Issue {
    /// How much the issue matters.
    pub severity: Severity,
    /// Stable machine-readable issue code.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Source location associated with the issue.
    pub provenance: Provenance,
    /// Related node identifier, when the issue belongs to a node.
    pub node_id: Option<String>,
}

impl Issue {
    /// Construct an issue that belongs to no node.
    #[must_use]
    pub fn new(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        provenance: Provenance,
    ) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            provenance,
            node_id: None,
        }
    }

    /// Attribute the issue to one node.
    #[must_use]
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }
}

/// File and one-based line attribution in an adapter's source register.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Provenance {
    file: String,
    // Stored as the wire writes it, zero for "no exact line", so the derived
    // serialization stays the interface document's; `line()` is the only accessor.
    line: u32,
}

impl Provenance {
    /// Construct provenance from a path and a one-based line, zero when no exact
    /// line is available.
    #[must_use]
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }

    /// Return the source path as reported by the adapter.
    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Return the one-based source line, or `None` when no exact line is available.
    ///
    /// ```
    /// use latt_adapter::Provenance;
    ///
    /// assert_eq!(Provenance::new_file("r.md").line(), None);
    /// assert_eq!(Provenance::new("r.md", 3).line().map(|line| line.get()), Some(3));
    /// ```
    #[must_use]
    pub fn line(&self) -> Option<NonZeroU32> {
        NonZeroU32::new(self.line)
    }

    /// Construct file-only provenance with line zero.
    #[must_use]
    pub fn new_file(file: impl Into<String>) -> Self {
        Self::new(file, 0)
    }
}

impl Document {
    /// Construct an empty document at [`INTERFACE_VERSION`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a document-level parse error.
    pub fn parse_error(&mut self, message: impl Into<String>, file: impl Into<String>, line: u32) {
        self.findings.push(Issue::new(
            Severity::Error,
            codes::PARSE_ERROR,
            message,
            Provenance::new(file, line),
        ));
    }

    /// Add a parse error attributed to one node.
    pub fn node_parse_error(
        &mut self,
        message: impl Into<String>,
        file: impl Into<String>,
        line: u32,
        node_id: impl Into<String>,
    ) {
        self.findings.push(
            Issue::new(
                Severity::Error,
                codes::PARSE_ERROR,
                message,
                Provenance::new(file, line),
            )
            .with_node_id(node_id),
        );
    }

    /// Add a warning for an invalid ordering pathway.
    pub fn pathway_invalid(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.findings.push(Issue::new(
            Severity::Warning,
            codes::PATHWAY_INVALID,
            message,
            Provenance::new_file(file),
        ));
    }

    /// Add an informational unresolved external-reference finding.
    pub fn external_ref(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.findings.push(Issue::new(
            Severity::Info,
            codes::EXTERNAL_REF,
            message,
            Provenance::new_file(file),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::Severity;

    #[test]
    fn severity_serializes_to_the_wire_spelling() {
        let spelled: Vec<_> = [
            Severity::Error,
            Severity::Warning,
            Severity::Info,
            Severity::Hint,
        ]
        .iter()
        .map(|severity| serde_json::to_value(severity).unwrap())
        .collect();
        assert_eq!(spelled, ["error", "warning", "info", "hint"]);
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::Provenance;

    #[test]
    fn line_zero_reads_as_no_line_and_writes_as_zero() {
        let whole_file = Provenance::new_file("r.md");
        assert_eq!(whole_file.file(), "r.md");
        assert_eq!(whole_file.line(), None);
        assert_eq!(
            serde_json::to_string(&whole_file).unwrap(),
            r#"{"file":"r.md","line":0}"#
        );
        assert_eq!(
            Provenance::new("r.md", 3).line().map(|line| line.get()),
            Some(3)
        );
    }
}

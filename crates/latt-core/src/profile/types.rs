//! The loaded profile's types: kinds, attrs, conditions and the validation plan.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use serde_norway::Value;

use crate::types::{Provenance, Severity};

/// The type an attr declares, parsed once at load so every later check matches
/// on a variant instead of re-comparing the profile's spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttrType {
    /// A JSON boolean.
    Bool,
    /// A string holding a calendar date, `YYYY-MM-DD`.
    Date,
    /// A string from the schema's `values`.
    Enum,
    /// A JSON integer.
    Int,
    /// A JSON array; with `items` set, each element has that type.
    List,
    /// Any JSON string.
    String,
}

impl AttrType {
    /// Parse the profile's spelling, or `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "bool" => Some(Self::Bool),
            "date" => Some(Self::Date),
            "enum" => Some(Self::Enum),
            "int" => Some(Self::Int),
            "list" => Some(Self::List),
            "string" => Some(Self::String),
            _ => None,
        }
    }

    /// The profile's spelling, which is also what findings quote.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Date => "date",
            Self::Enum => "enum",
            Self::Int => "int",
            Self::List => "list",
            Self::String => "string",
        }
    }

    /// True when a `list` attr may declare this as its element type.
    #[must_use]
    pub fn is_list_item(self) -> bool {
        !matches!(self, Self::Enum | Self::List)
    }
}

impl std::fmt::Display for AttrType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One attr's declared schema on a node kind.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttrSchema {
    /// The attr's value type. Not a node or edge kind.
    pub kind: AttrType,
    /// A node of the kind without this attr is a finding.
    pub required: bool,
    /// The permitted values; set exactly when `kind` is [`AttrType::Enum`].
    pub values: Option<Vec<String>>,
    /// The element type; set, if at all, only when `kind` is [`AttrType::List`].
    pub items: Option<AttrType>,
}

/// A declared node kind: its ID shape, attr schemas, and presentation hints.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NodeKind {
    /// The compiled pattern, anchored to the whole ID. Python matches with
    /// `fullmatch`; Rust's `is_match` is a search.
    pub(super) id_pattern: Regex,
    /// The pattern as the profile wrote it. `ID_FORMAT` quotes this, so the
    /// anchoring above must not leak into a finding's text.
    pub id_pattern_source: String,
    /// Declared attrs by name. An attr the register carries and this map does
    /// not name is not checked.
    pub attrs: BTreeMap<String, AttrSchema>,
    /// The attr to show in a trace row's summary column. `None` means the kind
    /// has no summary and the column is blank.
    pub summary_attr: Option<String>,
    /// The attrs a text-ranking consumer reads, in declared order. `None` and
    /// `Some([])` are different answers: absent leaves the consumer to fall back
    /// to `summary_attr`, while an empty list declares the kind offers no text.
    pub text_attrs: Option<Vec<String>>,
    /// A literal line prefix at which a ranking consumer subdivides this kind's
    /// text. A literal rather than a regex because the core validates it and a
    /// separate program applies it: Rust's engine is linear-time and Python's
    /// backtracks, so a pattern this crate accepts is not one the consumer can
    /// safely run.
    pub text_chunk_line_prefix: Option<String>,
    /// Position in the profile's `node_kinds` mapping, which is the order trace
    /// entries group by. Carried on the kind because `node_kinds` is keyed for
    /// lookup and no longer remembers how it was written.
    pub declared_index: usize,
    /// Exempts this kind's nodes from `UNREFERENCED`/`UNTRACED`. Validation
    /// policy only; `query orphans` still reports them.
    pub orphan_ok: bool,
    /// Owner-tool actions by verb, in declared order. The core gives no verb
    /// a meaning; browse binds keys to some names by convention.
    #[doc(hidden)]
    pub actions: Vec<(String, ActionDecl)>,
}

impl NodeKind {
    /// True when `id` matches the whole of the declared `id_pattern`.
    #[must_use]
    pub fn id_matches(&self, id: &str) -> bool {
        self.id_pattern.is_match(id)
    }
}

/// How browse runs an owner action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub enum ActionMode {
    /// Hand the terminal to the program, then resume and reload the register.
    Interactive,
    /// Run to completion with stdin closed and show its output.
    Capture,
    /// Start the program without waiting for it.
    Detached,
}

pub use crate::template::{Segment, Template};

/// An owner-tool action a node kind declares: an argv template and a mode.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct ActionDecl {
    /// The argv, one template per argument; the first names the program and
    /// holds no placeholder.
    pub run: Vec<Template>,
    /// How browse runs it.
    pub mode: ActionMode,
}

/// A declared edge kind: the node-kind pairs it may connect.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EdgeKind {
    /// The `(source kind, target kind)` pairs an edge of this kind may join;
    /// any other pair is a finding.
    pub allowed: Vec<(String, String)>,
    /// Targets may live in another fuse source, so an unresolved target is a
    /// hint-severity `VACANCY` rather than the code's resolved severity.
    pub cross_source: bool,
}

impl EdgeKind {
    /// True when `(src_kind, tgt_kind)` is one of the `allowed` pairs.
    #[must_use]
    pub fn admits(&self, src_kind: &str, tgt_kind: &str) -> bool {
        self.allowed
            .iter()
            .any(|(s, t)| s == src_kind && t == tgt_kind)
    }
}

/// Ties a finding code's severity to a node attr's position on a pathway.
///
/// Carries no demotion target: a demoted finding becomes `info`, always.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathwayBinding {
    /// The pathway, by the name the register declares it under.
    pub pathway: String,
    /// The node attr holding the node's position on that pathway.
    pub position_attr: String,
}

/// A value that ordering operators (`lt`, `gt`, `lte`, `gte`) can compare.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Comparable {
    /// Signed integer comparison operand.
    Int(i128),
    /// String comparison operand, including date strings.
    Str(String),
}

impl std::fmt::Display for Comparable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{n}"),
            Self::Str(s) => write!(f, "{}", serde_json::Value::String(s.clone())),
        }
    }
}

/// A single condition operator parsed from a CONSTRAINT entry.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[doc(hidden)]
pub enum ConditionOp {
    /// Attribute equals the supplied JSON value.
    Eq(serde_json::Value),
    /// Attribute is absent or does not equal the supplied JSON value.
    Ne(serde_json::Value),
    /// Attribute equals one of the supplied JSON values.
    In(Vec<serde_json::Value>),
    /// Attribute orders before the supplied value.
    Lt(Comparable),
    /// Attribute orders after the supplied value.
    Gt(Comparable),
    /// Attribute orders before or equals the supplied value.
    Lte(Comparable),
    /// Attribute orders after or equals the supplied value.
    Gte(Comparable),
    /// String attribute matches the compiled regular expression.
    Matches(Regex),
    /// Attribute presence equals the supplied boolean.
    Present(bool),
}

/// One attr-name → operator pair: a profile's when/expect/reject entry, or a
/// query filter.
///
/// Opaque, so the operator set and the pattern engine behind `matches` can change
/// without a break: build one with a constructor, or with [`Condition::parse`] for
/// the command line's filter grammar.
///
/// # Examples
///
/// ```
/// use latt_core::profile::{Comparable, Condition};
///
/// let open = Condition::eq("status", serde_json::json!("open"));
/// assert_eq!(open.attr(), "status");
/// let ranked = Condition::gte("rank", Comparable::Int(3));
/// let parsed = Condition::parse("rank>=3")?;
/// assert_eq!(parsed.attr(), ranked.attr());
/// assert!(Condition::matches("id", "[unclosed").is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct Condition {
    pub(crate) attr: String,
    pub(crate) op: ConditionOp,
}

/// The filter operators, longest first, so the first match at a position is the
/// longest there.
pub(crate) const FILTER_OPERATORS: [&str; 7] = [">=", "<=", "!=", "~=", ">", "<", "="];

impl Condition {
    pub(crate) fn new(attr: impl Into<String>, op: ConditionOp) -> Self {
        Self {
            attr: attr.into(),
            op,
        }
    }

    /// Hold when the attr equals `value`.
    #[must_use]
    pub fn eq(attr: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(attr, ConditionOp::Eq(value))
    }

    /// Hold when the attr is absent or does not equal `value`.
    #[must_use]
    pub fn ne(attr: impl Into<String>, value: serde_json::Value) -> Self {
        Self::new(attr, ConditionOp::Ne(value))
    }

    /// Hold when the attr equals one of `values`.
    #[must_use]
    pub fn one_of(attr: impl Into<String>, values: Vec<serde_json::Value>) -> Self {
        Self::new(attr, ConditionOp::In(values))
    }

    /// Hold when the attr orders before `value`.
    #[must_use]
    pub fn lt(attr: impl Into<String>, value: Comparable) -> Self {
        Self::new(attr, ConditionOp::Lt(value))
    }

    /// Hold when the attr orders after `value`.
    #[must_use]
    pub fn gt(attr: impl Into<String>, value: Comparable) -> Self {
        Self::new(attr, ConditionOp::Gt(value))
    }

    /// Hold when the attr orders before or equals `value`.
    #[must_use]
    pub fn lte(attr: impl Into<String>, value: Comparable) -> Self {
        Self::new(attr, ConditionOp::Lte(value))
    }

    /// Hold when the attr orders after or equals `value`.
    #[must_use]
    pub fn gte(attr: impl Into<String>, value: Comparable) -> Self {
        Self::new(attr, ConditionOp::Gte(value))
    }

    /// Hold when the attr is a string the regular expression `pattern` matches.
    ///
    /// # Errors
    ///
    /// [`FilterError`] naming the pattern and the attr when `pattern` is not a
    /// valid regular expression.
    pub fn matches(attr: impl Into<String>, pattern: &str) -> Result<Self, FilterError> {
        let attr = attr.into();
        let regex = Regex::new(pattern).map_err(|error| {
            FilterError::new(format!("invalid pattern '{pattern}' for '{attr}': {error}"))
        })?;
        Ok(Self::new(attr, ConditionOp::Matches(regex)))
    }

    /// Hold when the attr's presence equals `present`.
    #[must_use]
    pub fn present(attr: impl Into<String>, present: bool) -> Self {
        Self::new(attr, ConditionOp::Present(present))
    }

    /// Parse one command-line attribute filter, `<attr><operator><value>`.
    ///
    /// Operators: `=` and `!=` compare for equality, reading the value as an
    /// integer, then a bool, then a string; `<`, `>`, `<=` and `>=` compare
    /// integers numerically and anything else as a string; `~=` matches the value
    /// as a regex. The filter splits where the earliest operator begins, taking the
    /// longest one that begins there, so an attr name never contains an operator
    /// and a value may: `status=a>b` is `status` equal to `a>b`, and `rank>=3` is
    /// `rank` at or above `3`, not `rank` above `=3`.
    ///
    /// # Errors
    ///
    /// [`FilterError`] naming the filter when it has no operator, an empty attr
    /// name, or a `~=` pattern that is not a valid regex.
    ///
    /// # Examples
    ///
    /// ```
    /// use latt_core::profile::Condition;
    ///
    /// assert_eq!(Condition::parse("status=a>b")?.attr(), "status");
    /// let error = Condition::parse("no-operator").unwrap_err();
    /// assert_eq!(error.message(), "invalid filter 'no-operator': expected an attribute operator");
    /// let error = Condition::parse("=x").unwrap_err();
    /// assert_eq!(error.message(), "invalid filter '=x': empty attribute name");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(value: &str) -> Result<Self, FilterError> {
        let (attr, operator, raw) = value
            .char_indices()
            .find_map(|(at, _)| {
                let rest = &value[at..];
                FILTER_OPERATORS
                    .into_iter()
                    .find(|operator| rest.starts_with(operator))
                    .map(|operator| (&value[..at], operator, &rest[operator.len()..]))
            })
            .ok_or_else(|| {
                FilterError::new(format!(
                    "invalid filter '{value}': expected an attribute operator"
                ))
            })?;
        if attr.is_empty() {
            return Err(FilterError::new(format!(
                "invalid filter '{value}': empty attribute name"
            )));
        }

        let integer = || {
            raw.parse::<i64>()
                .ok()
                .map(serde_json::Value::from)
                .or_else(|| raw.parse::<u64>().ok().map(serde_json::Value::from))
        };
        let comparable = || {
            raw.parse::<i64>()
                .ok()
                .map(|n| Comparable::Int(i128::from(n)))
                .or_else(|| {
                    raw.parse::<u64>()
                        .ok()
                        .map(|n| Comparable::Int(i128::from(n)))
                })
                .unwrap_or_else(|| Comparable::Str(raw.to_string()))
        };
        let equality_value = || {
            integer()
                .or_else(|| raw.parse::<bool>().ok().map(serde_json::Value::from))
                .unwrap_or_else(|| serde_json::Value::String(raw.to_string()))
        };
        let op =
            match operator {
                "=" => ConditionOp::Eq(equality_value()),
                "!=" => ConditionOp::Ne(equality_value()),
                "~=" => ConditionOp::Matches(Regex::new(raw).map_err(|error| {
                    FilterError::new(format!("invalid filter '{value}': {error}"))
                })?),
                "<" => ConditionOp::Lt(comparable()),
                ">" => ConditionOp::Gt(comparable()),
                "<=" => ConditionOp::Lte(comparable()),
                ">=" => ConditionOp::Gte(comparable()),
                _ => unreachable!(),
            };
        Ok(Self::new(attr, op))
    }

    /// Return the name of the attr the condition inspects.
    #[must_use]
    pub fn attr(&self) -> &str {
        &self.attr
    }

    /// Return the operator. Unstable: the operator set is not a supported shape.
    #[doc(hidden)]
    #[must_use]
    pub fn op(&self) -> &ConditionOp {
        &self.op
    }
}

/// A filter or pattern that cannot become a [`Condition`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilterError {
    message: String,
}

impl FilterError {
    fn new(message: String) -> Self {
        Self { message }
    }

    /// Return the explanation, the same text `Display` writes.
    ///
    /// It is for people and may change editorially; do not parse it.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FilterError {}

/// A parsed CONSTRAINT validation entry.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[doc(hidden)]
pub struct ConstraintConfig {
    /// The node kind the constraint applies to.
    pub kind: String,
    /// Guard: the constraint is skipped for a node unless every condition holds.
    pub when: Vec<Condition>,
    /// Every condition must hold.
    pub expect: Vec<Condition>,
    /// No condition may hold.
    pub reject: Vec<Condition>,
    /// Replaces the generated finding message.
    pub message: Option<String>,
    /// Severity for this entry's findings, ahead of the `CONSTRAINT` code's own.
    pub severity: Option<Severity>,
}

/// The conditions in one `EDGE_WHERE` block, split by the end of the edge each one reads.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
#[doc(hidden)]
pub struct EdgeConditions {
    /// Conditions on the source node's attrs.
    pub source: Vec<Condition>,
    /// Conditions on the target node's attrs.
    pub target: Vec<Condition>,
    /// Conditions on the edge's own attrs.
    pub edge: Vec<Condition>,
}

impl EdgeConditions {
    /// True when no end carries a condition.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.source.is_empty() && self.target.is_empty() && self.edge.is_empty()
    }
}

/// A parsed `EDGE_WHERE` validation entry.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[doc(hidden)]
pub struct EdgeWhereConfig {
    /// The edge kind whose edges the entry examines.
    pub edge_kind: String,
    /// Guard: the entry is skipped for an edge unless every condition holds.
    pub when: EdgeConditions,
    /// Every condition, on whichever end it names, must hold.
    pub expect: EdgeConditions,
    /// No condition, on whichever end it names, may hold.
    pub reject: EdgeConditions,
    /// Replaces the generated finding message.
    pub message: Option<String>,
    /// Severity for this entry's findings, ahead of the `EDGE_WHERE` code's own.
    pub severity: Option<Severity>,
}

/// Which end of its edges an `EDGE_COUNT` entry's counted node sits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum CountEnd {
    /// The counted node is the source; its outgoing edges are counted.
    Out,
    /// The counted node is the target; its incoming edges are counted.
    In,
}

/// A parsed `EDGE_COUNT` validation entry.
///
/// The loader guarantees at least one of `min` and `max` is set, and that
/// `min <= max` when both are.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[doc(hidden)]
pub struct EdgeCountConfig {
    /// The edge kind whose edges are counted.
    pub edge_kind: String,
    /// The node kind whose nodes are counted, including those with no edges.
    pub node_kind: String,
    /// Which end of each edge the counted node sits at; `Out` when the entry
    /// declares none. Named for the YAML key `direction`; the type is
    /// `CountEnd` so it is not mistaken for the traversal `query::Direction`.
    pub direction: CountEnd,
    /// A far-end attr whose distinct values each get their own count. A far
    /// end without the attr, or with it null, is its own finding and joins no
    /// group.
    pub group_by: Option<String>,
    /// The smallest count that passes.
    pub min: Option<u64>,
    /// The largest count that passes.
    pub max: Option<u64>,
    /// Limits the counted nodes to those every condition holds for.
    pub where_: Vec<Condition>,
    /// Replaces the generated finding message.
    pub message: Option<String>,
    /// Severity for this entry's findings, ahead of the `EDGE_COUNT` code's own.
    pub severity: Option<Severity>,
}

/// A parsed SUPPRESS entry, merged across every entry naming its code.
///
/// `node_ids: None` suppresses every finding of the code, and supersedes any
/// ID-specific entry for the same code.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct SuppressConfig {
    /// The finding code suppressed.
    pub code: String,
    /// The nodes whose findings of that code are suppressed; `None` for all.
    pub node_ids: Option<Vec<String>>,
}

/// A compiled `COVERAGE` entry: its kinds are declared, its blocks parsed.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct CoverageSpec {
    pub target_kind: String,
    pub edge_kind: String,
    pub where_: Vec<Condition>,
    pub source_where: Vec<Condition>,
}

/// A compiled `COVERAGE_DEEP` entry.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct CoverageDeepSpec {
    pub target_kind: String,
    pub via: String,
    pub evidence: String,
    pub where_: Vec<Condition>,
    pub source_where: Vec<Condition>,
}

/// A compiled `SUMMARY` entry.
///
/// Its kind and attrs are names the register may or may not carry; `summary`
/// reads whatever is there.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct SummarySpec {
    pub node_kind: String,
    pub status_attr: String,
    pub group_by_attr: String,
}

/// What one `validations:` entry configures, compiled once at load.
///
/// The three per-kind configs index into the plan's vectors so the supported
/// slice accessors keep their signatures.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub enum ValidationSpec {
    Coverage(CoverageSpec),
    CoverageDeep(CoverageDeepSpec),
    Summary(SummarySpec),
    /// Index into [`ValidationPlan::constraints`].
    Constraint(usize),
    /// Index into [`ValidationPlan::edge_wheres`].
    EdgeWhere(usize),
    /// Index into [`ValidationPlan::edge_counts`].
    EdgeCount(usize),
    /// The entry as declared; [`ValidationPlan::suppressions`] holds the merge.
    Suppress(SuppressConfig),
    /// A code configured by `severity` alone, the core's or an adapter's.
    SeverityOnly,
    /// An entry the loader compiled with faults and did not refuse: a coverage
    /// entry on the profile `fuse` composes, or a `SUMMARY` entry anywhere.
    /// The command that would run it reports them instead — `validate` as one
    /// `CONFIG_ERROR` per fault, `summary` as its error — so a declared check
    /// never fails in silence.
    Faulty(Vec<String>),
}

/// One `validations:` entry, in declaration order.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct PlanEntry {
    pub code: String,
    pub spec: ValidationSpec,
    /// The entry's own `severity`, which outranks the code's for its findings.
    pub severity: Option<Severity>,
    pub pathway: Option<PathwayBinding>,
    /// The entry's declared `message`, rendered onto each finding it produces
    /// (validation spec, declared messages render placeholders).
    pub message: Option<Template>,
}

/// Every check a profile declares, compiled once at load.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct ValidationPlan {
    pub(super) entries: Vec<PlanEntry>,
    pub(super) constraints: Vec<ConstraintConfig>,
    pub(super) edge_wheres: Vec<EdgeWhereConfig>,
    pub(super) edge_counts: Vec<EdgeCountConfig>,
    pub(super) suppressions: Vec<SuppressConfig>,
    /// Per-code severity, last write wins.
    pub(super) overrides: BTreeMap<String, Severity>,
    pub(super) pathway_bindings: BTreeMap<String, PathwayBinding>,
}

impl ValidationPlan {
    /// The entries in declaration order.
    pub fn entries(&self) -> &[PlanEntry] {
        &self.entries
    }

    /// The `CONSTRAINT` entries, in declaration order.
    #[doc(hidden)]
    pub fn constraints(&self) -> &[ConstraintConfig] {
        &self.constraints
    }

    /// The `EDGE_WHERE` entries, in declaration order.
    #[doc(hidden)]
    pub fn edge_wheres(&self) -> &[EdgeWhereConfig] {
        &self.edge_wheres
    }

    /// The `EDGE_COUNT` entries, in declaration order.
    #[doc(hidden)]
    pub fn edge_counts(&self) -> &[EdgeCountConfig] {
        &self.edge_counts
    }

    /// One entry per suppressed code, in first-declared order.
    #[doc(hidden)]
    pub fn suppressions(&self) -> &[SuppressConfig] {
        &self.suppressions
    }

    /// Per-code severity overrides.
    pub fn overrides(&self) -> &BTreeMap<String, Severity> {
        &self.overrides
    }

    /// Finding-code bindings to pathway position attrs.
    pub fn pathway_bindings(&self) -> &BTreeMap<String, PathwayBinding> {
        &self.pathway_bindings
    }
}

/// The declared vocabulary of a register: node kinds, edge kinds, checks.
#[derive(Clone, Debug)]
pub struct Profile {
    pub(super) name: String,
    pub(super) profile_version: String,
    pub(super) node_kinds: BTreeMap<String, NodeKind>,
    pub(super) edge_kinds: BTreeMap<String, EdgeKind>,
    pub(super) plan: ValidationPlan,
    pub(super) pathways: Vec<String>,
    /// The profile after inheritance resolution (if `extends:` was declared) and
    /// parsing, kept so `resolved_document` can re-emit sections the core does
    /// not model (the `adapter:` namespace above all).
    pub(super) raw: Value,
    /// The notes file `annotations` names, read at load. Kept out of `raw`, so
    /// the resolved document never carries a machine-specific path.
    pub(super) notes: Option<NotesFile>,
}

/// A notes file as loaded: its resolved path and its raw list items, or `None`
/// for the items when the file does not exist. Items stay raw because a
/// malformed entry is a `NOTE_INVALID` finding, not a load error.
#[derive(Clone, Debug)]
pub(crate) struct NotesFile {
    pub(crate) path: PathBuf,
    pub(crate) items: Option<Vec<NoteItem>>,
}

/// One top-level list item of a notes file, with the line its `-` is on
/// (`None` when the list is not in block style).
#[derive(Clone, Debug)]
pub(crate) struct NoteItem {
    pub(crate) line: Option<NonZeroU32>,
    pub(crate) value: Value,
}

impl Profile {
    /// Return the profile's declared name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the declared profile schema version.
    pub fn profile_version(&self) -> &str {
        &self.profile_version
    }

    /// Refuse the profile unless its `profile_version` is exactly `exact`.
    ///
    /// The opt-in pin a caller states on top of the loader's major check.
    /// `exact` is held to the key's own X.Y.Z shape first; under that shape
    /// equal text is equal version, so the comparison needs no version
    /// arithmetic.
    ///
    /// # Errors
    ///
    /// [`ProfileError::Invalid`](super::ProfileError::Invalid) when `exact` is
    /// not X.Y.Z without leading zeros, or names a version other than the
    /// profile's.
    ///
    /// # Examples
    ///
    /// ```
    /// use latt_core::profile::load_profile_value;
    ///
    /// let yaml = "name: p\nprofile_version: '1.2.0'\nnode_kinds: {}\nedge_kinds: {}\n";
    /// let profile = load_profile_value(latt_core::serde_norway::from_str(yaml)?, "example")?;
    /// profile.require_version("1.2.0")?;
    /// assert!(profile.require_version("1.3.0").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn require_version(&self, exact: &str) -> Result<(), super::ProfileError> {
        super::check_xyz_version("required profile_version", exact, u64::MAX)
            .map_err(super::ProfileError::Invalid)?;
        if exact == self.profile_version {
            return Ok(());
        }
        Err(super::ProfileError::Invalid(format!(
            "profile {}: profile_version is '{}', but '{exact}' is required",
            self.name, self.profile_version
        )))
    }

    /// Return node-kind declarations keyed by name.
    pub fn node_kinds(&self) -> &BTreeMap<String, NodeKind> {
        &self.node_kinds
    }

    /// Return edge-kind declarations keyed by name.
    pub fn edge_kinds(&self) -> &BTreeMap<String, EdgeKind> {
        &self.edge_kinds
    }

    /// Return per-code severity overrides.
    pub fn validation_overrides(&self) -> &BTreeMap<String, Severity> {
        self.plan.overrides()
    }

    /// Return every validation entry as compiled at load.
    #[doc(hidden)]
    pub fn validation_plan(&self) -> &ValidationPlan {
        &self.plan
    }

    /// Return declared pathway names in profile order.
    pub fn pathways(&self) -> &[String] {
        &self.pathways
    }

    /// Return finding-code bindings to pathway position attributes.
    pub fn pathway_bindings(&self) -> &BTreeMap<String, PathwayBinding> {
        self.plan.pathway_bindings()
    }

    /// Return parsed constraint validators in declaration order.
    #[doc(hidden)]
    pub fn constraint_configs(&self) -> &[ConstraintConfig] {
        self.plan.constraints()
    }

    /// Return parsed `EDGE_WHERE` validators in declaration order.
    #[doc(hidden)]
    pub fn edge_where_configs(&self) -> &[EdgeWhereConfig] {
        self.plan.edge_wheres()
    }

    /// Return parsed `EDGE_COUNT` validators in declaration order.
    #[doc(hidden)]
    pub fn edge_count_configs(&self) -> &[EdgeCountConfig] {
        self.plan.edge_counts()
    }

    /// One entry per suppressed code, in first-declared order.
    #[doc(hidden)]
    pub fn suppressions(&self) -> &[SuppressConfig] {
        self.plan.suppressions()
    }

    /// The resolved path of the notes file `annotations` names, if declared.
    ///
    /// The file need not exist.
    pub fn annotations(&self) -> Option<&Path> {
        self.notes.as_ref().map(|notes| notes.path.as_path())
    }

    /// Return where the first notes entry for `code` on `node` is: the notes file
    /// and the line its `-` is on.
    ///
    /// `None` when no entry names that pair or the file is missing. The location's
    /// `line()` is `None` when the entry has no line Lattice can name, as in a
    /// flow-style list.
    #[must_use]
    pub fn note_location(&self, code: &str, node: &str) -> Option<Provenance> {
        let notes = self.notes.as_ref()?;
        notes
            .items
            .as_ref()?
            .iter()
            .find(|item| {
                let finding = item.value.get("finding");
                let field = |key| finding.and_then(|f| f.get(key)).and_then(Value::as_str);
                field("code") == Some(code) && field("node") == Some(node)
            })
            .map(|item| Provenance::at_line(notes.path.display().to_string(), item.line))
    }

    pub(crate) fn notes(&self) -> Option<&NotesFile> {
        self.notes.as_ref()
    }

    /// The value of `adapter.native` if the profile declares one.
    pub fn native_adapter(&self) -> Option<&str> {
        self.adapter_key("native")?.as_str()
    }

    /// How long the core waits for an external adapter program:
    /// `adapter.timeout_seconds`, or 300 seconds when absent.
    pub fn adapter_timeout(&self) -> Duration {
        let seconds = self
            .adapter_key(TIMEOUT_KEYS[0])
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_ADAPTER_TIMEOUT_SECONDS);
        Duration::from_secs(seconds)
    }

    /// The `adapter:` block as the profile wrote it, for `describe` to print;
    /// `None` when the profile has none. The core reads none of its keys here.
    #[must_use]
    #[doc(hidden)]
    pub fn adapter_block(&self) -> Option<&Value> {
        self.raw.as_mapping()?.get(Value::String("adapter".into()))
    }

    fn adapter_key(&self, key: &str) -> Option<&Value> {
        let adapter = self
            .raw
            .as_mapping()?
            .get(Value::String("adapter".into()))?;
        adapter.as_mapping()?.get(Value::String(key.into()))
    }
}

pub(super) const DEFAULT_ADAPTER_TIMEOUT_SECONDS: u64 = 300;

/// The `adapter:` keys the core validates as positive integers. The first is
/// the core's own bound; the second passes through for adapters to read.
pub(super) const TIMEOUT_KEYS: [&str; 2] = ["timeout_seconds", "command_timeout_seconds"];

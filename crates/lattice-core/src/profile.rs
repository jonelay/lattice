//! Profile loading: the declared vocabulary of a register, read from YAML.
//!
//! The profile is where all domain knowledge lives. Nothing below names a kind, an
//! ID shape or a check — it reads what the file declares and hands it to validation.
//!
//! `extra` has no counterpart here. Python's `Profile` carries the unrecognised
//! top-level keys because adapters read `adapter:` out of it, and an adapter is now
//! a separate program that opens the profile itself.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;
use serde_norway::Value;

use crate::types::Severity;

/// The attribute types a profile may declare, in the order error messages list them.
pub const VALID_ATTR_TYPES: &[&str] = &["bool", "date", "enum", "int", "list", "string"];
/// The element types a `list` attribute may declare.
pub const VALID_LIST_ITEM_TYPES: &[&str] = &["bool", "date", "int", "string"];
/// The highest `profile_version` major this core will load.
pub const SUPPORTED_MAJOR_VERSION: u64 = 1;

/// Keys a validation entry admits, per code. Anything else is a profile error
/// rather than a silently ignored line.
fn config_keys(code: &str) -> &'static [&'static str] {
    match code {
        "COVERAGE" => &["edge_kind", "severity", "target_kind", "where"],
        "COVERAGE_DEEP" => &["evidence", "severity", "target_kind", "via", "where"],
        "CONSTRAINT" => &["expect", "kind", "message", "reject", "severity", "when"],
        "SUMMARY" => &["group_by_attr", "node_kind", "severity", "status_attr"],
        _ => &["severity"],
    }
}

/// The one `validations` entry that is a selector over findings rather than a
/// code's configuration. Takes neither `severity` nor a pathway binding: a
/// setting on a selector would have nothing to act on.
const SUPPRESS_KEYS: &[&str] = &["code", "node_ids"];

/// Available on every validation entry, for a code the core implements or one it
/// does not: the binding is orthogonal to what the code means.
const PATHWAY_BINDING_KEYS: &[&str] = &["pathway", "position_attr"];

#[derive(Debug)]
pub struct ProfileError(pub String);

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProfileError {}

fn err<T>(message: impl Into<String>) -> Result<T, ProfileError> {
    Err(ProfileError(message.into()))
}

/// The YAML value's runtime type — `string`, `int`, `float`, `bool`, `list`,
/// `null`, or `object`. Declared semantic types such as `date` remain strings
/// at this layer and are named separately when an expected schema type is known.
pub(crate) fn name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) => {
            if n.is_f64() {
                "float"
            } else {
                "int"
            }
        }
        Value::String(_) => "string",
        Value::Sequence(_) => "list",
        Value::Mapping(_) | Value::Tagged(_) => "object",
    }
}

/// The type an attr declares, parsed once at load so every later check matches
/// on a variant instead of re-comparing the profile's spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrType {
    Bool,
    Date,
    Enum,
    Int,
    List,
    String,
}

impl AttrType {
    /// Parse the profile's spelling, or `None` for anything else.
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

#[derive(Clone, Debug)]
pub struct AttrSchema {
    pub kind: AttrType,
    pub required: bool,
    pub values: Option<Vec<String>>,
    pub items: Option<AttrType>,
}

#[derive(Debug)]
pub struct NodeKind {
    /// The compiled pattern, anchored to the whole ID — Python matches with
    /// `fullmatch`, and Rust's `is_match` is a search.
    id_pattern: Regex,
    /// The pattern as the profile wrote it. `ID_FORMAT` quotes this, so the
    /// anchoring above must not leak into a finding's text.
    pub id_pattern_source: String,
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
    /// policy only — `query orphans` still reports them as the ask-time fact.
    pub orphan_ok: bool,
}

impl NodeKind {
    #[must_use]
    pub fn id_matches(&self, id: &str) -> bool {
        self.id_pattern.is_match(id)
    }
}

#[derive(Debug)]
pub struct EdgeKind {
    pub allowed: Vec<(String, String)>,
    pub cross_source: bool,
}

impl EdgeKind {
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
#[derive(Clone, Debug)]
pub struct PathwayBinding {
    pub pathway: String,
    pub position_attr: String,
}

/// A value that ordering operators (`lt`, `gt`, `lte`, `gte`) can compare.
#[derive(Clone, Debug, PartialEq)]
pub enum Comparable {
    Int(i128),
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
pub enum ConditionOp {
    Eq(serde_json::Value),
    Not(serde_json::Value),
    In(Vec<serde_json::Value>),
    Lt(Comparable),
    Gt(Comparable),
    Lte(Comparable),
    Gte(Comparable),
    Matches(Regex),
    Present(bool),
}

/// One attr-name → operator pair in a when/expect/reject block.
#[derive(Clone, Debug)]
pub struct Condition {
    pub attr: String,
    pub op: ConditionOp,
}

/// A parsed CONSTRAINT validation entry.
#[derive(Clone, Debug)]
pub struct ConstraintConfig {
    pub kind: String,
    pub when: Vec<Condition>,
    pub expect: Vec<Condition>,
    pub reject: Vec<Condition>,
    pub message: Option<String>,
    pub severity: Option<Severity>,
}

/// A parsed SUPPRESS entry, merged across every entry naming its code.
///
/// `node_ids: None` suppresses every finding of the code, and supersedes any
/// ID-specific entry for the same code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppressConfig {
    pub code: String,
    pub node_ids: Option<Vec<String>>,
}

/// One validation entry's configuration, as declared.
pub type ValidationConfig = BTreeMap<String, Value>;

/// The declared vocabulary of a register: node kinds, edge kinds, checks.
#[derive(Debug)]
pub struct Profile {
    name: String,
    profile_version: String,
    node_kinds: BTreeMap<String, NodeKind>,
    edge_kinds: BTreeMap<String, EdgeKind>,
    validation_overrides: BTreeMap<String, Severity>,
    validation_configs: BTreeMap<String, Vec<ValidationConfig>>,
    constraint_configs: Vec<ConstraintConfig>,
    suppressions: Vec<SuppressConfig>,
    pathways: Vec<String>,
    pathway_bindings: BTreeMap<String, PathwayBinding>,
    /// The profile after inheritance resolution (if `extends:` was declared) and
    /// parsing, kept so `resolved_document` can re-emit sections the core does
    /// not model (the `adapter:` namespace above all).
    raw: Value,
}

impl Profile {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn profile_version(&self) -> &str {
        &self.profile_version
    }

    pub fn node_kinds(&self) -> &BTreeMap<String, NodeKind> {
        &self.node_kinds
    }

    pub fn edge_kinds(&self) -> &BTreeMap<String, EdgeKind> {
        &self.edge_kinds
    }

    pub fn validation_overrides(&self) -> &BTreeMap<String, Severity> {
        &self.validation_overrides
    }

    pub fn validation_configs(&self) -> &BTreeMap<String, Vec<ValidationConfig>> {
        &self.validation_configs
    }

    pub fn pathways(&self) -> &[String] {
        &self.pathways
    }

    pub fn pathway_bindings(&self) -> &BTreeMap<String, PathwayBinding> {
        &self.pathway_bindings
    }

    pub fn constraint_configs(&self) -> &[ConstraintConfig] {
        &self.constraint_configs
    }

    /// One entry per suppressed code, in first-declared order.
    pub fn suppressions(&self) -> &[SuppressConfig] {
        &self.suppressions
    }
}

/// Convert a parsed profile value to JSON, refusing what JSON cannot carry.
///
/// Refusal matters because a coerced value would round-trip *changed*, and the
/// resolved document's guarantee is that it round-trips as declared.
fn to_json(value: &Value) -> Result<serde_json::Value, ProfileError> {
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => {
            let text = n.to_string();
            serde_json::from_str(&text)
                .map_err(|_| ProfileError(format!("number '{text}' has no JSON form")))?
        }
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Sequence(items) => {
            serde_json::Value::Array(items.iter().map(to_json).collect::<Result<_, _>>()?)
        }
        Value::Mapping(m) => {
            let mut object = serde_json::Map::new();
            for (key, item) in m {
                let Value::String(key) = key else {
                    return err(format!("mapping key {key:?} is not a string"));
                };
                object.insert(key.clone(), to_json(item)?);
            }
            serde_json::Value::Object(object)
        }
        Value::Tagged(t) => return err(format!("tagged value '{}' has no JSON form", t.tag)),
    })
}

fn to_comparable(value: &Value) -> Option<Comparable> {
    match value {
        Value::String(s) => Some(Comparable::Str(s.clone())),
        Value::Number(n) => {
            let v = n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from))?;
            Some(Comparable::Int(v))
        }
        _ => None,
    }
}

/// Serialize the resolved profile document handed to adapters.
///
/// The profile after inheritance resolution, plus `resolved_schema`, so that
/// sections the core does not consume — `adapter:` above all — reach the
/// adapter unchanged.
pub fn resolved_document(profile: &Profile) -> Result<String, ProfileError> {
    let mut document = match to_json(&profile.raw)? {
        serde_json::Value::Object(object) => object,
        _ => unreachable!("load_profile requires a mapping at top level"),
    };
    document.insert(
        "resolved_schema".into(),
        serde_json::Value::String("1".into()),
    );
    serde_json::to_string(&serde_json::Value::Object(document))
        .map_err(|e| ProfileError(format!("could not serialize resolved profile: {e}")))
}

fn as_mapping<'a>(value: &'a Value, what: &str) -> Result<&'a serde_norway::Mapping, ProfileError> {
    match value {
        Value::Mapping(m) => Ok(m),
        other => err(format!("{what}: expected a mapping, got {}", name(other))),
    }
}

fn key_name(key: &Value) -> String {
    match key {
        Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

/// Build one attribute schema, accepting the bare-type-name shorthand.
///
/// `text: string` is sugar for `text: {type: string}`; both reach here.
fn parse_attr(kind_name: &str, attr_name: &str, raw: &Value) -> Result<AttrSchema, ProfileError> {
    let where_ = format!("node kind '{kind_name}', attr '{attr_name}'");
    let shorthand;
    let raw = match raw {
        Value::String(type_name) => {
            shorthand = Value::Mapping(
                [(
                    Value::String("type".into()),
                    Value::String(type_name.clone()),
                )]
                .into_iter()
                .collect(),
            );
            &shorthand
        }
        other => other,
    };
    let mapping = as_mapping(raw, &where_)?;

    let kind = match mapping.get(Value::String("type".into())) {
        None | Some(Value::Null) => return err(format!("{where_}: missing 'type'")),
        Some(Value::String(text)) => AttrType::parse(text).ok_or_else(|| {
            ProfileError(format!(
                "{where_}: unknown type '{text}' (valid: {})",
                VALID_ATTR_TYPES.join(", ")
            ))
        })?,
        Some(other) => {
            return err(format!(
                "{where_}: 'type' must be a string, got {}",
                name(other)
            ));
        }
    };

    let required = match mapping.get(Value::String("required".into())) {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => {
            return err(format!(
                "{where_}: 'required' must be a boolean, got {}",
                name(other)
            ));
        }
    };

    let mut values = None;
    if kind == AttrType::Enum {
        let raw_values = mapping.get(Value::String("values".into()));
        let Some(Value::Sequence(items)) = raw_values else {
            return err(format!(
                "{where_}: enum type requires 'values' (a list of strings)"
            ));
        };
        if items.is_empty() {
            return err(format!(
                "{where_}: enum type requires 'values' (a list of strings)"
            ));
        }
        let mut collected = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::String(text) => collected.push(text.clone()),
                other => {
                    return err(format!(
                        "{where_}: enum value must be a string, got {}",
                        name(other)
                    ));
                }
            }
        }
        values = Some(collected);
    }

    let mut items_type = None;
    if kind == AttrType::List {
        let raw_items = mapping.get(Value::String("items".into()));
        let Some(Value::String(text)) = raw_items else {
            return err(format!("{where_}: list type requires 'items'"));
        };
        if text.is_empty() {
            return err(format!("{where_}: list type requires 'items'"));
        }
        let Some(item_type) = AttrType::parse(text).filter(|t| t.is_list_item()) else {
            return err(format!(
                "{where_}: list items type '{text}' is not a valid primitive (valid: {})",
                VALID_LIST_ITEM_TYPES.join(", ")
            ));
        };
        items_type = Some(item_type);
    }

    Ok(AttrSchema {
        kind,
        required,
        values,
        items: items_type,
    })
}

/// Build one node kind, compiling its required id_pattern.
fn parse_node_kind(
    kind_name: &str,
    raw: &Value,
    declared_index: usize,
) -> Result<NodeKind, ProfileError> {
    let where_ = format!("node kind '{kind_name}'");
    let mapping = as_mapping(raw, &where_)?;

    let source = match mapping.get(Value::String("id_pattern".into())) {
        None | Some(Value::Null) => return err(format!("{where_}: missing 'id_pattern'")),
        Some(Value::String(text)) => text.clone(),
        Some(other) => {
            return err(format!(
                "{where_}: id_pattern must be a string, got {}",
                name(other)
            ));
        }
    };
    // Python matches IDs with `fullmatch`; the Rust equivalent is an explicitly
    // anchored pattern, since `is_match` searches. The profile's own anchors, if
    // any, still apply inside the group.
    let id_pattern = Regex::new(&format!(r"\A(?:{source})\z"))
        .map_err(|e| ProfileError(format!("{where_}: invalid id_pattern: {e}")))?;

    let mut attrs = BTreeMap::new();
    match mapping.get(Value::String("attrs".into())) {
        None | Some(Value::Null) => {}
        Some(Value::Mapping(raw_attrs)) => {
            for (attr_key, attr_raw) in raw_attrs {
                let attr_name = key_name(attr_key);
                attrs.insert(
                    attr_name.clone(),
                    parse_attr(kind_name, &attr_name, attr_raw)?,
                );
            }
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'attrs' must be a mapping, got {}",
                name(other)
            ));
        }
    }

    let summary_attr = match mapping.get(Value::String("summary_attr".into())) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let Some(schema) = attrs.get(text) else {
                return err(format!(
                    "{where_}: summary_attr '{text}' is not a declared attr \
                     (declared: {:?})",
                    attrs.keys().collect::<Vec<_>>()
                ));
            };
            if schema.kind == AttrType::List {
                return err(format!(
                    "{where_}: summary_attr '{text}' is a list, \
                     but the summary column is a short scalar label"
                ));
            }
            Some(text.clone())
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'summary_attr' must be a string, got {}",
                name(other)
            ));
        }
    };

    let text_attrs = match mapping.get(Value::String("text_attrs".into())) {
        None | Some(Value::Null) => None,
        Some(Value::Sequence(items)) => {
            let mut names: Vec<String> = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(text) = item else {
                    return err(format!(
                        "{where_}: 'text_attrs' entries must be strings, got {}",
                        name(item)
                    ));
                };
                let Some(schema) = attrs.get(text) else {
                    return err(format!(
                        "{where_}: text_attrs '{text}' is not a declared attr \
                         (declared: {:?})",
                        attrs.keys().collect::<Vec<_>>()
                    ));
                };
                if !matches!(schema.kind, AttrType::String | AttrType::Enum) {
                    return err(format!(
                        "{where_}: text_attrs '{text}' is {} '{}', \
                         but ranked text must be textual (string or enum)",
                        if schema.kind == AttrType::Int {
                            "an"
                        } else {
                            "a"
                        },
                        schema.kind.as_str()
                    ));
                }
                if names.contains(text) {
                    return err(format!(
                        "{where_}: text_attrs names '{text}' twice, \
                         which would rank its text twice"
                    ));
                }
                names.push(text.clone());
            }
            Some(names)
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'text_attrs' must be a list, got {}",
                name(other)
            ));
        }
    };

    let text_chunk_line_prefix = match mapping.get(Value::String("text_chunk_line_prefix".into())) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            if text.trim().is_empty() {
                return err(format!(
                    "{where_}: 'text_chunk_line_prefix' is empty or all whitespace, \
                     which prefixes almost every line and so names no cut point"
                ));
            }
            if text.contains('\n') || text.contains('\r') {
                return err(format!(
                    "{where_}: 'text_chunk_line_prefix' spans a line boundary, \
                     but it is compared against one line at a time and could never match"
                ));
            }
            // A kind offering no text has nothing to subdivide, so the key would
            // sit in the profile doing nothing. The schema rejects a partial pathway
            // binding on the same grounds.
            let has_text = match &text_attrs {
                Some(names) => !names.is_empty(),
                None => summary_attr.is_some(),
            };
            if !has_text {
                return err(format!(
                    "{where_}: 'text_chunk_line_prefix' is declared but the kind offers \
                     no text to chunk (no non-empty 'text_attrs', and no 'summary_attr' \
                     to fall back to)"
                ));
            }
            Some(text.clone())
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'text_chunk_line_prefix' must be a string, got {}",
                name(other)
            ));
        }
    };

    let orphan_ok = match mapping.get(Value::String("orphan_ok".into())) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => {
            return err(format!(
                "{where_}: 'orphan_ok' must be a bool, got {}",
                name(other)
            ));
        }
    };

    Ok(NodeKind {
        id_pattern,
        id_pattern_source: source,
        attrs,
        summary_attr,
        text_attrs,
        text_chunk_line_prefix,
        declared_index,
        orphan_ok,
    })
}

/// Build one edge kind, checking each allowed pair names a declared kind.
///
/// Endpoint kinds are resolved at load time so a typo in `allowed` fails on the
/// profile rather than silently admitting no edge at validation.
fn parse_edge_kind(
    kind_name: &str,
    raw: &Value,
    valid_node_kinds: &BTreeMap<String, NodeKind>,
) -> Result<EdgeKind, ProfileError> {
    let where_ = format!("edge kind '{kind_name}'");
    let mapping = as_mapping(raw, &where_)?;

    let mut allowed = Vec::new();
    match mapping.get(Value::String("allowed".into())) {
        None | Some(Value::Null) => {}
        Some(Value::Sequence(pairs)) => {
            for pair in pairs {
                let Value::Sequence(ends) = pair else {
                    return err(format!(
                        "{where_}: allowed entry must be [source_kind, target_kind]"
                    ));
                };
                if ends.len() != 2 {
                    return err(format!(
                        "{where_}: allowed entry must be [source_kind, target_kind]"
                    ));
                }
                let mut resolved = Vec::with_capacity(2);
                for end in ends {
                    let Value::String(kind_ref) = end else {
                        return err(format!(
                            "{where_}: references undefined node kind '{}'",
                            key_name(end)
                        ));
                    };
                    if !valid_node_kinds.contains_key(kind_ref) {
                        return err(format!(
                            "{where_}: references undefined node kind '{kind_ref}'"
                        ));
                    }
                    resolved.push(kind_ref.clone());
                }
                allowed.push((resolved[0].clone(), resolved[1].clone()));
            }
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'allowed' must be a list, got {}",
                name(other)
            ));
        }
    }

    let cross_source = match mapping.get(Value::String("cross_source".into())) {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(other) => {
            return err(format!(
                "{where_}: 'cross_source' must be a bool, got {}",
                name(other)
            ));
        }
    };

    Ok(EdgeKind {
        allowed,
        cross_source,
    })
}

fn check_version(raw: &Value) -> Result<String, ProfileError> {
    let Value::String(version) = raw else {
        return err(format!(
            "profile_version must be a string, got {}",
            name(raw)
        ));
    };
    let parts: Vec<&str> = version.split('.').collect();
    let valid = parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        });
    if !valid {
        return err(format!(
            "profile_version '{version}' is not a valid version \
             (expected X.Y.Z where each component is a non-negative \
             integer without leading zeros)"
        ));
    }
    let major: u64 = parts[0].parse().unwrap_or(u64::MAX);
    if major > SUPPORTED_MAJOR_VERSION {
        return err(format!(
            "unsupported profile version {version} \
             (supported major: {SUPPORTED_MAJOR_VERSION})"
        ));
    }
    Ok(version.clone())
}

/// Recursively merge `parent` and `child` Values.
///
/// Mappings deep-merge (child keys win on conflict); sequences and scalars
/// are replaced whole by the child.
fn merge_values(parent: Value, child: Value) -> Value {
    match (parent, child) {
        (Value::Mapping(mut parent_map), Value::Mapping(child_map)) => {
            for (key, child_val) in child_map {
                let merged = match parent_map.get(&key) {
                    Some(parent_val) => merge_values(parent_val.clone(), child_val),
                    None => child_val,
                };
                parent_map.insert(key, merged);
            }
            Value::Mapping(parent_map)
        }
        (_, child) => child,
    }
}

/// Parse one YAML file and, if it declares `extends`, recurse into the parent
/// and merge. Returns the merged raw value with `extends` stripped.
fn load_raw(path: &Path, visited: &mut Vec<std::path::PathBuf>) -> Result<Value, ProfileError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| ProfileError(format!("failed to resolve profile {}: {e}", path.display())))?;
    if visited.contains(&canonical) {
        return err(format!(
            "profile inheritance cycle: {} was already visited",
            path.display()
        ));
    }
    visited.push(canonical);

    let text = std::fs::read_to_string(path)
        .map_err(|e| ProfileError(format!("failed to read profile {}: {e}", path.display())))?;
    let mut raw: Value = serde_norway::from_str(&text)
        .map_err(|e| ProfileError(format!("failed to read profile {}: {e}", path.display())))?;

    let extends_key = Value::String("extends".into());
    if let Value::Mapping(ref mut top) = raw
        && let Some(extends_val) = top.shift_remove(&extends_key)
    {
        let Value::String(ref parent_path_str) = extends_val else {
            return err(format!(
                "profile {}: 'extends' must be a string, got {}",
                path.display(),
                name(&extends_val)
            ));
        };
        if !top.contains_key(Value::String("profile_version".into())) {
            return err(format!(
                "profile {}: child must declare its own 'profile_version' \
                 (inheriting it from a parent is not allowed)",
                path.display()
            ));
        }
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let parent_path = base_dir.join(parent_path_str);
        let parent_raw = load_raw(&parent_path, visited)?;
        if !matches!(parent_raw, Value::Mapping(_)) {
            return err(format!(
                "profile {}: parent '{}' is not a YAML mapping",
                path.display(),
                parent_path.display()
            ));
        }
        raw = merge_values(parent_raw, raw);
    }

    Ok(raw)
}

const VALID_CONDITION_OPS: &[&str] = &[
    "eq", "gt", "gte", "in", "lt", "lte", "matches", "not", "present",
];

pub(crate) fn parse_condition_block(
    value: Option<&Value>,
    config_name: &str,
    block_name: &str,
) -> Result<Vec<Condition>, ProfileError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Mapping(map) = value else {
        return err(format!(
            "{config_name} config: '{block_name}' must be a mapping, got {}",
            name(value)
        ));
    };
    let mut conditions = Vec::new();
    for (attr_key, op_value) in map {
        let attr = key_name(attr_key);
        let Value::Mapping(op_map) = op_value else {
            return err(format!(
                "{config_name} config: '{block_name}.{attr}' must be a mapping, got {}",
                name(op_value)
            ));
        };
        if op_map.len() != 1 {
            return err(format!(
                "{config_name} config: '{block_name}.{attr}' must have exactly one operator"
            ));
        }
        let (op_key, op_val) = op_map.iter().next().unwrap();
        let op_name = key_name(op_key);
        if !VALID_CONDITION_OPS.contains(&op_name.as_str()) {
            return err(format!(
                "{config_name} config: '{block_name}.{attr}' unknown operator '{op_name}' \
                 (valid: {VALID_CONDITION_OPS:?})"
            ));
        }
        let op = match op_name.as_str() {
            "eq" => ConditionOp::Eq(to_json(op_val)?),
            "not" => ConditionOp::Not(to_json(op_val)?),
            "lt" | "gt" | "lte" | "gte" => {
                let comparable = to_comparable(op_val).ok_or_else(|| {
                    ProfileError(format!(
                        "{config_name} config: '{block_name}.{attr}.{op_name}' \
                         must be an integer or a string, got {}",
                        name(op_val)
                    ))
                })?;
                match op_name.as_str() {
                    "lt" => ConditionOp::Lt(comparable),
                    "gt" => ConditionOp::Gt(comparable),
                    "lte" => ConditionOp::Lte(comparable),
                    _ => ConditionOp::Gte(comparable),
                }
            }
            "in" => {
                let Value::Sequence(items) = op_val else {
                    return err(format!(
                        "{config_name} config: '{block_name}.{attr}.in' must be a list"
                    ));
                };
                let json_items: Vec<serde_json::Value> =
                    items.iter().map(to_json).collect::<Result<_, _>>()?;
                ConditionOp::In(json_items)
            }
            "matches" => {
                let Value::String(pattern) = op_val else {
                    return err(format!(
                        "{config_name} config: '{block_name}.{attr}.matches' must be a string"
                    ));
                };
                let regex = Regex::new(pattern).map_err(|e| {
                    ProfileError(format!(
                        "{config_name} config: '{block_name}.{attr}.matches' \
                         invalid regex '{pattern}': {e}"
                    ))
                })?;
                ConditionOp::Matches(regex)
            }
            "present" => {
                let Value::Bool(b) = op_val else {
                    return err(format!(
                        "{config_name} config: '{block_name}.{attr}.present' must be a bool"
                    ));
                };
                ConditionOp::Present(*b)
            }
            _ => unreachable!(),
        };
        conditions.push(Condition { attr, op });
    }
    Ok(conditions)
}

/// Read and fully validate a profile YAML file.
///
/// Fails on any malformed or unsupported input; a returned `Profile` is
/// structurally sound, so callers need not re-check it.
pub fn load_profile(path: &Path) -> Result<Profile, ProfileError> {
    let raw = load_raw(path, &mut Vec::new())?;
    load_profile_value(raw, &format!("profile {}", path.display()))
}

/// Build a `Profile` from an already-parsed YAML `Value`.
pub fn load_profile_value(raw: Value, where_: &str) -> Result<Profile, ProfileError> {
    let where_ = where_.to_string();
    let Value::Mapping(top) = &raw else {
        return err(format!("{where_}: expected a YAML mapping at top level"));
    };

    for key in ["name", "profile_version", "node_kinds", "edge_kinds"] {
        if !top.contains_key(Value::String(key.into())) {
            return err(format!("{where_}: missing required key '{key}'"));
        }
    }

    let name_value = &top[Value::String("name".into())];
    let Value::String(profile_name) = name_value else {
        return err(format!(
            "{where_}: 'name' must be a string, got {}",
            name(name_value)
        ));
    };

    let profile_version = check_version(&top[Value::String("profile_version".into())])?;

    let raw_node_kinds = &top[Value::String("node_kinds".into())];
    let Value::Mapping(raw_node_kinds) = raw_node_kinds else {
        return err(format!(
            "{where_}: 'node_kinds' must be a mapping, got {}",
            name(raw_node_kinds)
        ));
    };
    let mut node_kinds = BTreeMap::new();
    for (declared_index, (key, value)) in raw_node_kinds.iter().enumerate() {
        let kind_name = key_name(key);
        let empty = Value::Mapping(Default::default());
        let value = if matches!(value, Value::Null) {
            &empty
        } else {
            value
        };
        node_kinds.insert(
            kind_name.clone(),
            parse_node_kind(&kind_name, value, declared_index)?,
        );
    }

    let raw_edge_kinds = &top[Value::String("edge_kinds".into())];
    let Value::Mapping(raw_edge_kinds) = raw_edge_kinds else {
        return err(format!(
            "{where_}: 'edge_kinds' must be a mapping, got {}",
            name(raw_edge_kinds)
        ));
    };
    let mut edge_kinds = BTreeMap::new();
    for (key, value) in raw_edge_kinds {
        let kind_name = key_name(key);
        let empty = Value::Mapping(Default::default());
        let value = if matches!(value, Value::Null) {
            &empty
        } else {
            value
        };
        edge_kinds.insert(
            kind_name.clone(),
            parse_edge_kind(&kind_name, value, &node_kinds)?,
        );
    }

    let empty_list = Value::Sequence(Vec::new());
    let raw_validations = match top.get(Value::String("validations".into())) {
        None | Some(Value::Null) => &empty_list,
        Some(value) => value,
    };
    let Value::Sequence(raw_validations) = raw_validations else {
        return err(format!(
            "{where_}: 'validations' must be a list, got {}",
            name(raw_validations)
        ));
    };

    // As with the document's arrays: absent means none, explicit null is
    // malformed. `validations` differs and accepts null, matching the reference
    // core rather than being made consistent with it.
    let raw_pathways = match top.get(Value::String("pathways".into())) {
        None => Vec::new(),
        Some(Value::Sequence(items)) => {
            let mut pathways = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(pathway) = item else {
                    return err(format!(
                        "{where_}: 'pathways' must be a list of pathway names, got {}. \
                         Pathway values are target state and are read from the register, \
                         never declared here",
                        name(item)
                    ));
                };
                pathways.push(pathway.clone());
            }
            pathways
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'pathways' must be a list of pathway names, got {}. \
                 Pathway values are target state and are read from the register, \
                 never declared here",
                name(other)
            ));
        }
    };

    let mut validation_overrides: BTreeMap<String, Severity> = BTreeMap::new();
    let mut validation_configs: BTreeMap<String, Vec<ValidationConfig>> = BTreeMap::new();
    let mut pathway_bindings: BTreeMap<String, PathwayBinding> = BTreeMap::new();
    let mut suppressions: Vec<SuppressConfig> = Vec::new();

    for entry in raw_validations {
        let Value::Mapping(entry) = entry else {
            return err(format!(
                "validations entry must be a mapping, got {}",
                name(entry)
            ));
        };
        for (code_key, config) in entry {
            let code = key_name(code_key);
            let Value::Mapping(config) = config else {
                return err(format!(
                    "validation override for '{code}' must be a mapping, got {}",
                    name(config)
                ));
            };

            if code == "SUPPRESS" {
                merge_suppression(&mut suppressions, parse_suppress(config)?);
                continue;
            }

            // Last-write-wins, matching the Python core: a second `severity:` for
            // one code overwrites the first rather than being reported.
            if let Some(severity) = config.get(Value::String("severity".into()))
                && !matches!(severity, Value::Null)
            {
                let Value::String(text) = severity else {
                    return err(format!(
                        "validation override for '{code}': 'severity' must be a string, \
                             got {}",
                        name(severity)
                    ));
                };
                let Some(parsed) = Severity::parse(text) else {
                    return err(format!(
                        "validation override for '{code}': invalid severity '{text}' \
                             (valid: error, info, warning)"
                    ));
                };
                validation_overrides.insert(code.clone(), parsed);
            }

            let allowed: Vec<&str> = config_keys(&code)
                .iter()
                .chain(PATHWAY_BINDING_KEYS.iter())
                .copied()
                .collect();
            let mut unknown: Vec<String> = config
                .keys()
                .map(key_name)
                .filter(|k| !allowed.contains(&k.as_str()))
                .collect();
            if !unknown.is_empty() {
                unknown.sort();
                let mut valid = allowed.clone();
                valid.sort_unstable();
                return err(format!(
                    "validation config for '{code}': unknown keys {unknown:?} \
                     (valid: {valid:?})"
                ));
            }

            if matches!(code.as_str(), "COVERAGE" | "COVERAGE_DEEP") {
                parse_condition_block(config.get(Value::String("where".into())), &code, "where")?;
            }

            let stored: ValidationConfig = config
                .iter()
                .map(|(k, v)| (key_name(k), v.clone()))
                .collect();
            validation_configs
                .entry(code.clone())
                .or_default()
                .push(stored);

            let present: Vec<&str> = PATHWAY_BINDING_KEYS
                .iter()
                .copied()
                .filter(|k| config.contains_key(Value::String((*k).into())))
                .collect();
            if present.is_empty() {
                continue;
            }
            if present.len() != PATHWAY_BINDING_KEYS.len() {
                let missing: Vec<&str> = PATHWAY_BINDING_KEYS
                    .iter()
                    .copied()
                    .filter(|k| !present.contains(k))
                    .collect();
                return err(format!(
                    "validation config for '{code}': pathway binding needs both \
                     {PATHWAY_BINDING_KEYS:?}, missing {missing:?}"
                ));
            }
            let Value::String(pathway) = &config[Value::String("pathway".into())] else {
                return err(format!(
                    "validation config for '{code}': 'pathway' must be a string"
                ));
            };
            if !raw_pathways.contains(pathway) {
                let mut declared = raw_pathways.clone();
                declared.sort();
                return err(format!(
                    "validation config for '{code}': undeclared pathway '{pathway}' \
                     (declared: {declared:?})"
                ));
            }
            if pathway_bindings.contains_key(&code) {
                return err(format!(
                    "validation config for '{code}': a second pathway binding. \
                     Repeated configurations are honoured, but two bindings give \
                     severity resolution two answers for one finding"
                ));
            }
            let Value::String(position_attr) = &config[Value::String("position_attr".into())]
            else {
                return err(format!(
                    "validation config for '{code}': 'position_attr' must be a string"
                ));
            };
            pathway_bindings.insert(
                code.clone(),
                PathwayBinding {
                    pathway: pathway.clone(),
                    position_attr: position_attr.clone(),
                },
            );
        }
    }

    let mut constraint_configs: Vec<ConstraintConfig> = Vec::new();
    for config in validation_configs.get("CONSTRAINT").unwrap_or(&Vec::new()) {
        let kind = match config.get("kind") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => {
                return err(format!(
                    "CONSTRAINT config: 'kind' must be a string, got {}",
                    name(other)
                ));
            }
            None => {
                return err("CONSTRAINT config missing required key: kind".to_string());
            }
        };
        if !node_kinds.contains_key(&kind) {
            return err(format!(
                "CONSTRAINT config: kind '{kind}' not in profile node kinds"
            ));
        }
        let has_expect = config.contains_key("expect");
        let has_reject = config.contains_key("reject");
        if !has_expect && !has_reject {
            return err(
                "CONSTRAINT config: requires at least one of 'expect' or 'reject'".to_string(),
            );
        }
        let when = parse_condition_block(config.get("when"), "CONSTRAINT", "when")?;
        let expect = parse_condition_block(config.get("expect"), "CONSTRAINT", "expect")?;
        let reject = parse_condition_block(config.get("reject"), "CONSTRAINT", "reject")?;
        let message = config.get("message").and_then(|v| {
            if let Value::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        });
        let severity = config
            .get("severity")
            .map(|v| {
                let Value::String(s) = v else {
                    return err(format!(
                        "CONSTRAINT config: 'severity' must be a string, got {}",
                        name(v)
                    ));
                };
                Severity::parse(s).ok_or_else(|| {
                    ProfileError(format!("CONSTRAINT config: invalid severity '{s}'"))
                })
            })
            .transpose()?;
        constraint_configs.push(ConstraintConfig {
            kind,
            when,
            expect,
            reject,
            message,
            severity,
        });
    }

    Ok(Profile {
        name: profile_name.clone(),
        profile_version,
        node_kinds,
        edge_kinds,
        validation_overrides,
        validation_configs,
        constraint_configs,
        suppressions,
        pathways: raw_pathways,
        pathway_bindings,
        raw,
    })
}

/// Parse one SUPPRESS entry, refusing the code that reports a broken profile:
/// silencing CONFIG_ERROR would defeat "unreadable input is reported, never
/// dropped".
pub(crate) fn parse_suppress(
    config: &serde_norway::Mapping,
) -> Result<SuppressConfig, ProfileError> {
    let mut unknown: Vec<String> = config
        .keys()
        .map(key_name)
        .filter(|k| !SUPPRESS_KEYS.contains(&k.as_str()))
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return err(format!(
            "validation config for 'SUPPRESS': unknown keys {unknown:?} \
             (valid: {SUPPRESS_KEYS:?})"
        ));
    }
    let code = match config.get(Value::String("code".into())) {
        None | Some(Value::Null) => {
            return err("validation config for 'SUPPRESS': missing required key 'code'");
        }
        Some(Value::String(text)) => text.clone(),
        Some(other) => {
            return err(format!(
                "validation config for 'SUPPRESS': 'code' must be a string, got {}",
                name(other)
            ));
        }
    };
    if code == "CONFIG_ERROR" {
        return err(
            "validation config for 'SUPPRESS': CONFIG_ERROR cannot be suppressed; \
             it reports that the profile itself is broken",
        );
    }
    let node_ids = match config.get(Value::String("node_ids".into())) {
        None | Some(Value::Null) => None,
        Some(Value::Sequence(items)) => {
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(id) = item else {
                    return err(format!(
                        "validation config for 'SUPPRESS': 'node_ids' entries must be \
                         strings, got {}",
                        name(item)
                    ));
                };
                ids.push(id.clone());
            }
            Some(ids)
        }
        Some(other) => {
            return err(format!(
                "validation config for 'SUPPRESS': 'node_ids' must be a list, got {}",
                name(other)
            ));
        }
    };
    Ok(SuppressConfig { code, node_ids })
}

/// Union the `node_ids` of entries naming one code; a suppress-all wins outright.
fn merge_suppression(suppressions: &mut Vec<SuppressConfig>, entry: SuppressConfig) {
    let Some(existing) = suppressions.iter_mut().find(|s| s.code == entry.code) else {
        suppressions.push(entry);
        return;
    };
    let (Some(ids), Some(new_ids)) = (existing.node_ids.as_mut(), entry.node_ids) else {
        existing.node_ids = None;
        return;
    };
    for id in new_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
}

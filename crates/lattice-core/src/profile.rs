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
pub const VALID_ATTR_TYPES: &[&str] = &["bool", "enum", "int", "list", "string"];
/// The element types a `list` attribute may declare.
pub const VALID_LIST_ITEM_TYPES: &[&str] = &["bool", "int", "string"];
/// The highest `profile_version` major this core will load.
pub const SUPPORTED_MAJOR_VERSION: u64 = 1;

/// Keys a validation entry admits, per code. Anything else is a profile error
/// rather than a silently ignored line.
fn config_keys(code: &str) -> &'static [&'static str] {
    match code {
        "COVERAGE" => &["edge_kind", "severity", "target_kind"],
        "COVERAGE_DEEP" => &["evidence", "severity", "target_kind", "via"],
        "SUMMARY" => &["group_by_attr", "node_kind", "severity", "status_attr"],
        _ => &["severity"],
    }
}

/// Available on every validation entry, for a code the core implements or one it
/// does not: the binding is orthogonal to what the code means.
const AXIS_BINDING_KEYS: &[&str] = &["axis", "position_attr"];

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

/// The YAML value's type in the profile's own attr-type vocabulary — `string`,
/// `int`, `float`, `bool`, `list` — extended with `null` and `object`, so a
/// profile error names types in the same words its author writes.
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

#[derive(Clone, Debug)]
pub struct AttrSchema {
    pub kind: String,
    pub required: bool,
    pub values: Option<Vec<String>>,
    pub items: Option<String>,
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
    /// Exempts this kind's nodes from `ORPHAN_NODE`. Validation policy only —
    /// `query orphans` still reports them as the ask-time fact they are.
    pub orphan_ok: bool,
}

impl NodeKind {
    pub fn id_matches(&self, id: &str) -> bool {
        self.id_pattern.is_match(id)
    }
}

#[derive(Debug)]
pub struct EdgeKind {
    pub allowed: Vec<(String, String)>,
}

impl EdgeKind {
    pub fn admits(&self, src_kind: &str, tgt_kind: &str) -> bool {
        self.allowed
            .iter()
            .any(|(s, t)| s == src_kind && t == tgt_kind)
    }
}

/// Ties a finding code's severity to a node attr's position on an axis.
///
/// Carries no demotion target: a demoted finding becomes `info`, always.
#[derive(Clone, Debug)]
pub struct AxisBinding {
    pub axis: String,
    pub position_attr: String,
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
    axes: Vec<String>,
    axis_bindings: BTreeMap<String, AxisBinding>,
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

    pub fn axes(&self) -> &[String] {
        &self.axes
    }

    pub fn axis_bindings(&self) -> &BTreeMap<String, AxisBinding> {
        &self.axis_bindings
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
        Some(Value::String(text)) => text.clone(),
        Some(other) => {
            return err(format!(
                "{where_}: 'type' must be a string, got {}",
                name(other)
            ));
        }
    };
    if !VALID_ATTR_TYPES.contains(&kind.as_str()) {
        return err(format!(
            "{where_}: unknown type '{kind}' (valid: {})",
            VALID_ATTR_TYPES.join(", ")
        ));
    }

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
    if kind == "enum" {
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
    if kind == "list" {
        let raw_items = mapping.get(Value::String("items".into()));
        let Some(Value::String(text)) = raw_items else {
            return err(format!("{where_}: list type requires 'items'"));
        };
        if text.is_empty() {
            return err(format!("{where_}: list type requires 'items'"));
        }
        if !VALID_LIST_ITEM_TYPES.contains(&text.as_str()) {
            return err(format!(
                "{where_}: list items type '{text}' is not a valid primitive (valid: {})",
                VALID_LIST_ITEM_TYPES.join(", ")
            ));
        }
        items_type = Some(text.clone());
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
            if schema.kind == "list" {
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
                if schema.kind != "string" && schema.kind != "enum" {
                    return err(format!(
                        "{where_}: text_attrs '{text}' is {} '{}', \
                         but ranked text must be textual (string or enum)",
                        if schema.kind == "int" { "an" } else { "a" },
                        schema.kind
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
            // sit in the profile doing nothing. The schema rejects a partial axis
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

    Ok(EdgeKind { allowed })
}

fn check_version(raw: &Value) -> Result<String, ProfileError> {
    let Value::String(version) = raw else {
        return err(format!(
            "profile_version must be a string, got {}",
            name(raw)
        ));
    };
    let parts: Vec<&str> = version.split('.').collect();
    let semver = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !semver {
        return err(format!(
            "profile_version '{version}' is not valid semver (expected X.Y.Z)"
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

/// Read and fully validate a profile YAML file.
///
/// Fails on any malformed or unsupported input; a returned `Profile` is
/// structurally sound, so callers need not re-check it.
pub fn load_profile(path: &Path) -> Result<Profile, ProfileError> {
    let raw = load_raw(path, &mut Vec::new())?;

    let where_ = format!("profile {}", path.display());
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
    let raw_axes = match top.get(Value::String("axes".into())) {
        None => Vec::new(),
        Some(Value::Sequence(items)) => {
            let mut axes = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(axis) = item else {
                    return err(format!(
                        "{where_}: 'axes' must be a list of axis names, got {}. \
                         Axis values are target state and are read from the register, \
                         never declared here",
                        name(item)
                    ));
                };
                axes.push(axis.clone());
            }
            axes
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'axes' must be a list of axis names, got {}. \
                 Axis values are target state and are read from the register, \
                 never declared here",
                name(other)
            ));
        }
    };

    let mut validation_overrides: BTreeMap<String, Severity> = BTreeMap::new();
    let mut validation_configs: BTreeMap<String, Vec<ValidationConfig>> = BTreeMap::new();
    let mut axis_bindings: BTreeMap<String, AxisBinding> = BTreeMap::new();

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
                .chain(AXIS_BINDING_KEYS.iter())
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

            let stored: ValidationConfig = config
                .iter()
                .map(|(k, v)| (key_name(k), v.clone()))
                .collect();
            validation_configs
                .entry(code.clone())
                .or_default()
                .push(stored);

            let present: Vec<&str> = AXIS_BINDING_KEYS
                .iter()
                .copied()
                .filter(|k| config.contains_key(Value::String((*k).into())))
                .collect();
            if present.is_empty() {
                continue;
            }
            if present.len() != AXIS_BINDING_KEYS.len() {
                let missing: Vec<&str> = AXIS_BINDING_KEYS
                    .iter()
                    .copied()
                    .filter(|k| !present.contains(k))
                    .collect();
                return err(format!(
                    "validation config for '{code}': axis binding needs both \
                     {AXIS_BINDING_KEYS:?}, missing {missing:?}"
                ));
            }
            let Value::String(axis) = &config[Value::String("axis".into())] else {
                return err(format!(
                    "validation config for '{code}': 'axis' must be a string"
                ));
            };
            if !raw_axes.contains(axis) {
                let mut declared = raw_axes.clone();
                declared.sort();
                return err(format!(
                    "validation config for '{code}': undeclared axis '{axis}' \
                     (declared: {declared:?})"
                ));
            }
            if axis_bindings.contains_key(&code) {
                return err(format!(
                    "validation config for '{code}': a second axis binding. \
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
            axis_bindings.insert(
                code.clone(),
                AxisBinding {
                    axis: axis.clone(),
                    position_attr: position_attr.clone(),
                },
            );
        }
    }

    Ok(Profile {
        name: profile_name.clone(),
        profile_version,
        node_kinds,
        edge_kinds,
        validation_overrides,
        validation_configs,
        axes: raw_axes,
        axis_bindings,
        raw,
    })
}

//! Parsing node kinds, their attrs and actions, and edge kinds.

use std::collections::BTreeMap;

use regex::Regex;
use serde_norway::Value;

use crate::template::{Scope, parse_template};

use super::raw::{as_mapping, key_name, reject_unknown_keys, to_json};
use super::types::{ActionDecl, ActionMode, AttrSchema, AttrType, EdgeKind, NodeKind, Segment};
use super::{ProfileError, VALID_ATTR_TYPES, VALID_LIST_ITEM_TYPES, err, name};

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
    reject_unknown_keys(mapping, ATTR_KEYS, &where_)?;

    let kind = match mapping.get(Value::String("type".into())) {
        None | Some(Value::Null) => return err(format!("{where_}: missing 'type'")),
        Some(Value::String(text)) => AttrType::parse(text).ok_or_else(|| {
            ProfileError::Invalid(format!(
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

/// Build one node kind, compiling its required `id_pattern`.
pub(super) fn parse_node_kind(
    kind_name: &str,
    raw: &Value,
    declared_index: usize,
) -> Result<NodeKind, ProfileError> {
    let where_ = format!("node kind '{kind_name}'");
    let mapping = as_mapping(raw, &where_)?;
    reject_unknown_keys(mapping, NODE_KIND_KEYS, &where_)?;

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
        .map_err(|e| ProfileError::Invalid(format!("{where_}: invalid id_pattern: {e}")))?;

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

    // Which addresses `id_names` admits is the source-existence resolver's to
    // say, so the loader asks it rather than keeping its own list.
    if let Some(value) = mapping.get(Value::String("id_names".into())) {
        latt_adapter::validate_id_names(&to_json(value)?)
            .map_err(|error| ProfileError::Invalid(format!("{where_}: 'id_names' {error}")))?;
    }

    let actions = match mapping.get(Value::String("actions".into())) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Mapping(raw_actions)) => {
            let mut actions = Vec::with_capacity(raw_actions.len());
            for (verb_key, action_raw) in raw_actions {
                let Value::String(verb) = verb_key else {
                    return err(format!(
                        "{where_}: action names must be strings, got {}",
                        name(verb_key)
                    ));
                };
                if verb.is_empty() {
                    return err(format!("{where_}: an action name is empty"));
                }
                actions.push((
                    verb.clone(),
                    parse_action(&where_, verb, action_raw, &attrs)?,
                ));
            }
            actions
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'actions' must be a mapping, got {}",
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
        actions,
    })
}

/// Build one owner action, checking its placeholders against the kind's attrs.
fn parse_action(
    kind_where: &str,
    verb: &str,
    raw: &Value,
    attrs: &BTreeMap<String, AttrSchema>,
) -> Result<ActionDecl, ProfileError> {
    let where_ = format!("{kind_where}, action '{verb}'");
    let mapping = as_mapping(raw, &where_)?;
    reject_unknown_keys(mapping, ACTION_KEYS, &where_)?;

    let run = match mapping.get(Value::String("run".into())) {
        None | Some(Value::Null) => return err(format!("{where_}: missing 'run'")),
        Some(Value::Sequence(items)) if !items.is_empty() => {
            let mut run = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(text) = item else {
                    return err(format!(
                        "{where_}: 'run' entries must be strings, got {}",
                        name(item)
                    ));
                };
                run.push(parse_template(&where_, text, &Scope::action(attrs))?);
            }
            run
        }
        Some(Value::Sequence(_)) => {
            return err(format!("{where_}: 'run' is empty; it must name a program"));
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'run' must be a list of arguments, got {} \
                 (a shell string is not accepted)",
                name(other)
            ));
        }
    };
    if run[0]
        .segments
        .iter()
        .any(|s| !matches!(s, Segment::Literal(_)))
    {
        return err(format!(
            "{where_}: the program '{}' holds a placeholder; \
             the program must be written literally",
            run[0].source
        ));
    }

    let mode = match mapping.get(Value::String("mode".into())) {
        None | Some(Value::Null) => return err(format!("{where_}: missing 'mode'")),
        Some(Value::String(text)) => match text.as_str() {
            "interactive" => ActionMode::Interactive,
            "capture" => ActionMode::Capture,
            "detached" => ActionMode::Detached,
            _ => {
                return err(format!(
                    "{where_}: unknown mode '{text}' (valid: {ACTION_MODES:?})"
                ));
            }
        },
        Some(other) => {
            return err(format!(
                "{where_}: 'mode' must be a string, got {}",
                name(other)
            ));
        }
    };
    Ok(ActionDecl { run, mode })
}

/// Split one `run` argument into literal text and placeholders.
/// Build one edge kind, checking each allowed pair names a declared kind.
///
/// Endpoint kinds are resolved at load time so a typo in `allowed` fails on the
/// profile rather than silently admitting no edge at validation.
pub(super) fn parse_edge_kind(
    kind_name: &str,
    raw: &Value,
    valid_node_kinds: &BTreeMap<String, NodeKind>,
) -> Result<EdgeKind, ProfileError> {
    let where_ = format!("edge kind '{kind_name}'");
    let mapping = as_mapping(raw, &where_)?;
    reject_unknown_keys(mapping, EDGE_KIND_KEYS, &where_)?;

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

const NODE_KIND_KEYS: &[&str] = &[
    "attrs",
    "id_pattern",
    "actions",
    "id_names",
    "orphan_ok",
    "summary_attr",
    "text_attrs",
    "text_chunk_line_prefix",
];

const ATTR_KEYS: &[&str] = &["items", "required", "type", "values"];

const EDGE_KIND_KEYS: &[&str] = &["allowed", "cross_source"];

const ACTION_KEYS: &[&str] = &["mode", "run"];

const ACTION_MODES: &[&str] = &["interactive", "capture", "detached"];

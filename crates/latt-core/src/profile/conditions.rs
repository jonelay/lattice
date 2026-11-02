//! Parsing condition blocks and edge conditions, severity words and retired codes.

use regex::Regex;
use serde_norway::Value;

use crate::types::Severity;

use super::raw::{key_name, to_comparable, to_json};
use super::types::{Condition, ConditionOp, EdgeConditions};
use super::{ProfileError, err, name};

const VALID_CONDITION_OPS: &[&str] = &[
    "eq", "gt", "gte", "in", "lt", "lte", "matches", "ne", "present",
];

/// Parse a severity word, or the diagnostic for one lattice does not accept.
///
/// The diagnostic lists the accepted values and, for the lint-level words of
/// cargo-semver-checks and Rust, names lattice's spelling: `warn` is
/// `warning`, `deny` is `error`, and `allow` is a `SUPPRESS` entry, which
/// keeps the finding in `--format json` where an `allow` lint never runs.
/// Shared by the profile and fuse-profile loaders so both say the same thing.
pub(crate) fn parse_severity_word(text: &str, code: &str) -> Result<Severity, String> {
    if let Some(severity) = Severity::parse(text) {
        return Ok(severity);
    }
    let mut diagnostic = format!("invalid severity '{text}' (valid: error, hint, info, warning)");
    match text {
        "warn" => diagnostic.push_str("; lattice spells the lint level `warn` as `warning`"),
        "deny" => diagnostic.push_str("; lattice spells the lint level `deny` as `error`"),
        "allow" => diagnostic.push_str(&format!(
            "; there is no `allow`: write a `SUPPRESS: {{code: {code}}}` entry instead. A \
             suppressed finding is still computed and kept in `--format json`, where an \
             `allow` lint does not run"
        )),
        _ => {}
    }
    Err(diagnostic)
}

/// The refusal for a validation entry naming a retired code, or `None`.
///
/// An override naming a retired code would apply to nothing in silence, which
/// is the third invariant's failure mode in profile data.
pub(crate) fn retired_code_refusal(code: &str) -> Option<String> {
    crate::validate::retired_replacement(code).map(|(replacement, release)| {
        format!(
            "validation entry '{code}': the code was retired in {release}; \
             write {replacement} instead"
        )
    })
}

const EDGE_SELECTORS: &[&str] = &["source", "target", "edge"];

/// Parse an `EDGE_WHERE` block, whose keys name which end of the edge each
/// condition block reads rather than an attr.
pub(super) fn parse_edge_conditions(
    value: Option<&Value>,
    block_name: &str,
) -> Result<EdgeConditions, ProfileError> {
    let mut conditions = EdgeConditions::default();
    let Some(value) = value else {
        return Ok(conditions);
    };
    let Value::Mapping(map) = value else {
        return err(format!(
            "EDGE_WHERE config: '{block_name}' must be a mapping, got {}",
            name(value)
        ));
    };
    for (key, block) in map {
        let selector = key_name(key);
        let path = format!("{block_name}.{selector}");
        let slot = match selector.as_str() {
            "source" => &mut conditions.source,
            "target" => &mut conditions.target,
            "edge" => &mut conditions.edge,
            _ => {
                return err(format!(
                    "EDGE_WHERE config: '{path}' is not a selector (valid: {})",
                    EDGE_SELECTORS.join(", ")
                ));
            }
        };
        *slot = parse_condition_block(Some(block), "EDGE_WHERE", &path)?;
    }
    Ok(conditions)
}

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
        if op_name == "not" {
            return err(format!(
                "{config_name} config: '{block_name}.{attr}' uses the retired operator 'not'; \
                 write 'ne' (not equal, or absent)"
            ));
        }
        if !VALID_CONDITION_OPS.contains(&op_name.as_str()) {
            return err(format!(
                "{config_name} config: '{block_name}.{attr}' unknown operator '{op_name}' \
                 (valid: {VALID_CONDITION_OPS:?})"
            ));
        }
        let op = match op_name.as_str() {
            "eq" => ConditionOp::Eq(to_json(op_val)?),
            "ne" => ConditionOp::Ne(to_json(op_val)?),
            "lt" | "gt" | "lte" | "gte" => {
                let comparable = to_comparable(op_val).ok_or_else(|| {
                    ProfileError::Invalid(format!(
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
                    ProfileError::Invalid(format!(
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
        conditions.push(Condition::new(attr, op));
    }
    Ok(conditions)
}

//! Compiling the `validations:` list into the typed plan.

use std::collections::BTreeMap;

use serde_norway::Value;

use crate::template::{Scope, parse_template};
use crate::types::Severity;

use super::conditions::{
    parse_condition_block, parse_edge_conditions, parse_severity_word, retired_code_refusal,
};
use super::raw::{key_name, reject_unknown_keys};
use super::types::{
    Condition, ConstraintConfig, CountEnd, CoverageDeepSpec, CoverageSpec, EdgeCountConfig,
    EdgeKind, EdgeWhereConfig, NodeKind, PathwayBinding, PlanEntry, SummarySpec, SuppressConfig,
    Template, ValidationPlan, ValidationSpec,
};
use super::{PATHWAY_BINDING_KEYS, ProfileError, SUPPRESS_KEYS, config_keys, err, name};

/// A config key's value: `Ok(Some)` for a string, `Ok(None)` when the key is
/// absent, `Err(type name)` for a declared value of any other type.
///
/// Wrong type and absence are distinct on purpose. Reading a mistyped value
/// as "missing" (or stringifying it into a kind check) would report the wrong
/// fault, and a declared config must never fail in silence.
fn config_str<'c>(config: &'c EntryConfig, key: &str) -> Result<Option<&'c str>, &'static str> {
    match config.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(other) => Err(name(other)),
    }
}

/// Which declared-kind table a config key must name into.
#[derive(Clone, Copy)]
enum KindSpace {
    Node,
    Edge,
}

const COVERAGE_KIND_KEYS: [(&str, KindSpace); 2] = [
    ("target_kind", KindSpace::Node),
    ("edge_kind", KindSpace::Edge),
];
const COVERAGE_DEEP_KIND_KEYS: [(&str, KindSpace); 3] = [
    ("target_kind", KindSpace::Node),
    ("via", KindSpace::Edge),
    ("evidence", KindSpace::Edge),
];

/// Whether a profile's coverage entries are checked when it loads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kinds {
    /// A standard profile declares every kind it will see, so a coverage entry
    /// naming an undeclared one is a broken setup: a load error.
    Checked,
    /// The profile `fuse` composes has the kinds the sources' traces carried,
    /// so the validator reports the same faults as `CONFIG_ERROR` findings.
    Deferred,
}

/// Read a validation entry's kind-naming keys, collecting every fault.
///
/// Each key is typed and checked against the declared kinds independently of
/// its siblings; one missing or mistyped key never masks another's fault
/// (validation spec, config typing). Returns one slot per requested key,
/// `None` where the key was missing, mistyped, or named an undeclared kind,
/// beside the fault messages in key order, missing keys last.
fn kind_keys<'c, const N: usize>(
    code: &str,
    config: &'c EntryConfig,
    keys: [(&'static str, KindSpace); N],
    node_kinds: &BTreeMap<String, NodeKind>,
    edge_kinds: &BTreeMap<String, EdgeKind>,
) -> ([Option<&'c str>; N], Vec<String>) {
    let mut faults = Vec::new();
    let mut missing = Vec::new();
    let out = keys.map(|(key, space)| {
        let value = match config_str(config, key) {
            Ok(Some(value)) => value,
            Ok(None) => {
                missing.push(key);
                return None;
            }
            Err(got) => {
                faults.push(format!(
                    "{code} config: '{key}' must be a string, got {got}"
                ));
                return None;
            }
        };
        let (declared, noun) = match space {
            KindSpace::Node => (node_kinds.contains_key(value), "node"),
            KindSpace::Edge => (edge_kinds.contains_key(value), "edge"),
        };
        if !declared {
            faults.push(format!(
                "{code} config: {key} '{value}' not in profile {noun} kinds"
            ));
        }
        declared.then_some(value)
    });
    if !missing.is_empty() {
        faults.push(format!(
            "{code} config missing required keys: {}",
            missing.join(", ")
        ));
    }
    (out, faults)
}

/// One `validations:` entry between the per-entry checks and its compiled spec.
struct RawEntry {
    code: String,
    config: EntryConfig,
    severity: Option<Severity>,
    pathway: Option<PathwayBinding>,
    /// The coverage codes' `where` and `source_where`, parsed with the entry so
    /// a malformed block is reported in declaration order with the other
    /// per-entry faults.
    blocks: Option<(Vec<Condition>, Vec<Condition>)>,
    message: Option<Template>,
    spec: Option<ValidationSpec>,
    /// Faults in `message` or the pathway binding. They do not stop the entry,
    /// so its body (a `CONSTRAINT`'s condition blocks) is still checked and
    /// one refusal names both.
    faults: Vec<String>,
}
/// Compile every `validations:` entry into the plan, or return every fault.
///
/// Faults are collected, one per entry check, so a profile with three mistakes
/// is fixed in one edit (profile-schema spec, a refused profile names every
/// independent fault). Their order is the one the loader has always reported:
/// each entry's own checks in declaration order, then every coverage entry's
/// kind faults, then `CONSTRAINT`, `EDGE_WHERE` and `EDGE_COUNT` entries by
/// code. Under `Kinds::Deferred` a coverage entry's kind faults compile to a
/// `Faulty` entry instead of refusing the profile.
pub(super) fn compile_validations(
    raw_validations: &[Value],
    pathways: &[String],
    node_kinds: &BTreeMap<String, NodeKind>,
    edge_kinds: &BTreeMap<String, EdgeKind>,
    kinds: Kinds,
) -> Result<ValidationPlan, Vec<String>> {
    let mut plan = ValidationPlan::default();
    let mut raw_entries: Vec<RawEntry> = Vec::new();
    let mut faults: Vec<String> = Vec::new();

    for entry in raw_validations {
        let Value::Mapping(entry) = entry else {
            faults.push(format!(
                "validations entry must be a mapping, got {}",
                name(entry)
            ));
            continue;
        };
        for (code_key, config) in entry {
            match compile_entry(code_key, config, pathways, node_kinds, &mut plan) {
                Ok(mut raw) => {
                    faults.append(&mut raw.faults);
                    raw_entries.push(raw);
                }
                Err(fault) => faults.push(fault.to_string()),
            }
        }
    }

    let mut coverage_faults: Vec<String> = Vec::new();
    let mut deep_faults: Vec<String> = Vec::new();
    for entry in &mut raw_entries {
        let (where_, source_where) = entry.blocks.take().unwrap_or_default();
        match entry.code.as_str() {
            "COVERAGE" => {
                let (keys, entry_faults) = kind_keys(
                    "COVERAGE",
                    &entry.config,
                    COVERAGE_KIND_KEYS,
                    node_kinds,
                    edge_kinds,
                );
                entry.spec = Some(match keys {
                    [Some(target_kind), Some(edge_kind)] => {
                        ValidationSpec::Coverage(CoverageSpec {
                            target_kind: target_kind.to_string(),
                            edge_kind: edge_kind.to_string(),
                            where_,
                            source_where,
                        })
                    }
                    _ => {
                        coverage_faults.extend(entry_faults.iter().cloned());
                        ValidationSpec::Faulty(entry_faults)
                    }
                });
            }
            "COVERAGE_DEEP" => {
                let (keys, entry_faults) = kind_keys(
                    "COVERAGE_DEEP",
                    &entry.config,
                    COVERAGE_DEEP_KIND_KEYS,
                    node_kinds,
                    edge_kinds,
                );
                entry.spec = Some(match keys {
                    [Some(target_kind), Some(via), Some(evidence)] => {
                        ValidationSpec::CoverageDeep(CoverageDeepSpec {
                            target_kind: target_kind.to_string(),
                            via: via.to_string(),
                            evidence: evidence.to_string(),
                            where_,
                            source_where,
                        })
                    }
                    _ => {
                        deep_faults.extend(entry_faults.iter().cloned());
                        ValidationSpec::Faulty(entry_faults)
                    }
                });
            }
            "SUMMARY" => entry.spec = Some(compile_summary(&entry.config)),
            _ => {}
        }
    }
    if kinds == Kinds::Checked {
        faults.append(&mut coverage_faults);
        faults.append(&mut deep_faults);
    }

    for entry in raw_entries.iter_mut().filter(|e| e.code == "CONSTRAINT") {
        match parse_constraint(&entry.config, entry.severity, node_kinds) {
            Ok(config) => {
                plan.constraints.push(config);
                entry.spec = Some(ValidationSpec::Constraint(plan.constraints.len() - 1));
            }
            Err(fault) => faults.push(fault.to_string()),
        }
    }
    for entry in raw_entries.iter_mut().filter(|e| e.code == "EDGE_WHERE") {
        match parse_edge_where(&entry.config, entry.severity, edge_kinds) {
            Ok(config) => {
                plan.edge_wheres.push(config);
                entry.spec = Some(ValidationSpec::EdgeWhere(plan.edge_wheres.len() - 1));
            }
            Err(fault) => faults.push(fault.to_string()),
        }
    }
    for entry in raw_entries.iter_mut().filter(|e| e.code == "EDGE_COUNT") {
        match parse_edge_count(&entry.config, entry.severity, node_kinds, edge_kinds) {
            Ok(config) => {
                plan.edge_counts.push(config);
                entry.spec = Some(ValidationSpec::EdgeCount(plan.edge_counts.len() - 1));
            }
            Err(fault) => faults.push(fault.to_string()),
        }
    }
    if !faults.is_empty() {
        return Err(faults);
    }

    plan.entries = raw_entries
        .into_iter()
        .map(|entry| PlanEntry {
            code: entry.code,
            spec: entry.spec.unwrap_or(ValidationSpec::SeverityOnly),
            severity: entry.severity,
            pathway: entry.pathway,
            message: entry.message,
        })
        .collect();
    Ok(plan)
}

/// Run one entry's own checks: its code, severity, keys, condition blocks and
/// pathway binding. Suppressions, overrides and bindings land on the plan here;
/// the compiled spec comes after every entry has been seen.
fn compile_entry(
    code_key: &Value,
    config: &Value,
    pathways: &[String],
    node_kinds: &BTreeMap<String, NodeKind>,
    plan: &mut ValidationPlan,
) -> Result<RawEntry, ProfileError> {
    let code = key_name(code_key);
    if let Some(refusal) = retired_code_refusal(&code) {
        return err(refusal);
    }
    let Value::Mapping(config) = config else {
        return err(format!(
            "validation override for '{code}' must be a mapping, got {}",
            name(config)
        ));
    };

    if code == "SUPPRESS" {
        let parsed = parse_suppress(config)?;
        merge_suppression(&mut plan.suppressions, parsed.clone());
        return Ok(RawEntry {
            code,
            config: EntryConfig::new(),
            severity: None,
            pathway: None,
            blocks: None,
            message: None,
            spec: Some(ValidationSpec::Suppress(parsed)),
            faults: Vec::new(),
        });
    }

    // Last-write-wins, matching the Python core: a second `severity:` for
    // one code overwrites the first rather than being reported.
    let mut severity = None;
    if let Some(value) = config.get(Value::String("severity".into()))
        && !matches!(value, Value::Null)
    {
        let Value::String(text) = value else {
            return err(format!(
                "validation override for '{code}': 'severity' must be a string, \
                     got {}",
                name(value)
            ));
        };
        let parsed = parse_severity_word(text, &code).map_err(|diagnostic| {
            ProfileError::Invalid(format!("validation override for '{code}': {diagnostic}"))
        })?;
        plan.overrides.insert(code.clone(), parsed);
        severity = Some(parsed);
    }

    let mut allowed: Vec<&str> = config_keys(&code)
        .iter()
        .chain(PATHWAY_BINDING_KEYS.iter())
        .copied()
        .collect();
    allowed.sort_unstable();
    reject_unknown_keys(config, &allowed, &format!("validation config for '{code}'"))?;

    let blocks = if matches!(code.as_str(), "COVERAGE" | "COVERAGE_DEEP") {
        let where_ =
            parse_condition_block(config.get(Value::String("where".into())), &code, "where")?;
        let source_where = parse_condition_block(
            config.get(Value::String("source_where".into())),
            &code,
            "source_where",
        )?;
        Some((where_, source_where))
    } else {
        None
    };

    let stored: EntryConfig = config
        .iter()
        .map(|(k, v)| (key_name(k), v.clone()))
        .collect();

    let mut faults = Vec::new();
    let message = parse_entry_message(&stored, &code).and_then(|text| {
        text.map(|text| {
            parse_template(
                &format!("validation config for '{code}', message"),
                &text,
                &message_scope(&code, &stored, node_kinds),
            )
        })
        .transpose()
    });
    let message = message.unwrap_or_else(|fault| {
        faults.push(fault.to_string());
        None
    });

    let pathway = parse_pathway_binding(config, &code, pathways).unwrap_or_else(|fault| {
        faults.push(fault.to_string());
        None
    });
    if let Some(binding) = &pathway {
        if plan.pathway_bindings.contains_key(&code) {
            faults.push(format!(
                "validation config for '{code}': a second pathway binding. \
                 Repeated configurations are honoured, but two bindings give \
                 severity resolution two answers for one finding"
            ));
        } else {
            plan.pathway_bindings.insert(code.clone(), binding.clone());
        }
    }

    Ok(RawEntry {
        code,
        config: stored,
        severity,
        pathway,
        blocks,
        message,
        spec: None,
        faults,
    })
}

/// What an entry's `message` may name: the finding always; the node only when
/// every finding of the code carries one; attrs only when the entry names the
/// kind they belong to (validation spec, declared messages render placeholders).
fn message_scope<'a>(
    code: &str,
    config: &EntryConfig,
    node_kinds: &'a BTreeMap<String, NodeKind>,
) -> Scope<'a> {
    let node = if crate::validate::code_carries_node(code) {
        Ok(())
    } else {
        Err(format!("findings of '{code}' carry none"))
    };
    let kind_key = match code {
        "CONSTRAINT" => Some("kind"),
        "EDGE_COUNT" => Some("node_kind"),
        "COVERAGE" | "COVERAGE_DEEP" => Some("target_kind"),
        _ => None,
    };
    let attrs = match kind_key {
        None => Err(format!(
            "'{code}' names no node kind; only CONSTRAINT (kind), EDGE_COUNT (node_kind), \
             COVERAGE and COVERAGE_DEEP (target_kind) read attrs"
        )),
        Some(key) => match config_str(config, key) {
            Ok(Some(kind)) => node_kinds
                .get(kind)
                .map(|declared| &declared.attrs)
                .ok_or_else(|| format!("kind '{kind}' is not declared")),
            _ => Err(format!("'{key}' names no declared kind")),
        },
    };
    Scope {
        finding: true,
        node,
        location: false,
        attrs,
    }
}

/// Read an entry's pathway binding: both keys or neither, naming a declared pathway.
fn parse_pathway_binding(
    config: &serde_norway::Mapping,
    code: &str,
    pathways: &[String],
) -> Result<Option<PathwayBinding>, ProfileError> {
    let present: Vec<&str> = PATHWAY_BINDING_KEYS
        .iter()
        .copied()
        .filter(|k| config.contains_key(Value::String((*k).into())))
        .collect();
    if present.is_empty() {
        return Ok(None);
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
    if !pathways.contains(pathway) {
        let mut declared = pathways.to_vec();
        declared.sort();
        return err(format!(
            "validation config for '{code}': undeclared pathway '{pathway}' \
             (declared: {declared:?})"
        ));
    }
    let Value::String(position_attr) = &config[Value::String("position_attr".into())] else {
        return err(format!(
            "validation config for '{code}': 'position_attr' must be a string"
        ));
    };
    Ok(Some(PathwayBinding {
        pathway: pathway.clone(),
        position_attr: position_attr.clone(),
    }))
}

/// Compile a `SUMMARY` entry. A missing or mistyped key is not a load error:
/// `validate` runs without it and `summary` reports the first fault (cli spec,
/// summary rejects a mistyped config).
fn compile_summary(config: &EntryConfig) -> ValidationSpec {
    let required = |key: &str| match config_str(config, key) {
        Ok(Some(value)) => Ok(value.to_string()),
        Ok(None) => {
            Err("SUMMARY config requires node_kind, status_attr, group_by_attr".to_string())
        }
        // A mistyped value is not an absence: rolling up nothing at exit 0
        // would be silence about a config the profile declared.
        Err(got) => Err(format!(
            "SUMMARY config: '{key}' must be a string, got {got}"
        )),
    };
    let compiled = || -> Result<SummarySpec, String> {
        Ok(SummarySpec {
            node_kind: required("node_kind")?,
            status_attr: required("status_attr")?,
            group_by_attr: required("group_by_attr")?,
        })
    };
    match compiled() {
        Ok(spec) => ValidationSpec::Summary(spec),
        Err(fault) => ValidationSpec::Faulty(vec![fault]),
    }
}

fn parse_constraint(
    config: &EntryConfig,
    severity: Option<Severity>,
    node_kinds: &BTreeMap<String, NodeKind>,
) -> Result<ConstraintConfig, ProfileError> {
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
        return err("CONSTRAINT config: requires at least one of 'expect' or 'reject'".to_string());
    }
    let when = parse_condition_block(config.get("when"), "CONSTRAINT", "when")?;
    let expect = parse_condition_block(config.get("expect"), "CONSTRAINT", "expect")?;
    let reject = parse_condition_block(config.get("reject"), "CONSTRAINT", "reject")?;
    if expect.is_empty() && reject.is_empty() {
        return err(no_conditions("CONSTRAINT"));
    }
    let message = parse_entry_message(config, "CONSTRAINT")?;
    Ok(ConstraintConfig {
        kind,
        when,
        expect,
        reject,
        message,
        severity,
    })
}

fn parse_edge_where(
    config: &EntryConfig,
    severity: Option<Severity>,
    edge_kinds: &BTreeMap<String, EdgeKind>,
) -> Result<EdgeWhereConfig, ProfileError> {
    let edge_kind = match config.get("edge_kind") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return err(format!(
                "EDGE_WHERE config: 'edge_kind' must be a string, got {}",
                name(other)
            ));
        }
        None => {
            return err("EDGE_WHERE config missing required key: edge_kind".to_string());
        }
    };
    if !edge_kinds.contains_key(&edge_kind) {
        return err(format!(
            "EDGE_WHERE config: edge_kind '{edge_kind}' not in profile edge kinds"
        ));
    }
    if !config.contains_key("expect") && !config.contains_key("reject") {
        return err("EDGE_WHERE config: requires at least one of 'expect' or 'reject'".to_string());
    }
    let expect = parse_edge_conditions(config.get("expect"), "expect")?;
    let reject = parse_edge_conditions(config.get("reject"), "reject")?;
    if expect.is_empty() && reject.is_empty() {
        return err(no_conditions("EDGE_WHERE"));
    }
    Ok(EdgeWhereConfig {
        edge_kind,
        when: parse_edge_conditions(config.get("when"), "when")?,
        expect,
        reject,
        message: parse_entry_message(config, "EDGE_WHERE")?,
        severity,
    })
}

/// Parse one SUPPRESS entry, refusing the code that reports a broken profile:
/// silencing `CONFIG_ERROR` would defeat "unreadable input is reported, never
/// dropped".
pub(crate) fn parse_suppress(
    config: &serde_norway::Mapping,
) -> Result<SuppressConfig, ProfileError> {
    reject_unknown_keys(config, SUPPRESS_KEYS, "validation config for 'SUPPRESS'")?;
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

/// One validation entry's configuration as declared, between the per-entry
/// checks and its compiled spec.
type EntryConfig = BTreeMap<String, Value>;

/// Parse one `EDGE_COUNT` entry, rejecting a node kind no allowed pair puts
/// at the counted end: every such node would count zero, which reads as a
/// finding when it is a typo.
fn parse_edge_count(
    config: &EntryConfig,
    severity: Option<Severity>,
    node_kinds: &BTreeMap<String, NodeKind>,
    edge_kinds: &BTreeMap<String, EdgeKind>,
) -> Result<EdgeCountConfig, ProfileError> {
    let required = |key: &str| match config.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => err(format!(
            "EDGE_COUNT config: '{key}' must be a string, got {}",
            name(other)
        )),
        None => err(format!("EDGE_COUNT config missing required key: {key}")),
    };
    let edge_kind = required("edge_kind")?;
    let node_kind = required("node_kind")?;
    let Some(declared) = edge_kinds.get(&edge_kind) else {
        return err(format!(
            "EDGE_COUNT config: edge_kind '{edge_kind}' not in profile edge kinds"
        ));
    };
    if !node_kinds.contains_key(&node_kind) {
        return err(format!(
            "EDGE_COUNT config: node_kind '{node_kind}' not in profile node kinds"
        ));
    }
    let direction = match config.get("direction") {
        None => CountEnd::Out,
        Some(Value::String(s)) if s == "out" => CountEnd::Out,
        Some(Value::String(s)) if s == "in" => CountEnd::In,
        Some(other) => {
            let shown = other
                .as_str()
                .map_or_else(|| name(other).to_string(), str::to_owned);
            return err(format!(
                "EDGE_COUNT config: invalid direction '{shown}' (valid: out, in)"
            ));
        }
    };
    let at_counted_end = declared.allowed.iter().any(|(src, tgt)| match direction {
        CountEnd::Out => *src == node_kind,
        CountEnd::In => *tgt == node_kind,
    });
    if !at_counted_end {
        let (end, dir) = match direction {
            CountEnd::Out => ("source", "out"),
            CountEnd::In => ("target", "in"),
        };
        return err(format!(
            "EDGE_COUNT config: no allowed pair of edge kind '{edge_kind}' has node kind \
             '{node_kind}' as {end}, so direction {dir} would count zero for every node"
        ));
    }
    let bound = |key: &str| {
        config
            .get(key)
            .map(|v| {
                v.as_u64().ok_or_else(|| {
                    ProfileError::Invalid(format!(
                        "EDGE_COUNT config: '{key}' must be a non-negative integer"
                    ))
                })
            })
            .transpose()
    };
    let (min, max) = (bound("min")?, bound("max")?);
    match (min, max) {
        (None, None) => {
            return err("EDGE_COUNT config: requires at least one of 'min' or 'max'".to_string());
        }
        (Some(lo), Some(hi)) if lo > hi => {
            return err(format!("EDGE_COUNT config: 'min' {lo} exceeds 'max' {hi}"));
        }
        _ => {}
    }
    let group_by = match config.get("group_by") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => {
            return err(format!(
                "EDGE_COUNT config: 'group_by' must be a string, got {}",
                name(other)
            ));
        }
    };
    Ok(EdgeCountConfig {
        edge_kind,
        node_kind,
        direction,
        group_by,
        min,
        max,
        where_: parse_condition_block(config.get("where"), "EDGE_COUNT", "where")?,
        message: parse_entry_message(config, "EDGE_COUNT")?,
        severity,
    })
}

/// Read an entry's `message`; a value of any other type is an error rather
/// than a declared message quietly replaced by the generated one.
fn parse_entry_message(config: &EntryConfig, code: &str) -> Result<Option<String>, ProfileError> {
    match config.get("message") {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => err(format!(
            "{code} config: 'message' must be a string, got {}",
            name(other)
        )),
    }
}

/// The error for an entry whose blocks are present but empty: it would check
/// nothing and so report a clean register.
fn no_conditions(code: &str) -> String {
    format!("{code} config: 'expect' and 'reject' hold no conditions, so the entry checks nothing")
}

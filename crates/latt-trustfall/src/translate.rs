//! Validation-plan entries as Trustfall queries.
//!
//! Each check shape with a Trustfall spelling becomes one or more queries whose
//! results, unioned, should name the nodes lattice reports under the code. Where
//! Trustfall has no single spelling — a negated conjunction (`CONSTRAINT.expect`),
//! an ordering operator that lattice fails on an absent attr, the deep-coverage
//! fixpoint — several queries are combined in Rust, and each combination states
//! its set algebra, so the independence claim is honest about where
//! lattice-written code re-enters. A shape that cannot be translated at all is a
//! [`Translation::Skip`] with its reason, never a silent pass.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

use latt_core::profile::{
    AttrSchema, Comparable, Condition, ConditionOp, EdgeConditions, Profile, ValidationSpec,
};
use serde_json::Value;
use trustfall_core::ir::FieldValue;

use crate::schema::{attr_field, in_field, out_field, type_name};

/// One query and the output column holding the node ID it reports.
#[derive(Clone, Debug)]
pub struct Query {
    pub text: String,
    pub args: BTreeMap<Arc<str>, FieldValue>,
    pub output: &'static str,
}

/// The queries behind one deep-coverage entry; the fixpoint runs in Rust.
#[derive(Clone, Debug)]
pub struct DeepPlan {
    /// The `where`-matching target-kind nodes: `id`.
    pub targets: Query,
    /// Target-kind nodes with an admitted `evidence` edge in: `id`.
    pub evidenced: Query,
    /// `(parent, child)` over `via` edges between target-kind nodes, or `None`
    /// when `via == evidence`, which lattice reads as "no children".
    pub via_pairs: Option<Query>,
}

/// How an entry's findings are reproduced.
#[derive(Clone, Debug)]
pub enum Translation {
    /// The union of every query's output column.
    Union(Vec<Query>),
    /// Targets minus the least fixed point of covered nodes (see [`DeepPlan`]).
    Deep(DeepPlan),
    /// No translation; the reason names what Trustfall or the schema lacks.
    Skip(String),
}

/// One check and its translation.
#[derive(Clone, Debug)]
pub struct Translated {
    pub code: String,
    /// What the check is, for a report: the entry's shape and kinds.
    pub entry: String,
    pub translation: Translation,
}

/// A condition against one typed property, or a constant outcome when the
/// condition's constant cannot be compared with the property's type: lattice's
/// type-aware `eq` on an `int` attr never equals a string, so the condition
/// never holds, whatever the node.
#[derive(Clone, Debug)]
enum Clause {
    Filter {
        field: String,
        op: &'static str,
        arg: Option<FieldValue>,
    },
    Never,
    Always,
}

/// Every check the profile declares, translated.
#[must_use]
pub fn translations(profile: &Profile) -> Vec<Translated> {
    let mut out = Vec::new();
    let pairs: Vec<&(String, String)> = profile
        .edge_kinds()
        .values()
        .flat_map(|edge_kind| &edge_kind.allowed)
        .collect();
    for (kind, declared) in profile.node_kinds() {
        if declared.orphan_ok {
            continue;
        }
        let can_in = pairs.iter().any(|(_, tgt)| tgt == kind);
        let can_out = pairs.iter().any(|(src, _)| src == kind);
        for (code, field, possible, side) in [
            ("UNREFERENCED", "in_edges", can_in, "target"),
            ("UNTRACED", "out_edges", can_out, "source"),
        ] {
            // A direction no allowed pair permits is checked by asking nothing:
            // the empty union states the expected answer, so a validator that
            // still reported there would disagree rather than go unexamined.
            if !possible {
                out.push(Translated {
                    code: code.into(),
                    entry: format!("{code} over kind '{kind}' (no allowed pair has it as {side})"),
                    translation: Translation::Union(Vec::new()),
                });
                continue;
            }
            out.push(Translated {
                code: code.into(),
                entry: format!("{code} over kind '{kind}'"),
                translation: Translation::Union(vec![Query {
                    text: format!(
                        "{{ NodesOfKind(kind: \"{kind}\") {{ id @output {field} @fold \
                         @transform(op: \"count\") @filter(op: \"=\", value: [\"$zero\"]) }} }}"
                    ),
                    args: args([("zero", FieldValue::Int64(0))]),
                    output: "id",
                }]),
            });
        }
    }
    out.push(Translated {
        code: "VACANCY".into(),
        entry: "VACANCY over every edge".into(),
        translation: Translation::Union(vec![Query {
            text: "{ Edges { resolved @filter(op: \"=\", value: [\"$no\"]) source_id @output } }"
                .into(),
            args: args([("no", FieldValue::Boolean(false))]),
            output: "source_id",
        }]),
    });
    for entry in profile.validation_plan().entries() {
        let (entry_text, translation) = match &entry.spec {
            ValidationSpec::Coverage(spec) => (
                format!("COVERAGE {} <- {}", spec.target_kind, spec.edge_kind),
                coverage(
                    profile,
                    &spec.target_kind,
                    &spec.edge_kind,
                    &spec.where_,
                    &spec.source_where,
                ),
            ),
            ValidationSpec::CoverageDeep(spec) => (
                format!(
                    "COVERAGE_DEEP {} via {} evidence {}",
                    spec.target_kind, spec.via, spec.evidence
                ),
                deep(profile, spec),
            ),
            ValidationSpec::Constraint(index) => {
                let rule = &profile.constraint_configs()[*index];
                (
                    format!("CONSTRAINT on {}", rule.kind),
                    constraint(profile, rule),
                )
            }
            ValidationSpec::EdgeWhere(index) => {
                let rule = &profile.edge_where_configs()[*index];
                (
                    format!("EDGE_WHERE on {}", rule.edge_kind),
                    edge_where(profile, rule),
                )
            }
            ValidationSpec::EdgeCount(index) => {
                let rule = &profile.edge_count_configs()[*index];
                (
                    format!(
                        "EDGE_COUNT {} {:?} {}",
                        rule.node_kind, rule.direction, rule.edge_kind
                    ),
                    edge_count(profile, rule),
                )
            }
            _ => continue,
        };
        out.push(Translated {
            code: entry.code.clone(),
            entry: entry_text,
            translation,
        });
    }
    out
}

fn args<const N: usize>(pairs: [(&str, FieldValue); N]) -> BTreeMap<Arc<str>, FieldValue> {
    pairs
        .into_iter()
        .map(|(name, value)| (Arc::from(name), value))
        .collect()
}

fn str_value(text: &str) -> FieldValue {
    FieldValue::String(text.into())
}

/// The attrs a kind declares, or why a condition on it cannot be typed.
fn attrs_of<'p>(
    profile: &'p Profile,
    kind: &str,
) -> Result<&'p BTreeMap<String, AttrSchema>, String> {
    profile
        .node_kinds()
        .get(kind)
        .map(|declared| &declared.attrs)
        .ok_or_else(|| format!("kind '{kind}' is not declared"))
}

fn is_text(schema: &AttrSchema) -> bool {
    matches!(schema.kind.as_str(), "string" | "date" | "enum")
}

/// A JSON constant as the property's type, or `None` when the types differ and
/// lattice's type-aware comparison could never match.
fn constant(value: &Value, schema: &AttrSchema) -> Option<FieldValue> {
    match (schema.kind.as_str(), value) {
        ("int", Value::Number(n)) => n
            .as_i64()
            .map(FieldValue::Int64)
            .or_else(|| n.as_u64().map(FieldValue::Uint64)),
        ("bool", Value::Bool(b)) => Some(FieldValue::Boolean(*b)),
        ("string" | "date" | "enum", Value::String(s)) => Some(str_value(s)),
        _ => None,
    }
}

fn comparable(value: &Comparable, schema: &AttrSchema) -> Option<FieldValue> {
    match value {
        Comparable::Int(n) if schema.kind.as_str() == "int" => {
            i64::try_from(*n).ok().map(FieldValue::Int64)
        }
        Comparable::Str(s) if is_text(schema) => Some(str_value(s)),
        _ => None,
    }
}

/// The condition as lattice evaluates it when it must hold (`when`, `where`,
/// `reject`): one clause.
fn positive(cond: &Condition, attrs: &BTreeMap<String, AttrSchema>) -> Result<Clause, String> {
    let schema = attrs.get(cond.attr()).ok_or_else(|| {
        format!(
            "attr '{}' is not declared, so it has no typed property",
            cond.attr()
        )
    })?;
    let field = attr_field(cond.attr());
    let filter = |op: &'static str, arg: Option<FieldValue>| Clause::Filter {
        field: field.clone(),
        op,
        arg,
    };
    // A list attr compares as a whole; its spellings are rare enough that
    // the cross-check skips them rather than guess.
    if schema.kind.as_str() == "list" && !matches!(cond.op(), ConditionOp::Present(_)) {
        return Err(format!("attr '{}' is a list", cond.attr()));
    }
    Ok(match cond.op() {
        ConditionOp::Eq(v) => constant(v, schema).map_or(Clause::Never, |c| filter("=", Some(c))),
        // `ne` holds for an absent attr; Trustfall's `!=` holds for null too.
        ConditionOp::Ne(v) => constant(v, schema).map_or(Clause::Always, |c| filter("!=", Some(c))),
        ConditionOp::In(values) => {
            let typed: Vec<FieldValue> =
                values.iter().filter_map(|v| constant(v, schema)).collect();
            if typed.is_empty() {
                Clause::Never
            } else {
                filter("one_of", Some(FieldValue::List(typed.into())))
            }
        }
        ConditionOp::Lt(c) => comparable(c, schema).map_or(Clause::Never, |c| filter("<", Some(c))),
        ConditionOp::Gt(c) => comparable(c, schema).map_or(Clause::Never, |c| filter(">", Some(c))),
        ConditionOp::Lte(c) => {
            comparable(c, schema).map_or(Clause::Never, |c| filter("<=", Some(c)))
        }
        ConditionOp::Gte(c) => {
            comparable(c, schema).map_or(Clause::Never, |c| filter(">=", Some(c)))
        }
        ConditionOp::Matches(regex) => {
            if !is_text(schema) {
                return Err(format!(
                    "attr '{}' is not text; lattice matches its text form, which the typed property has not",
                    cond.attr()
                ));
            }
            filter("regex", Some(str_value(regex.as_str())))
        }
        ConditionOp::Present(true) => filter("is_not_null", None),
        ConditionOp::Present(false) => filter("is_null", None),
        other => return Err(format!("operator {other:?} has no translation")),
    })
}

/// The condition's failure (`expect`): a disjunction of clauses. An ordering
/// operator fails on an absent or mistyped attr too, which Trustfall's
/// comparison never says, so the negation is `<op>` ∪ `is_null`.
fn negated(cond: &Condition, attrs: &BTreeMap<String, AttrSchema>) -> Result<Vec<Clause>, String> {
    let schema = attrs.get(cond.attr()).ok_or_else(|| {
        format!(
            "attr '{}' is not declared, so it has no typed property",
            cond.attr()
        )
    })?;
    let field = attr_field(cond.attr());
    let filter = |op: &'static str, arg: Option<FieldValue>| Clause::Filter {
        field: field.clone(),
        op,
        arg,
    };
    if schema.kind.as_str() == "list" && !matches!(cond.op(), ConditionOp::Present(_)) {
        return Err(format!("attr '{}' is a list", cond.attr()));
    }
    let ordered = |op: &'static str, c: &Comparable| {
        comparable(c, schema).map_or(vec![Clause::Always], |c| {
            vec![filter(op, Some(c)), filter("is_null", None)]
        })
    };
    Ok(match cond.op() {
        ConditionOp::Eq(v) => {
            vec![constant(v, schema).map_or(Clause::Always, |c| filter("!=", Some(c)))]
        }
        ConditionOp::Ne(v) => {
            vec![constant(v, schema).map_or(Clause::Never, |c| filter("=", Some(c)))]
        }
        ConditionOp::In(values) => {
            let typed: Vec<FieldValue> =
                values.iter().filter_map(|v| constant(v, schema)).collect();
            if typed.is_empty() {
                vec![Clause::Always]
            } else {
                vec![filter("not_one_of", Some(FieldValue::List(typed.into())))]
            }
        }
        ConditionOp::Lt(c) => ordered(">=", c),
        ConditionOp::Gt(c) => ordered("<=", c),
        ConditionOp::Lte(c) => ordered(">", c),
        ConditionOp::Gte(c) => ordered("<", c),
        ConditionOp::Matches(regex) => {
            if !is_text(schema) {
                return Err(format!(
                    "attr '{}' is not text; lattice matches its text form, which the typed property has not",
                    cond.attr()
                ));
            }
            vec![filter("not_regex", Some(str_value(regex.as_str())))]
        }
        ConditionOp::Present(true) => vec![filter("is_null", None)],
        ConditionOp::Present(false) => vec![filter("is_not_null", None)],
        other => return Err(format!("operator {other:?} has no translation")),
    })
}

/// A query under construction: filter lines and the arguments they name.
#[derive(Clone, Default)]
struct Builder {
    lines: Vec<String>,
    args: BTreeMap<Arc<str>, FieldValue>,
}

impl Builder {
    fn filter(&mut self, clause: &Clause) {
        let Clause::Filter { field, op, arg } = clause else {
            return;
        };
        match arg {
            None => self.lines.push(format!("{field} @filter(op: \"{op}\")")),
            Some(value) => {
                let name = format!("v{}", self.args.len());
                self.lines.push(format!(
                    "{field} @filter(op: \"{op}\", value: [\"${name}\"])"
                ));
                self.args.insert(name.into(), value.clone());
            }
        }
    }

    fn arg(&mut self, name: &str, value: FieldValue) {
        self.args.insert(name.into(), value);
    }
}

/// `clauses` conjoined as filters; `None` when one is `Never`.
fn conjoin(builder: &mut Builder, clauses: &[Clause]) -> Option<()> {
    for clause in clauses {
        match clause {
            Clause::Never => return None,
            Clause::Always => {}
            filter => builder.filter(filter),
        }
    }
    Some(())
}

fn constraint(profile: &Profile, rule: &latt_core::profile::ConstraintConfig) -> Translation {
    let attrs = match attrs_of(profile, &rule.kind) {
        Ok(attrs) => attrs,
        Err(reason) => return Translation::Skip(reason),
    };
    let mut base = Builder::default();
    let when: Result<Vec<Clause>, String> = rule.when.iter().map(|c| positive(c, attrs)).collect();
    let when = match when {
        Ok(when) => when,
        Err(reason) => return Translation::Skip(format!("when: {reason}")),
    };
    // A `when` that can never hold skips every node: no findings.
    if conjoin(&mut base, &when).is_none() {
        return Translation::Union(Vec::new());
    }
    // Findings = when ∧ (¬e1 ∨ ¬e2 ∨ … ∨ r1 ∨ r2 ∨ …): one query per disjunct.
    let mut alternatives: Vec<Clause> = Vec::new();
    for cond in &rule.expect {
        match negated(cond, attrs) {
            Ok(clauses) => alternatives.extend(clauses),
            Err(reason) => return Translation::Skip(format!("expect: {reason}")),
        }
    }
    for cond in &rule.reject {
        match positive(cond, attrs) {
            Ok(clause) => alternatives.push(clause),
            Err(reason) => return Translation::Skip(format!("reject: {reason}")),
        }
    }
    let mut queries = Vec::new();
    for alternative in alternatives {
        let mut builder = base.clone();
        if conjoin(&mut builder, &[alternative]).is_none() {
            continue;
        }
        queries.push(node_query(&rule.kind, &builder, ""));
    }
    Translation::Union(queries)
}

/// `{ NodesOfKind(kind: "<kind>") { ... on K { id @output <lines> <tail> } } }`.
fn node_query(kind: &str, builder: &Builder, tail: &str) -> Query {
    let mut text = format!(
        "{{ NodesOfKind(kind: \"{kind}\") {{ ... on {} {{ id @output ",
        type_name(kind)
    );
    for line in &builder.lines {
        let _ = write!(text, "{line} ");
    }
    text.push_str(tail);
    text.push_str("} } }");
    Query {
        text,
        args: builder.args.clone(),
        output: "id",
    }
}

/// The one kind at one end of an edge kind's allowed pairs whose other end is
/// `other`, or why the conditions on that end cannot be typed.
fn single_kind(
    profile: &Profile,
    edge_kind: &str,
    source: bool,
    other: Option<&str>,
) -> Result<String, String> {
    let declared = profile
        .edge_kinds()
        .get(edge_kind)
        .ok_or_else(|| format!("edge kind '{edge_kind}' is not declared"))?;
    let kinds: BTreeSet<&str> = declared
        .allowed
        .iter()
        .filter(|(src, tgt)| other.is_none_or(|o| if source { tgt == o } else { src == o }))
        .map(|(src, tgt)| if source { src.as_str() } else { tgt.as_str() })
        .collect();
    match kinds.len() {
        1 => Ok(kinds.into_iter().next().unwrap_or_default().to_owned()),
        n => Err(format!(
            "edge kind '{edge_kind}' allows {n} {} kinds, and typed properties need one",
            if source { "source" } else { "target" }
        )),
    }
}

/// A fold over an edge field counting the admitted far ends, or `None` when
/// `source_where` can never admit one (then the count is zero by construction).
fn coverage_fold(
    profile: &Profile,
    field: &str,
    edge_kind: &str,
    target: &str,
    source_where: &[Condition],
    builder: &mut Builder,
    op: &str,
) -> Result<Option<String>, String> {
    let mut inner = String::new();
    if !source_where.is_empty() {
        let source_kind = single_kind(profile, edge_kind, true, Some(target))?;
        let attrs = attrs_of(profile, &source_kind)?;
        let mut filters = Builder {
            args: builder.args.clone(),
            ..Default::default()
        };
        for cond in source_where {
            match positive(cond, attrs)? {
                Clause::Never => return Ok(None),
                Clause::Always => {}
                clause => filters.filter(&clause),
            }
        }
        builder.args = filters.args;
        inner = format!(
            "... on {} {{ {} }}",
            type_name(&source_kind),
            filters.lines.join(" ")
        );
    }
    builder.arg("zero", FieldValue::Int64(0));
    let selection = if inner.is_empty() {
        String::new()
    } else {
        format!(" {{ {inner} }}")
    };
    Ok(Some(format!(
        "{field} @fold @transform(op: \"count\") @filter(op: \"{op}\", value: [\"$zero\"]){selection}"
    )))
}

fn coverage(
    profile: &Profile,
    target: &str,
    edge_kind: &str,
    where_: &[Condition],
    source_where: &[Condition],
) -> Translation {
    let attrs = match attrs_of(profile, target) {
        Ok(attrs) => attrs,
        Err(reason) => return Translation::Skip(reason),
    };
    let mut builder = Builder::default();
    let where_: Result<Vec<Clause>, String> = where_.iter().map(|c| positive(c, attrs)).collect();
    match where_ {
        Ok(clauses) => {
            if conjoin(&mut builder, &clauses).is_none() {
                return Translation::Union(Vec::new());
            }
        }
        Err(reason) => return Translation::Skip(format!("where: {reason}")),
    }
    match coverage_fold(
        profile,
        &in_field(edge_kind),
        edge_kind,
        target,
        source_where,
        &mut builder,
        "=",
    ) {
        // No source is ever admitted: every where-matching target is uncovered.
        Ok(None) => Translation::Union(vec![node_query(target, &builder, "")]),
        Ok(Some(fold)) => Translation::Union(vec![node_query(target, &builder, &fold)]),
        Err(reason) => Translation::Skip(format!("source_where: {reason}")),
    }
}

fn deep(profile: &Profile, spec: &latt_core::profile::CoverageDeepSpec) -> Translation {
    let target = spec.target_kind.as_str();
    let attrs = match attrs_of(profile, target) {
        Ok(attrs) => attrs,
        Err(reason) => return Translation::Skip(reason),
    };
    let mut targets = Builder::default();
    let where_: Result<Vec<Clause>, String> =
        spec.where_.iter().map(|c| positive(c, attrs)).collect();
    match where_ {
        Ok(clauses) => {
            if conjoin(&mut targets, &clauses).is_none() {
                return Translation::Union(Vec::new());
            }
        }
        Err(reason) => return Translation::Skip(format!("where: {reason}")),
    }
    let mut evidenced = Builder::default();
    let evidenced = match coverage_fold(
        profile,
        &in_field(&spec.evidence),
        &spec.evidence,
        target,
        &spec.source_where,
        &mut evidenced,
        ">",
    ) {
        Ok(Some(fold)) => node_query(target, &evidenced, &fold),
        // Nothing is ever evidenced: the query returns no node.
        Ok(None) => {
            let mut none = Builder::default();
            none.arg("zero", FieldValue::Int64(0));
            node_query(
                target,
                &none,
                "in_edges @fold @transform(op: \"count\") @filter(op: \"<\", value: [\"$zero\"])",
            )
        }
        Err(reason) => return Translation::Skip(format!("source_where: {reason}")),
    };
    let via_pairs = (spec.via != spec.evidence).then(|| Query {
        text: format!(
            "{{ NodesOfKind(kind: \"{kind}\") {{ ... on {t} {{ id @output(name: \"parent\") {via} {{ ... on {t} {{ id @output(name: \"child\") }} }} }} }} }}",
            t = type_name(target),
            kind = target,
            via = in_field(&spec.via)
        ),
        args: BTreeMap::new(),
        output: "parent",
    });
    Translation::Deep(DeepPlan {
        targets: node_query(target, &targets, ""),
        evidenced,
        via_pairs,
    })
}

fn edge_count(profile: &Profile, rule: &latt_core::profile::EdgeCountConfig) -> Translation {
    if rule.group_by.is_some() {
        return Translation::Skip(
            "group_by: a count per far-end value has no fold spelling".into(),
        );
    }
    let attrs = match attrs_of(profile, &rule.node_kind) {
        Ok(attrs) => attrs,
        Err(reason) => return Translation::Skip(reason),
    };
    let mut base = Builder::default();
    let where_: Result<Vec<Clause>, String> =
        rule.where_.iter().map(|c| positive(c, attrs)).collect();
    match where_ {
        Ok(clauses) => {
            if conjoin(&mut base, &clauses).is_none() {
                return Translation::Union(Vec::new());
            }
        }
        Err(reason) => return Translation::Skip(format!("where: {reason}")),
    }
    let field = match rule.direction {
        latt_core::profile::CountEnd::Out => out_field(&rule.edge_kind),
        latt_core::profile::CountEnd::In => in_field(&rule.edge_kind),
        #[allow(unreachable_patterns)]
        _ => return Translation::Skip("unknown direction".into()),
    };
    // Findings = (count < min) ∪ (count > max).
    let mut queries = Vec::new();
    for (bound, op) in [(rule.min, "<"), (rule.max, ">")] {
        let Some(bound) = bound else { continue };
        let mut builder = base.clone();
        builder.arg("bound", FieldValue::Uint64(bound));
        let fold = format!(
            "{field} @fold @transform(op: \"count\") @filter(op: \"{op}\", value: [\"$bound\"])"
        );
        queries.push(node_query(&rule.node_kind, &builder, &fold));
    }
    Translation::Union(queries)
}

fn edge_where(profile: &Profile, rule: &latt_core::profile::EdgeWhereConfig) -> Translation {
    let ends = |conds: &EdgeConditions| !conds.edge.is_empty();
    if ends(&rule.when) || ends(&rule.expect) || ends(&rule.reject) {
        return Translation::Skip("conditions on edge attrs: edges declare no attr schema, so EdgeRecord has no typed property".into());
    }
    let needs_source = !rule.when.source.is_empty()
        || !rule.expect.source.is_empty()
        || !rule.reject.source.is_empty();
    let needs_target = !rule.when.target.is_empty()
        || !rule.expect.target.is_empty()
        || !rule.reject.target.is_empty();
    let source_kind = if needs_source {
        match single_kind(profile, &rule.edge_kind, true, None) {
            Ok(kind) => Some(kind),
            Err(reason) => return Translation::Skip(reason),
        }
    } else {
        None
    };
    let target_kind = if needs_target {
        match single_kind(profile, &rule.edge_kind, false, None) {
            Ok(kind) => Some(kind),
            Err(reason) => return Translation::Skip(reason),
        }
    } else {
        None
    };
    let attrs_for = |kind: &Option<String>| -> Result<&BTreeMap<String, AttrSchema>, String> {
        attrs_of(profile, kind.as_deref().unwrap_or_default())
    };
    // (end, clause) pairs: `when` conjoined, then one query per disjunct.
    let mut when: Vec<(bool, Clause)> = Vec::new();
    let mut alternatives: Vec<(bool, Clause)> = Vec::new();
    let block = |conds: &EdgeConditions,
                 out: &mut Vec<(bool, Clause)>,
                 negate: bool|
     -> Result<(), String> {
        for (source, list) in [(true, &conds.source), (false, &conds.target)] {
            let attrs = if list.is_empty() {
                continue;
            } else {
                attrs_for(if source { &source_kind } else { &target_kind })?
            };
            for cond in list {
                if negate {
                    out.extend(negated(cond, attrs)?.into_iter().map(|c| (source, c)));
                } else {
                    out.push((source, positive(cond, attrs)?));
                }
            }
        }
        Ok(())
    };
    if let Err(reason) = block(&rule.when, &mut when, false) {
        return Translation::Skip(format!("when: {reason}"));
    }
    if when.iter().any(|(_, c)| matches!(c, Clause::Never)) {
        return Translation::Union(Vec::new());
    }
    if let Err(reason) = block(&rule.expect, &mut alternatives, true) {
        return Translation::Skip(format!("expect: {reason}"));
    }
    if let Err(reason) = block(&rule.reject, &mut alternatives, false) {
        return Translation::Skip(format!("reject: {reason}"));
    }
    let mut queries = Vec::new();
    for alternative in alternatives {
        if matches!(alternative.1, Clause::Never) {
            continue;
        }
        let mut source = Builder::default();
        let mut target = Builder::default();
        let mut shared = Builder::default();
        shared.arg("edge", str_value(&rule.edge_kind));
        for (is_source, clause) in when.iter().chain(std::iter::once(&alternative)) {
            let end = if *is_source { &mut source } else { &mut target };
            // One argument namespace across both ends.
            end.args = shared.args.clone();
            end.filter(clause);
            shared.args = end.args.clone();
        }
        let coerce = |kind: &Option<String>, builder: &Builder, name: &str| match kind {
            Some(kind) => format!(
                "{name} {{ ... on {} {{ id @output(name: \"{name}_node\") {} }} }}",
                type_name(kind),
                builder.lines.join(" ")
            ),
            None => format!("{name} {{ id @output(name: \"{name}_node\") }}"),
        };
        let text = format!(
            "{{ Edges {{ kind @filter(op: \"=\", value: [\"$edge\"]) source_id @output {} {} }} }}",
            coerce(&source_kind, &source, "source"),
            coerce(&target_kind, &target, "target")
        );
        queries.push(Query {
            text,
            args: shared.args,
            output: "source_id",
        });
    }
    Translation::Union(queries)
}

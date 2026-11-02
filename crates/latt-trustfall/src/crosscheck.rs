//! Run lattice and the translated queries over one graph and compare.
//!
//! Lattice's side is `validate` with no `--strict`, suppressed findings
//! included: suppression is lattice-only state and says nothing about whether
//! the check fired. The query side is the union over every translation of a
//! code. A code is compared only when every entry declaring it translated; one
//! skipped entry skips the code, with the reason reported, because a partial
//! union would read as lattice over-reporting.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use latt_core::graph::LatticeGraph;
use latt_core::profile::Profile;
use latt_core::validate::validate;
use trustfall_core::frontend::parse;
use trustfall_core::interpreter::execution::interpret_ir;
use trustfall_core::schema::Schema;

use crate::adapter::LatticeAdapter;
use crate::deliberate::{Deliberate, explained_by};
use crate::schema::schema_text;
use crate::translate::{DeepPlan, Query, Translation, translations};

/// One code's disagreement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub code: String,
    pub entries: Vec<String>,
    pub only_lattice: BTreeSet<String>,
    pub only_query: BTreeSet<String>,
}

/// A disagreement a listed deliberate difference accounts for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explained {
    pub code: String,
    pub node_id: String,
    pub rule: &'static str,
}

/// What one run compared, skipped and found.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub compared: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub mismatches: Vec<Mismatch>,
    pub explained: Vec<Explained>,
}

impl Report {
    /// True when nothing disagrees beyond the listed deliberate differences.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// Run every query and collect its output column.
fn run(schema: &Schema, adapter: &Arc<LatticeAdapter<'_>>, query: &Query) -> BTreeSet<String> {
    let indexed = parse(schema, &query.text).unwrap_or_else(|e| panic!("{}\n{e}", query.text));
    interpret_ir(Arc::clone(adapter), indexed, Arc::new(query.args.clone()))
        .unwrap_or_else(|e| panic!("{}\n{e}", query.text))
        .map(|row| {
            row[query.output]
                .as_str()
                .unwrap_or_else(|| panic!("{}: {} is not a string", query.text, query.output))
                .to_owned()
        })
        .collect()
}

/// Targets minus the least fixed point: seed with the evidenced targets, then
/// a parent is covered once every `via` child is. Set algebra in Rust, as the
/// design licenses; the sets themselves come from Trustfall.
fn deep(schema: &Schema, adapter: &Arc<LatticeAdapter<'_>>, plan: &DeepPlan) -> BTreeSet<String> {
    let targets = run(schema, adapter, &plan.targets);
    let mut covered: HashSet<String> = run(schema, adapter, &plan.evidenced).into_iter().collect();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(pairs) = &plan.via_pairs {
        let indexed = parse(schema, &pairs.text).unwrap_or_else(|e| panic!("{}\n{e}", pairs.text));
        for row in interpret_ir(Arc::clone(adapter), indexed, Arc::new(pairs.args.clone()))
            .unwrap_or_else(|e| panic!("{}\n{e}", pairs.text))
        {
            let parent = row["parent"].as_str().expect("a string").to_owned();
            let child = row["child"].as_str().expect("a string").to_owned();
            children.entry(parent).or_default().push(child);
        }
    }
    loop {
        let newly: Vec<String> = children
            .iter()
            .filter(|(parent, kids)| {
                !covered.contains(*parent) && kids.iter().all(|k| covered.contains(k))
            })
            .map(|(parent, _)| parent.clone())
            .collect();
        if newly.is_empty() {
            break;
        }
        covered.extend(newly);
    }
    targets
        .into_iter()
        .filter(|t| !covered.contains(t))
        .collect()
}

/// Per code: the entries declaring it, the union of their query results, and the
/// first skip reason, when one entry had no translation.
type CodeSlot = (Vec<String>, BTreeSet<String>, Option<String>);

/// Compare lattice and the translations over `graph` under `profile`.
#[must_use]
pub fn crosscheck(profile: &Profile, graph: &LatticeGraph) -> Report {
    let schema = Schema::parse(schema_text(profile).expect("the schema renders"))
        .expect("the schema parses");
    let adapter = Arc::new(LatticeAdapter::new(graph, profile));

    let mut lattice: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let issues = validate(graph, profile, false);
    for issue in &issues {
        if let Some(id) = &issue.node_id {
            lattice
                .entry(issue.code.as_str())
                .or_default()
                .insert(id.clone());
        }
    }

    let mut by_code: BTreeMap<String, CodeSlot> = BTreeMap::new();
    for translated in translations(profile) {
        let slot = by_code.entry(translated.code.clone()).or_default();
        slot.0.push(translated.entry.clone());
        match translated.translation {
            Translation::Skip(reason) => {
                slot.2
                    .get_or_insert_with(|| format!("{}: {reason}", translated.entry));
            }
            Translation::Union(queries) => {
                for query in &queries {
                    slot.1.extend(run(&schema, &adapter, query));
                }
            }
            Translation::Deep(plan) => slot.1.extend(deep(&schema, &adapter, &plan)),
        }
    }

    let mut report = Report::default();
    for (code, (entries, from_query, skipped)) in by_code {
        if let Some(reason) = skipped {
            report.skipped.push((code, reason));
            continue;
        }
        let from_lattice = lattice.remove(code.as_str()).unwrap_or_default();
        report.compared.push(code.clone());
        let mut mismatch = Mismatch {
            code: code.clone(),
            entries,
            only_lattice: from_lattice.difference(&from_query).cloned().collect(),
            only_query: from_query.difference(&from_lattice).cloned().collect(),
        };
        for side in [true, false] {
            let set = if side {
                &mut mismatch.only_lattice
            } else {
                &mut mismatch.only_query
            };
            let kept: BTreeSet<String> = set
                .iter()
                .filter(
                    |id| match explained_by(profile, graph, &issues, &code, id, side) {
                        Some(Deliberate { rule, .. }) => {
                            report.explained.push(Explained {
                                code: code.clone(),
                                node_id: (*id).clone(),
                                rule,
                            });
                            false
                        }
                        None => true,
                    },
                )
                .cloned()
                .collect();
            *set = kept;
        }
        if !mismatch.only_lattice.is_empty() || !mismatch.only_query.is_empty() {
            report.mismatches.push(mismatch);
        }
    }
    report
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} ({})", self.code, self.entries.join("; "))?;
        if !self.only_lattice.is_empty() {
            writeln!(f, "  only lattice: {:?}", self.only_lattice)?;
        }
        if !self.only_query.is_empty() {
            writeln!(f, "  only query:   {:?}", self.only_query)?;
        }
        Ok(())
    }
}

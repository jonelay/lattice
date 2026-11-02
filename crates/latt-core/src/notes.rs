//! Finding notes: matching the profile's notes-file entries to findings by code
//! and node, and reporting the entries that are malformed, stale or expired.

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroU32;

use serde_norway::Value;

use crate::graph::LatticeGraph;
use crate::profile::{NoteItem, Profile};
use crate::types::{CivilDate, Issue, Note, Provenance, Until};
use crate::validate::{FindingCode, issue};

const ENTRY_KEYS: &[&str] = &["at", "by", "finding", "note", "until"];
const FINDING_KEYS: &[&str] = &["code", "node"];
const UNTIL_KEYS: &[&str] = &["pathway", "position"];

/// One entry that passed the schema.
struct Entry {
    code: String,
    node: String,
    note: Note,
    line: Option<NonZeroU32>,
}

/// Attach each valid entry's note to the findings it matches, and append the
/// `NOTE_*` findings. A no-op for a profile without `annotations`.
pub(crate) fn attach(
    issues: &mut Vec<Issue>,
    graph: &LatticeGraph,
    profile: &Profile,
    today: CivilDate,
) {
    let Some(notes) = profile.notes() else {
        return;
    };
    let file = notes.path.display().to_string();
    let at = |line: Option<NonZeroU32>| Provenance::at_line(file.clone(), line);
    let mut out = Vec::new();
    let Some(items) = &notes.items else {
        out.push(issue(
            FindingCode::NoteFileMissing,
            format!("notes file {file} does not exist"),
            at(None),
            profile,
            None,
        ));
        issues.extend(out);
        return;
    };

    // Each entry names one (code, node) pair; finding the findings it covers by
    // lookup rather than by scanning them all keeps a long notes file from
    // costing notes × findings comparisons on every run.
    let mut by_key: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (at, finding) in issues.iter().enumerate() {
        if let Some(node) = finding.node_id.as_deref() {
            by_key.entry((&finding.code, node)).or_default().push(at);
        }
    }
    let mut matched_at: Vec<(usize, Note)> = Vec::new();

    let mut first_line: BTreeMap<(String, String), Option<NonZeroU32>> = BTreeMap::new();
    for item in items {
        let entry = match parse(item, graph, profile, today) {
            Ok(entry) => entry,
            Err(fault) => {
                out.push(invalid(fault, at(item.line), profile));
                continue;
            }
        };
        let key = (entry.code.clone(), entry.node.clone());
        if let Some(first) = first_line.get(&key) {
            let at_first = first.map_or_else(String::new, |line| format!(" at line {line}"));
            out.push(invalid(
                format!(
                    "duplicates the entry for {} on '{}'{at_first}",
                    entry.code, entry.node
                ),
                at(entry.line),
                profile,
            ));
            continue;
        }
        first_line.insert(key, entry.line);

        let positions = by_key
            .get(&(entry.code.as_str(), entry.node.as_str()))
            .map_or(&[][..], Vec::as_slice);
        let matched = !positions.is_empty();
        matched_at.extend(positions.iter().map(|&at| (at, entry.note.clone())));
        let subject = format!("note on {} for '{}'", entry.code, entry.node);
        if !matched {
            out.push(issue(
                FindingCode::NoteUnused,
                format!("{subject} matched no finding"),
                at(entry.line),
                profile,
                None,
            ));
        } else if entry.note.expired {
            let until = entry
                .note
                .until
                .as_ref()
                .map_or_else(String::new, until_text);
            out.push(issue(
                FindingCode::NoteExpired,
                format!("{subject} has expired: until {until}"),
                at(entry.line),
                profile,
                None,
            ));
        }
    }
    for (at, note) in matched_at {
        issues[at].note = Some(note);
    }
    issues.extend(out);
}

fn invalid(fault: String, provenance: Provenance, profile: &Profile) -> Issue {
    issue(
        FindingCode::NoteInvalid,
        format!("notes entry: {fault}"),
        provenance,
        profile,
        None,
    )
}

/// The `until` value as the notes file spells it.
pub(crate) fn until_text(until: &Until) -> String {
    match until {
        Until::Date(date) => date.to_string(),
        Until::Pathway { pathway, position } => format!("{pathway} {position}"),
    }
}

/// Check one item against the entry schema and decide whether it has expired.
/// The error is the fault, worded to follow "notes entry: ".
fn parse(
    item: &NoteItem,
    graph: &LatticeGraph,
    profile: &Profile,
    today: CivilDate,
) -> Result<Entry, String> {
    let Value::Mapping(map) = &item.value else {
        return Err("expected a mapping with 'finding' and 'note'".into());
    };
    known_keys(map, ENTRY_KEYS, "")?;

    let finding = match map.get("finding") {
        Some(Value::Mapping(finding)) => finding,
        Some(_) => return Err("'finding' must be a mapping of 'code' and 'node'".into()),
        None => return Err("missing required key 'finding'".into()),
    };
    known_keys(finding, FINDING_KEYS, "finding.")?;
    let code = string(finding, "code", "finding.")?.ok_or("missing required key 'finding.code'")?;
    let node = string(finding, "node", "finding.")?.ok_or("missing required key 'finding.node'")?;

    let text = string(map, "note", "")?.ok_or("missing required key 'note'")?;
    if text.trim().is_empty() {
        return Err("'note' must not be empty".into());
    }
    let by = string(map, "by", "")?;
    let at = string(map, "at", "")?
        .map(|value| date(&value, "at"))
        .transpose()?;

    let (until, expired) = match map.get("until") {
        None => (None, false),
        Some(Value::String(value)) => {
            let until = date(value, "until")?;
            (Some(Until::Date(until)), today > until)
        }
        Some(Value::Mapping(until)) => {
            known_keys(until, UNTIL_KEYS, "until.")?;
            let pathway = string(until, "pathway", "until.")?
                .ok_or("missing required key 'until.pathway'")?;
            let position = string(until, "position", "until.")?
                .ok_or("missing required key 'until.position'")?;
            if !profile.pathways().contains(&pathway) {
                return Err(format!(
                    "until: the profile declares no pathway '{pathway}'"
                ));
            }
            let Some(current) = graph.pathway(&pathway) else {
                return Err(format!(
                    "until: the register supplies no position on pathway '{pathway}'"
                ));
            };
            if !current.is_member(&position) {
                return Err(format!(
                    "until: '{position}' is not a position on pathway '{pathway}'"
                ));
            }
            let expired = !current.is_after(&position);
            (Some(Until::Pathway { pathway, position }), expired)
        }
        Some(_) => {
            return Err(
                "'until' must be a YYYY-MM-DD date or a mapping of 'pathway' and 'position'".into(),
            );
        }
    };

    let mut note = Note::new(text);
    note.until = until;
    note.by = by;
    note.at = at;
    note.expired = expired;
    Ok(Entry {
        code,
        node,
        note,
        line: item.line,
    })
}

fn known_keys(map: &serde_norway::Mapping, accepted: &[&str], prefix: &str) -> Result<(), String> {
    let unknown: Vec<String> = map
        .keys()
        .map(|key| {
            key.as_str()
                .map_or_else(|| format!("{key:?}"), str::to_owned)
        })
        .filter(|key| !accepted.contains(&key.as_str()))
        .map(|key| format!("{prefix}{key}"))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let accepted: Vec<String> = accepted
        .iter()
        .map(|key| format!("{prefix}{key}"))
        .collect();
    Err(format!(
        "unknown keys {unknown:?}; accepted keys are {accepted:?}"
    ))
}

fn string(map: &serde_norway::Mapping, key: &str, prefix: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("'{prefix}{key}' must be a string")),
    }
}

fn date(value: &str, key: &str) -> Result<CivilDate, String> {
    CivilDate::parse(value)
        .ok_or_else(|| format!("'{key}' value '{value}' is not a YYYY-MM-DD calendar date"))
}

#[cfg(test)]
mod tests {
    use super::known_keys;

    #[test]
    fn known_keys_names_the_prefix() {
        let map: serde_norway::Mapping = serde_norway::from_str("code: x\nkind: y\n").unwrap();
        let error = known_keys(&map, &["code", "node"], "finding.").unwrap_err();
        assert!(error.contains("finding.kind"), "{error}");
    }
}

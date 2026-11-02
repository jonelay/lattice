use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::time::Duration;

use latt_adapter::process::{BoundedError, describe};
use latt_adapter::{Document, Edge, Node, Provenance};
use regex::Regex;
use serde_json::Value;

use crate::gitdb::{self, BRANCH, TreeEntry};

#[derive(Debug, Default)]
struct IssueData {
    tags: Vec<String>,
    dependencies: Vec<String>,
}

fn branch_file(path: &str) -> String {
    format!("{BRANCH}:{path}")
}

/// Undo ent's tag filename escape, as its own reader does: `,0` is `,` and `,1`
/// is `/`. Any other escape is one ent itself refuses to read.
fn tag_from_filename(filename: &str) -> Result<String, String> {
    let mut tokens = filename.split(',');
    let mut tag = tokens.next().unwrap_or_default().to_owned();
    for token in tokens {
        match token.chars().next() {
            Some('0') => tag.push(','),
            Some('1') => tag.push('/'),
            Some(other) => return Err(format!("invalid tag escape ',{other}'")),
            None => return Err("tag filename ends in an unfinished ',' escape".to_owned()),
        }
        tag.push_str(&token[1..]);
    }
    Ok(tag)
}

fn python_utf8_error(error: std::str::Utf8Error, bytes: &[u8]) -> String {
    let position = error.valid_up_to();
    let byte = bytes.get(position).copied().unwrap_or_default();
    let reason = if error.error_len() == Some(1) {
        "invalid start byte"
    } else {
        "invalid continuation byte"
    };
    format!("'utf-8' codec can't decode byte 0x{byte:02x} in position {position}: {reason}")
}

fn parse_batch(
    document: &mut Document,
    target: &Path,
    wanted: &[(String, String)],
    bound: Duration,
) -> HashMap<String, String> {
    if wanted.is_empty() {
        return HashMap::new();
    }
    let object_ids: Vec<&str> = wanted.iter().map(|(_, oid)| oid.as_str()).collect();
    let output = match gitdb::cat_file_batch(target, &object_ids, bound) {
        Ok(output) => output,
        Err(error @ BoundedError::TimedOut { .. }) => {
            document.parse_error(describe("git cat-file --batch", &error), ".", 0);
            return HashMap::new();
        }
        Err(error) => {
            for (path, _) in wanted {
                document.parse_error(
                    format!("{path}: could not read object: {error}"),
                    branch_file(path),
                    0,
                );
            }
            return HashMap::new();
        }
    };

    // A corrupt object is answered `missing` with exit 0; only stderr says why.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        document.parse_error(
            format!(
                "git cat-file --batch failed with {}: {}",
                output.status,
                stderr.trim()
            ),
            ".",
            0,
        );
    } else if !stderr.trim().is_empty() {
        document.parse_error(
            format!("git cat-file --batch reported: {}", stderr.trim()),
            ".",
            0,
        );
    }

    let mut texts = HashMap::new();
    let mut position = 0;
    for (path, _) in wanted {
        let Some(relative_end) = output.stdout[position..]
            .iter()
            .position(|byte| *byte == b'\n')
        else {
            document.parse_error(
                format!("{path}: malformed response from git cat-file"),
                branch_file(path),
                0,
            );
            continue;
        };
        let header_end = position + relative_end;
        let header = String::from_utf8_lossy(&output.stdout[position..header_end]);
        position = header_end + 1;
        let fields: Vec<&str> = header.split(' ').collect();
        if fields.get(1) == Some(&"missing") {
            document.parse_error(
                format!("{path}: object not in the repository"),
                branch_file(path),
                0,
            );
            continue;
        }
        let Some(size) = fields.get(2).and_then(|value| value.parse::<usize>().ok()) else {
            document.parse_error(
                format!("{path}: malformed response from git cat-file"),
                branch_file(path),
                0,
            );
            continue;
        };
        let Some(blob_end) = position.checked_add(size) else {
            document.parse_error(
                format!("{path}: malformed response from git cat-file"),
                branch_file(path),
                0,
            );
            continue;
        };
        let Some(blob) = output.stdout.get(position..blob_end) else {
            document.parse_error(
                format!("{path}: truncated response from git cat-file"),
                branch_file(path),
                0,
            );
            position = output.stdout.len();
            continue;
        };
        position = blob_end.saturating_add(1).min(output.stdout.len());
        match latt_adapter::decode_utf8(blob) {
            Ok(text) => {
                texts.insert(path.clone(), text.to_owned());
            }
            Err(error) => document.parse_error(
                format!(
                    "{path}: not valid UTF-8: {}",
                    python_utf8_error(error, blob)
                ),
                branch_file(path),
                0,
            ),
        }
    }
    texts
}

fn classify_entries(
    document: &mut Document,
    entries: Vec<TreeEntry>,
) -> (BTreeMap<String, IssueData>, Vec<(String, String)>) {
    let hex32 = "[0-9a-f]{32}";
    let issue_dir = Regex::new(&format!(r"^({hex32})/")).expect("constant regex is valid");
    let fetched = Regex::new(&format!(
        r"^(?P<issue>{hex32})/(?P<field>description|author|state|assignee)$"
    ))
    .expect("constant regex is valid");
    let unfetched = Regex::new(&format!(
        r"^(README\.md|{hex32}/(creation_time|done_time)|{hex32}/tags/[^/]+|{hex32}/dependencies/{hex32}|{hex32}/comments/{hex32}/(author|creation_time|description))$"
    ))
    .expect("constant regex is valid");

    let mut issues = BTreeMap::<String, IssueData>::new();
    let mut wanted = Vec::new();
    for entry in entries {
        if let Some(captures) = issue_dir.captures(&entry.path) {
            issues.entry(captures[1].to_owned()).or_default();
        }
        if entry.object_type != "blob" {
            document.parse_error(
                format!("{}: unexpected {} entry", entry.path, entry.object_type),
                branch_file(&entry.path),
                0,
            );
        } else if fetched.is_match(&entry.path) {
            wanted.push((entry.path, entry.oid));
        } else if unfetched.is_match(&entry.path) {
            let parts: Vec<&str> = entry.path.split('/').collect();
            if parts.len() == 3 && parts[1] == "tags" {
                match tag_from_filename(parts[2]) {
                    Ok(tag) => issues
                        .get_mut(parts[0])
                        .expect("issue path initialized")
                        .tags
                        .push(tag),
                    Err(error) => document.parse_error(
                        format!("{}: {error}", entry.path),
                        branch_file(&entry.path),
                        0,
                    ),
                }
            } else if parts.len() == 3 && parts[1] == "dependencies" {
                issues
                    .get_mut(parts[0])
                    .expect("issue path initialized")
                    .dependencies
                    .push(parts[2].to_owned());
            }
        } else {
            document.parse_error(
                format!("{}: no reader for this file shape", entry.path),
                branch_file(&entry.path),
                0,
            );
        }
    }
    (issues, wanted)
}

/// Read all issues from a pinned ent register tree without checking it out.
pub(crate) fn read(
    document: &mut Document,
    target: &Path,
    entries: Vec<TreeEntry>,
    bound: Duration,
) {
    let (issues, wanted) = classify_entries(document, entries);
    let fetched_paths: BTreeSet<&str> = wanted.iter().map(|(path, _)| path.as_str()).collect();
    let texts = parse_batch(document, target, &wanted, bound);

    for (id, mut issue) in issues {
        issue.tags.sort();
        issue.dependencies.sort();
        let description_path = format!("{id}/description");
        let author_path = format!("{id}/author");
        let assignee_path = format!("{id}/assignee");
        let state_path = format!("{id}/state");
        let mut attrs = BTreeMap::new();
        if let Some(summary) = texts.get(&description_path).map(|text| {
            text.split_once('\n')
                .map_or(text.as_str(), |(first, _)| first)
                .trim()
                .to_owned()
        }) {
            attrs.insert("summary".to_owned(), Value::String(summary));
        }
        if let Some(author) = texts.get(&author_path).map(|text| text.trim().to_owned()) {
            attrs.insert("author".to_owned(), Value::String(author));
        }
        if let Some(assignee) = texts.get(&assignee_path).map(|text| text.trim().to_owned()) {
            attrs.insert("assignee".to_owned(), Value::String(assignee));
        }
        if let Some(state) = texts.get(&state_path).map_or_else(
            || (!fetched_paths.contains(state_path.as_str())).then(|| "new".to_owned()),
            |text| Some(text.trim().to_owned()),
        ) {
            attrs.insert("state".to_owned(), Value::String(state));
        }
        if !issue.tags.is_empty() {
            attrs.insert(
                "tags".to_owned(),
                Value::Array(issue.tags.into_iter().map(Value::String).collect()),
            );
        }
        // Each attr is a file in the issue's directory, its value on line 1: a
        // finding about one attr names that file, not the issue's directory.
        let mut node = Node::new(id.clone(), "issue", Provenance::new_file(branch_file(&id)))
            .with_attrs(attrs);
        for (attr, path) in [
            ("summary", &description_path),
            ("author", &author_path),
            ("assignee", &assignee_path),
            ("state", &state_path),
        ] {
            if node.attr(attr).is_some() && texts.contains_key(path) {
                node = node.with_attr_provenance(attr, Provenance::new(branch_file(path), 1));
            }
        }
        let declared_by = document.nodes.len();
        document.nodes.push(node);
        for dependency in issue.dependencies {
            let provenance =
                Provenance::new_file(format!("{BRANCH}:{id}/dependencies/{dependency}"));
            document.edges.push(
                Edge::new(id.clone(), dependency, "depends_on", provenance)
                    .declared_by(declared_by),
            );
        }
    }
}

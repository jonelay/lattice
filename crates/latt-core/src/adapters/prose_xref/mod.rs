//! Prose cross-reference adapter: `§` references in Markdown as a Lattice document.
//!
//! Reads backtick-delimited `§` references from Markdown files under a profile's
//! content directories, and emits `document` and `section` nodes and `xref` edges.

mod config;
mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use latt_adapter::{Document, Edge, Node, Provenance};
use serde_json::Value;

use config::{Config, relative};
use parse::{Heading, XRefKind};

/// The codes this adapter alone emits, owned by `latt_adapter::codes` so the
/// shipped-code registry is one list.
pub use latt_adapter::codes::{AMBIGUOUS_REF, WRAPPED_REF};
/// The codes this adapter emits beyond the ones every adapter shares.
pub const CODES: &[&str] = &[WRAPPED_REF, AMBIGUOUS_REF];

/// Build the interface document and write it to `out` as one
/// newline-terminated JSON line.
///
/// # Errors
///
/// As [`build_document_value`], and when `out` cannot be written.
pub fn write_document(
    profile: &Path,
    target: &Path,
    out: &mut impl std::io::Write,
) -> Result<(), String> {
    // Typed, as md and toml write theirs: the same bytes in every serde_json build.
    serde_json::to_writer(&mut *out, &build_doc(profile, target)?)
        .map_err(|error| format!("could not write interface document: {error}"))?;
    out.write_all(b"\n")
        .and_then(|()| out.flush())
        .map_err(|error| format!("could not write interface document: {error}"))
}

/// Build the interface document as a JSON value.
///
/// `profile` is a path to the *resolved* profile document — the JSON that
/// `latt show` prints — not the profile's YAML source. Unreadable files and
/// missing content directories are not errors: they arrive as `PARSE_ERROR`
/// findings in the returned document. A reference that does not resolve to one
/// section has one of three outcomes:
///
/// - a file it names that is not in the register, cannot be read, or lies
///   outside the target is a `PARSE_ERROR`, and no edge;
/// - a heading no section of the named file carries is still an `xref` edge,
///   whose dangling target the core reports as `VACANCY`, plus a `WRAPPED_REF`
///   info finding when a line break split the reference from its file;
/// - a heading several sections match is an `AMBIGUOUS_REF` warning, and no edge.
///
/// # Errors
///
/// A message when the profile cannot be read or configures the adapter wrongly
/// (the CLI's exit 2).
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join(format!("latt-adapter-prose-xref-doc-{}", std::process::id()));
/// std::fs::create_dir_all(dir.join("target/ref"))?;
/// std::fs::write(
///     dir.join("target/ref/a.md"),
///     "# A\n\n## X\n\n## Dup\n\n## Dup\n\n\
///      See § `X`.\n\nSee § `Missing`.\n\nSee § `Dup`.\n\nSee `gone.md` § `X`.\n",
/// )?;
/// std::fs::write(dir.join("profile.json"),
///     r#"{"resolved_schema": "1",
///         "adapter": {"paths": {"files": ["**/*.md"], "content_dirs": ["ref"]}}}"#)?;
/// let document =
///     latt_core::adapters::prose_xref::build_document_value(&dir.join("profile.json"), &dir.join("target"));
/// std::fs::remove_dir_all(&dir)?;
/// let document = document?;
///
/// // Resolved, and dangling: both are edges; the core reports the second as VACANCY.
/// assert_eq!(document["edges"].as_array().map(Vec::len), Some(2));
/// assert_eq!(document["edges"][0]["tgt"], "ref/a.md#A::X");
/// assert_eq!(document["edges"][1]["tgt"], "ref/a.md#Missing");
/// // Ambiguous, and a file not in the register: findings, and no edge.
/// assert_eq!(document["findings"].as_array().map(Vec::len), Some(2));
/// assert_eq!(document["findings"][0]["code"], "AMBIGUOUS_REF");
/// assert_eq!(document["findings"][1]["code"], "PARSE_ERROR");
/// assert_eq!(document["findings"][1]["message"], "target file not found: gone.md");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn build_document_value(profile: &Path, target: &Path) -> Result<Value, String> {
    serde_json::to_value(build_doc(profile, target)?)
        .map_err(|e| format!("serialization failed: {e}"))
}

fn build_doc(profile: &Path, target: &Path) -> Result<Document, String> {
    let config = Config::load(profile)?;
    let mut doc = Document::default();
    let files = config.collect_files(target, &mut doc);

    // Every discovered file, readable or not: references resolve against this set,
    // so a reference into an unreadable file is reported as such, not as missing.
    let discovered: BTreeSet<String> = files.iter().map(|path| relative(path, target)).collect();
    let mut headings_by_file: BTreeMap<String, Vec<Heading>> = BTreeMap::new();
    let mut texts_by_rel: BTreeMap<String, String> = BTreeMap::new();
    // Each file's node entry indices: the document's, then one per heading in
    // `headings_by_file` order. An xref edge is declared by the entry of the
    // section enclosing it, so a repeated section's edges go unread with it.
    let mut entries_by_file: BTreeMap<String, (usize, Vec<usize>)> = BTreeMap::new();

    for file_path in &files {
        let rel = relative(file_path, target);
        let text = match decode(&rel, std::fs::read(file_path)) {
            Ok((text, unread)) => {
                for (line, message) in unread {
                    doc.parse_error(message, rel.as_str(), line);
                }
                text
            }
            Err(message) => {
                doc.parse_error(message, rel.as_str(), 0);
                continue;
            }
        };

        let document_index = doc.nodes.len();
        doc.nodes.push(Node::new(
            rel.clone(),
            "document",
            Provenance::new_file(&rel),
        ));

        let headings = parse::collect_headings(&text);
        let mut section_indices = Vec::with_capacity(headings.len());
        for heading in &headings {
            let section_id = format!("{}#{}", rel, heading.path);
            section_indices.push(doc.nodes.len());
            doc.nodes.push(
                Node::new(section_id, "section", Provenance::new(&rel, heading.line)).with_attrs(
                    BTreeMap::from([
                        ("heading".into(), Value::String(heading.text.clone())),
                        ("level".into(), Value::Number(heading.level.into())),
                    ]),
                ),
            );
        }
        headings_by_file.insert(rel.clone(), headings);
        entries_by_file.insert(rel.clone(), (document_index, section_indices));
        texts_by_rel.insert(rel, text);
    }

    for (rel, text) in &texts_by_rel {
        let xrefs = parse::collect_xrefs(text);

        // The file the previous reference named: resolved, or the raw name that
        // failed to resolve, so a carried-forward reference inherits the failure
        // instead of silently binding to an older file or to this one.
        let mut last_target_file: Option<Result<&str, String>> = None;

        for xref in &xrefs {
            match xref.kind {
                XRefKind::Resolved {
                    ref file,
                    ref heading,
                    carry_forward,
                    ref wrapped_file,
                } => {
                    let target_rel = match (file, carry_forward, &last_target_file) {
                        (None, true, Some(last)) => last.clone(),
                        _ => resolve_target(
                            file.as_deref(),
                            rel,
                            target,
                            &discovered,
                            &config.content_dirs,
                        ),
                    };
                    if file.is_some() {
                        last_target_file = Some(target_rel.clone());
                    }

                    let target_file = match &target_rel {
                        Ok(target_file) => *target_file,
                        Err(message) => {
                            doc.parse_error(message.as_str(), rel.as_str(), xref.line);
                            continue;
                        }
                    };

                    let Some(file_headings) = headings_by_file.get(target_file) else {
                        doc.parse_error(
                            format!("target file unreadable: {target_file}"),
                            rel.as_str(),
                            xref.line,
                        );
                        continue;
                    };
                    let matches = matching_headings(file_headings, heading);

                    match matches.len() {
                        0 => {
                            if let (None, Some(wrapped)) = (file, wrapped_file) {
                                doc.findings.push(latt_adapter::Issue::new(
                                    latt_adapter::Severity::Info,
                                    WRAPPED_REF,
                                    format!(
                                        "§ `{heading}` names no file on its line, so it \
                                         resolved against the citing file {rel}; the previous \
                                         line ends with `{wrapped}`, and a file applies only \
                                         to a § on its own line"
                                    ),
                                    Provenance::new(rel.as_str(), xref.line),
                                ));
                            }
                            let src_id = nearest_section(rel, xref.line, &headings_by_file);
                            let tgt_id = format!("{target_file}#{heading}");
                            let declared_by = declaring_entry(
                                rel,
                                xref.line,
                                &headings_by_file,
                                &entries_by_file,
                            );
                            doc.edges.push(
                                Edge::new(
                                    src_id,
                                    tgt_id,
                                    "xref",
                                    Provenance::new(rel.as_str(), xref.line),
                                )
                                .declared_by(declared_by),
                            );
                        }
                        1 => {
                            let matched = matches[0];
                            let src_id = nearest_section(rel, xref.line, &headings_by_file);
                            let tgt_id = format!("{}#{}", target_file, matched.path);
                            let declared_by = declaring_entry(
                                rel,
                                xref.line,
                                &headings_by_file,
                                &entries_by_file,
                            );
                            doc.edges.push(
                                Edge::new(
                                    src_id,
                                    tgt_id,
                                    "xref",
                                    Provenance::new(rel.as_str(), xref.line),
                                )
                                .declared_by(declared_by),
                            );
                        }
                        _ => {
                            doc.findings.push(latt_adapter::Issue::new(
                                latt_adapter::Severity::Warning,
                                AMBIGUOUS_REF,
                                format!(
                                    "heading '{}' matches {} sections in {}",
                                    heading,
                                    matches.len(),
                                    target_file,
                                ),
                                Provenance::new(rel.as_str(), xref.line),
                            ));
                        }
                    }
                }
                XRefKind::Empty => {
                    doc.parse_error("§ with no target", rel.as_str(), xref.line);
                }
                XRefKind::UnclosedFence => {
                    doc.parse_error(
                        "code fence is never closed: headings and references after it are not read",
                        rel.as_str(),
                        xref.line,
                    );
                }
                XRefKind::BareUndelimited { ref text } => {
                    doc.parse_error(
                        format!("bare § without backtick delimiters: § {text}"),
                        rel.as_str(),
                        xref.line,
                    );
                }
            }
        }
    }

    Ok(doc)
}

/// Convert a file read into text and one `PARSE_ERROR` per line that is not UTF-8,
/// or the `PARSE_ERROR` message for why the file could not be read at all.
///
/// Each such line is blanked: a heading or reference read from replacement
/// characters would name text the register never wrote. The line is reported
/// where it is, as written, and withholds only itself.
fn decode(rel: &str, read: io::Result<Vec<u8>>) -> Result<(String, Vec<(u32, String)>), String> {
    let bytes = read.map_err(|error| format!("could not read '{rel}': {error}"))?;
    if let Some(refusal) = super::too_many_lines(&bytes) {
        return Err(format!("{rel}: {refusal}"));
    }
    let (text, invalid) = latt_adapter::decode_lines(&bytes);
    if invalid.is_empty() {
        return Ok((text, Vec::new()));
    }
    let blank: BTreeSet<u32> = invalid.iter().map(|line| line.line).collect();
    let text = (1u32..)
        .zip(text.split('\n'))
        .map(|(number, line)| if blank.contains(&number) { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    let messages = invalid
        .into_iter()
        .map(|line| {
            let message = format!(
                "{rel}: line is not valid UTF-8 at column {}, so it was not read: {}",
                line.column, line.as_written
            );
            (line.line, message)
        })
        .collect();
    Ok((text, messages))
}

/// Resolve the file a reference names to a discovered file, or the `PARSE_ERROR`
/// message for why it names none.
fn resolve_target<'a>(
    file_ref: Option<&str>,
    current_rel: &'a str,
    target_root: &Path,
    discovered: &'a BTreeSet<String>,
    content_dirs: &[String],
) -> Result<&'a str, String> {
    let Some(raw) = file_ref else {
        return Ok(current_rel);
    };

    if let Some(found) = discovered.get(raw) {
        return Ok(found.as_str());
    }

    // Sibling-relative resolution.
    if let Some(parent) = Path::new(current_rel).parent() {
        let sibling = parent.join(raw).to_string_lossy().into_owned();
        let Some(normalized) = normalize_path(&sibling) else {
            return Err(format!("target outside the register: {raw}"));
        };
        if let Some(found) = discovered.get(normalized.as_str()) {
            return Ok(found.as_str());
        }
    }

    // Root-relative resolution.
    let cleaned = raw.trim_start_matches("./");
    if let Some(found) = discovered.get(cleaned) {
        return Ok(found.as_str());
    }

    // Try each content dir prefix.
    let path = Path::new(cleaned);
    let without_prefix: Option<&Path> = path.components().next().and_then(|c| {
        let s = c.as_os_str().to_string_lossy();
        if content_dirs.iter().any(|d| d == s.as_ref()) {
            Some(path.strip_prefix(&*s).unwrap_or(path))
        } else {
            None
        }
    });
    if let Some(stripped) = without_prefix {
        for dir in content_dirs {
            let candidate = format!("{}/{}", dir, stripped.display());
            if let Some(found) = discovered.get(candidate.as_str()) {
                return Ok(found.as_str());
            }
        }
    } else {
        for dir in content_dirs {
            let candidate = format!("{dir}/{cleaned}");
            if let Some(found) = discovered.get(candidate.as_str()) {
                return Ok(found.as_str());
            }
        }
    }

    // Check with the root as an absolute path.
    let abs = target_root.join(cleaned);
    if abs.exists() {
        let rel = relative(&abs, target_root);
        if let Some(found) = discovered.get(rel.as_str()) {
            return Ok(found.as_str());
        }
    }

    Err(format!("target file not found: {raw}"))
}

/// Fold `.` and `..` out of a `/`-separated path relative to the register root;
/// `None` when a `..` climbs past the root, out of the register.
fn normalize_path(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Collect the headings a reference names: exact matches when there are any, so
/// `Arc` beside `Arc: One` targets `Arc`; otherwise the qualified-prefix matches.
fn matching_headings<'a>(headings: &'a [Heading], reference: &str) -> Vec<&'a Heading> {
    let exact: Vec<&Heading> = headings
        .iter()
        .filter(|h| heading_matches_exactly(&h.text, reference))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    headings
        .iter()
        .filter(|h| heading_matches_prefix(&h.text, reference))
        .collect()
}

fn heading_matches_exactly(heading_text: &str, reference: &str) -> bool {
    heading_text == reference
        || strip_markdown_emphasis(heading_text) == strip_markdown_emphasis(reference)
}

/// True when the reference is the heading's text before a qualifier:
/// `"Stage 8: The Mixing Frontier [working]"` matches "Stage 8".
fn heading_matches_prefix(heading_text: &str, reference: &str) -> bool {
    let plain_heading = strip_markdown_emphasis(heading_text);
    let plain_ref = strip_markdown_emphasis(reference);
    [": ", " (", " [", " — "].iter().any(|separator| {
        plain_heading
            .split_once(separator)
            .is_some_and(|(prefix, _)| prefix == plain_ref)
    })
}

fn strip_markdown_emphasis(text: &str) -> &str {
    let mut s = text;
    for marker in ["**", "__", "*", "_"] {
        while s.starts_with(marker) && s.ends_with(marker) && s.len() > 2 * marker.len() {
            s = s[marker.len()..s.len() - marker.len()].trim();
        }
    }
    s
}

fn nearest_section(
    rel: &str,
    line: u32,
    headings_by_file: &BTreeMap<String, Vec<Heading>>,
) -> String {
    // `collect_headings` emits headings in ascending line order, so the last one
    // at or before `line` sits just before the partition point.
    if let Some(headings) = headings_by_file.get(rel) {
        let after = headings.partition_point(|heading| heading.line <= line);
        if let Some(heading) = after.checked_sub(1).and_then(|index| headings.get(index)) {
            return format!("{}#{}", rel, heading.path);
        }
    }
    rel.to_owned()
}

/// Return the node entry index of what [`nearest_section`] names: the
/// enclosing section's entry, or the file's own before its first heading.
fn declaring_entry(
    rel: &str,
    line: u32,
    headings_by_file: &BTreeMap<String, Vec<Heading>>,
    entries_by_file: &BTreeMap<String, (usize, Vec<usize>)>,
) -> usize {
    let (document_index, section_indices) = &entries_by_file[rel];
    let after = headings_by_file[rel].partition_point(|heading| heading.line <= line);
    after
        .checked_sub(1)
        .map_or(*document_index, |index| section_indices[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `write_document` writes the typed document, struct fields in declared order
    /// whatever map order `serde_json` was built with, and it reads back as the
    /// value `build_document_value` returns, over every outcome the adapter has.
    #[test]
    fn write_document_writes_the_typed_document() {
        let root = std::env::temp_dir().join(format!("latt-prose-bytes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ref")).unwrap();
        let profile = root.join("profile.json");
        let paths = r#"{"files": ["**/*.md"], "content_dirs": ["ref", "absent"]}"#;
        std::fs::write(
            &profile,
            format!(r#"{{"resolved_schema": "1", "adapter": {{"paths": {paths}}}}}"#),
        )
        .unwrap();
        std::fs::write(root.join("ref/b.md"), "# B\n## Y\n").unwrap();
        let cases: [&[u8]; 9] = [
            b"# A\n## X\nSee \xc2\xa7 `X` and \xc2\xa7 `Missing`.\n",
            b"# A\n## Dup\n## Dup\nSee \xc2\xa7 `Dup`.\nSee `gone.md` \xc2\xa7 `X`.\n",
            b"# A\n## **X**\n### _Y_\nSee \xc2\xa7 `X` and `b.md` \xc2\xa7 `Y`.\n",
            b"# A\nSee `b.md`\n\xc2\xa7 `Nowhere`.\n",
            b"# A\n```\n## Hidden\n\xc2\xa7 `Hidden`\n",
            b"# A\nBare \xc2\xa7 text.\n`../../out.md` \xc2\xa7 `X`\n",
            b"# A caf\xe9\n## \"Quoted\" \\ \xc3\xa9\nSee \xc2\xa7 `\"Quoted\" \\ \xc3\xa9`.\n",
            b"",
            b"\xef\xbb\xbf# A\n",
        ];
        for text in cases {
            std::fs::write(root.join("ref/a.md"), text).unwrap();
            let mut expected = serde_json::to_vec(&build_doc(&profile, &root).unwrap()).unwrap();
            expected.push(b'\n');
            let mut actual = Vec::new();
            write_document(&profile, &root, &mut actual).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "{}",
                String::from_utf8_lossy(text)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&actual).unwrap(),
                build_document_value(&profile, &root).unwrap(),
                "{}",
                String::from_utf8_lossy(text)
            );
        }
        // Every outcome above was reached, so agreement is not empty lists agreeing.
        let mut codes = BTreeSet::new();
        let mut edges = 0;
        for text in cases {
            std::fs::write(root.join("ref/a.md"), text).unwrap();
            let value = build_document_value(&profile, &root).unwrap();
            edges += value["edges"].as_array().unwrap().len();
            for finding in value["findings"].as_array().unwrap() {
                codes.insert(finding["code"].as_str().unwrap().to_owned());
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
        assert!(edges >= 4, "{edges}");
        for code in ["AMBIGUOUS_REF", "WRAPPED_REF", "PARSE_ERROR"] {
            assert!(codes.contains(code), "{code} not reached: {codes:?}");
        }
    }

    #[test]
    fn normalize_path_refuses_to_climb_past_the_root() {
        assert_eq!(normalize_path("ref/./a.md").as_deref(), Some("ref/a.md"));
        assert_eq!(
            normalize_path("ref/../outline/o.md").as_deref(),
            Some("outline/o.md")
        );
        assert_eq!(normalize_path("ref/../a.md").as_deref(), Some("a.md"));
        assert_eq!(normalize_path("ref/../../a.md"), None);
        assert_eq!(normalize_path("../a.md"), None);
    }

    #[test]
    fn decode_reports_io_error() {
        let result = decode("ref/a.md", Err(io::Error::other("denied")));
        assert_eq!(result.unwrap_err(), "could not read 'ref/a.md': denied");
    }

    /// A line that is not UTF-8 is reported where it is, as written, and blanked,
    /// so no heading or reference is read from it; every other line survives.
    #[test]
    fn decode_blanks_only_the_lines_that_are_not_utf8() {
        let (text, unread) = decode(
            "ref/a.md",
            Ok(b"# A\n# B caf\xe9\nSee \xc2 `x`\n# C\n".to_vec()),
        )
        .unwrap();
        assert_eq!(text, "# A\n\n\n# C\n");
        assert_eq!(
            unread,
            vec![
                (2, r"ref/a.md: line is not valid UTF-8 at column 8, so it was not read: # B caf\xE9".to_owned()),
                (3, r"ref/a.md: line is not valid UTF-8 at column 5, so it was not read: See \xC2 `x`".to_owned()),
            ]
        );
    }

    #[test]
    fn decode_accepts_utf8() {
        let (text, unread) = decode("ref/a.md", Ok(b"# A\n".to_vec())).unwrap();
        assert_eq!((text.as_str(), unread.len()), ("# A\n", 0));
    }

    fn heading(line: u32, path: &str) -> Heading {
        Heading {
            text: path.rsplit("::").next().unwrap_or(path).to_owned(),
            level: 1,
            line,
            path: path.to_owned(),
        }
    }

    /// The enclosing section is the last heading at or before the reference's
    /// line; a reference above every heading belongs to the file itself.
    #[test]
    fn nearest_section_is_the_last_heading_at_or_before_the_line() {
        let headings = BTreeMap::from([(
            "a.md".to_owned(),
            vec![
                heading(3, "A"),
                heading(7, "A::B"),
                heading(7, "A::C"),
                heading(12, "D"),
            ],
        )]);

        // Before the first heading: the file, not a section.
        assert_eq!(nearest_section("a.md", 1, &headings), "a.md");
        assert_eq!(nearest_section("a.md", 2, &headings), "a.md");
        // On the heading's own line, it encloses the reference.
        assert_eq!(nearest_section("a.md", 3, &headings), "a.md#A");
        assert_eq!(nearest_section("a.md", 6, &headings), "a.md#A");
        // Equal lines: the later heading wins, as the reverse scan chose.
        assert_eq!(nearest_section("a.md", 7, &headings), "a.md#A::C");
        assert_eq!(nearest_section("a.md", 11, &headings), "a.md#A::C");
        assert_eq!(nearest_section("a.md", 12, &headings), "a.md#D");
        assert_eq!(nearest_section("a.md", 500, &headings), "a.md#D");
        // A file with no headings, and a file not seen at all.
        let empty = BTreeMap::from([("e.md".to_owned(), Vec::new())]);
        assert_eq!(nearest_section("e.md", 4, &empty), "e.md");
        assert_eq!(nearest_section("missing.md", 4, &headings), "missing.md");
    }
}

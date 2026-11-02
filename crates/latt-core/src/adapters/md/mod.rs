//! Markdown adapter: reads `|`-delimited tables from `.md` files under a profile's
//! per-heading kind and column mappings, and emits a Lattice interface document.

mod config;
mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use latt_adapter::{Document, Edge, Node, Provenance};
use serde_json::Value;

use config::{Config, IdColumn, TableConfig};

fn id_index(config: &TableConfig, headers: &[String]) -> Option<usize> {
    match &config.id_column {
        IdColumn::Index(index) => (*index < headers.len()).then_some(*index),
        IdColumn::Name(name) => headers.iter().position(|header| header == name),
    }
}

fn mapped_columns<'a>(
    headers: &[String],
    mapping: &'a BTreeMap<String, String>,
) -> Vec<(usize, &'a str)> {
    mapping
        .iter()
        .filter_map(|(header, output)| {
            headers
                .iter()
                .position(|candidate| candidate == header)
                .map(|index| (index, output.as_str()))
        })
        .collect()
}

/// Pluralize a count of table rows for a message.
fn rows(count: usize) -> String {
    if count == 1 {
        "1 row".into()
    } else {
        format!("{count} rows")
    }
}

fn read_table(
    document: &mut Document,
    config: &TableConfig,
    table: parse::Table,
    file: &str,
    unread: &BTreeSet<usize>,
) {
    let table_line = table.line;
    let attributes = mapped_columns(&table.headers, &config.column_map);
    let edges = mapped_columns(&table.headers, &config.edge_columns);
    let declared_columns: BTreeSet<_> = config
        .column_map
        .keys()
        .chain(config.edge_columns.keys())
        .collect();
    for column in declared_columns {
        if !table.headers.contains(column) {
            document.parse_error(
                format!("configured column '{column}' was not found in table headers"),
                file,
                table_line as u32,
            );
        }
    }

    let Some(id_index) = id_index(config, &table.headers) else {
        let column = match &config.id_column {
            IdColumn::Index(index) => index.to_string(),
            IdColumn::Name(name) => format!("'{name}'"),
        };
        document.parse_error(
            format!("configured ID column {column} was not found in table headers"),
            file,
            table_line as u32,
        );
        return;
    };

    // A row on an undecodable line was reported where it is; it withholds only itself.
    for row in table
        .rows
        .into_iter()
        .filter(|row| !unread.contains(&row.line))
    {
        if row.cells.len() < table.headers.len() {
            document.parse_error(
                format!(
                    "table row has fewer columns than its header ({} < {})",
                    row.cells.len(),
                    table.headers.len()
                ),
                file,
                row.line as u32,
            );
            continue;
        }
        if row.cells.len() > table.headers.len() {
            document.parse_error(
                format!(
                    "table row has more columns than its header ({} > {})",
                    row.cells.len(),
                    table.headers.len()
                ),
                file,
                row.line as u32,
            );
            continue;
        }

        let id = row.cells[id_index].trim();
        let id = if config.strip_emphasis {
            strip_wrapping_emphasis(id)
        } else {
            id
        };
        if id.is_empty() {
            document.parse_error("table row has an empty ID column", file, row.line as u32);
            continue;
        }

        let attrs = attributes
            .iter()
            .map(|(index, name)| ((*name).to_owned(), Value::String(row.cells[*index].clone())))
            .collect();
        let provenance = Provenance::new(file, row.line as u32);
        let declared_by = document.nodes.len();
        document
            .nodes
            .push(Node::new(id, &config.kind, provenance.clone()).with_attrs(attrs));

        for (index, kind) in &edges {
            for target in split_targets(&row.cells[*index]) {
                let target = if config.strip_emphasis {
                    strip_wrapping_emphasis(target)
                } else {
                    target
                };
                document.edges.push(
                    Edge::new(id, target, *kind, provenance.clone()).declared_by(declared_by),
                );
            }
        }
    }
}

/// Remove the emphasis that wraps the whole of `value`, until none does.
///
/// `**`, `__`, `*`, `_` or a backtick run of equal length at both ends, trimmed inside;
/// a pair with nothing inside is kept, so `****` is not an empty ID, and so is one whose
/// marker recurs inside as a run of its own length, as in `**a** b **c**`, which is two
/// spans. Unlike
/// prose-xref's heading match, which takes the four emphasis markers in one fixed-order
/// pass, this repeats until a fixed point and takes backticks, so `` _**`X`**_ `` is `X`.
fn strip_wrapping_emphasis(value: &str) -> &str {
    let mut value = value;
    loop {
        let ticks = value.len() - value.trim_start_matches('`').len();
        let marker = if ticks > 0 {
            &value[..ticks]
        } else {
            match ["**", "__", "*", "_"]
                .into_iter()
                .find(|marker| value.starts_with(marker))
            {
                Some(marker) => marker,
                None => return value,
            }
        };
        let width = marker.len();
        if value.len() <= 2 * width || !value.ends_with(marker) {
            return value;
        }
        let inner = value[width..value.len() - width].trim();
        if inner.is_empty() || has_run(inner, marker) {
            return value;
        }
        value = inner;
    }
}

/// True when `text` holds a maximal run of `marker`'s character exactly as long as it.
///
/// That run would close the span `marker` opens, as a backtick run of equal length closes
/// a code span; a longer or shorter run (`**` inside `*`) does not.
fn has_run(text: &str, marker: &str) -> bool {
    let Some(mark) = marker.chars().next() else {
        return false;
    };
    let mut run = 0;
    for character in text.chars().chain(std::iter::once('\0')) {
        if character == mark {
            run += 1;
        } else {
            if run == marker.len() {
                return true;
            }
            run = 0;
        }
    }
    false
}

/// Split an edge cell on the commas that separate targets, trimmed, empties omitted.
///
/// A comma inside `[...]` or `(...)` belongs to a Markdown link's text or
/// destination, not to the list: splitting there emits fragments such as `[R-2`
/// that name nothing and hide the edge the row declares.
fn split_targets(cell: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (index, character) in cell.char_indices() {
        match character {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                targets.push(&cell[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    targets.push(&cell[start..]);
    targets
        .into_iter()
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .collect()
}

fn build_doc(config: &Config, target: &Path) -> Result<Document, String> {
    let matcher = latt_adapter::FileMatcher::new(&config.files)?;
    let mut document = Document::default();
    for (path, file) in latt_adapter::selected_files(&mut document, target, &matcher) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                document.parse_error(format!("could not read '{file}': {error}"), file, 0);
                continue;
            }
        };
        if let Some(refusal) = super::too_many_lines(&bytes) {
            document.parse_error(format!("{file}: {refusal}"), file, 0);
            continue;
        }
        let (text, invalid) = latt_adapter::decode_lines(&bytes);
        for line in &invalid {
            document.parse_error(
                format!(
                    "{file}: line is not valid UTF-8 at column {}, so it was not read: {}",
                    line.column, line.as_written
                ),
                &file,
                line.line,
            );
        }
        let unread: BTreeSet<usize> = invalid.iter().map(|line| line.line as usize).collect();
        let scanned = parse::scan_unread(&text, &unread);
        for table in &scanned.withheld {
            document.parse_error(
                format!(
                    "{file}: the table at line {} was not read: line {} of its header is not \
                     valid UTF-8, so its {} were not read",
                    table.line,
                    table.cause,
                    rows(table.rows)
                ),
                &file,
                table.line as u32,
            );
        }
        if let Some(line) = scanned.unclosed_fence {
            document.parse_error(
                "code fence is never closed: tables and headings after it are not read",
                &file,
                line as u32,
            );
        }
        for mismatch in scanned.mismatches {
            let heading_text = mismatch.heading.unwrap_or("");
            if config.multi_table
                || config
                    .tables
                    .iter()
                    .any(|table_config| table_config.heading.is_match(heading_text))
            {
                document.parse_error(
                    format!(
                        "table header has {} columns but its separator row has {}; \
                         the table was not read",
                        mismatch.headers, mismatch.separators
                    ),
                    &file,
                    mismatch.line as u32,
                );
            }
        }
        for (heading, table) in scanned.tables {
            // A multi-table heading picks the kind; an unreadable one picks nothing a
            // reader wrote, so its tables are withheld rather than filed under a kind.
            if config.multi_table
                && let Some(heading_line) = table.heading_line.filter(|line| unread.contains(line))
            {
                document.parse_error(
                    format!(
                        "{file}: the table at line {} was not read: its heading at line \
                         {heading_line} is not valid UTF-8, so its {} were not read",
                        table.line,
                        rows(table.rows.len())
                    ),
                    &file,
                    table.line as u32,
                );
                continue;
            }
            let heading_text = heading.unwrap_or("");
            if let Some(table_config) = config
                .tables
                .iter()
                .find(|table_config| table_config.heading.is_match(heading_text))
            {
                read_table(&mut document, table_config, table, &file, &unread);
            } else if config.multi_table {
                let heading = heading.unwrap_or("no heading");
                document.parse_error(
                    format!("table under heading '{heading}' matches no table configuration"),
                    &file,
                    table.line as u32,
                );
            }
        }
    }
    latt_adapter::check_sources(&mut document, &config.node_kinds, target)
        .map_err(|error| format!("could not load profile: {error}"))?;
    Ok(document)
}

/// Build an interface document from a resolved profile and target and write it
/// to `out` as one newline-terminated JSON line.
///
/// # Errors
///
/// As [`build_document_value`], and when `out` cannot be written.
pub fn write_document(
    profile: &Path,
    target: &Path,
    out: &mut impl std::io::Write,
) -> Result<(), String> {
    let config = Config::load(profile)?;
    let target =
        latt_adapter::absolute_path(target.to_owned()).map_err(|error| error.to_string())?;
    let document = build_doc(&config, &target)?;
    serde_json::to_writer(&mut *out, &document)
        .map_err(|error| format!("could not write interface document: {error}"))?;
    out.write_all(b"\n")
        .and_then(|()| out.flush())
        .map_err(|error| format!("could not write interface document: {error}"))
}

/// Build an interface document as a `serde_json::Value`, skipping the
/// string roundtrip for in-process callers.
///
/// `profile` is a path to the *resolved* profile document — the JSON that
/// `latt show` prints, carrying `"resolved_schema": "1"` — not the
/// profile's YAML source. Unreadable register input is not an error: it
/// arrives as `PARSE_ERROR` findings in the returned document.
///
/// # Errors
///
/// A message when the profile cannot be read, is not a resolved document, or
/// configures the adapter wrongly (the CLI's exit 2).
///
/// ```
/// let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
/// let profile = std::env::temp_dir().join(format!("latt-adapter-md-doc-{}.json", std::process::id()));
/// std::fs::write(&profile, r#"{"resolved_schema": "1",
///     "adapter": {"paths": {"files": ["*.md"]}, "table": {"id_column": "ID",
///                 "column_map": {"Status": "status"},
///                 "edge_columns": {"Traces To": "traces_to"}}},
///     "node_kinds": {"requirement": {"attrs": {"status": {"type": "string"}}}}}"#)?;
/// let document = latt_core::adapters::md::build_document_value(&profile, &root.join("tests/fixtures/mini-md"));
/// std::fs::remove_file(&profile)?;
/// let document = document?;
/// assert!(!document["nodes"].as_array().unwrap().is_empty());
/// assert!(!document["edges"].as_array().unwrap().is_empty());
///
/// let error = latt_core::adapters::md::build_document_value(&root.join("profiles/md.yaml"), &root).unwrap_err();
/// assert!(error.starts_with("could not load profile"), "a YAML profile is not a resolved document: {error}");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn build_document_value(profile: &Path, target: &Path) -> Result<Value, String> {
    let config = Config::load(profile)?;
    let target =
        latt_adapter::absolute_path(target.to_owned()).map_err(|error| error.to_string())?;
    let document = build_doc(&config, &target)?;
    serde_json::to_value(&document)
        .map_err(|error| format!("could not serialize interface document: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{split_targets, strip_wrapping_emphasis};

    #[test]
    fn strip_emphasis_removes_wrapping_markers_only() {
        for (cell, stripped) in [
            ("**1.1.1**", "1.1.1"),
            ("__X__", "X"),
            ("*X*", "X"),
            ("_X_", "X"),
            ("`X`", "X"),
            ("``X``", "X"),
            ("**`X`**", "X"),
            ("_**`X`**_", "X"),
            ("** X **", "X"),
            ("****", "****"),
            ("``", "``"),
            ("**X", "**X"),
            ("``X`", "``X`"),
            ("**a** b **c**", "**a** b **c**"),
            ("***X***", "X"),
            ("*_**X**_*", "X"),
            ("`a``b`", "a``b"),
            ("`a`b`", "`a`b`"),
            ("[**X**](x.md)", "[**X**](x.md)"),
            ("snake_case_id", "snake_case_id"),
        ] {
            assert_eq!(strip_wrapping_emphasis(cell), stripped, "{cell}");
        }
    }

    #[test]
    fn a_comma_inside_link_text_or_destination_does_not_split() {
        assert_eq!(
            split_targets("[R-2, R-3](http://x/a,b), R-4"),
            ["[R-2, R-3](http://x/a,b)", "R-4"]
        );
        assert_eq!(split_targets(" R-1 ,, R-2 , "), ["R-1", "R-2"]);
        assert_eq!(split_targets("R-1], R-2"), ["R-1]", "R-2"]);
    }

    #[test]
    fn sources_unknown_id_names_form_is_a_broken_profile() {
        let dir =
            std::env::temp_dir().join(format!("latt-adapter-md-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("r.md"), "| ID |\n|----|\n| t.py::x |\n").unwrap();
        let profile = dir.join("profile.json");
        std::fs::write(
            &profile,
            r#"{"resolved_schema": "1",
                "adapter": {"paths": {"files": ["*.md"]}, "table": {"id_column": "ID",
                            "column_map": {}, "edge_columns": {}}},
                "node_kinds": {"test": {"id_names": "jest"}}}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        let error = super::write_document(&profile, &dir, &mut out).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            error.contains("node kind 'test'") && error.contains("'jest'"),
            "{error}"
        );
        assert!(out.is_empty(), "no document is written");
    }
}

#[cfg(test)]
mod properties {
    use proptest::prelude::*;

    use super::strip_wrapping_emphasis;

    /// A value with no marker character, trimmed and non-empty.
    fn plain_value() -> impl Strategy<Value = String> {
        "[A-Za-z0-9][A-Za-z0-9 .:-]{0,10}".prop_map(|value| value.trim().to_owned())
    }

    /// Distinct emphasis markers, innermost first, with at most one backtick run among them.
    ///
    /// A marker repeated in the nesting (`**` in `**`) recurs inside as a run of its own
    /// length, which the rule keeps by design, so it is excluded.
    fn wraps() -> impl Strategy<Value = Vec<String>> {
        (
            Just(vec!["**", "__", "*", "_"]).prop_shuffle(),
            0..=4_usize,
            prop::option::of(1..=3_usize),
            any::<prop::sample::Index>(),
        )
            .prop_map(|(markers, take, ticks, at)| {
                let mut wraps: Vec<String> =
                    markers.into_iter().take(take).map(str::to_owned).collect();
                if let Some(ticks) = ticks {
                    wraps.insert(at.index(wraps.len() + 1), "`".repeat(ticks));
                }
                wraps
            })
    }

    proptest! {
        /// Recover the value from any nesting of distinct wrapping markers, spaced or not.
        #[test]
        fn wrapped_value_strips_back(value in plain_value(), wraps in wraps(), spaced: bool) {
            let pad = if spaced { " " } else { "" };
            let wrapped = wraps
                .iter()
                .fold(value.clone(), |inner, marker| format!("{marker}{pad}{inner}{pad}{marker}"));
            prop_assert_eq!(strip_wrapping_emphasis(&wrapped), value);
        }
    }
}

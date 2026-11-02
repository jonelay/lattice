use std::collections::BTreeSet;

use latt_adapter::fence::FenceState;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Table {
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<Row>,
    pub(crate) line: usize,
    /// One-based line of the heading the table sits under, if any.
    pub(crate) heading_line: Option<usize>,
}

/// A table not returned because its header or separator row is not valid UTF-8.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Withheld {
    pub(crate) line: usize,
    pub(crate) rows: usize,
    /// The undecodable line: the header's or the separator's.
    pub(crate) cause: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Row {
    pub(crate) cells: Vec<String>,
    pub(crate) line: usize,
}

fn split_cells(inner: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' && chars.peek() == Some(&'|') {
            chars.next();
            current.push('|');
        } else if character == '|' {
            cells.push(current.trim().to_owned());
            current = String::new();
        } else {
            current.push(character);
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

fn cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let without_leading = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = without_leading.strip_suffix('|').unwrap_or(without_leading);
    Some(split_cells(inner))
}

fn is_separator_cell(cell: &str) -> bool {
    let middle = cell.strip_prefix(':').unwrap_or(cell);
    let middle = middle.strip_suffix(':').unwrap_or(middle);
    !middle.is_empty() && middle.chars().all(|character| character == '-')
}

fn is_separator(row: &[String]) -> bool {
    !row.is_empty() && row.iter().all(|cell| is_separator_cell(cell))
}

/// A header row whose separator row is a separator of a different width.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Mismatch<'a> {
    pub(crate) heading: Option<&'a str>,
    pub(crate) line: usize,
    pub(crate) headers: usize,
    pub(crate) separators: usize,
}

/// Tables found in one file, and the would-be tables that could not be read.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Scan<'a> {
    pub(crate) tables: Vec<(Option<&'a str>, Table)>,
    pub(crate) mismatches: Vec<Mismatch<'a>>,
    /// One-based line of a fence never closed: nothing after it was read.
    pub(crate) unclosed_fence: Option<usize>,
    pub(crate) withheld: Vec<Withheld>,
}

/// Strip a line's blockquote markers (`>`, `> `, nested `>>`), up to three spaces before each.
///
/// A table or heading inside a blockquote is still one, so it is read rather than
/// mistaken for a row whose first cell is `>` and dropped with no finding.
fn unquote(line: &str) -> &str {
    let mut rest = line;
    loop {
        let indent = rest.len() - rest.trim_start_matches(' ').len();
        let Some(after) = rest[indent..].strip_prefix('>').filter(|_| indent <= 3) else {
            return rest;
        };
        rest = after.strip_prefix(' ').unwrap_or(after);
    }
}

#[cfg(test)]
fn tables(text: &str) -> Vec<(Option<&str>, Table)> {
    scan(text).tables
}

#[cfg(test)]
pub(crate) fn scan(text: &str) -> Scan<'_> {
    scan_unread(text, &BTreeSet::new())
}

/// Find markdown tables while preserving each data row's one-based source line.
///
/// A header over a separator row of another width is not a table, but it is
/// plainly meant as one, so it is returned as a mismatch for the caller to
/// report rather than skipped with every row beneath it.
///
/// Fenced code is not register input: a table or heading inside a fence is an
/// example, not a declaration. Lines are classified before any table logic
/// sees them, so a fence can neither open a table nor end one early.
///
/// `unread` holds the lines that are not valid UTF-8. A header or separator row
/// on such a line defines no columns anyone wrote, so its table is withheld
/// whole rather than read under a header it does not have. Rows on such lines
/// are returned as usual; the caller skips them.
pub(crate) fn scan_unread<'a>(text: &'a str, unread: &BTreeSet<usize>) -> Scan<'a> {
    let mut fences = FenceState::new();
    let lines: Vec<&str> = (1..)
        .zip(text.lines().map(unquote))
        .map(|(number, line)| {
            if fences.in_code(line, number) {
                ""
            } else {
                line
            }
        })
        .collect();
    let mut found = Vec::new();
    let mut mismatches = Vec::new();
    let mut heading = None;
    let mut heading_line = None;
    let mut withheld = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let hash_count = lines[index]
            .chars()
            .take_while(|character| *character == '#')
            .count();
        if hash_count > 0 && lines[index].as_bytes().get(hash_count) == Some(&b' ') {
            heading = Some(lines[index][hash_count + 1..].trim());
            heading_line = Some(index + 1);
            index += 1;
            continue;
        }
        if index + 1 >= lines.len() {
            break;
        }
        let Some(headers) = cells(lines[index]) else {
            index += 1;
            continue;
        };
        let Some(separator) = cells(lines[index + 1]) else {
            index += 1;
            continue;
        };
        if !is_separator(&separator) {
            index += 1;
            continue;
        }
        if headers.len() != separator.len() {
            mismatches.push(Mismatch {
                heading,
                line: index + 1,
                headers: headers.len(),
                separators: separator.len(),
            });
            index += 2;
            while index < lines.len() && cells(lines[index]).is_some() {
                index += 1;
            }
            continue;
        }

        let line = index + 1;
        index += 2;
        let mut rows = Vec::new();
        while index < lines.len() {
            let Some(row) = cells(lines[index]) else {
                break;
            };
            rows.push(Row {
                cells: row,
                line: index + 1,
            });
            index += 1;
        }
        if let Some(cause) = [line, line + 1].into_iter().find(|n| unread.contains(n)) {
            withheld.push(Withheld {
                line,
                rows: rows.len(),
                cause,
            });
            continue;
        }
        found.push((
            heading,
            Table {
                headers,
                rows,
                line,
                heading_line,
            },
        ));
    }

    Scan {
        tables: found,
        mismatches,
        unclosed_fence: fences.unclosed().map(|line| line as usize),
        withheld,
    }
}

#[cfg(test)]
mod tests {
    use super::{Mismatch, Row, Table, scan, tables};

    #[test]
    fn separator_of_another_width_is_a_mismatch_not_silence() {
        let text = "## Reqs\n| ID | Title | Parent |\n|---|---|\n| A | a | |\n| B | b | A |\n";
        let result = scan(text);
        assert!(result.tables.is_empty());
        assert_eq!(
            result.mismatches,
            vec![Mismatch {
                heading: Some("Reqs"),
                line: 2,
                headers: 3,
                separators: 2,
            }]
        );
    }

    #[test]
    fn finds_rows_and_source_lines() {
        let text = "heading\n\n| ID | Text |\n| --- | :---: |\n| A | hello |\n";
        assert_eq!(
            tables(text),
            vec![(
                None,
                Table {
                    headers: vec!["ID".to_owned(), "Text".to_owned()],
                    rows: vec![Row {
                        cells: vec!["A".to_owned(), "hello".to_owned()],
                        line: 5,
                    }],
                    line: 3,
                    heading_line: None,
                },
            )]
        );
    }

    #[test]
    fn preserves_short_rows_for_the_caller_to_report() {
        let text = "| ID | Text | More |\n| --- | --- | --- |\n| A | short |\n";
        assert_eq!(tables(text)[0].1.rows[0].cells.len(), 2);
    }

    #[test]
    fn ignores_pipe_text_without_a_separator_row() {
        assert!(tables("one | two\nnot a separator\n").is_empty());
    }

    #[test]
    fn escaped_pipes_are_not_column_separators() {
        let text = "| ID | Text |\n| --- | --- |\n| A | hello \\| world |\n";
        let result = tables(text);
        assert_eq!(result[0].1.rows[0].cells.len(), 2);
        assert_eq!(result[0].1.rows[0].cells[1], "hello | world");
    }

    #[test]
    fn preserves_non_ascii_headers_and_cell_content() {
        let text = "| Identifiant | Résumé |\n| --- | --- |\n| 要件-1 | café \\| 東京 |\n";
        let result = tables(text);
        assert_eq!(result[0].1.headers, ["Identifiant", "Résumé"]);
        assert_eq!(result[0].1.rows[0].cells, ["要件-1", "café | 東京"]);
    }

    #[test]
    fn accepts_left_and_right_aligned_separator_cells() {
        let text = "| ID | Text |\n| :--- | ---: |\n| A | hello |\n";
        assert_eq!(tables(text).len(), 1);
    }

    #[test]
    fn a_separator_cell_needs_only_one_hyphen() {
        for separator in ["|-|-|", "|--|--|", "| :-: | -: |", "|:-|---|"] {
            let text = format!("| ID | Text |\n{separator}\n| A | hello |\n");
            assert_eq!(tables(&text).len(), 1, "{separator}");
        }
    }

    #[test]
    fn a_separator_cell_without_a_hyphen_is_not_a_separator() {
        assert!(tables("| ID | Text |\n| : | :: |\n| A | hello |\n").is_empty());
    }

    #[test]
    fn tracks_the_heading_above_a_table() {
        let text = "##  Requirements  \n\n| ID | Text |\n| --- | --- |\n| A | hello |\n";
        assert_eq!(tables(text)[0].0, Some("Requirements"));
    }

    #[test]
    fn a_table_before_any_heading_has_no_heading() {
        let text = "| ID | Text |\n| --- | --- |\n| A | hello |\n";
        assert_eq!(tables(text)[0].0, None);
    }

    #[test]
    fn a_deeper_heading_replaces_the_previous_heading() {
        let text = "## Section\n### Subsection\n| ID | Text |\n| --- | --- |\n| A | hello |\n";
        assert_eq!(tables(text)[0].0, Some("Subsection"));
    }

    #[test]
    fn a_fenced_table_is_code_not_register_input() {
        let text = "```\n| ID | Text |\n| --- | --- |\n| A | in fence |\n```\n\n\
                    | ID | Text |\n| --- | --- |\n| B | real |\n";
        let result = tables(text);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.rows[0].cells[0], "B");
    }

    #[test]
    fn a_fenced_heading_does_not_select_the_table_below() {
        let text = "## Real\n~~~md\n## Fenced\n~~~\n| ID | Text |\n| --- | --- |\n| A | a |\n";
        assert_eq!(tables(text)[0].0, Some("Real"));
    }

    #[test]
    fn a_blockquoted_table_and_heading_are_read() {
        let text = "> ## Quoted\n>\n> | ID | Text |\n> | --- | --- |\n> | A | a, b |\n\n\
                    >> | ID | Text |\n>>|---|---|\n>> | B | b |\n";
        let result = tables(text);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, Some("Quoted"));
        assert_eq!(result[0].1.headers, ["ID", "Text"]);
        assert_eq!(result[0].1.rows[0].cells, ["A", "a, b"]);
        assert_eq!(result[0].1.rows[0].line, 5);
        assert_eq!(result[1].1.rows[0].cells, ["B", "b"]);
    }

    #[test]
    fn a_fence_inside_a_blockquote_is_code() {
        let text = "> ```\n> | ID | Text |\n> | --- | --- |\n> | A | a |\n> ```\n";
        let result = scan(text);
        assert!(result.tables.is_empty());
        assert_eq!(result.unclosed_fence, None);
    }

    #[test]
    fn an_unclosed_fence_is_reported_where_it_opens() {
        let text = "| ID | Text |\n| --- | --- |\n| A | a |\n\n````\n| ID | Text |\n| --- | --- |\n| B | b |\n```\n";
        let result = scan(text);
        assert_eq!(result.tables.len(), 1);
        assert_eq!(result.unclosed_fence, Some(5));
    }
}

#[cfg(test)]
mod properties {
    use proptest::prelude::*;

    use super::{scan, scan_unread, split_cells};

    /// Cell text that survives `trim` unchanged, with no pipe and no backslash.
    fn plain_cell() -> impl Strategy<Value = String> {
        "[A-Za-z0-9 _.:,-]{0,12}".prop_map(|cell| cell.trim().to_owned())
    }

    /// Cell text that may hold a pipe but ends on no backslash, so `\|` is its only escape.
    fn piped_cell() -> impl Strategy<Value = String> {
        "[A-Za-z0-9 _.|\\\\-]{0,12}"
            .prop_map(|cell| cell.trim().trim_end_matches(['\\', ' ']).to_owned())
    }

    proptest! {
        /// Report exactly the rows given a byte that is not UTF-8, and return every
        /// other row as written: one stray byte withholds only its own line.
        #[test]
        fn undecodable_rows_withhold_only_themselves(
            rows in prop::collection::vec((plain_cell(), any::<bool>()), 1..20),
        ) {
            let mut bytes = b"| ID | Text |\n|---|---|\n".to_vec();
            for (index, (text, corrupt)) in rows.iter().enumerate() {
                bytes.extend(format!("| R{index} | {text}").as_bytes());
                if *corrupt {
                    bytes.push(0xff);
                }
                bytes.extend(b" |\n");
            }
            let (text, invalid) = latt_adapter::decode_lines(&bytes);
            let unread: std::collections::BTreeSet<usize> =
                invalid.iter().map(|line| line.line as usize).collect();
            let corrupt: std::collections::BTreeSet<usize> =
                (3..).zip(&rows).filter(|(_, (_, c))| *c).map(|(line, _)| line).collect();
            prop_assert_eq!(&unread, &corrupt);

            let scanned = scan_unread(&text, &unread);
            prop_assert!(scanned.withheld.is_empty());
            prop_assert_eq!(scanned.tables.len(), 1);
            let read: Vec<(usize, Vec<String>)> = scanned.tables[0].1.rows.iter()
                .filter(|row| !unread.contains(&row.line))
                .map(|row| (row.line, row.cells.clone()))
                .collect();
            let expected: Vec<(usize, Vec<String>)> = (3..).zip(rows.iter().enumerate())
                .filter(|(_, (_, (_, c)))| !*c)
                .map(|(line, (index, (text, _)))| (line, vec![format!("R{index}"), text.clone()]))
                .collect();
            prop_assert_eq!(read, expected);
        }

        /// Recover every cell of a row joined with `|`: a pipe splits, nothing is lost or merged.
        #[test]
        fn joined_plain_cells_split_back(cells in prop::collection::vec(plain_cell(), 1..6)) {
            prop_assert_eq!(split_cells(&cells.join("|")), cells);
        }

        /// Recover every cell when its pipes are written `\|`: escaping is the inverse of the split.
        #[test]
        fn escaped_pipes_round_trip(cells in prop::collection::vec(piped_cell(), 1..6)) {
            let joined = cells
                .iter()
                .map(|cell| cell.replace('|', "\\|"))
                .collect::<Vec<_>>()
                .join("|");
            prop_assert_eq!(split_cells(&joined), cells);
        }

        /// Produce one more cell than unescaped pipes on any input: no pipe is dropped or doubled.
        #[test]
        fn cell_count_is_unescaped_pipes_plus_one(inner in "[^\\r\\n]{0,40}") {
            let mut unescaped = 0;
            let mut chars = inner.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' && chars.peek() == Some(&'|') {
                    chars.next();
                } else if c == '|' {
                    unescaped += 1;
                }
            }
            prop_assert_eq!(split_cells(&inner).len(), unescaped + 1);
        }

        /// Keep every row of a table whatever its width, so a short or long row reaches
        /// the caller to be reported rather than vanishing at the parse.
        #[test]
        fn every_row_survives_the_scan(
            headers in 1usize..5,
            widths in prop::collection::vec(1usize..7, 1..8),
        ) {
            let mut text = format!("|{}\n|{}\n", " h |".repeat(headers), " - |".repeat(headers));
            for width in &widths {
                text.push_str(&format!("|{}\n", " x |".repeat(*width)));
            }
            let result = scan(&text);
            prop_assert_eq!(result.tables.len(), 1);
            let table = &result.tables[0].1;
            prop_assert_eq!(table.headers.len(), headers);
            let seen: Vec<usize> = table.rows.iter().map(|row| row.cells.len()).collect();
            prop_assert_eq!(&seen, &widths);
            let lines: Vec<usize> = table.rows.iter().map(|row| row.line).collect();
            prop_assert_eq!(lines, (3..3 + widths.len()).collect::<Vec<_>>());
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Table {
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<Row>,
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
    middle.chars().count() >= 3 && middle.chars().all(|character| character == '-')
}

fn is_separator(row: &[String]) -> bool {
    !row.is_empty() && row.iter().all(|cell| is_separator_cell(cell))
}

/// Find markdown tables while preserving each data row's one-based source line.
pub(crate) fn tables(text: &str) -> Vec<Table> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut index = 0;

    while index + 1 < lines.len() {
        let Some(headers) = cells(lines[index]) else {
            index += 1;
            continue;
        };
        let Some(separator) = cells(lines[index + 1]) else {
            index += 1;
            continue;
        };
        if headers.len() != separator.len() || !is_separator(&separator) {
            index += 1;
            continue;
        }

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
        found.push(Table { headers, rows });
    }

    found
}

#[cfg(test)]
mod tests {
    use super::{Row, Table, tables};

    #[test]
    fn finds_rows_and_source_lines() {
        let text = "heading\n\n| ID | Text |\n| --- | :---: |\n| A | hello |\n";
        assert_eq!(
            tables(text),
            vec![Table {
                headers: vec!["ID".to_owned(), "Text".to_owned()],
                rows: vec![Row {
                    cells: vec!["A".to_owned(), "hello".to_owned()],
                    line: 5,
                }],
            }]
        );
    }

    #[test]
    fn preserves_short_rows_for_the_caller_to_report() {
        let text = "| ID | Text | More |\n| --- | --- | --- |\n| A | short |\n";
        assert_eq!(tables(text)[0].rows[0].cells.len(), 2);
    }

    #[test]
    fn ignores_pipe_text_without_a_separator_row() {
        assert!(tables("one | two\nnot a separator\n").is_empty());
    }

    #[test]
    fn escaped_pipes_are_not_column_separators() {
        let text = "| ID | Text |\n| --- | --- |\n| A | hello \\| world |\n";
        let result = tables(text);
        assert_eq!(result[0].rows[0].cells.len(), 2);
        assert_eq!(result[0].rows[0].cells[1], "hello | world");
    }

    #[test]
    fn preserves_non_ascii_headers_and_cell_content() {
        let text = "| Identifiant | Résumé |\n| --- | --- |\n| 要件-1 | café \\| 東京 |\n";
        let result = tables(text);
        assert_eq!(result[0].headers, ["Identifiant", "Résumé"]);
        assert_eq!(result[0].rows[0].cells, ["要件-1", "café | 東京"]);
    }

    #[test]
    fn accepts_left_and_right_aligned_separator_cells() {
        let text = "| ID | Text |\n| :--- | ---: |\n| A | hello |\n";
        assert_eq!(tables(text).len(), 1);
    }
}

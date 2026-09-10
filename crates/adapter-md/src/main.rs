mod config;
mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use adapter_core::{Document, Edge, Node, Provenance};
use clap::Parser;
use regex::Regex;
use serde_json::Value;

use config::{Config, IdColumn, TableConfig};
#[derive(Debug, Parser)]
#[command(about = "Read markdown tables into a lattice interface document")]
struct Args {
    #[arg(long)]
    profile: PathBuf,
    #[arg(long)]
    target: PathBuf,
}

struct FileMatcher {
    patterns: Vec<Regex>,
}

impl FileMatcher {
    fn new(patterns: &[String]) -> Result<Self, String> {
        let patterns = patterns
            .iter()
            .map(|pattern| {
                Regex::new(&glob_regex(pattern))
                    .map_err(|error| format!("invalid file glob '{pattern}' in profile: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { patterns })
    }

    fn matches(&self, path: &str) -> bool {
        self.patterns.iter().any(|pattern| pattern.is_match(path))
    }
}

fn glob_regex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut regex = String::from("^");
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                if chars.get(index) == Some(&'/') {
                    regex.push_str("(?:.*/)?");
                    index += 1;
                } else {
                    regex.push_str(".*");
                }
            }
            '*' => {
                regex.push_str("[^/]*");
                index += 1;
            }
            '?' => {
                regex.push_str("[^/]");
                index += 1;
            }
            character => {
                regex.push_str(&regex::escape(&character.to_string()));
                index += 1;
            }
        }
    }
    regex.push('$');
    regex
}

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map_or_else(
            || {
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into(),
                )
            },
            |relative| relative.to_string_lossy().replace('\\', "/"),
        )
}

fn visit_directory(
    document: &mut Document<'_>,
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            let file = display_path(directory, root);
            document.parse_error(
                format!("could not read directory '{file}': {error}"),
                file,
                0,
            );
            return;
        }
    };

    for result in entries {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                document.parse_error(
                    format!(
                        "could not read an entry in '{}': {error}",
                        directory.display()
                    ),
                    display_path(directory, root),
                    0,
                );
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                let file = display_path(&path, root);
                document.parse_error(format!("could not inspect '{file}': {error}"), file, 0);
                continue;
            }
        };
        if file_type.is_dir() {
            visit_directory(document, root, &path, files);
        } else if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            files.push(path);
        }
    }
}

fn selected_files(
    document: &mut Document<'_>,
    target: &Path,
    matcher: &FileMatcher,
) -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    if target.is_dir() {
        visit_directory(document, target, target, &mut paths);
    } else if target.is_file() {
        paths.push(target.to_owned());
    } else {
        document.parse_error(
            format!(
                "target '{}' does not exist or is unreadable",
                target.display()
            ),
            target.display().to_string(),
            0,
        );
    }

    let mut selected: Vec<_> = paths
        .into_iter()
        .filter_map(|path| {
            let display = display_path(&path, target);
            matcher.matches(&display).then_some((path, display))
        })
        .collect();
    selected.sort_by(|left, right| left.1.cmp(&right.1));
    selected
}

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

fn read_table<'a>(
    document: &mut Document<'a>,
    config: &'a TableConfig,
    table: parse::Table,
    file: &str,
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

    for row in table.rows {
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
        if id.is_empty() {
            document.parse_error("table row has an empty ID column", file, row.line as u32);
            continue;
        }

        let attrs = attributes
            .iter()
            .map(|(index, name)| ((*name).to_owned(), Value::String(row.cells[*index].clone())))
            .collect();
        let provenance = Provenance::new(file, row.line as u32);
        document.nodes.push(Node {
            id: id.to_owned(),
            kind: &config.kind,
            attrs,
            provenance: provenance.clone(),
        });

        for (index, kind) in &edges {
            for target in row.cells[*index]
                .split(',')
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                document.edges.push(Edge {
                    src: id.to_owned(),
                    tgt: target.to_owned(),
                    kind,
                    attrs: BTreeMap::new(),
                    provenance: provenance.clone(),
                });
            }
        }
    }
}

/// Read every configured markdown file into an append-only interface document.
fn build_document<'a>(config: &'a Config, target: &Path) -> Result<Document<'a>, String> {
    let matcher = FileMatcher::new(&config.files)?;
    let mut document = Document::default();
    for (path, file) in selected_files(&mut document, target, &matcher) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                document.parse_error(format!("could not read '{file}': {error}"), file, 0);
                continue;
            }
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) => {
                document.parse_error(format!("{file}: not valid UTF-8: {error}"), file, 0);
                continue;
            }
        };
        for (heading, table) in parse::tables(text) {
            let heading_text = heading.as_deref().unwrap_or("");
            if let Some(table_config) = config
                .tables
                .iter()
                .find(|table_config| table_config.heading.is_match(heading_text))
            {
                read_table(&mut document, table_config, table, &file);
            } else if config.multi_table {
                let heading = heading.as_deref().unwrap_or("no heading");
                document.parse_error(
                    format!("table under heading '{heading}' matches no table configuration"),
                    &file,
                    table.line as u32,
                );
            }
        }
    }
    Ok(document)
}

fn absolute_path(path: PathBuf) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(std::fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn run(args: Args) -> Result<(), String> {
    let config = Config::load(&args.profile)?;
    let target = absolute_path(args.target).map_err(|error| error.to_string())?;
    let document = build_document(&config, &target)?;

    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, &document)
        .map_err(|error| format!("could not serialize interface document: {error}"))?;
    writeln!(writer).map_err(|error| format!("could not write interface document: {error}"))?;
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(error) = run(args) {
        eprintln!("Error: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::glob_regex;
    use regex::Regex;

    #[test]
    fn recursive_glob_also_matches_a_root_file() {
        let pattern = Regex::new(&glob_regex("**/*.md")).unwrap();
        assert!(pattern.is_match("one.md"));
        assert!(pattern.is_match("nested/one.md"));
        assert!(!pattern.is_match("one.toml"));
    }
}

mod config;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use adapter_core::{Document, Edge, Issue, Node, Pathway, Provenance};
use clap::Parser;
use regex::Regex;
use serde_json::{Number, Value as JsonValue};
use toml::Value as TomlValue;

use config::{Config, HeaderConfig, IdPrefix, Mode, PathwayConfig, TableConfig};

#[derive(Debug, Parser)]
#[command(about = "Read TOML registers into a lattice interface document")]
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

fn toml_type(value: &TomlValue) -> &'static str {
    match value {
        TomlValue::String(_) => "string",
        TomlValue::Integer(_) => "integer",
        TomlValue::Float(_) => "float",
        TomlValue::Boolean(_) => "boolean",
        TomlValue::Datetime(_) => "datetime",
        TomlValue::Array(_) => "array",
        TomlValue::Table(_) => "table",
    }
}

fn json_value(value: &TomlValue) -> Result<JsonValue, &'static str> {
    match value {
        TomlValue::String(value) => Ok(JsonValue::String(value.clone())),
        TomlValue::Integer(value) => Ok(JsonValue::Number(Number::from(*value))),
        TomlValue::Float(value) => Number::from_f64(*value)
            .map(JsonValue::Number)
            .ok_or("non-finite float"),
        TomlValue::Boolean(value) => Ok(JsonValue::Bool(*value)),
        TomlValue::Datetime(value) => Ok(JsonValue::String(value.to_string())),
        TomlValue::Array(_) => Err("array"),
        TomlValue::Table(_) => Err("table"),
    }
}

fn mapped_attributes(
    document: &mut Document<'_>,
    source: &toml::map::Map<String, TomlValue>,
    mapping: &BTreeMap<String, String>,
    file: &str,
    node_id: Option<&str>,
) -> BTreeMap<String, JsonValue> {
    let mut attrs = BTreeMap::new();
    for (key, output) in mapping {
        let Some(value) = source.get(key) else {
            continue;
        };
        match json_value(value) {
            Ok(value) => {
                attrs.insert(output.clone(), value);
            }
            Err(shape) => {
                let message =
                    format!("{file}: mapped key '{key}' has type {shape}, expected a scalar value");
                if let Some(node_id) = node_id {
                    document.node_parse_error(message, file, 0, node_id.to_owned());
                } else {
                    document.parse_error(message, file, 0);
                }
            }
        }
    }
    attrs
}

fn read_header<'a>(
    document: &mut Document<'a>,
    config: &'a HeaderConfig,
    root: &toml::map::Map<String, TomlValue>,
    file: &str,
    file_stem: &str,
) {
    let Some(value) = root.get(&config.table) else {
        document.parse_error(
            format!("{file}: no [{}] header table", config.table),
            file,
            0,
        );
        return;
    };
    let TomlValue::Table(table) = value else {
        document.parse_error(
            format!(
                "{file}: header '{}' has type {}, expected a table",
                config.table,
                toml_type(value)
            ),
            file,
            0,
        );
        return;
    };
    let id = if config.id == "file_stem" {
        file_stem.to_owned()
    } else {
        match table.get(&config.id) {
            Some(TomlValue::String(id)) => id.clone(),
            Some(value) => {
                document.parse_error(
                    format!(
                        "{file}: header ID key '{}' has type {}, expected a string",
                        config.id,
                        toml_type(value)
                    ),
                    file,
                    0,
                );
                return;
            }
            None => {
                document.parse_error(
                    format!("{file}: header declares no ID key '{}'", config.id),
                    file,
                    0,
                );
                return;
            }
        }
    };
    let attrs = mapped_attributes(document, table, &config.key_map, file, Some(&id));
    document.nodes.push(Node {
        id,
        kind: &config.kind,
        attrs,
        provenance: Provenance::new(file, 0),
    });
}

fn read_edges<'a>(
    document: &mut Document<'a>,
    config: &'a TableConfig,
    row: &toml::map::Map<String, TomlValue>,
    node_id: &str,
    file: &str,
) {
    for (key, kind) in &config.edge_keys {
        let Some(value) = row.get(key) else {
            continue;
        };
        let targets: Option<Vec<&str>> = match value {
            TomlValue::String(target) => Some(vec![target]),
            TomlValue::Array(targets)
                if targets
                    .iter()
                    .all(|target| matches!(target, TomlValue::String(_))) =>
            {
                Some(targets.iter().filter_map(TomlValue::as_str).collect())
            }
            _ => None,
        };
        let Some(targets) = targets else {
            document.node_parse_error(
                format!(
                    "{file}: item '{node_id}' has edge key '{key}' of type {}, expected a string or array of strings",
                    toml_type(value)
                ),
                file,
                0,
                node_id.to_owned(),
            );
            continue;
        };
        for target in targets {
            document.edges.push(Edge {
                src: node_id.to_owned(),
                tgt: target.to_owned(),
                kind,
                attrs: BTreeMap::new(),
                provenance: Provenance::new(file, 0),
            });
        }
    }
}

fn read_rows<'a>(
    document: &mut Document<'a>,
    config: &'a TableConfig,
    rows: &[TomlValue],
    table_name: &str,
    file: &str,
    file_stem: &str,
    id_prefix: Option<IdPrefix>,
) {
    for (index, value) in rows.iter().enumerate() {
        let TomlValue::Table(row) = value else {
            document.parse_error(
                format!(
                    "{file}: {table_name} row {index} has type {}, expected a table",
                    toml_type(value)
                ),
                file,
                0,
            );
            continue;
        };
        read_record(
            document,
            config,
            row,
            &format!("{table_name} row {index}"),
            file,
            file_stem,
            id_prefix,
        );
    }
}

fn read_record<'a>(
    document: &mut Document<'a>,
    config: &'a TableConfig,
    record: &toml::map::Map<String, TomlValue>,
    description: &str,
    file: &str,
    file_stem: &str,
    id_prefix: Option<IdPrefix>,
) {
    let Some(TomlValue::String(raw_id)) = record.get(&config.id_key) else {
        document.parse_error(
            format!(
                "{file}: {description} declares no string '{}' ID key",
                config.id_key
            ),
            file,
            0,
        );
        return;
    };
    let id = match id_prefix {
        Some(IdPrefix::FileStem) => format!("{file_stem}/{raw_id}"),
        None => raw_id.clone(),
    };
    let attrs = mapped_attributes(document, record, &config.key_map, file, Some(&id));
    document.nodes.push(Node {
        id: id.clone(),
        kind: &config.kind,
        attrs,
        provenance: Provenance::new(file, 0),
    });
    read_edges(document, config, record, &id, file);
}

fn read_file<'a>(
    document: &mut Document<'a>,
    config: &'a Config,
    value: &TomlValue,
    file: &str,
    file_stem: &str,
) {
    let Some(root) = value.as_table() else {
        document.parse_error(format!("{file}: TOML root is not a table"), file, 0);
        return;
    };

    if let Some(header) = &config.header {
        read_header(document, header, root, file, file_stem);
    }

    for (name, value) in root {
        if let Some(table) = config.tables.get(name) {
            match value {
                TomlValue::Array(rows)
                    if rows.iter().all(|row| matches!(row, TomlValue::Table(_))) =>
                {
                    read_rows(
                        document,
                        table,
                        rows,
                        name,
                        file,
                        file_stem,
                        config.id_prefix,
                    );
                }
                _ => document.parse_error(
                    format!(
                        "{file}: configured table '{name}' has type {}, expected an array of tables",
                        toml_type(value)
                    ),
                    file,
                    0,
                ),
            }
        } else if let TomlValue::Array(rows) = value
            && !rows.is_empty()
            && rows.iter().all(|row| matches!(row, TomlValue::Table(_)))
        {
            document.parse_error(
                format!(
                    "{file}: no reader for array-of-tables '{name}' ({} rows not read)",
                    rows.len()
                ),
                file,
                0,
            );
        }
    }
}

fn read_directory_file<'a>(
    document: &mut Document<'a>,
    config: &'a Config,
    value: &TomlValue,
    file: &str,
    file_stem: &str,
) {
    let Some(root) = value.as_table() else {
        document.parse_error(format!("{file}: TOML root is not a table"), file, 0);
        return;
    };
    let table = config
        .tables
        .values()
        .next()
        .expect("configuration requires at least one table");
    read_record(
        document,
        table,
        root,
        "record",
        file,
        file_stem,
        config.id_prefix,
    );
}

fn dot_path<'a>(value: &'a TomlValue, path: &str) -> Option<&'a TomlValue> {
    let mut current = value;
    for component in path.split('.') {
        current = current.as_table()?.get(component)?;
    }
    Some(current)
}

fn attach_pathway(
    document: &mut Document<'_>,
    config: &PathwayConfig,
    parsed: &BTreeMap<String, TomlValue>,
) {
    let source_file = config.source_file.replace('\\', "/");
    let Some(source) = parsed.get(&source_file) else {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' source file was not read",
                config.source_file, config.name
            ),
            &config.source_file,
        );
        return;
    };
    let order = dot_path(source, &config.order_key);
    let current = dot_path(source, &config.current_key);
    if order.is_none() && current.is_none() {
        return;
    }
    let (Some(order), Some(current)) = (order, current) else {
        let missing = if order.is_none() {
            &config.order_key
        } else {
            &config.current_key
        };
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' declares only one of the pair, missing '{missing}'",
                config.source_file, config.name
            ),
            &config.source_file,
        );
        return;
    };
    let (TomlValue::Array(raw_order), TomlValue::String(current)) = (order, current) else {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' expects '{}' to be an array of strings and '{}' to be a string",
                config.source_file, config.name, config.order_key, config.current_key
            ),
            &config.source_file,
        );
        return;
    };
    let Some(order): Option<Vec<String>> = raw_order
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect()
    else {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' order contains a non-string value",
                config.source_file, config.name
            ),
            &config.source_file,
        );
        return;
    };
    let unique: BTreeSet<_> = order.iter().collect();
    if order.is_empty() || unique.len() != order.len() || !order.contains(current) {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' requires distinct positions and current '{}' among them",
                config.source_file, config.name, current
            ),
            &config.source_file,
        );
        return;
    }
    document.pathways.push(Pathway {
        name: config.name.clone(),
        order,
        current: current.clone(),
    });
}

fn parse_line(error: &toml::de::Error, text: &str) -> usize {
    error
        .span()
        .map(|span| {
            text[..span.start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1
        })
        .unwrap_or(0)
}

/// Read every configured TOML file into an append-only interface document.
fn build_document<'a>(config: &'a Config, target: &Path) -> Result<Document<'a>, String> {
    let matcher = FileMatcher::new(&config.files)?;
    let mut document = Document::default();
    let mut parsed = BTreeMap::new();
    let target_is_readable = target.is_dir() || target.is_file();
    let files = selected_files(&mut document, target, &matcher);
    if target_is_readable && files.is_empty() {
        document.findings.push(Issue {
            severity: "info",
            code: "NO_MATCHING_FILES",
            message: "no files matched adapter.paths.files".to_owned(),
            provenance: Provenance::new(".", 0),
            node_id: None,
        });
    }
    for (path, file) in files {
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
        let value: TomlValue = match toml::from_str(text) {
            Ok(value) => value,
            Err(error) => {
                let line = parse_line(&error, text);
                document.parse_error(
                    format!("{file}: could not parse TOML: {error}"),
                    file,
                    line as u32,
                );
                continue;
            }
        };
        let file_stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        match config.mode.unwrap_or_default() {
            Mode::Tables => read_file(&mut document, config, &value, &file, &file_stem),
            Mode::Directory => {
                read_directory_file(&mut document, config, &value, &file, &file_stem)
            }
        }
        parsed.insert(file, value);
    }
    if target_is_readable && let Some(pathway) = &config.pathway {
        attach_pathway(&mut document, pathway, &parsed);
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
    use std::collections::BTreeMap;

    use super::{attach_pathway, glob_regex, json_value};
    use crate::config::{Mode, PathwayConfig};
    use adapter_core::Document;
    use regex::Regex;
    use serde_json::json;
    use toml::Value as TomlValue;

    #[test]
    fn recursive_glob_also_matches_a_root_file() {
        let pattern = Regex::new(&glob_regex("**/*.toml")).unwrap();
        assert!(pattern.is_match("one.toml"));
        assert!(pattern.is_match("nested/one.toml"));
        assert!(!pattern.is_match("one.md"));
    }

    #[test]
    fn scalar_types_map_without_stringifying() {
        assert_eq!(json_value(&TomlValue::Integer(3)).unwrap(), json!(3));
        assert_eq!(json_value(&TomlValue::Boolean(true)).unwrap(), json!(true));
        assert!(json_value(&TomlValue::Array(Vec::new())).is_err());
    }

    #[test]
    fn directory_mode_config_deserializes() {
        let mode: Mode = serde_json::from_str(r#""directory""#).unwrap();
        assert!(matches!(mode, Mode::Directory));
    }

    #[test]
    fn valid_pathway_is_attached() {
        let source: TomlValue =
            toml::from_str("[register]\nstage_order = [\"s1\", \"s2\"]\ncurrent_stage = \"s2\"\n")
                .unwrap();
        let mut parsed = BTreeMap::new();
        parsed.insert("registers/index.toml".to_owned(), source);
        let config = PathwayConfig {
            source_file: "registers/index.toml".to_owned(),
            name: "stage".to_owned(),
            order_key: "register.stage_order".to_owned(),
            current_key: "register.current_stage".to_owned(),
        };
        let mut document = Document::default();
        attach_pathway(&mut document, &config, &parsed);
        assert_eq!(document.pathways.len(), 1);
        assert_eq!(document.pathways[0].current, "s2");
    }
}

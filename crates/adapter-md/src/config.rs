use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) files: Vec<String>,
    pub(crate) tables: Vec<TableConfig>,
    pub(crate) multi_table: bool,
}

#[derive(Debug)]
pub(crate) struct TableConfig {
    pub(crate) heading: Regex,
    pub(crate) kind: String,
    pub(crate) id_column: IdColumn,
    pub(crate) column_map: BTreeMap<String, String>,
    pub(crate) edge_columns: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum IdColumn {
    Index(usize),
    Name(String),
}

#[derive(Debug, Deserialize)]
struct ProfileDocument {
    resolved_schema: String,
    adapter: Adapter,
    node_kinds: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct Adapter {
    paths: Paths,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    table: Presence<Table>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    tables: Presence<Vec<RawTableConfig>>,
}

#[derive(Debug, Default)]
enum Presence<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

fn deserialize_nullable<'de, D, T>(deserializer: D) -> Result<Presence<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(match Option::<T>::deserialize(deserializer)? {
        Some(value) => Presence::Value(value),
        None => Presence::Null,
    })
}

#[derive(Debug, Deserialize)]
struct Paths {
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Table {
    id_column: IdColumn,
    column_map: BTreeMap<String, String>,
    edge_columns: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawTableConfig {
    heading: String,
    kind: String,
    id_column: IdColumn,
    column_map: BTreeMap<String, String>,
    edge_columns: BTreeMap<String, String>,
}

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
        Self::from_slice(&bytes, path)
    }

    fn from_slice(bytes: &[u8], path: &Path) -> Result<Self, String> {
        let profile: ProfileDocument = serde_json::from_slice(bytes)
            .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;

        if profile.resolved_schema != "1" {
            return Err(format!(
                "could not load profile {}: not a resolved profile document",
                path.display()
            ));
        }
        if profile.adapter.paths.files.is_empty()
            || profile
                .adapter
                .paths
                .files
                .iter()
                .any(|pattern| pattern.is_empty())
        {
            return Err(format!(
                "could not load profile {}: 'adapter.paths.files' must contain a file glob",
                path.display()
            ));
        }
        let (tables, multi_table) = match (profile.adapter.table, profile.adapter.tables) {
            (Presence::Value(table), Presence::Absent) => {
                if profile.node_kinds.len() != 1 {
                    return Err(format!(
                        "could not load profile {}: md profiles using 'adapter.table' must declare exactly one node kind, found {}",
                        path.display(),
                        profile.node_kinds.len()
                    ));
                }
                let kind = profile
                    .node_kinds
                    .into_keys()
                    .next()
                    .expect("one node kind was checked");
                let heading = Regex::new("").expect("an empty regex is valid");
                (
                    vec![TableConfig {
                        heading,
                        kind,
                        id_column: table.id_column,
                        column_map: table.column_map,
                        edge_columns: table.edge_columns,
                    }],
                    false,
                )
            }
            (Presence::Absent, Presence::Value(tables)) => {
                let tables = tables
                    .into_iter()
                    .map(|table| {
                        if !profile.node_kinds.contains_key(&table.kind) {
                            return Err(format!(
                                "could not load profile {}: table kind '{}' is not declared in 'node_kinds'",
                                path.display(),
                                table.kind
                            ));
                        }
                        let heading = Regex::new(&table.heading).map_err(|error| {
                            format!(
                                "could not load profile {}: invalid heading regex '{}': {error}",
                                path.display(),
                                table.heading
                            )
                        })?;
                        Ok(TableConfig {
                            heading,
                            kind: table.kind,
                            id_column: table.id_column,
                            column_map: table.column_map,
                            edge_columns: table.edge_columns,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                (tables, true)
            }
            (Presence::Absent, Presence::Absent) => {
                return Err(format!(
                    "could not load profile {}: declare one of 'adapter.table' or 'adapter.tables'",
                    path.display()
                ));
            }
            _ => {
                return Err(format!(
                    "could not load profile {}: declare only one of 'adapter.table' or 'adapter.tables'",
                    path.display()
                ));
            }
        };

        Ok(Self {
            files: profile.adapter.paths.files,
            tables,
            multi_table,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::Config;

    fn profile(adapter_tables: &str, node_kinds: &str) -> String {
        format!(
            r#"{{
                "resolved_schema": "1",
                "adapter": {{
                    "paths": {{"files": ["*.md"]}},
                    {adapter_tables}
                }},
                "node_kinds": {node_kinds}
            }}"#
        )
    }

    fn load(text: &str) -> Result<Config, String> {
        Config::from_slice(text.as_bytes(), Path::new("profile.json"))
    }

    #[test]
    fn wraps_a_single_table_as_a_catch_all_config() {
        let text = profile(
            r#""table": {
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }"#,
            r#"{"requirement": {}}"#,
        );
        let config = load(&text).unwrap();
        assert!(!config.multi_table);
        assert_eq!(config.tables.len(), 1);
        assert_eq!(config.tables[0].kind, "requirement");
        assert!(config.tables[0].heading.is_match("any heading"));
    }

    #[test]
    fn loads_plural_table_configs_and_compiles_headings() {
        let text = profile(
            r#""tables": [{
                "heading": "^Domain [0-9]+$", "kind": "requirement",
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }]"#,
            r#"{"requirement": {}, "stakeholder": {}}"#,
        );
        let config = load(&text).unwrap();
        assert!(config.multi_table);
        assert!(config.tables[0].heading.is_match("Domain 02"));
        assert!(!config.tables[0].heading.is_match("Stakeholders"));
    }

    #[test]
    fn rejects_both_or_neither_table_form() {
        let both = profile(
            r#""table": {
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }, "tables": []"#,
            r#"{"requirement": {}}"#,
        );
        let neither = profile(r#""other": null"#, r#"{"requirement": {}}"#);
        assert!(load(&both).unwrap_err().contains("only one"));
        assert!(load(&neither).unwrap_err().contains("declare one"));
    }

    #[test]
    fn rejects_null_table_or_tables_as_present() {
        let table_with_null_tables = profile(
            r#""table": {
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }, "tables": null"#,
            r#"{"requirement": {}}"#,
        );
        assert!(
            load(&table_with_null_tables)
                .unwrap_err()
                .contains("only one")
        );

        let null_table_with_tables = profile(
            r#""table": null, "tables": [{
                "heading": "", "kind": "requirement",
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }]"#,
            r#"{"requirement": {}}"#,
        );
        assert!(
            load(&null_table_with_tables)
                .unwrap_err()
                .contains("only one")
        );

        let both_null = profile(r#""table": null, "tables": null"#, r#"{"requirement": {}}"#);
        assert!(load(&both_null).unwrap_err().contains("only one"));
    }

    #[test]
    fn rejects_undeclared_kinds_and_invalid_heading_regexes() {
        let undeclared = profile(
            r#""tables": [{
                "heading": "Requirements", "kind": "missing",
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }]"#,
            r#"{"requirement": {}}"#,
        );
        let invalid_regex = profile(
            r#""tables": [{
                "heading": "[", "kind": "requirement",
                "id_column": "ID", "column_map": {}, "edge_columns": {}
            }]"#,
            r#"{"requirement": {}}"#,
        );
        assert!(load(&undeclared).unwrap_err().contains("not declared"));
        assert!(
            load(&invalid_regex)
                .unwrap_err()
                .contains("invalid heading regex")
        );
    }
}

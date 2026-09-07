use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) files: Vec<String>,
    pub(crate) id_column: IdColumn,
    pub(crate) column_map: BTreeMap<String, String>,
    pub(crate) edge_columns: BTreeMap<String, String>,
    pub(crate) node_kind: String,
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
    table: Table,
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

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
        let profile: ProfileDocument = serde_json::from_slice(&bytes)
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
        if profile.node_kinds.len() != 1 {
            return Err(format!(
                "could not load profile {}: mdtable profiles must declare exactly one node kind, found {}",
                path.display(),
                profile.node_kinds.len()
            ));
        }
        let node_kind = profile
            .node_kinds
            .into_keys()
            .next()
            .expect("one node kind was checked");

        Ok(Self {
            files: profile.adapter.paths.files,
            id_column: profile.adapter.table.id_column,
            column_map: profile.adapter.table.column_map,
            edge_columns: profile.adapter.table.edge_columns,
            node_kind,
        })
    }
}

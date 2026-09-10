use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) files: Vec<String>,
    pub(crate) tables: BTreeMap<String, TableConfig>,
    pub(crate) mode: Option<Mode>,
    pub(crate) id_prefix: Option<IdPrefix>,
    pub(crate) header: Option<HeaderConfig>,
    pub(crate) pathway: Option<PathwayConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Tables,
    Directory,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IdPrefix {
    FileStem,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TableConfig {
    pub(crate) kind: String,
    pub(crate) id_key: String,
    pub(crate) key_map: BTreeMap<String, String>,
    pub(crate) edge_keys: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeaderConfig {
    pub(crate) table: String,
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) key_map: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PathwayConfig {
    pub(crate) source_file: String,
    pub(crate) name: String,
    pub(crate) order_key: String,
    pub(crate) current_key: String,
}

#[derive(Debug, Deserialize)]
struct ProfileDocument {
    resolved_schema: String,
    adapter: Adapter,
}

#[derive(Debug, Deserialize)]
struct Adapter {
    paths: Paths,
    tables: BTreeMap<String, TableConfig>,
    #[serde(default)]
    mode: Option<Mode>,
    #[serde(default)]
    id_prefix: Option<IdPrefix>,
    #[serde(default)]
    header: Option<HeaderConfig>,
    #[serde(default)]
    pathway: Option<PathwayConfig>,
}

#[derive(Debug, Deserialize)]
struct Paths {
    files: Vec<String>,
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
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
                .any(|value| !nonempty(value))
        {
            return Err(format!(
                "could not load profile {}: 'adapter.paths.files' must contain a file glob",
                path.display()
            ));
        }
        if profile.adapter.tables.is_empty() {
            return Err(format!(
                "could not load profile {}: 'adapter.tables' must declare at least one table",
                path.display()
            ));
        }
        if matches!(profile.adapter.mode, Some(Mode::Directory)) && profile.adapter.header.is_some()
        {
            return Err(format!(
                "could not load profile {}: 'adapter.header' is not supported when 'adapter.mode' is 'directory'",
                path.display()
            ));
        }
        for (name, table) in &profile.adapter.tables {
            if !nonempty(name)
                || !nonempty(&table.kind)
                || !nonempty(&table.id_key)
                || table
                    .key_map
                    .iter()
                    .any(|(key, value)| !nonempty(key) || !nonempty(value))
                || table
                    .edge_keys
                    .iter()
                    .any(|(key, value)| !nonempty(key) || !nonempty(value))
            {
                return Err(format!(
                    "could not load profile {}: adapter table '{name}' has an empty required value",
                    path.display()
                ));
            }
        }
        if let Some(header) = &profile.adapter.header
            && (!nonempty(&header.table)
                || !nonempty(&header.kind)
                || !nonempty(&header.id)
                || header
                    .key_map
                    .iter()
                    .any(|(key, value)| !nonempty(key) || !nonempty(value)))
        {
            return Err(format!(
                "could not load profile {}: 'adapter.header' has an empty required value",
                path.display()
            ));
        }
        if let Some(pathway) = &profile.adapter.pathway
            && [
                &pathway.source_file,
                &pathway.name,
                &pathway.order_key,
                &pathway.current_key,
            ]
            .into_iter()
            .any(|value| !nonempty(value))
        {
            return Err(format!(
                "could not load profile {}: 'adapter.pathway' has an empty required value",
                path.display()
            ));
        }

        Ok(Self {
            files: profile.adapter.paths.files,
            tables: profile.adapter.tables,
            mode: profile.adapter.mode,
            id_prefix: profile.adapter.id_prefix,
            header: profile.adapter.header,
            pathway: profile.adapter.pathway,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{IdPrefix, ProfileDocument};

    #[test]
    fn resolved_profile_decodes_the_adapter_vocabulary() {
        let profile: ProfileDocument = serde_json::from_str(
            r#"{
                "resolved_schema":"1",
                "adapter":{
                    "paths":{"files":["registers/*.toml"]},
                    "id_prefix":"file_stem",
                    "tables":{"item":{
                        "kind":"item", "id_key":"id",
                        "key_map":{"stage":"stage"},
                        "edge_keys":{"refs":"references"}
                    }}
                }
            }"#,
        )
        .unwrap();
        assert!(matches!(
            profile.adapter.id_prefix,
            Some(IdPrefix::FileStem)
        ));
        assert_eq!(profile.adapter.tables["item"].kind, "item");
    }
}

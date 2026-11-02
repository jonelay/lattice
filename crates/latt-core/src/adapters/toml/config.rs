use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

/// Built only through `new`, so config consumers never re-check for blank values.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct NonEmptyString(String);

impl NonEmptyString {
    pub(crate) fn new(value: String, what: &str) -> Result<Self, String> {
        if value.trim().is_empty() {
            return Err(format!("{what} must not be empty"));
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for NonEmptyString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for NonEmptyString {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NonEmptyString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn field(value: String, parent: &str, name: &str) -> Result<NonEmptyString, String> {
    NonEmptyString::new(value, &format!("'{parent}.{name}'"))
}

/// How one `key_map` key becomes attrs.
#[derive(Debug)]
pub(crate) enum AttrMapping {
    /// The whole value is the attr, so an array with an entry the adapter cannot
    /// read sets nothing: its readable part would be a list the register never
    /// declared.
    Whole(NonEmptyString),
    /// Each attr is the entries of one shape, so an entry no shape claims costs
    /// no attr.
    Shapes(Shapes),
}

/// The entry shapes a `key_map` key declares: scalar entries into one attr, and
/// each named field of table entries into one. Several may name the same attr.
#[derive(Debug)]
pub(crate) struct Shapes {
    pub(crate) scalars: Option<NonEmptyString>,
    pub(crate) tables: BTreeMap<NonEmptyString, NonEmptyString>,
}

pub(crate) type KeyMap = BTreeMap<NonEmptyString, AttrMapping>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShapes {
    #[serde(default)]
    scalars: Option<String>,
    #[serde(default)]
    tables: BTreeMap<String, String>,
}

fn attr_mapping(value: serde_json::Value, at: &str) -> Result<AttrMapping, String> {
    let shapes: RawShapes = match value {
        serde_json::Value::String(attr) => {
            return Ok(AttrMapping::Whole(NonEmptyString::new(
                attr,
                &format!("'{at}'"),
            )?));
        }
        value @ serde_json::Value::Object(_) => {
            serde_json::from_value(value).map_err(|error| format!("'{at}': {error}"))?
        }
        _ => return Err(format!("'{at}' must be an attr name or a table of shapes")),
    };
    if shapes.scalars.is_none() && shapes.tables.is_empty() {
        return Err(format!(
            "'{at}' declares no shape: give 'scalars', 'tables' or both"
        ));
    }
    let tables_at = format!("{at}.tables");
    Ok(AttrMapping::Shapes(Shapes {
        scalars: shapes
            .scalars
            .map(|s| field(s, at, "scalars"))
            .transpose()?,
        tables: shapes
            .tables
            .into_iter()
            .map(|(name, attr)| {
                let name = NonEmptyString::new(name, &format!("a key of '{tables_at}'"))?;
                let attr = field(attr, &tables_at, name.as_str())?;
                Ok((name, attr))
            })
            .collect::<Result<_, String>>()?,
    }))
}

fn key_map(map: BTreeMap<String, serde_json::Value>, key: &str) -> Result<KeyMap, String> {
    map.into_iter()
        .map(|(name, value)| {
            let at = format!("{key}.{name}");
            let name = NonEmptyString::new(name, &format!("a key of '{key}'"))?;
            Ok((name, attr_mapping(value, &at)?))
        })
        .collect()
}

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) files: Vec<NonEmptyString>,
    pub(crate) tables: BTreeMap<NonEmptyString, TableConfig>,
    pub(crate) mode: Option<Mode>,
    pub(crate) id_prefix: Option<IdPrefix>,
    pub(crate) header: Option<HeaderConfig>,
    pub(crate) pathway: Option<PathwayConfig>,
    /// The resolved profile's kinds, for the source-existence check.
    pub(crate) node_kinds: BTreeMap<String, serde_json::Value>,
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

/// A table-level ID prefix: `kind` puts the node kind before the row's ID, so
/// two tables whose rows share names stay apart in the one ID namespace.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TableIdPrefix {
    Kind,
}

#[derive(Debug)]
pub(crate) struct TableConfig {
    pub(crate) kind: NonEmptyString,
    pub(crate) id_key: NonEmptyString,
    pub(crate) id_prefix: Option<TableIdPrefix>,
    pub(crate) key_map: KeyMap,
    pub(crate) edge_keys: BTreeMap<NonEmptyString, EdgeKey>,
}

/// One `edge_keys` entry. The string form is `kind` alone: the value is the
/// target ID as written. `table` builds the target ID the way that table
/// builds a row's ID in the same file; `fallback` is the key read when the
/// edge key is absent; `entry_key` names the field of a table entry that holds
/// its target.
#[derive(Debug)]
pub(crate) struct EdgeKey {
    pub(crate) kind: NonEmptyString,
    pub(crate) table: Option<NonEmptyString>,
    pub(crate) fallback: Option<NonEmptyString>,
    pub(crate) entry_key: Option<NonEmptyString>,
}

#[derive(Debug)]
pub(crate) struct HeaderConfig {
    pub(crate) table: NonEmptyString,
    pub(crate) kind: NonEmptyString,
    pub(crate) id: NonEmptyString,
    pub(crate) key_map: KeyMap,
}

#[derive(Debug)]
pub(crate) struct PathwayConfig {
    pub(crate) source_file: NonEmptyString,
    pub(crate) name: NonEmptyString,
    pub(crate) order_key: NonEmptyString,
    pub(crate) current_key: NonEmptyString,
}

#[derive(Debug, Deserialize)]
struct ProfileDocument {
    adapter: Adapter,
    #[serde(default)]
    node_kinds: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Adapter {
    paths: Paths,
    tables: BTreeMap<String, RawTable>,
    #[serde(default)]
    mode: Option<Mode>,
    #[serde(default)]
    id_prefix: Option<IdPrefix>,
    #[serde(default)]
    header: Option<RawHeader>,
    #[serde(default)]
    pathway: Option<RawPathway>,
    /// Core-owned keys the resolved document carries through unchanged.
    #[serde(default, rename = "native")]
    _native: Option<serde_json::Value>,
    #[serde(default, rename = "timeout_seconds")]
    _timeout_seconds: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Paths {
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTable {
    kind: String,
    id_key: String,
    #[serde(default)]
    id_prefix: Option<TableIdPrefix>,
    /// Optional: a table of IDs alone is a register. A string or a table of
    /// shapes per key, read by hand as `edge_keys` is.
    #[serde(default)]
    key_map: BTreeMap<String, serde_json::Value>,
    /// A string or a table per key; read by hand so the table form's errors
    /// name the entry rather than an untagged enum's "no variant matched".
    #[serde(default)]
    edge_keys: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEdgeEntry {
    kind: String,
    #[serde(default)]
    table: Option<String>,
    #[serde(default)]
    fallback: Option<String>,
    #[serde(default)]
    entry_key: Option<String>,
}

fn edge_key(value: serde_json::Value, at: &str) -> Result<EdgeKey, String> {
    let entry = match value {
        serde_json::Value::String(kind) => RawEdgeEntry {
            kind,
            table: None,
            fallback: None,
            entry_key: None,
        },
        value @ serde_json::Value::Object(_) => {
            serde_json::from_value(value).map_err(|error| format!("'{at}': {error}"))?
        }
        _ => {
            return Err(format!(
                "'{at}' must be an edge kind or a table with 'kind'"
            ));
        }
    };
    Ok(EdgeKey {
        kind: field(entry.kind, at, "kind")?,
        table: entry.table.map(|t| field(t, at, "table")).transpose()?,
        fallback: entry
            .fallback
            .map(|f| field(f, at, "fallback"))
            .transpose()?,
        entry_key: entry
            .entry_key
            .map(|k| field(k, at, "entry_key"))
            .transpose()?,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeader {
    table: String,
    kind: String,
    id: String,
    #[serde(default)]
    key_map: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPathway {
    source_file: String,
    name: String,
    order_key: String,
    current_key: String,
}

impl RawTable {
    fn validate(self, key: &str) -> Result<TableConfig, String> {
        let edge_keys = self
            .edge_keys
            .into_iter()
            .map(|(name, value)| {
                let at = format!("{key}.edge_keys.{name}");
                let name = NonEmptyString::new(name, &format!("a key of '{key}.edge_keys'"))?;
                Ok((name, edge_key(value, &at)?))
            })
            .collect::<Result<_, String>>()?;
        Ok(TableConfig {
            kind: field(self.kind, key, "kind")?,
            id_key: field(self.id_key, key, "id_key")?,
            id_prefix: self.id_prefix,
            key_map: key_map(self.key_map, &format!("{key}.key_map"))?,
            edge_keys,
        })
    }
}

impl RawHeader {
    fn validate(self, key: &str) -> Result<HeaderConfig, String> {
        Ok(HeaderConfig {
            table: field(self.table, key, "table")?,
            kind: field(self.kind, key, "kind")?,
            id: field(self.id, key, "id")?,
            key_map: key_map(self.key_map, &format!("{key}.key_map"))?,
        })
    }
}

impl RawPathway {
    fn validate(self, key: &str) -> Result<PathwayConfig, String> {
        Ok(PathwayConfig {
            source_file: field(self.source_file, key, "source_file")?,
            name: field(self.name, key, "name")?,
            order_key: field(self.order_key, key, "order_key")?,
            current_key: field(self.current_key, key, "current_key")?,
        })
    }
}

impl Adapter {
    fn validate(self) -> Result<Config, String> {
        let files = self
            .paths
            .files
            .into_iter()
            .map(|glob| NonEmptyString::new(glob, "a glob in 'adapter.paths.files'"))
            .collect::<Result<Vec<_>, _>>()?;
        if files.is_empty() {
            return Err("'adapter.paths.files' must contain a file glob".to_owned());
        }
        if self.tables.is_empty() {
            return Err("'adapter.tables' must declare at least one table".to_owned());
        }
        if matches!(self.mode, Some(Mode::Directory)) && self.tables.len() > 1 {
            return Err(format!(
                "'adapter.tables' must declare exactly one table when 'adapter.mode' is \
                 'directory'; found {}",
                self.tables.len()
            ));
        }
        if matches!(self.mode, Some(Mode::Directory)) && self.header.is_some() {
            return Err(
                "'adapter.header' is not supported when 'adapter.mode' is 'directory'".to_owned(),
            );
        }
        let tables = self
            .tables
            .into_iter()
            .map(|(name, table)| {
                let name = NonEmptyString::new(name, "a table name in 'adapter.tables'")?;
                let table = table.validate(&format!("adapter.tables.{name}"))?;
                Ok((name, table))
            })
            .collect::<Result<BTreeMap<NonEmptyString, TableConfig>, String>>()?;
        for (name, table) in &tables {
            for (key, edge) in &table.edge_keys {
                if let Some(target) = &edge.table
                    && !tables.contains_key(target.as_str())
                {
                    return Err(format!(
                        "'adapter.tables.{name}.edge_keys.{key}.table' names '{target}', \
                         which 'adapter.tables' does not declare"
                    ));
                }
            }
        }
        Ok(Config {
            files,
            tables,
            mode: self.mode,
            id_prefix: self.id_prefix,
            header: self
                .header
                .map(|header| header.validate("adapter.header"))
                .transpose()?,
            pathway: self
                .pathway
                .map(|pathway| pathway.validate("adapter.pathway"))
                .transpose()?,
            node_kinds: BTreeMap::new(),
        })
    }
}

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let profile: ProfileDocument = latt_adapter::load_resolved_profile(path)?;
        let mut config = profile
            .adapter
            .validate()
            .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
        config.node_kinds = profile.node_kinds;
        Ok(config)
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

    #[test]
    fn an_edge_entry_with_an_unknown_key_names_the_entry() {
        let text = r#"{"resolved_schema":"1","adapter":{"paths":{"files":["*.toml"]},
            "tables":{"item":{"kind":"item","id_key":"id",
            "edge_keys":{"refs":{"kind":"references","target":"item"}}}}}}"#;
        let profile: ProfileDocument = serde_json::from_str(text).unwrap();
        let error = profile.adapter.validate().unwrap_err();
        assert!(
            error.starts_with("'adapter.tables.item.edge_keys.refs': unknown field `target`"),
            "{error}"
        );
    }

    #[test]
    fn key_map_and_edge_keys_are_optional() {
        // A table of IDs alone is a register; the maps default to empty. The
        // header's key_map is the same shape and defaults the same way.
        let profile: ProfileDocument = serde_json::from_str(
            r#"{
                "resolved_schema":"1",
                "adapter":{
                    "paths":{"files":["registers/*.toml"]},
                    "tables":{"item":{"kind":"item", "id_key":"id"}},
                    "header":{"table":"register", "kind":"register", "id":"file_stem"}
                }
            }"#,
        )
        .unwrap();
        let config = profile.adapter.validate().unwrap();
        assert!(config.tables["item"].key_map.is_empty());
        assert!(config.tables["item"].edge_keys.is_empty());
        assert!(config.header.unwrap().key_map.is_empty());
    }

    #[test]
    fn blank_required_values_name_their_key() {
        let cases = [
            (
                r#""tables":{"item":{"kind":" ","id_key":"id","key_map":{},"edge_keys":{}}}"#,
                "'adapter.tables.item.kind' must not be empty",
            ),
            (
                r#""tables":{"item":{"kind":"item","id_key":"id","key_map":{"stage":""},"edge_keys":{}}}"#,
                "'adapter.tables.item.key_map.stage' must not be empty",
            ),
            (
                r#""tables":{"item":{"kind":"item","id_key":"id","key_map":{},"edge_keys":{}}},"header":{"table":"t","kind":"k","id":"","key_map":{}}"#,
                "'adapter.header.id' must not be empty",
            ),
            (
                r#""tables":{"item":{"kind":"item","id_key":"id","key_map":{},"edge_keys":{}}},"pathway":{"source_file":"f","name":"","order_key":"o","current_key":"c"}"#,
                "'adapter.pathway.name' must not be empty",
            ),
            (
                r#""mode":"directory","tables":{"a":{"kind":"item","id_key":"id","key_map":{},"edge_keys":{}},"b":{"kind":"item","id_key":"id","key_map":{},"edge_keys":{}}}"#,
                "'adapter.tables' must declare exactly one table when 'adapter.mode' is 'directory'; found 2",
            ),
        ];
        for (adapter, message) in cases {
            let text = format!(
                r#"{{"resolved_schema":"1","adapter":{{"paths":{{"files":["*.toml"]}},{adapter}}}}}"#
            );
            let profile: ProfileDocument = serde_json::from_str(&text).unwrap();
            assert_eq!(profile.adapter.validate().unwrap_err(), message);
        }
    }

    #[test]
    fn unknown_adapter_keys_are_named() {
        let table =
            r#""tables":{"item":{"kind":"item","id_key":"id","key_map":{},"edge_keys":{}}}"#;
        let cases = [
            (format!(r#""paths":{{"files":["*.toml"]}},{table},"modes":"tables""#), "modes"),
            (r#""paths":{"file":["*.toml"]},"tables":{}"#.to_owned(), "file"),
            (
                r#""paths":{"files":["*.toml"]},"tables":{"item":{"kind":"item","id_key":"id","key_map":{},"edge_key":{}}}"#.to_owned(),
                "edge_key",
            ),
            (
                format!(r#""paths":{{"files":["*.toml"]}},{table},"header":{{"table":"t","kind":"k","id":"i","key_map":{{}},"keymap":{{}}}}"#),
                "keymap",
            ),
            (
                format!(r#""paths":{{"files":["*.toml"]}},{table},"pathway":{{"source_file":"f","name":"n","order_key":"o","current":"c"}}"#),
                "current",
            ),
        ];
        for (adapter, key) in cases {
            let text = format!(r#"{{"resolved_schema":"1","adapter":{{{adapter}}}}}"#);
            let error = serde_json::from_str::<ProfileDocument>(&text).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field `{key}`")),
                "{error}"
            );
        }

        let core_owned = format!(
            r#"{{"resolved_schema":"1","adapter":{{"paths":{{"files":["*.toml"]}},{table},"native":"toml","timeout_seconds":30}}}}"#
        );
        serde_json::from_str::<ProfileDocument>(&core_owned).unwrap();
    }
}

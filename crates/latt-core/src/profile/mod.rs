//! Profile loading: the declared vocabulary of a register, read from YAML.
//!
//! The profile is where all domain knowledge lives. Nothing below names a kind, an
//! ID shape or a check. It reads what the file declares and hands it to validation.
//!
//! An unrecognised top-level key is refused, not carried. The `adapter:` block is
//! kept verbatim in `raw` and reaches the adapter through the core-resolved document
//! that [`resolved_document`] prints. The core reads `native`, to choose a built-in
//! adapter, and validates the timeout keys; every other key is the adapter's.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod compile;
mod conditions;
mod kinds;
mod raw;
mod types;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_norway::Value;

use compile::compile_validations;
pub(crate) use compile::{Kinds, parse_suppress};
pub(crate) use conditions::{parse_condition_block, parse_severity_word, retired_code_refusal};
use kinds::{parse_edge_kind, parse_node_kind};
pub(crate) use raw::check_xyz_version;
pub use raw::resolved_document;
pub(crate) use raw::to_json;
use raw::{
    check_adapter_timeouts, check_version, key_name, load_raw, read_notes, reject_unknown_keys,
};
#[cfg(test)]
pub(crate) use types::FILTER_OPERATORS;
pub(crate) use types::NoteItem;
pub use types::{
    ActionDecl, ActionMode, AttrSchema, AttrType, Comparable, Condition, ConditionOp,
    ConstraintConfig, CountEnd, CoverageDeepSpec, CoverageSpec, EdgeConditions, EdgeCountConfig,
    EdgeKind, EdgeWhereConfig, FilterError, NodeKind, PathwayBinding, PlanEntry, Profile, Segment,
    SummarySpec, SuppressConfig, Template, ValidationPlan, ValidationSpec,
};

/// The attribute types a profile may declare, in the order error messages list them.
#[doc(hidden)]
pub const VALID_ATTR_TYPES: &[&str] = &["bool", "date", "enum", "int", "list", "string"];
/// The element types a `list` attribute may declare.
#[doc(hidden)]
pub const VALID_LIST_ITEM_TYPES: &[&str] = &["bool", "date", "int", "string"];
/// The highest `profile_version` major this core will load.
#[doc(hidden)]
pub const SUPPORTED_MAJOR_VERSION: u64 = 1;

/// Keys a validation entry admits, per code. Anything else is a profile error
/// rather than a silently ignored line.
fn config_keys(code: &str) -> &'static [&'static str] {
    match code {
        "COVERAGE" => &[
            "edge_kind",
            "message",
            "severity",
            "source_where",
            "target_kind",
            "where",
        ],
        "COVERAGE_DEEP" => &[
            "evidence",
            "message",
            "severity",
            "source_where",
            "target_kind",
            "via",
            "where",
        ],
        "CONSTRAINT" => &["expect", "kind", "message", "reject", "severity", "when"],
        "EDGE_COUNT" => &[
            "direction",
            "edge_kind",
            "group_by",
            "max",
            "message",
            "min",
            "node_kind",
            "severity",
            "where",
        ],
        "EDGE_WHERE" => &[
            "edge_kind",
            "expect",
            "message",
            "reject",
            "severity",
            "when",
        ],
        "SUMMARY" => &[
            "group_by_attr",
            "message",
            "node_kind",
            "severity",
            "status_attr",
        ],
        _ => &["message", "severity"],
    }
}

/// The one `validations` entry that is a selector over findings rather than a
/// code's configuration. Takes neither `severity` nor a pathway binding: a
/// setting on a selector would have nothing to act on.
const SUPPRESS_KEYS: &[&str] = &["code", "node_ids"];

/// Available on every validation entry, for a code the core implements or one it
/// does not: the binding is orthogonal to what the code means.
const PATHWAY_BINDING_KEYS: &[&str] = &["pathway", "position_attr"];

/// Keys the loader admits at each declaration level. `extends` is stripped
/// before the merged value reaches these checks; `adapter` is admitted whole
/// and its contents left to the adapter, apart from `native` and the timeout keys.
const TOP_LEVEL_KEYS: &[&str] = &[
    "adapter",
    "annotations",
    "edge_kinds",
    "name",
    "node_kinds",
    "pathways",
    "profile_version",
    "validations",
];
/// The tail of a [`ProfileError::NotFound`] message: the candidates by name, or
/// the directory searched when there is none.
fn describe_candidates(path: &Path, candidates: &[PathBuf]) -> String {
    if candidates.is_empty() {
        return format!(
            "no profile found in '{}'",
            profile_directory(path).display()
        );
    }
    let names: Vec<String> = candidates
        .iter()
        .map(|c| {
            c.file_name().map_or_else(
                || c.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
        .collect();
    format!("profiles beside it: {}", names.join(", "))
}

/// The directory a profile path names: its parent, or `.` for a bare name.
fn profile_directory(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// The `.yaml` and `.yml` files beside `path` whose text has a line starting
/// `node_kinds:`, in name order. A text scan, so a candidate that would not
/// load is still named; the user asked where the profile is, not whether it
/// loads. An unreadable directory or file contributes nothing.
#[must_use]
#[doc(hidden)]
pub fn profile_candidates(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(profile_directory(path)) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .filter(|candidate| {
            std::fs::read_to_string(candidate)
                .is_ok_and(|text| text.lines().any(|line| line.starts_with("node_kinds:")))
        })
        .collect();
    found.sort();
    found
}

/// A profile value or file that cannot be loaded under the supported schema.
///
/// `Display` names the profile and what went wrong; the I/O and YAML causes
/// are `source()`s, for a printer that walks the chain.
///
/// # Examples
///
/// ```
/// use latt_core::profile::{ProfileError, load_profile_value};
///
/// let raw = latt_core::serde_norway::from_str("name: demo\n")?;
/// let error = load_profile_value(raw, "demo").unwrap_err();
/// assert!(matches!(error, ProfileError::Invalid(_)));
/// assert!(error.to_string().contains("demo"), "{error}");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProfileError {
    /// The named path is not a file. `candidates` are the `.yaml`/`.yml` files
    /// beside it that declare `node_kinds`, in name order, so a misnamed
    /// `--profile` is told what was probably meant; that one directory is
    /// read, and none beneath it.
    #[error("failed to read profile {}: no such file; {}", path.display(), describe_candidates(path, candidates))]
    NotFound {
        /// The profile path as given.
        path: PathBuf,
        /// Profiles in the same directory, by name.
        candidates: Vec<PathBuf>,
    },
    /// The file could not be read, or its path resolved.
    #[error("failed to read profile {}", path.display())]
    Io {
        /// The profile file, or the parent an `extends` names.
        path: PathBuf,
        /// What the filesystem reported.
        #[source]
        source: std::io::Error,
    },
    /// The file is not YAML.
    #[error("failed to parse profile {}", path.display())]
    Yaml {
        /// The profile or `extends` parent file, or the origin a host passed to
        /// [`prepare_profile_str`](crate::service::prepare_profile_str).
        path: PathBuf,
        /// The parser's error, with its line and column.
        #[source]
        source: serde_norway::Error,
    },
    /// The value violates the schema; the text says where and how.
    #[error("{0}")]
    Invalid(String),
}

fn err<T>(message: impl Into<String>) -> Result<T, ProfileError> {
    Err(ProfileError::Invalid(message.into()))
}

/// The YAML value's runtime type: `string`, `int`, `float`, `bool`, `list`,
/// `null`, or `object`. Declared semantic types such as `date` remain strings
/// at this layer and are named separately when an expected schema type is known.
pub(crate) fn name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) => {
            if n.is_f64() {
                "float"
            } else {
                "int"
            }
        }
        Value::String(_) => "string",
        Value::Sequence(_) => "list",
        Value::Mapping(_) | Value::Tagged(_) => "object",
    }
}

/// Read and fully validate a profile YAML file.
///
/// Fails on any malformed or unsupported input; a returned `Profile` is
/// structurally sound, so callers need not re-check it.
///
/// # Errors
///
/// [`ProfileError::NotFound`] when the file or an `extends` parent is not there,
/// [`ProfileError::Io`] when one cannot be read,
/// [`ProfileError::Yaml`] when one is not YAML, and [`ProfileError::Invalid`]
/// for an inheritance cycle or any schema violation.
///
/// # Examples
///
/// ```
/// use latt_core::profile::load_profile;
///
/// let dir = std::env::temp_dir().join(format!("lattice-doc-{}", std::process::id()));
/// std::fs::create_dir_all(&dir)?;
/// let path = dir.join("profile.yaml");
/// std::fs::write(&path, r#"
/// name: demo
/// profile_version: "1.0.0"
/// node_kinds:
///   req:
///     id_pattern: "^REQ-\\d+$"
/// edge_kinds: {}
/// "#)?;
///
/// let profile = load_profile(&path)?;
/// assert_eq!(profile.name(), "demo");
/// # std::fs::remove_dir_all(&dir)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn load_profile(path: &Path) -> Result<Profile, ProfileError> {
    let mut raw = load_raw(path, &mut Vec::new())?;
    let annotations = match raw {
        Value::Mapping(ref mut top) => top.shift_remove(Value::String("annotations".into())),
        _ => None,
    };
    let mut profile = load_profile_value(raw, &format!("profile {}", path.display()))?;
    if let Some(Value::String(notes_path)) = annotations {
        profile.notes = Some(read_notes(PathBuf::from(notes_path))?);
    }
    Ok(profile)
}

/// Build a `Profile` from an already-parsed YAML `Value`.
///
/// `where_` prefixes the top-level shape errors; errors from nested checks,
/// such as the version check, carry their own context. Reads no file, so inheritance must
/// already be resolved: an `extends` key is refused as an unknown key.
///
/// # Errors
///
/// [`ProfileError::Invalid`] for any schema violation.
///
/// # Examples
///
/// ```
/// use latt_core::profile::load_profile_value;
///
/// let raw = latt_core::serde_norway::from_str(r#"
/// name: demo
/// profile_version: "1.0.0"
/// node_kinds:
///   req:
///     id_pattern: "^REQ-\\d+$"
/// edge_kinds: {}
/// "#)?;
/// let profile = load_profile_value(raw, "demo")?;
/// assert_eq!(profile.name(), "demo");
/// assert!(profile.node_kinds().contains_key("req"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn load_profile_value(raw: Value, where_: &str) -> Result<Profile, ProfileError> {
    load_profile_value_with(raw, where_, Kinds::Checked)
}

/// As [`load_profile_value`], deferring the coverage kind check when `kinds`
/// says so; every other load check runs either way.
pub(crate) fn load_profile_value_with(
    raw: Value,
    where_: &str,
    kinds: Kinds,
) -> Result<Profile, ProfileError> {
    let where_ = where_.to_string();
    let Value::Mapping(top) = &raw else {
        return err(format!("{where_}: expected a YAML mapping at top level"));
    };

    for key in ["name", "profile_version", "node_kinds", "edge_kinds"] {
        if !top.contains_key(Value::String(key.into())) {
            return err(format!("{where_}: missing required key '{key}'"));
        }
    }
    if top.contains_key(Value::String("annotations".into())) {
        return err(format!(
            "{where_}: 'annotations' needs a profile file to resolve its path against; \
             load the profile from its file"
        ));
    }
    reject_unknown_keys(top, TOP_LEVEL_KEYS, &where_)?;

    let name_value = &top[Value::String("name".into())];
    let Value::String(profile_name) = name_value else {
        return err(format!(
            "{where_}: 'name' must be a string, got {}",
            name(name_value)
        ));
    };

    let profile_version = check_version(&top[Value::String("profile_version".into())])?;
    check_adapter_timeouts(top, &where_)?;

    let raw_node_kinds = &top[Value::String("node_kinds".into())];
    let Value::Mapping(raw_node_kinds) = raw_node_kinds else {
        return err(format!(
            "{where_}: 'node_kinds' must be a mapping, got {}",
            name(raw_node_kinds)
        ));
    };
    let mut faults: Vec<String> = Vec::new();
    let mut node_kinds = BTreeMap::new();
    for (declared_index, (key, value)) in raw_node_kinds.iter().enumerate() {
        let kind_name = key_name(key);
        let empty = Value::Mapping(serde_norway::Mapping::default());
        let value = if matches!(value, Value::Null) {
            &empty
        } else {
            value
        };
        match parse_node_kind(&kind_name, value, declared_index) {
            Ok(kind) => {
                node_kinds.insert(kind_name.clone(), kind);
            }
            Err(fault) => faults.push(fault.to_string()),
        }
    }

    let raw_edge_kinds = &top[Value::String("edge_kinds".into())];
    let Value::Mapping(raw_edge_kinds) = raw_edge_kinds else {
        return err(format!(
            "{where_}: 'edge_kinds' must be a mapping, got {}",
            name(raw_edge_kinds)
        ));
    };
    let mut edge_kinds = BTreeMap::new();
    for (key, value) in raw_edge_kinds {
        let kind_name = key_name(key);
        let empty = Value::Mapping(serde_norway::Mapping::default());
        let value = if matches!(value, Value::Null) {
            &empty
        } else {
            value
        };
        match parse_edge_kind(&kind_name, value, &node_kinds) {
            Ok(kind) => {
                edge_kinds.insert(kind_name.clone(), kind);
            }
            Err(fault) => faults.push(fault.to_string()),
        }
    }

    let empty_list = Value::Sequence(Vec::new());
    let raw_validations = match top.get(Value::String("validations".into())) {
        None | Some(Value::Null) => &empty_list,
        Some(value) => value,
    };
    let Value::Sequence(raw_validations) = raw_validations else {
        return err(format!(
            "{where_}: 'validations' must be a list, got {}",
            name(raw_validations)
        ));
    };

    // As with the document's arrays: absent means none, explicit null is
    // malformed. `validations` differs and accepts null, matching the reference
    // core rather than being made consistent with it.
    let raw_pathways = match top.get(Value::String("pathways".into())) {
        None => Vec::new(),
        Some(Value::Sequence(items)) => {
            let mut pathways = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(pathway) = item else {
                    return err(format!(
                        "{where_}: 'pathways' must be a list of pathway names, got {}. \
                         Pathway values are target state and are read from the register, \
                         never declared here",
                        name(item)
                    ));
                };
                pathways.push(pathway.clone());
            }
            pathways
        }
        Some(other) => {
            return err(format!(
                "{where_}: 'pathways' must be a list of pathway names, got {}. \
                 Pathway values are target state and are read from the register, \
                 never declared here",
                name(other)
            ));
        }
    };

    // The kind sections' faults and the entries' faults are independent, so
    // one refusal names them all, one per line. An entry naming a kind that
    // failed to parse adds a dependent fault; the spec allows that.
    let plan = match compile_validations(
        raw_validations,
        &raw_pathways,
        &node_kinds,
        &edge_kinds,
        kinds,
    ) {
        Ok(plan) if faults.is_empty() => plan,
        Ok(_) => return err(faults.join("\n")),
        Err(more) => {
            faults.extend(more);
            return err(faults.join("\n"));
        }
    };

    Ok(Profile {
        name: profile_name.clone(),
        profile_version,
        node_kinds,
        edge_kinds,
        plan,
        pathways: raw_pathways,
        raw,
        notes: None,
    })
}

/// `docs/profiles.md` restates `config_keys` and `docs/adapters.md` the
/// timeout default; a restatement that drifts is a wrong reference.
#[cfg(test)]
mod docs_config_tables {
    use super::types::DEFAULT_ADAPTER_TIMEOUT_SECONDS;
    use super::{SUPPRESS_KEYS, config_keys, load_profile_value};

    fn doc(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs")
            .join(name);
        std::fs::read_to_string(path).unwrap()
    }

    /// The backticked words of one `` - `CODE` - `k1`, `k2` `` bullet, with
    /// any parenthetical aside dropped.
    fn backticked(line: &str) -> Vec<String> {
        let mut depth = 0usize;
        let outside: String = line
            .chars()
            .filter(|c| match c {
                '(' => {
                    depth += 1;
                    false
                }
                ')' => {
                    depth -= 1;
                    false
                }
                _ => depth == 0,
            })
            .collect();
        outside
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    // Requirement: development-verification/Verify documentation and supported feature configurations
    #[test]
    fn per_code_configuration_keys_match_the_loader() {
        let text = doc("profiles.md");
        let bullets: Vec<&str> = text
            .split("Configuration keys per code:")
            .nth(1)
            .expect("docs/profiles.md lists configuration keys per code")
            .lines()
            .skip_while(|line| line.trim().is_empty())
            .take_while(|line| line.starts_with("- "))
            .collect();
        let mut documented = Vec::new();
        for bullet in &bullets {
            let words = backticked(bullet);
            if bullet.starts_with("- any other code") {
                assert_eq!(words, ["message", "severity"], "the catch-all bullet");
                assert_eq!(
                    config_keys("PARSE_ERROR"),
                    ["message", "severity"],
                    "the catch-all arm"
                );
                continue;
            }
            let (code, keys) = words.split_first().expect("a bullet names its code");
            let mut keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            keys.sort_unstable();
            let expected: &[&str] = match code.as_str() {
                "SUPPRESS" => SUPPRESS_KEYS,
                code => config_keys(code),
            };
            assert_eq!(
                keys, expected,
                "{code}: docs/profiles.md keys differ from the loader"
            );
            documented.push(code.clone());
        }
        let configured = [
            "COVERAGE",
            "COVERAGE_DEEP",
            "CONSTRAINT",
            "EDGE_COUNT",
            "EDGE_WHERE",
            "SUMMARY",
            "SUPPRESS",
        ];
        for code in configured {
            assert!(documented.contains(&code.to_owned()), "{code}: no bullet");
        }
        assert_eq!(
            bullets.len(),
            configured.len() + 1,
            "the configured codes and the catch-all"
        );
    }

    // Requirement: development-verification/Verify documentation and supported feature configurations
    #[test]
    fn the_documented_timeout_default_is_the_one_applied() {
        let text = doc("adapters.md");
        let row = text
            .lines()
            .find(|line| line.starts_with("| `timeout_seconds` |"))
            .expect("docs/adapters.md tabulates timeout_seconds");
        let documented: u64 = row.split('|').nth(2).unwrap().trim().parse().unwrap();
        assert_eq!(documented, DEFAULT_ADAPTER_TIMEOUT_SECONDS);
        let yaml = "name: p\nprofile_version: '1.0.0'\nnode_kinds: {}\nedge_kinds: {}\n";
        let profile = load_profile_value(serde_norway::from_str(yaml).unwrap(), "t").unwrap();
        assert_eq!(profile.adapter_timeout().as_secs(), documented);
    }
}

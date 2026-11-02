//! Reading the YAML: `extends` merging, version checks, value coercions and the notes file.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use serde_norway::Value;

use super::types::{Comparable, NoteItem, NotesFile, Profile, TIMEOUT_KEYS};
use super::{ProfileError, SUPPORTED_MAJOR_VERSION, err, name};

/// Reject a zero or negative bound so it never silently means "no bound".
pub(super) fn check_adapter_timeouts(
    top: &serde_norway::Mapping,
    where_: &str,
) -> Result<(), ProfileError> {
    let Some(Value::Mapping(adapter)) = top.get(Value::String("adapter".into())) else {
        return Ok(());
    };
    for key in TIMEOUT_KEYS {
        let Some(value) = adapter.get(Value::String(key.into())) else {
            continue;
        };
        if !matches!(value.as_u64(), Some(seconds) if seconds > 0) {
            return err(format!(
                "{where_}: 'adapter.{key}' must be a positive integer, got {}",
                name(value)
            ));
        }
    }
    Ok(())
}

/// Convert a parsed profile value to JSON, refusing what JSON cannot carry.
///
/// Refusal matters because a coerced value would round-trip *changed*, and the
/// resolved document's guarantee is that it round-trips as declared.
pub(crate) fn to_json(value: &Value) -> Result<serde_json::Value, ProfileError> {
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => {
            let text = n.to_string();
            serde_json::from_str(&text)
                .map_err(|_| ProfileError::Invalid(format!("number '{text}' has no JSON form")))?
        }
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Sequence(items) => {
            serde_json::Value::Array(items.iter().map(to_json).collect::<Result<_, _>>()?)
        }
        Value::Mapping(m) => {
            let mut object = serde_json::Map::new();
            for (key, item) in m {
                let Value::String(key) = key else {
                    return err(format!("mapping key {key:?} is not a string"));
                };
                object.insert(key.clone(), to_json(item)?);
            }
            serde_json::Value::Object(object)
        }
        Value::Tagged(t) => return err(format!("tagged value '{}' has no JSON form", t.tag)),
    })
}

pub(super) fn to_comparable(value: &Value) -> Option<Comparable> {
    match value {
        Value::String(s) => Some(Comparable::Str(s.clone())),
        Value::Number(n) => {
            let v = n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from))?;
            Some(Comparable::Int(v))
        }
        _ => None,
    }
}

/// Serialize the resolved profile document handed to adapters.
///
/// The profile after inheritance resolution, plus `resolved_schema`, so that
/// sections the core does not consume (`adapter:` above all) reach the
/// adapter unchanged.
///
/// # Errors
///
/// [`ProfileError::Invalid`] when the profile holds a value JSON cannot
/// carry, such as a tagged scalar or a non-string key.
///
/// # Examples
///
/// ```
/// use latt_core::profile::{load_profile_value, resolved_document};
///
/// let yaml = "name: p\nprofile_version: '1.0.0'\nnode_kinds: {}\nedge_kinds: {}\n\
///             adapter: {label_map: {bug: issue}}\n";
/// let profile = load_profile_value(latt_core::serde_norway::from_str(yaml)?, "example")?;
/// let document: serde_json::Value = serde_json::from_str(&resolved_document(&profile)?)?;
/// assert_eq!(document["resolved_schema"], "1");
/// assert_eq!(document["adapter"]["label_map"]["bug"], "issue", "an adapter key the core does not read passes through");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn resolved_document(profile: &Profile) -> Result<String, ProfileError> {
    let serde_json::Value::Object(mut document) = to_json(&profile.raw)? else {
        unreachable!("load_profile requires a mapping at top level")
    };
    document.insert(
        "resolved_schema".into(),
        serde_json::Value::String("1".into()),
    );
    serde_json::to_string(&serde_json::Value::Object(document))
        .map_err(|e| ProfileError::Invalid(format!("could not serialize resolved profile: {e}")))
}

pub(super) fn as_mapping<'a>(
    value: &'a Value,
    what: &str,
) -> Result<&'a serde_norway::Mapping, ProfileError> {
    match value {
        Value::Mapping(m) => Ok(m),
        other => err(format!("{what}: expected a mapping, got {}", name(other))),
    }
}

pub(super) fn key_name(key: &Value) -> String {
    match key {
        Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

/// Name every key of `mapping` outside `allowed`, so a typo is a load error
/// rather than a setting that silently does nothing. `allowed` must be sorted.
pub(super) fn reject_unknown_keys(
    mapping: &serde_norway::Mapping,
    allowed: &[&str],
    where_: &str,
) -> Result<(), ProfileError> {
    let mut unknown: Vec<String> = mapping
        .keys()
        .map(key_name)
        .filter(|k| !allowed.contains(&k.as_str()))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    err(format!(
        "{where_}: unknown keys {unknown:?} (valid: {allowed:?})"
    ))
}

pub(super) fn check_version(raw: &Value) -> Result<String, ProfileError> {
    let Value::String(version) = raw else {
        return err(format!(
            "profile_version must be a string, got {}",
            name(raw)
        ));
    };
    check_xyz_version("profile_version", version, SUPPORTED_MAJOR_VERSION)
        .map_err(ProfileError::Invalid)?;
    Ok(version.clone())
}

/// Check an X.Y.Z version axis, such as `profile_version` or a fuse manifest's
/// `manifest_version`, and refuse a major above `supported_major`.
pub(crate) fn check_xyz_version(
    key: &str,
    version: &str,
    supported_major: u64,
) -> Result<(), String> {
    let parts: Vec<&str> = version.split('.').collect();
    let valid = parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        });
    if !valid {
        return Err(format!(
            "{key} '{version}' is not a valid version \
             (expected X.Y.Z where each component is a non-negative \
             integer without leading zeros)"
        ));
    }
    let major: u64 = parts[0].parse().unwrap_or(u64::MAX);
    if major > supported_major {
        return Err(format!(
            "unsupported {} {version} (supported major: {supported_major})",
            key.replace('_', " ")
        ));
    }
    Ok(())
}

/// Recursively merge `parent` and `child` Values.
///
/// Mappings deep-merge (child keys win on conflict); sequences and scalars
/// are replaced whole by the child.
fn merge_values(parent: Value, child: Value) -> Value {
    match (parent, child) {
        (Value::Mapping(mut parent_map), Value::Mapping(child_map)) => {
            for (key, child_val) in child_map {
                match parent_map.get_mut(&key) {
                    // Writing through the slot keeps the key's position, as the
                    // re-insert it replaces did.
                    Some(parent_val) => {
                        let parent_val_owned = std::mem::take(parent_val);
                        *parent_val = merge_values(parent_val_owned, child_val);
                    }
                    None => {
                        parent_map.insert(key, child_val);
                    }
                }
            }
            Value::Mapping(parent_map)
        }
        (_, child) => child,
    }
}

/// Parse one YAML file and, if it declares `extends`, recurse into the parent
/// and merge. Returns the merged raw value with `extends` stripped.
pub(super) fn load_raw(
    path: &Path,
    visited: &mut Vec<std::path::PathBuf>,
) -> Result<Value, ProfileError> {
    let io = |source: std::io::Error| {
        if source.kind() == std::io::ErrorKind::NotFound {
            return ProfileError::NotFound {
                path: path.to_owned(),
                candidates: super::profile_candidates(path),
            };
        }
        ProfileError::Io {
            path: path.to_owned(),
            source,
        }
    };
    let canonical = path.canonicalize().map_err(io)?;
    if visited.contains(&canonical) {
        return err(format!(
            "profile inheritance cycle: {} was already visited",
            path.display()
        ));
    }
    visited.push(canonical);

    let text = std::fs::read_to_string(path).map_err(io)?;
    let mut raw: Value = serde_norway::from_str(&text).map_err(|source| ProfileError::Yaml {
        path: path.to_owned(),
        source,
    })?;

    let annotations_key = Value::String("annotations".into());
    if let Value::Mapping(ref mut top) = raw
        && let Some(declared) = top.get_mut(&annotations_key)
    {
        let Value::String(relative) = declared else {
            return err(format!(
                "profile {}: 'annotations' must be a string path, got {}",
                path.display(),
                name(declared)
            ));
        };
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let resolved = std::path::absolute(base_dir.join(&*relative)).map_err(io)?;
        *declared = Value::String(resolved.to_string_lossy().into_owned());
    }

    let extends_key = Value::String("extends".into());
    if let Value::Mapping(ref mut top) = raw
        && let Some(extends_val) = top.shift_remove(&extends_key)
    {
        let Value::String(ref parent_path_str) = extends_val else {
            return err(format!(
                "profile {}: 'extends' must be a string, got {}",
                path.display(),
                name(&extends_val)
            ));
        };
        if !top.contains_key(Value::String("profile_version".into())) {
            return err(format!(
                "profile {}: child must declare its own 'profile_version' \
                 (inheriting it from a parent is not allowed)",
                path.display()
            ));
        }
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let parent_path = base_dir.join(parent_path_str);
        let parent_raw = load_raw(&parent_path, visited)?;
        if !matches!(parent_raw, Value::Mapping(_)) {
            return err(format!(
                "profile {}: parent '{}' is not a YAML mapping",
                path.display(),
                parent_path.display()
            ));
        }
        raw = merge_values(parent_raw, raw);
    }

    Ok(raw)
}

/// Read a notes file: a missing file is recorded rather than refused, because
/// it is reported as `NOTE_FILE_MISSING`; anything else unreadable, not YAML,
/// or not a list is a load error.
pub(super) fn read_notes(path: PathBuf) -> Result<NotesFile, ProfileError> {
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NotesFile { path, items: None });
        }
        Err(e) => return err(format!("failed to read notes file {}: {e}", path.display())),
    };
    let parsed: Value = serde_norway::from_str(&text).map_err(|e| {
        ProfileError::Invalid(format!(
            "failed to parse notes file {}: {e}",
            path.display()
        ))
    })?;
    let values = match parsed {
        Value::Null => Vec::new(),
        Value::Sequence(values) => values,
        other => {
            return err(format!(
                "notes file {}: expected a list of entries, got {}",
                path.display(),
                name(&other)
            ));
        }
    };
    // A start beyond u32 reads as line zero, the unknown line, rather than wrapping.
    let starts: Vec<u32> = text
        .lines()
        .zip(1_usize..)
        .filter(|(line, _)| *line == "-" || line.starts_with("- "))
        .map(|(_, number)| u32::try_from(number).unwrap_or(0))
        .collect();
    let block_style = starts.len() == values.len();
    let items = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| NoteItem {
            line: if block_style {
                NonZeroU32::new(starts[index])
            } else {
                None
            },
            value,
        })
        .collect();
    Ok(NotesFile {
        path,
        items: Some(items),
    })
}

#[cfg(test)]
mod merge_tests {
    use serde_norway::Value;

    use super::merge_values;

    fn yaml(text: &str) -> Value {
        serde_norway::from_str(text).unwrap()
    }

    fn keys(value: &Value) -> Vec<String> {
        value
            .as_mapping()
            .unwrap()
            .keys()
            .map(|k| k.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn an_overridden_key_keeps_the_parent_position() {
        let parent = yaml("a: 1\nb: {x: 1}\nc: 3\n");
        let child = yaml("c: 30\nb: {y: 2}\nd: 4\n");
        let merged = merge_values(parent, child);
        assert_eq!(keys(&merged), ["a", "b", "c", "d"]);
        assert_eq!(merged["c"], Value::from(30));
        assert_eq!(keys(&merged["b"]), ["x", "y"], "mappings deep-merge");
    }

    #[test]
    fn sequences_and_scalars_are_replaced_whole() {
        let parent = yaml("list: [1, 2, 3]\nscalar: old\n");
        let child = yaml("list: [9]\nscalar: {now: a map}\n");
        let merged = merge_values(parent, child);
        assert_eq!(merged["list"], yaml("[9]"));
        assert_eq!(merged["scalar"], yaml("{now: a map}"));
    }
}

#[cfg(test)]
mod version_properties {
    use proptest::prelude::*;

    use super::check_xyz_version;

    proptest! {
        /// Accept a formatted `X.Y.Z` exactly when its major is supported: the check and
        /// the formatter agree on every triple, however large the components.
        #[test]
        fn formatted_triples_are_judged_by_major(
            major: u64,
            minor: u64,
            patch: u64,
            supported in 0u64..4,
        ) {
            let version = format!("{major}.{minor}.{patch}");
            let result = check_xyz_version("profile_version", &version, supported);
            prop_assert_eq!(result.is_ok(), major <= supported, "{}", version);
        }

        /// Refuse any text that does not re-format from three parsed integers: an
        /// accepted version is the canonical spelling of the numbers it names.
        #[test]
        fn accepted_text_is_canonical(version in "[0-9.a-z]{0,12}") {
            let result = check_xyz_version("profile_version", &version, u64::MAX);
            let canonical = version
                .split('.')
                .map(|part| part.parse::<u64>().ok().map(|n| n.to_string()))
                .collect::<Option<Vec<_>>>()
                .filter(|parts| parts.len() == 3)
                .map(|parts| parts.join("."));
            prop_assert_eq!(result.is_ok(), canonical.as_deref() == Some(&*version), "{}", version);
        }
    }
}

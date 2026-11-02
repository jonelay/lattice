//! Loading and resolving the fuse manifest.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::profile::check_xyz_version;

use super::{FuseError, Manifest, SUPPORTED_MANIFEST_MAJOR};

pub(super) fn load_yaml(path: &Path) -> Result<Value, FuseError> {
    let text = std::fs::read_to_string(path).map_err(|source| FuseError::Read {
        path: path.to_owned(),
        source,
    })?;
    serde_norway::from_str(&text).map_err(|source| FuseError::Yaml {
        path: path.to_owned(),
        source,
    })
}

pub(super) fn nonempty(value: &str, where_: &str) -> Result<(), String> {
    if value.is_empty() {
        Err(format!("{where_}: expected a non-empty string"))
    } else {
        Ok(())
    }
}

pub(super) fn relative(base: &Path, path: &mut PathBuf, key: &str) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("'{key}' must be a non-empty relative path"));
    }
    *path = base.join(&*path);
    Ok(())
}

/// Read a manifest, resolving relative paths against its own directory.
///
/// Absolute and empty paths are refused: the manifest is meant to travel with
/// the registers it names.
///
/// An `adapter` that names a compiled-in native adapter (`md`, not `./md`) is
/// kept as written, the way `--adapter` reads it.
///
/// # Errors
///
/// [`FuseError::Read`] or [`FuseError::Yaml`] for a file that cannot be read or
/// parsed, and [`FuseError::Manifest`] for a path that does not resolve, a
/// missing or mistyped key, an empty value, an absolute or empty path, an empty
/// source list, or a source name that repeats or contains `:`.
pub fn load_manifest(path: &Path) -> Result<Manifest, FuseError> {
    let path = std::fs::canonicalize(path)
        .map_err(|e| FuseError::Manifest(format!("manifest '{}': {e}", path.display())))?;
    let raw = load_yaml(&path)?;
    let mut manifest: Manifest = serde_json::from_value(raw)
        .map_err(|e| FuseError::Manifest(format!("manifest '{}': {e}", path.display())))?;
    // Only the root has no parent, and reading it as YAML has already failed.
    let Some(base) = path.parent() else {
        return Err(FuseError::Manifest(format!(
            "manifest '{}': has no parent directory",
            path.display()
        )));
    };
    resolve_manifest(&mut manifest, base).map_err(FuseError::Manifest)?;
    Ok(manifest)
}

fn resolve_manifest(manifest: &mut Manifest, base: &Path) -> Result<(), String> {
    for (key, value) in [
        ("manifest_version", &manifest.manifest_version),
        ("name", &manifest.name),
        ("version", &manifest.version),
    ] {
        nonempty(value, key)?;
    }
    check_xyz_version(
        "manifest_version",
        &manifest.manifest_version,
        SUPPORTED_MANIFEST_MAJOR,
    )?;
    if manifest.sources.is_empty() {
        return Err("'sources' must be a non-empty list".into());
    }
    relative(base, &mut manifest.fuse_profile, "fuse_profile")?;
    let mut names = BTreeSet::new();
    for source in &mut manifest.sources {
        nonempty(&source.name, "source name")?;
        if source.name.contains(':') {
            return Err("source names must not contain ':'".into());
        }
        if !names.insert(source.name.clone()) {
            return Err(format!("duplicate source name '{}'", source.name));
        }
        relative(base, &mut source.profile, "profile")?;
        if !crate::document::is_native_name(source.adapter.as_os_str()) {
            relative(base, &mut source.adapter, "adapter")?;
        }
        relative(base, &mut source.target, "target")?;
    }
    Ok(())
}

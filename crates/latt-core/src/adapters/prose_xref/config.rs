use std::io;
use std::path::{Path, PathBuf};

use latt_adapter::{Document, Issue, Provenance, Severity};

use regex::Regex;
use serde::Deserialize;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) file_patterns: Vec<Regex>,
    /// The globs `file_patterns` compiles, as the profile spells them.
    pub(crate) files: Vec<String>,
    pub(crate) content_dirs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProfileDocument {
    adapter: Adapter,
}

/// A key this adapter does not read is a typo that would otherwise pass in
/// silence — `content_dir:` would scan no content directory at all.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Adapter {
    paths: Paths,
    // Core-owned keys the resolved document carries through.
    #[serde(default, rename = "native")]
    _native: Option<serde_json::Value>,
    #[serde(default, rename = "timeout_seconds")]
    _timeout_seconds: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Paths {
    files: Vec<String>,
    #[serde(default)]
    content_dirs: Vec<String>,
}

impl Config {
    pub(crate) fn load(profile: &Path) -> Result<Config, String> {
        let doc: ProfileDocument = latt_adapter::load_resolved_profile(profile)?;

        let patterns = doc
            .adapter
            .paths
            .files
            .iter()
            .map(|p| {
                Regex::new(&latt_adapter::glob_regex(p)).map_err(|e| format!("bad glob '{p}': {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        if doc.adapter.paths.content_dirs.is_empty() {
            return Err("adapter.paths.content_dirs is required".into());
        }
        let content_dirs = doc.adapter.paths.content_dirs;

        Ok(Config {
            file_patterns: patterns,
            files: doc.adapter.paths.files,
            content_dirs,
        })
    }

    /// Discover matching files under each content directory. Unreadable or missing
    /// directories become `PARSE_ERROR` findings; the walk continues past them. A
    /// readable target where nothing matches yields an info `NO_MATCHING_FILES`
    /// naming the globs and directories, so a mistyped glob never reads as an empty
    /// register.
    pub(crate) fn collect_files(&self, target: &Path, doc: &mut Document) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if !target.is_dir() {
            doc.parse_error(
                format!(
                    "target '{}' does not exist or is not a directory",
                    target.display()
                ),
                target.display().to_string(),
                0,
            );
            return files;
        }
        for dir_name in &self.content_dirs {
            let dir = target.join(dir_name);
            if !dir.is_dir() {
                doc.parse_error(
                    format!("content directory '{dir_name}' does not exist or is not a directory"),
                    dir_name.as_str(),
                    0,
                );
                continue;
            }
            self.walk_dir(&dir, target, doc, &mut files);
        }
        if files.is_empty() {
            doc.findings.push(Issue::new(
                Severity::Info,
                latt_adapter::codes::NO_MATCHING_FILES,
                format!(
                    "no files under adapter.paths.content_dirs {:?} matched \
                     adapter.paths.files {:?}",
                    self.content_dirs, self.files
                ),
                Provenance::new_file("."),
            ));
        }
        files.sort();
        files.retain(|path| {
            let raw = path.strip_prefix(target).unwrap_or(path);
            if raw.to_str().is_some() {
                return true;
            }
            let rel = relative(path, target);
            let message = format!("{rel}: path is not valid UTF-8: {:?}", raw.as_os_str());
            doc.parse_error(message, rel, 0);
            false
        });
        files
    }

    fn walk_dir(&self, dir: &Path, target: &Path, doc: &mut Document, files: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                walk_error(doc, relative(dir, target), "read directory", &error);
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    walk_error(doc, relative(dir, target), "read an entry in", &error);
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    walk_error(doc, relative(&path, target), "inspect", &error);
                    continue;
                }
            };
            // `entry.file_type()` does not follow symlinks, so a symlinked directory
            // cycle cannot recurse; symlinks that resolve to files are still read, and
            // a dangling one is kept so that reading it reports it.
            if file_type.is_dir() {
                self.walk_dir(&path, target, doc, files);
            } else if file_type.is_file()
                || (file_type.is_symlink() && latt_adapter::is_file_or_dangling(&path))
            {
                let rel = relative(&path, target);
                if self.file_patterns.iter().any(|p| p.is_match(&rel)) {
                    files.push(path);
                }
            }
        }
    }
}

pub(crate) fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn walk_error(doc: &mut Document, file: String, context: &str, error: &io::Error) {
    doc.parse_error(format!("could not {context} '{file}': {error}"), file, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_adapter_keys_are_named() {
        let document = |adapter: &str| format!(r#"{{"adapter":{{{adapter}}}}}"#);
        let cases = [
            (r#""paths":{"files":["*.md"]},"path":{}"#, "path"),
            (
                r#""paths":{"files":["*.md"],"content_dir":["ref"]}"#,
                "content_dir",
            ),
        ];
        for (adapter, key) in cases {
            let error = serde_json::from_str::<ProfileDocument>(&document(adapter)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field `{key}`")),
                "{error}"
            );
        }
        serde_json::from_str::<ProfileDocument>(&document(
            r#""paths":{"files":["*.md"]},"native":"prose-xref","timeout_seconds":30"#,
        ))
        .unwrap();
    }

    #[test]
    fn only_a_resolved_profile_document_loads() {
        let dir = std::env::temp_dir().join(format!(
            "latt-adapter-prose-xref-schema-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let adapter = r#""adapter":{"paths":{"files":["*.md"],"content_dirs":["ref"]}}"#;
        let load = |name: &str, marker: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("{{{marker}{adapter}}}")).unwrap();
            Config::load(&path)
        };
        let resolved = load("resolved.json", r#""resolved_schema":"1","#);
        let missing = load("missing.json", "");
        let wrong = load("wrong.json", r#""resolved_schema":"2","#);
        std::fs::remove_dir_all(&dir).unwrap();
        resolved.unwrap();
        for error in [missing.unwrap_err(), wrong.unwrap_err()] {
            assert!(
                error.ends_with("not a resolved profile document"),
                "{error}"
            );
        }
    }

    #[test]
    fn walk_error_reports_each_context() {
        for context in ["read directory", "read an entry in", "inspect"] {
            let mut doc = Document::default();
            walk_error(
                &mut doc,
                "ref/sub".into(),
                context,
                &io::Error::other("denied"),
            );
            assert_eq!(doc.findings.len(), 1);
            let finding = &doc.findings[0];
            assert_eq!(finding.code, "PARSE_ERROR");
            assert_eq!(
                finding.message,
                format!("could not {context} 'ref/sub': denied")
            );
            assert_eq!(finding.provenance.file(), "ref/sub");
            assert_eq!(finding.provenance.line(), None);
        }
    }
}

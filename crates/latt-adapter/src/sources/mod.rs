//! Source-existence check: a node whose kind declares `id_names` names a file,
//! or a definition in a source file, that must exist in the target.
//!
//! Address parsing, file resolution and the findings know no language; each
//! language is a private scanner that turns a file into the [`Scope`] of names
//! defined at the positions that language counts.

mod go;
mod python;
mod rust;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Document, Issue, Provenance, Severity};

const PRESETS: &str = "'file', 'cargo', 'pytest' and 'gotest'";
const LANGUAGES: &str = "'rust', 'python' and 'go'";
const BLOCK_KEYS: &str = "'language', 'separator' and 'params'";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Language {
    Rust,
    Python,
    Go,
}

impl Language {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Rust => ".rs",
            Self::Python => ".py",
            Self::Go => ".go",
        }
    }

    /// The longest symbol chain the language's position rule reaches.
    fn max_chain(self) -> Option<usize> {
        match self {
            Self::Rust => None,
            Self::Python => Some(2),
            Self::Go => Some(1),
        }
    }

    /// What a missing container is called in a finding.
    fn container_noun(self) -> &'static str {
        match self {
            Self::Rust => "module",
            Self::Python => "class",
            Self::Go => "container",
        }
    }

    fn scan(self, text: &str) -> Result<Scope, ScanError> {
        match self {
            Self::Rust => rust::scan(text),
            Self::Python => python::scan(text),
            Self::Go => go::scan(text),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Form {
    File,
    Symbol {
        language: Language,
        separator: String,
        params: Option<(char, char)>,
    },
}

/// The names one container defines, and the containers nested in it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Scope {
    names: BTreeSet<String>,
    children: BTreeMap<String, Scope>,
}

/// Why a scanner could not read a file, and the line where it stopped.
#[derive(Debug, PartialEq, Eq)]
struct ScanError {
    line: u32,
    reason: String,
}

impl ScanError {
    fn new(line: u32, reason: impl Into<String>) -> Self {
        Self {
            line,
            reason: reason.into(),
        }
    }
}

/// A character cursor that counts lines, shared by the scanners.
struct Cursor {
    chars: Vec<char>,
    pos: usize,
    line: u32,
}

impl Cursor {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            line: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(offset, c)| self.peek_at(offset) == Some(c))
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.pos += 1;
        }
    }

    fn ident(&mut self) -> String {
        let mut ident = String::new();
        while let Some(c) = self.peek() {
            if !is_ident_continue(c) {
                break;
            }
            ident.push(c);
            self.pos += 1;
        }
        ident
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn form_of(value: &Value) -> Result<Form, String> {
    match value {
        Value::String(preset) => match preset.as_str() {
            "file" => Ok(Form::File),
            "cargo" => Ok(Form::Symbol {
                language: Language::Rust,
                separator: "::".into(),
                params: None,
            }),
            "pytest" => Ok(Form::Symbol {
                language: Language::Python,
                separator: "::".into(),
                params: Some(('[', ']')),
            }),
            "gotest" => Ok(Form::Symbol {
                language: Language::Go,
                separator: "::".into(),
                params: None,
            }),
            other => Err(format!(
                "names unknown preset '{other}'; accepted presets are {PRESETS}"
            )),
        },
        Value::Object(block) => {
            if let Some(key) = block
                .keys()
                .find(|key| !matches!(key.as_str(), "language" | "separator" | "params"))
            {
                return Err(format!(
                    "has unknown key '{key}'; accepted keys are {BLOCK_KEYS}"
                ));
            }
            let language = match block.get("language") {
                None => {
                    return Err(format!(
                        "needs 'language'; accepted languages are {LANGUAGES}"
                    ));
                }
                Some(Value::String(name)) => Language::parse(name).ok_or_else(|| {
                    format!("names unknown language '{name}'; accepted languages are {LANGUAGES}")
                })?,
                Some(other) => {
                    return Err(format!(
                        "has a non-string 'language' {other}; accepted languages are {LANGUAGES}"
                    ));
                }
            };
            let separator = match block.get("separator") {
                Some(Value::String(separator)) if !separator.is_empty() => separator.clone(),
                _ => return Err("needs 'separator', a non-empty string".into()),
            };
            let params = match block.get("params") {
                None => None,
                Some(Value::String(pair)) => {
                    let chars: Vec<char> = pair.chars().collect();
                    match chars.as_slice() {
                        [open, close] => Some((*open, *close)),
                        _ => {
                            return Err(format!(
                                "has 'params' '{pair}'; it must be two characters, opener then closer"
                            ));
                        }
                    }
                }
                Some(other) => {
                    return Err(format!(
                        "has a non-string 'params' {other}; it must be two characters, opener then closer"
                    ));
                }
            };
            Ok(Form::Symbol {
                language,
                separator,
                params,
            })
        }
        other => Err(format!(
            "must be a preset name ({PRESETS}) or a mapping with {BLOCK_KEYS}, got {other}"
        )),
    }
}

/// Check that a node kind's `id_names` value is one the source-existence check
/// can honour.
///
/// The core's profile loader calls this, so the accepted presets, languages and
/// block keys have one owner. `null` reads as absent, as it does for the other
/// node-kind keys.
///
/// # Errors
///
/// A message, phrased to follow `'id_names'`, naming the offending value or
/// key and what is accepted.
///
/// ```
/// use serde_json::json;
///
/// latt_adapter::validate_id_names(&json!("cargo"))?;
/// latt_adapter::validate_id_names(&json!({"language": "go", "separator": "::"}))?;
/// let error = latt_adapter::validate_id_names(&json!("jest")).unwrap_err();
/// assert!(error.contains("'jest'") && error.contains("'gotest'"), "{error}");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn validate_id_names(value: &Value) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }
    form_of(value).map(|_| ())
}

/// Resolve every node whose kind declares `id_names` against `target`, and
/// append a warning `SOURCE_MISSING` for each one that does not resolve.
///
/// Only declared nodes are resolved; an ID named only as an edge target is the
/// core's `VACANCY`. Nothing in the target is imported, compiled or executed:
/// a symbol address is resolved by scanning its file. A file a scanner cannot
/// read is one warning `PARSE_ERROR`, and the nodes naming it are left
/// unchecked. Findings follow the document's existing ones, in node order.
///
/// # Errors
///
/// A kind's `id_names` is not a form [`validate_id_names`] accepts. That is a
/// broken profile, not malformed target content, so no finding is appended.
///
/// ```
/// use std::collections::BTreeMap;
/// use latt_adapter::{Document, Node, Provenance};
///
/// let target = std::env::temp_dir().join(format!("latt-adapter-sources-{}", std::process::id()));
/// std::fs::create_dir_all(&target)?;
/// std::fs::write(target.join("present.md"), "")?;
/// let kinds: BTreeMap<String, serde_json::Value> =
///     [("doc".to_owned(), serde_json::json!({"id_names": "file"}))].into_iter().collect();
///
/// let mut document = Document::new();
/// for id in ["present.md", "gone.md"] {
///     document.nodes.push(Node::new(id, "doc", Provenance::new("REGISTER.md", 3)));
/// }
/// latt_adapter::check_sources(&mut document, &kinds, &target)?;
/// std::fs::remove_dir_all(&target)?;
///
/// assert_eq!(document.findings.len(), 1);
/// assert_eq!(document.findings[0].code, "SOURCE_MISSING");
/// assert_eq!(document.findings[0].node_id.as_deref(), Some("gone.md"));
/// assert_eq!(document.findings[0].message, "'gone.md' does not resolve: no file at 'gone.md'");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn check_sources(
    document: &mut Document,
    node_kinds: &BTreeMap<String, Value>,
    target: &Path,
) -> Result<(), String> {
    let mut forms = BTreeMap::new();
    for (kind, declaration) in node_kinds {
        if let Some(value) = declaration.get("id_names").filter(|value| !value.is_null()) {
            let form = form_of(value)
                .map_err(|error| format!("node kind '{kind}': 'id_names' {error}"))?;
            forms.insert(kind.as_str(), form);
        }
    }
    if forms.is_empty() {
        return Ok(());
    }
    let mut resolver = Resolver::new(target);
    let mut findings = Vec::new();
    for node in &document.nodes {
        let Some(form) = forms.get(node.kind.as_str()) else {
            continue;
        };
        match resolver.resolve(&node.id, form) {
            Outcome::Resolved | Outcome::Unchecked => {}
            Outcome::Unreadable(issue) => findings.push(issue),
            Outcome::Missing(reason) => findings.push(
                Issue::new(
                    Severity::Warning,
                    crate::codes::SOURCE_MISSING,
                    format!("'{}' does not resolve: {reason}", node.id),
                    node.provenance.clone(),
                )
                .with_node_id(&node.id),
            ),
        }
    }
    document.findings.extend(findings);
    Ok(())
}

enum Outcome {
    Resolved,
    Missing(String),
    /// The file's scanner failed; this is the first node to meet it.
    Unreadable(Issue),
    /// The file's scanner failed and an earlier node already reported it.
    Unchecked,
}

/// A source file as one run read it: its scope, or the scan failure once
/// reported. Lives for one `check_sources` call, over one immutable target.
enum Indexed {
    Scope(Scope),
    Failed,
}

struct Resolver {
    target: Result<PathBuf, String>,
    index: HashMap<(PathBuf, Language), Indexed>,
}

impl Resolver {
    fn new(target: &Path) -> Self {
        Self {
            target: target.canonicalize().map_err(|error| error.to_string()),
            index: HashMap::new(),
        }
    }

    fn resolve(&mut self, id: &str, form: &Form) -> Outcome {
        let Form::Symbol {
            language,
            separator,
            params,
        } = form
        else {
            return match self.file(id) {
                Ok(_) => Outcome::Resolved,
                Err(reason) => Outcome::Missing(reason),
            };
        };
        let (path, chain) = match parse_address(id, *language, separator, *params) {
            Ok(address) => address,
            Err(reason) => return Outcome::Missing(reason),
        };
        let real = match self.file(path) {
            Ok(real) => real,
            Err(reason) => return Outcome::Missing(reason),
        };
        if chain.is_empty() {
            return Outcome::Resolved;
        }
        let key = (real, *language);
        let mut first_failure = None;
        let indexed =
            self.index
                .entry(key)
                .or_insert_with_key(|(real, language)| match read_scope(real, *language) {
                    Ok(scope) => Indexed::Scope(scope),
                    Err((line, reason)) => {
                        first_failure = Some(Issue::new(
                            Severity::Warning,
                            crate::codes::PARSE_ERROR,
                            format!(
                                "could not read {} source '{path}': {reason}",
                                language.name()
                            ),
                            Provenance::new(path, line),
                        ));
                        Indexed::Failed
                    }
                });
        let Indexed::Scope(scope) = indexed else {
            return first_failure.map_or(Outcome::Unchecked, Outcome::Unreadable);
        };
        match missing_prefix(scope, &chain) {
            None => Outcome::Resolved,
            Some((depth, true)) => Outcome::Missing(format!(
                "'{path}' defines no {} '{}'",
                language.container_noun(),
                chain[..=depth].join(separator)
            )),
            Some((_, false)) => {
                Outcome::Missing(format!("'{path}' defines no '{}'", chain.join(separator)))
            }
        }
    }

    /// Resolve a `/`-separated path to a regular file inside the target.
    fn file(&self, path: &str) -> Result<PathBuf, String> {
        if path.starts_with('/') || path.split('/').any(|component| component == "..") {
            return Err(format!("'{path}' is outside the target"));
        }
        let target = self
            .target
            .as_ref()
            .map_err(|error| format!("'{path}' could not be read: {error}"))?;
        let real = match target.join(path).canonicalize() {
            Ok(real) => real,
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                return Err(format!("no file at '{path}'"));
            }
            Err(error) => return Err(format!("'{path}' could not be read: {error}")),
        };
        if !real.starts_with(target) {
            return Err(format!("'{path}' is outside the target"));
        }
        match real.metadata() {
            Ok(metadata) if metadata.is_file() => Ok(real),
            Ok(_) => Err(format!("'{path}' is not a regular file")),
            Err(error) => Err(format!("'{path}' could not be read: {error}")),
        }
    }
}

/// Split a symbol address into its path and chain, applying the shape checks
/// that need no filesystem access.
fn parse_address<'a>(
    id: &'a str,
    language: Language,
    separator: &str,
    params: Option<(char, char)>,
) -> Result<(&'a str, Vec<&'a str>), String> {
    let mut address = id;
    if let Some((open, close)) = params
        && address.ends_with(close)
        && let Some(first) = address.find(separator)
    {
        let after = first + separator.len();
        if let Some(offset) = address[after..].find(open) {
            address = &address[..after + offset];
        }
    }
    let mut segments = address.split(separator);
    let path = segments.next().unwrap_or_default();
    let chain: Vec<&str> = segments.collect();
    if path.is_empty() || chain.iter().any(|name| name.is_empty()) {
        return Err("empty name in node ID".into());
    }
    if let Some(max) = language.max_chain()
        && chain.len() > max
    {
        return Err(format!(
            "{} addresses reach at most {max} names",
            language.name()
        ));
    }
    if !path.ends_with(language.extension()) {
        return Err(format!(
            "a {} address names a '{}' file",
            language.name(),
            language.extension()
        ));
    }
    Ok((path, chain))
}

/// The depth of the first chain step the scope lacks, and whether that step
/// is a container. `None` when the whole chain resolves.
fn missing_prefix(scope: &Scope, chain: &[&str]) -> Option<(usize, bool)> {
    let (last, containers) = chain.split_last()?;
    let mut current = scope;
    for (depth, name) in containers.iter().enumerate() {
        current = match current.children.get(*name) {
            Some(child) => child,
            None => return Some((depth, true)),
        };
    }
    (!current.names.contains(*last)).then_some((containers.len(), false))
}

fn read_scope(real: &Path, language: Language) -> Result<Scope, (u32, String)> {
    let bytes = std::fs::read(real).map_err(|error| (0, error.to_string()))?;
    if let Some(refusal) = beyond_lines(bytes.iter().filter(|byte| **byte == b'\n').count() + 1) {
        return Err((0, refusal));
    }
    let text = crate::decode_utf8(&bytes).map_err(|error| {
        let line = bytes[..error.valid_up_to()]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1;
        (
            u32::try_from(line).unwrap_or(u32::MAX),
            "invalid UTF-8".to_owned(),
        )
    })?;
    language
        .scan(text)
        .map_err(|error| (error.line, error.reason))
}

/// The refusal for a file of `line_count` lines, if it has more than fit `u32`.
///
/// Checked before scanning, so the scanners' line counter and every line they
/// report fit, and no finding is placed at a wrapped or clamped line.
fn beyond_lines(line_count: usize) -> Option<String> {
    (line_count > u32::MAX as usize).then(|| {
        format!(
            "file has {line_count} lines, more than the {} a provenance line can name",
            u32::MAX
        )
    })
}

#[cfg(test)]
mod tests;

//! Template Lattice adapter: reads line registers and prints an interface document.
//!
//! Usage: `lines-adapter --profile <resolved-profile.json> --target <dir>`
//!
//! The register is the files in one directory whose names end in one extension, each
//! line a declaration:
//!
//! ```text
//! <kind> <id> [<edge-kind> <target-id>]... : <text>
//! ```
//!
//! Copy this crate and change [`read_line`] for your own format. The contract it follows
//! is `docs/adapters.md`, "Writing your own adapter"; [`latt_adapter::Document`] builds a
//! document that meets it.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use latt_adapter::{Document, Edge, Issue, Node, Provenance, Severity, codes};
use serde_json::Value;

/// Keys of the profile's `adapter:` block that belong to the core; an adapter accepts them.
const CORE_KEYS: [&str; 3] = ["native", "timeout_seconds", "command_timeout_seconds"];
const DECLARATION: &str = "\"<kind> <id> [<edge-kind> <target-id>]... : <text>\"";
const BOM: &[u8] = b"\xef\xbb\xbf";

/// A reason the adapter cannot produce a document: printed, then exit 2.
struct Failure(String);

fn parse_args(args: &[String]) -> Result<(String, String), Failure> {
    const USAGE: &str = "usage: --profile <resolved-profile.json> --target <dir>";
    let (mut profile, mut target) = (None, None);
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let slot = match flag.as_str() {
            "--profile" => &mut profile,
            "--target" => &mut target,
            _ => return Err(Failure(format!("{USAGE}; unexpected {flag:?}"))),
        };
        let Some(value) = rest.next() else {
            return Err(Failure(format!("{USAGE}; unexpected {flag:?}")));
        };
        *slot = Some(value.clone());
    }
    match (profile, target) {
        (Some(profile), Some(target)) => Ok((profile, target)),
        _ => Err(Failure(USAGE.to_owned())),
    }
}

/// Return the directory and extension from the resolved profile's `adapter:` block.
fn load_config(path: &str) -> Result<(String, String), Failure> {
    // load_resolved_profile refuses a file that is not the core's resolved profile.
    let profile: Value = latt_adapter::load_resolved_profile(Path::new(path)).map_err(|error| {
        Failure(format!(
            "{path} is not a resolved profile ({error}); pass the file `latt show` prints"
        ))
    })?;
    let Some(adapter) = profile.get("adapter").and_then(Value::as_object) else {
        return Err(Failure(format!("{path}: the adapter block is missing")));
    };
    if let Some(key) = adapter.keys().find(|key| {
        !CORE_KEYS.contains(&key.as_str()) && *key != "directory" && *key != "extension"
    }) {
        return Err(Failure(format!("{path}: unknown adapter key {key:?}")));
    }
    let string = |key: &str| {
        adapter
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Failure(format!("{path}: adapter key {key:?} must be a string")))
    };
    Ok((string("directory")?, string("extension")?))
}

/// Read one line of a register file into the document.
fn read_line(document: &mut Document, raw: &[u8], file: &str, number: u32) {
    let line = match std::str::from_utf8(raw) {
        Ok(line) => line,
        Err(error) => {
            let column = error.valid_up_to() + 1;
            document.parse_error(
                format!("line is not valid UTF-8 at byte {column}, so it was not read"),
                file,
                number,
            );
            return;
        }
    };
    if line.trim().is_empty() || line.trim_start().starts_with('#') {
        return;
    }
    let Some((head, text)) = line.split_once(':') else {
        document.parse_error(
            format!("line is not a declaration {DECLARATION}, so it was not read"),
            file,
            number,
        );
        return;
    };
    let words: Vec<&str> = head.split_whitespace().collect();
    if words.len() < 2 || !words.len().is_multiple_of(2) {
        document.parse_error(
            format!("line is not a declaration {DECLARATION}, so it was not read"),
            file,
            number,
        );
        return;
    }
    let (kind, id) = (words[0], words[1]);
    let attrs = BTreeMap::from([("text".to_owned(), Value::from(text.trim()))]);
    // An edge names the node that declared it by index, not by ID: a repeated ID is the
    // core's to report, and it needs to know which occurrence declared the edge.
    let index = document.nodes.len();
    document
        .nodes
        .push(Node::new(id, kind, Provenance::new(file, number)).with_attrs(attrs));
    for pair in words[2..].chunks(2) {
        document.edges.push(
            Edge::new(id, pair[1], pair[0], Provenance::new(file, number)).declared_by(index),
        );
    }
}

fn read_file(document: &mut Document, path: &Path, file: &str) {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) => {
            document.parse_error(format!("file could not be read: {error}"), file, 0);
            return;
        }
    };
    let data = data.strip_prefix(BOM).unwrap_or(&data);
    for (index, raw) in data.split(|byte| *byte == b'\n').enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        read_line(
            document,
            raw.strip_suffix(b"\r").unwrap_or(raw),
            file,
            number,
        );
    }
}

/// Build the document for the register in `target/directory`.
fn build(target: &Path, directory: &str, extension: &str) -> Document {
    let mut document = Document::new();
    let root = target.join(directory);
    let mut entries: Vec<fs::DirEntry> = match fs::read_dir(&root) {
        Ok(listing) => {
            let mut entries = Vec::new();
            for entry in listing {
                match entry {
                    Ok(entry) => entries.push(entry),
                    Err(error) => document.parse_error(
                        format!("an entry of this directory could not be read: {error}"),
                        directory,
                        0,
                    ),
                }
            }
            entries
        }
        Err(_) => {
            document.parse_error(
                format!("directory \"{directory}\" does not exist or is not a directory"),
                directory,
                0,
            );
            return document;
        }
    };
    // Byte order of name, as the other templates read.
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut selected = 0;
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            document.parse_error(
                "a file name in this directory is not valid UTF-8, so it was not read",
                directory,
                0,
            );
            continue;
        };
        if !name.ends_with(extension) {
            continue;
        }
        selected += 1;
        let file = format!("{directory}/{name}");
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            document.parse_error("not a regular file, so it was not read", &file, 0);
            continue;
        }
        read_file(&mut document, &entry.path(), &file);
    }
    if selected == 0 {
        document.findings.push(Issue::new(
            Severity::Info,
            codes::NO_MATCHING_FILES,
            format!("no file in \"{directory}\" ends in \"{extension}\""),
            Provenance::new(directory, 0),
        ));
    }
    document
}

fn run() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (profile, target) = parse_args(&args)?;
    let (directory, extension) = load_config(&profile)?;
    let document = build(Path::new(&target), &directory, &extension);
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &document)
        .map_err(|error| Failure(format!("cannot write the document: {error}")))?;
    writeln!(out).map_err(|error| Failure(format!("cannot write the document: {error}")))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // Exit 2 with no document: the core then reports the adapter failed to run.
        Err(Failure(message)) => {
            eprintln!("lines-adapter: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the shared fixture with the configuration `../profile.yaml` gives and compare
    /// it with `../expected.json`. In a copy, point it at your own register.
    #[test]
    fn fixture_document() -> Result<(), Box<dyn std::error::Error>> {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let document = build(&examples.join("fixture"), "register", ".reg");
        let expected: Value = serde_json::from_slice(&fs::read(examples.join("expected.json"))?)?;
        assert_eq!(serde_json::to_value(&document)?, expected);
        Ok(())
    }
}

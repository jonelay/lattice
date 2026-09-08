use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::document::Document;

pub(crate) const BRANCH: &str = "entomologist-data";
const REFS: [&str; 2] = [
    "refs/heads/entomologist-data",
    "refs/remotes/origin/entomologist-data",
];

#[derive(Debug)]
pub(crate) struct TreeEntry {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) object_type: String,
}

fn git(target: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(target)
        .args(args)
        .output()
}

/// Resolve ent's data branch to one immutable commit before reading it.
pub(crate) fn resolve_commit(document: &mut Document, target: &Path) -> Option<String> {
    let toplevel = match git(target, &["rev-parse", "--show-toplevel"]) {
        Ok(output) if output.status.success() => output,
        Ok(_) => {
            document.parse_error(
                format!(
                    "target '{}' is not inside a git repository",
                    target.display()
                ),
                ".",
            );
            return None;
        }
        Err(error) => {
            document.parse_error(format!("failed to run git: {error}"), ".");
            return None;
        }
    };

    let root_text = match std::str::from_utf8(&toplevel.stdout) {
        Ok(text) => text.trim(),
        Err(error) => {
            document.parse_error(
                format!("git repository root is not valid UTF-8: {error}"),
                ".",
            );
            return None;
        }
    };
    let root = match std::fs::canonicalize(root_text) {
        Ok(root) => root,
        Err(error) => {
            document.parse_error(
                format!("could not resolve git repository root: {error}"),
                ".",
            );
            return None;
        }
    };
    let resolved_target = match std::fs::canonicalize(target) {
        Ok(path) => path,
        Err(_) => target.to_path_buf(),
    };
    if root != resolved_target {
        document.parse_error(
            format!(
                "target '{}' is not the repository root '{}'; refusing to read another checkout's register",
                target.display(),
                root.display()
            ),
            ".",
        );
        return None;
    }

    for git_ref in REFS {
        match git(target, &["rev-parse", "--verify", "--quiet", git_ref]) {
            Ok(output) if output.status.success() => {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_owned());
            }
            Ok(_) => {}
            Err(error) => {
                document.parse_error(format!("failed to run git: {error}"), ".");
                return None;
            }
        }
    }
    document.parse_error(
        format!("repository has no '{BRANCH}' branch (local or origin)"),
        ".",
    );
    None
}

/// List every recursive tree entry from the pinned register commit.
pub(crate) fn list_tree(
    document: &mut Document,
    target: &Path,
    commit: &str,
) -> Option<Vec<TreeEntry>> {
    let output = match git(target, &["ls-tree", "-r", "-z", commit]) {
        Ok(output) => output,
        Err(error) => {
            document.parse_error(format!("git ls-tree failed for {commit}: {error}"), ".");
            return None;
        }
    };
    if !output.status.success() {
        document.parse_error(
            format!(
                "git ls-tree failed for {commit}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            ".",
        );
        return None;
    }

    let mut entries = Vec::new();
    for raw_record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|r| !r.is_empty())
    {
        let Some(tab) = raw_record.iter().position(|byte| *byte == b'\t') else {
            document.parse_error("git ls-tree returned a malformed record", ".");
            continue;
        };
        let meta = match std::str::from_utf8(&raw_record[..tab]) {
            Ok(meta) => meta,
            Err(error) => {
                document.parse_error(
                    format!("git ls-tree metadata is not valid UTF-8: {error}"),
                    ".",
                );
                continue;
            }
        };
        let path = match std::str::from_utf8(&raw_record[tab + 1..]) {
            Ok(path) => path.to_owned(),
            Err(error) => {
                document.parse_error(format!("git ls-tree path is not valid UTF-8: {error}"), ".");
                continue;
            }
        };
        let mut fields = meta.split(' ');
        let (Some(_mode), Some(object_type), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            document.parse_error(format!("{path}: malformed git ls-tree metadata"), ".");
            continue;
        };
        entries.push(TreeEntry {
            path,
            oid: oid.to_owned(),
            object_type: object_type.to_owned(),
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Some(entries)
}

/// Fetch all represented blobs in one ordered `cat-file --batch` exchange.
pub(crate) fn cat_file_batch(target: &Path, object_ids: &[&str]) -> std::io::Result<Output> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let stdin = child.stdin.as_mut().expect("piped stdin is available");
        for oid in object_ids {
            writeln!(stdin, "{oid}")?;
        }
    }
    child.wait_with_output()
}

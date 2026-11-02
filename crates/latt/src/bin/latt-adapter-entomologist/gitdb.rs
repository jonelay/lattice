use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use latt_adapter::Document;
use latt_adapter::process::{BoundedError, Failure, run_labelled, spawn_bounded};

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

/// Run one git subcommand under `bound`.
///
/// Every call site reports expiry the same way, naming `git` and the subcommand.
fn git(target: &Path, args: &[&str], bound: Duration) -> Result<Output, Failure> {
    let mut command = Command::new("git");
    command.arg("-C").arg(target).args(args);
    run_labelled(command, &format!("git {}", args[0]), bound)
}

/// Resolve ent's data branch to one immutable commit before reading it.
///
/// `Err` is the message for a git that cannot be run; every other failure is a
/// finding in `document` and `Ok(None)`.
pub(crate) fn resolve_commit(
    document: &mut Document,
    target: &Path,
    bound: Duration,
) -> Result<Option<String>, String> {
    let toplevel = match git(target, &["rev-parse", "--show-toplevel"], bound) {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            // git's own reason matters: a safe.directory refusal is not a
            // missing repository, though both exit non-zero here.
            document.parse_error(
                format!(
                    "target '{}' is not inside a git repository: {}",
                    target.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                ".",
                0,
            );
            return Ok(None);
        }
        Err(Failure::Unrunnable(message)) => return Err(message),
        Err(Failure::Failed(message)) => {
            document.parse_error(message, ".", 0);
            return Ok(None);
        }
    };

    let root_text = match std::str::from_utf8(&toplevel.stdout) {
        Ok(text) => text.trim(),
        Err(error) => {
            document.parse_error(
                format!("git repository root is not valid UTF-8: {error}"),
                ".",
                0,
            );
            return Ok(None);
        }
    };
    let root = match std::fs::canonicalize(root_text) {
        Ok(root) => root,
        Err(error) => {
            document.parse_error(
                format!("could not resolve git repository root: {error}"),
                ".",
                0,
            );
            return Ok(None);
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
            0,
        );
        return Ok(None);
    }

    for git_ref in REFS {
        match git(
            target,
            &["rev-parse", "--verify", "--quiet", git_ref],
            bound,
        ) {
            Ok(output) if output.status.success() => {
                return Ok(Some(
                    String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                ));
            }
            Ok(_) => {}
            Err(Failure::Unrunnable(message)) => return Err(message),
            Err(Failure::Failed(message)) => {
                document.parse_error(message, ".", 0);
                return Ok(None);
            }
        }
    }
    document.parse_error(
        format!("repository has no '{BRANCH}' branch (local or origin)"),
        ".",
        0,
    );
    Ok(None)
}

/// List every recursive tree entry from the pinned register commit.
pub(crate) fn list_tree(
    document: &mut Document,
    target: &Path,
    commit: &str,
    bound: Duration,
) -> Option<Vec<TreeEntry>> {
    let output = match git(target, &["ls-tree", "-r", "-z", commit], bound) {
        Ok(output) => output,
        Err(Failure::Unrunnable(message) | Failure::Failed(message)) => {
            document.parse_error(
                format!("git ls-tree failed for {commit}: {message}"),
                ".",
                0,
            );
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
            0,
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
            document.parse_error("git ls-tree returned a malformed record", ".", 0);
            continue;
        };
        let meta = match std::str::from_utf8(&raw_record[..tab]) {
            Ok(meta) => meta,
            Err(error) => {
                document.parse_error(
                    format!("git ls-tree metadata is not valid UTF-8: {error}"),
                    ".",
                    0,
                );
                continue;
            }
        };
        let path = match std::str::from_utf8(&raw_record[tab + 1..]) {
            Ok(path) => path.to_owned(),
            Err(error) => {
                document.parse_error(
                    format!("git ls-tree path is not valid UTF-8: {error}"),
                    ".",
                    0,
                );
                continue;
            }
        };
        let mut fields = meta.split(' ');
        let (Some(_mode), Some(object_type), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            document.parse_error(format!("{path}: malformed git ls-tree metadata"), ".", 0);
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
///
/// The bound covers the whole exchange: one process, one deadline.
pub(crate) fn cat_file_batch(
    target: &Path,
    object_ids: &[&str],
    bound: Duration,
) -> Result<Output, BoundedError> {
    cat_file_batch_with(Path::new("git"), target, object_ids, bound)
}

fn cat_file_batch_with(
    git: &Path,
    target: &Path,
    object_ids: &[&str],
    bound: Duration,
) -> Result<Output, BoundedError> {
    let mut command = Command::new(git);
    command
        .arg("-C")
        .arg(target)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped());
    let mut child = spawn_bounded(command, bound)?;
    let stdin = child.take_stdin().expect("piped stdin is available");
    let request: String = object_ids.iter().map(|oid| format!("{oid}\n")).collect();
    // Written off-thread so a git that never reads cannot block us on a full
    // pipe before the bound is polled; a write failure surfaces as git's status,
    // which the caller reports. The writer is never joined: a descendant that
    // inherited the pipe can hold it open past the deadline, and the bound
    // covers the whole exchange. The thread ends when the pipe closes.
    std::thread::spawn(move || {
        let mut stdin = stdin;
        stdin.write_all(request.as_bytes())
    });
    child.wait_output()
}

#[cfg(test)]
#[path = "../../../tests/common/stub.rs"]
mod stub;

#[cfg(test)]
mod tests {
    use super::*;

    // Requirement: adapter-entomologist/Every git call the adapter runs is bounded
    #[test]
    fn a_descendant_holding_stdin_does_not_extend_the_bound() {
        // The stub never reads; its background child inherits the pipe and
        // outlives the kill. A request past the pipe buffer blocks the writer.
        let dir = std::env::temp_dir().join(format!("lattice_ent_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = dir.join("git");
        stub::write_stub(&git, "#!/bin/sh\nsleep 3 &\nexec sleep 30\n");
        let oids: Vec<String> = (0..5000).map(|i| format!("{i:040x}")).collect();
        let oids: Vec<&str> = oids.iter().map(String::as_str).collect();
        let started = std::time::Instant::now();
        let result = cat_file_batch_with(&git, &dir, &oids, Duration::from_millis(200));
        let elapsed = started.elapsed();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            matches!(result, Err(BoundedError::TimedOut { .. })),
            "{result:?}"
        );
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    }
}

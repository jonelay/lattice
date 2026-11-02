//! Materializing a git revision's register into a temporary directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use latt_adapter::process::spawn_bounded;

use super::QueryError;

/// One directory both revisions are materialized into, removed on drop.
///
/// One path, not one per revision, and that is load-bearing: adapters may
/// embed the target path in attrs, so two materialization directories would
/// make every such node "changed" in a diff of identical content. Extractions
/// are sequential; the second replaces the first.
#[derive(Debug)]
#[must_use = "the directory is removed when this guard drops"]
#[doc(hidden)]
pub struct MaterializationDir {
    path: PathBuf,
}

impl MaterializationDir {
    /// Create a fresh, process-unique directory under the system temp dir.
    ///
    /// # Errors
    ///
    /// A [`QueryError`] when the directory cannot be created.
    #[must_use = "the directory is removed when this guard drops"]
    pub fn new() -> Result<Self, QueryError> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "lattice-diff-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path)
            .map_err(|e| QueryError::new(format!("could not create {}: {e}", path.display())))?;
        Ok(Self { path })
    }

    /// The directory, valid until this guard drops.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MaterializationDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Extract the target's committed tree at `rev` into `into`, replacing
/// whatever revision was extracted there before.
///
/// `git archive` piped through `tar -x`, chosen over a worktree because it
/// touches none of the target's git state. There is nothing to deregister,
/// and the working tree stays exactly as the user left it.
///
/// Both processes run under `bound`. On expiry both are killed and the partial
/// extraction is removed, so a later revision never reads a mix of two trees.
///
/// # Errors
///
/// [`QueryError`] when `into` cannot be cleared or created, `git` or `tar`
/// cannot be started, either exits non-zero, or the pair exceeds `bound`.
#[doc(hidden)]
pub fn materialize_revision(
    target: &Path,
    rev: &str,
    into: &MaterializationDir,
    bound: Duration,
) -> Result<(), QueryError> {
    match std::fs::remove_dir_all(into.path()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Extracting over a tree that could not be cleared would mix two revisions.
        Err(e) => {
            return Err(QueryError::new(format!(
                "could not clear {} before extracting '{rev}': {e}",
                into.path().display()
            )));
        }
    }
    std::fs::create_dir_all(into.path())
        .map_err(|e| QueryError::new(format!("could not create {}: {e}", into.path().display())))?;

    let mut git = Command::new("git");
    git.arg("-C").arg(target).args(["archive", rev]);
    let mut archive = spawn_bounded(git, bound)
        .map_err(|e| QueryError::new(format!("could not run git: {e}")))?;
    let mut tar = Command::new("tar");
    tar.arg("-x").arg("-C").arg(into.path()).stdin(Stdio::from(
        archive.take_stdout().expect("stdout was piped"),
    ));
    let tar = spawn_bounded(tar, bound)
        .map_err(|e| QueryError::new(format!("could not run tar: {e}")))?;

    let outcome = archive
        .wait_output()
        .and_then(|archive| Ok((archive, tar.wait_output()?)));
    let (archive_output, tar_output) = match outcome {
        Ok(outputs) => outputs,
        Err(error) => {
            let _ = std::fs::remove_dir_all(into.path());
            return Err(QueryError::new(format!(
                "git archive '{rev}' in {}: {error}",
                target.display()
            )));
        }
    };

    if !archive_output.status.success() {
        let detail = String::from_utf8_lossy(&archive_output.stderr);
        return Err(QueryError::new(format!(
            "git archive '{rev}' failed in {}: {}",
            target.display(),
            detail.trim()
        )));
    }
    if !tar_output.status.success() {
        let detail = String::from_utf8_lossy(&tar_output.stderr);
        return Err(QueryError::new(format!(
            "tar failed extracting '{rev}': {}",
            detail.trim()
        )));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod materialize_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use super::{MaterializationDir, materialize_revision};

    #[test]
    fn a_tree_that_cannot_be_cleared_is_an_error_not_a_merge() {
        let into = MaterializationDir::new().unwrap();
        let locked = into.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("stale.md"), "old revision").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root (CI containers) ignores the mode, so there is no uncleared tree to test.
        if std::fs::write(locked.join("probe"), "").is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipped: directory permissions are not enforced for this user");
            return;
        }
        let result = materialize_revision(
            std::path::Path::new("."),
            "HEAD",
            &into,
            Duration::from_secs(5),
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.unwrap_err();
        assert!(
            error.message().contains("could not clear"),
            "{}",
            error.message()
        );
    }
}

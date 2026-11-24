//! Materializing a git revision's register into a temporary directory.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use latt_adapter::process::{BoundedError, Pollable, spawn_bounded};

use super::QueryError;

/// One directory both revisions are materialized into, removed on drop.
///
/// One path, not one per revision, and that is load-bearing: adapters may
/// embed the target path in attrs, so two materialization directories would
/// make every such node "changed" in a diff of identical content. Extractions
/// are sequential; the second replaces the first.
///
/// The directory is created exclusively with an unpredictable name and mode
/// 0700, so no other user can pre-create or enter it, and it never adopts a
/// path that already exists.
#[derive(Debug)]
#[must_use = "the directory is removed when this guard drops"]
#[doc(hidden)]
pub struct MaterializationDir {
    path: PathBuf,
    /// Device and inode at creation, so a path renamed and replaced since
    /// is recognised as not ours (Unix; elsewhere a real directory suffices).
    identity: Option<(u64, u64)>,
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// 128 random bits as 32 hex digits, from the OS (`/dev/urandom`), falling
/// back to std's per-instance hasher seeds where the device is unreadable.
fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    let from_os = std::fs::File::open("/dev/urandom")
        .and_then(|mut device| device.read_exact(&mut bytes))
        .is_ok();
    if !from_os {
        for half in bytes.chunks_mut(8) {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u32(std::process::id());
            half.copy_from_slice(&hasher.finish().to_le_bytes());
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Create a directory under `parent` that did not exist, trying each name
/// `next_name` yields until one is free.
fn create_exclusive(
    parent: &Path,
    mut next_name: impl FnMut() -> String,
) -> Result<PathBuf, QueryError> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    // A handful of collisions at 128 random bits means something is wrong.
    for _ in 0..16 {
        let path = parent.join(next_name());
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(QueryError::new(format!(
                    "could not create {}: {e}",
                    path.display()
                )));
            }
        }
    }
    Err(QueryError::new(format!(
        "could not create a fresh directory under {}",
        parent.display()
    )))
}

impl MaterializationDir {
    /// Create a fresh, private directory under the system temp dir.
    ///
    /// # Errors
    ///
    /// A [`QueryError`] when the directory cannot be created.
    #[must_use = "the directory is removed when this guard drops"]
    pub fn new() -> Result<Self, QueryError> {
        Self::new_in(&std::env::temp_dir())
    }

    fn new_in(parent: &Path) -> Result<Self, QueryError> {
        let path = create_exclusive(parent, || {
            format!("lattice-diff-{}-{}", std::process::id(), random_hex())
        })?;
        let identity = std::fs::symlink_metadata(&path)
            .map_err(|e| QueryError::new(format!("could not inspect {}: {e}", path.display())))
            .map(|meta| identity(&meta))?;
        Ok(Self { path, identity })
    }

    /// The directory, valid until this guard drops.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the path is still the real directory this guard created: not a
    /// symlink, and (on Unix) the same device and inode.
    fn is_ours(&self) -> bool {
        std::fs::symlink_metadata(&self.path)
            .is_ok_and(|meta| meta.file_type().is_dir() && identity(&meta) == self.identity)
    }
}

impl Drop for MaterializationDir {
    fn drop(&mut self) {
        // A path swapped for a symlink or another directory is not ours.
        if self.is_ours() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Remove everything inside `dir`, keeping `dir` and its mode.
fn empty_directory(dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if std::fs::symlink_metadata(&path)?.file_type().is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Whether a `git --version` line names a git that has `--end-of-options`
/// (2.24). An unparseable line counts as new enough.
fn version_has_end_of_options(line: &str) -> bool {
    let version = line.split_whitespace().nth(2).unwrap_or("");
    let mut parts = version.split('.').map(|part| part.parse::<u32>());
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= (2, 24),
        _ => true,
    }
}

/// Whether the `git` on `PATH` has `--end-of-options`, asked of `git --version`
/// within `bound`. Only an answer is remembered: a probe that expires is an
/// error and the next call asks again. A git that cannot be started or gives
/// no usable answer counts as new enough.
fn git_has_end_of_options(
    bound: Duration,
    expired: impl FnOnce() -> QueryError,
) -> Result<bool, QueryError> {
    static KNOWN: OnceLock<bool> = OnceLock::new();
    if let Some(known) = KNOWN.get() {
        return Ok(*known);
    }
    let mut git = Command::new("git");
    git.arg("--version").stdin(Stdio::null());
    let answer = match spawn_bounded(git, bound.max(Duration::from_millis(1)))
        .map_err(BoundedError::from)
        .and_then(|child| child.wait_output())
    {
        Ok(out) => version_has_end_of_options(&String::from_utf8_lossy(&out.stdout)),
        Err(BoundedError::TimedOut { .. }) => return Err(expired()),
        Err(_) => return Ok(true),
    };
    let _ = KNOWN.set(answer);
    Ok(answer)
}

/// The variables `git rev-parse --local-env-vars` lists (git 2.43) plus
/// `GIT_NAMESPACE`: each one selects a repository, its objects or its
/// configuration, and a git hook or `git rebase --exec` exports several.
const REPOSITORY_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

/// A git command for `target`'s own repository and the objects as stored:
/// repository-selecting variables from the environment do not apply, and
/// replacement objects (`refs/replace/*`) are not read.
fn target_git(target: &Path) -> Command {
    let mut git = Command::new("git");
    for name in REPOSITORY_ENV {
        git.env_remove(name);
    }
    git.arg("--no-replace-objects").arg("-C").arg(target);
    git
}

fn not_a_revision(target: &Path, rev: &str, reason: &str) -> QueryError {
    QueryError::new(format!(
        "'{}' is not a revision in {}: {reason}",
        rev.replace('\0', "\\0"),
        target.display()
    ))
}

/// Refuse a rev that would sit in an option position, without running git.
fn refuse_option_revision(target: &Path, rev: &str) -> Result<(), QueryError> {
    let refused = |reason: &str| not_a_revision(target, rev, reason);
    if rev.is_empty() {
        return Err(refused("empty revision"));
    }
    if rev.starts_with('-') {
        return Err(refused("a revision cannot start with '-'"));
    }
    if rev.contains('\0') {
        return Err(refused("a revision cannot contain NUL"));
    }
    Ok(())
}

/// Resolve `rev` in `target` to the object id it names, before the rev
/// reaches any command that takes it.
///
/// An empty rev, one that starts with `-` or one holding NUL is refused
/// without running git: such a value would sit in an option position.
/// The id, not the user's text, goes to every later git command.
fn resolve_revision(
    target: &Path,
    rev: &str,
    end_of_options: bool,
    bound: Duration,
    started: Instant,
) -> Result<String, QueryError> {
    refuse_option_revision(target, rev)?;
    let mut git = target_git(target);
    git.args(["rev-parse", "--verify"]);
    if end_of_options {
        git.arg("--end-of-options");
    }
    git.arg(rev).stdin(Stdio::null());
    // Bounded like the extraction: a ref store git cannot read (a FIFO under
    // `.git/refs`) must not hang the query past `timeout_seconds`.
    let remaining = bound.saturating_sub(started.elapsed());
    let output = spawn_bounded(git, remaining.max(Duration::from_millis(1)))
        .map_err(|e| QueryError::new(format!("could not run git: {e}")))?
        .wait_output()
        .map_err(|e| match e {
            BoundedError::TimedOut { .. } => QueryError::new(format!(
                "git rev-parse '{}' in {}: git exceeded {} s",
                rev.replace('\0', "\\0"),
                target.display(),
                seconds(bound)
            )),
            other => QueryError::new(format!("could not run git: {other}")),
        })?;
    let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let is_id = matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit());
    if !output.status.success() || !is_id {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().next().unwrap_or("").trim();
        let reason = if reason.is_empty() {
            "git did not resolve it"
        } else {
            reason
        };
        return Err(not_a_revision(target, rev, reason));
    }
    Ok(id)
}

/// What one tree entry becomes on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File {
        executable: bool,
    },
    Symlink,
    /// A submodule commit: an empty directory, as `git archive` made it.
    Gitlink,
}

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    shown: String,
    oid: String,
    kind: Kind,
}

/// Why a tree path may not be written under the scratch directory, or `Ok`.
///
/// A committed tree can hold `..`, `.git`, an absolute path or an empty
/// component when it was built by hand; none of them may leave or alter the
/// scratch directory.
fn check_path(path: &[u8]) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("empty path");
    }
    if path[0] == b'/' {
        return Err("absolute path");
    }
    for component in path.split(|byte| *byte == b'/') {
        // Off Unix a backslash is a separator and a colon names a drive or a
        // stream, so `..\x` or `C:\x` would leave the directory.
        if cfg!(not(unix)) && component.iter().any(|byte| matches!(byte, b'\\' | b':')) {
            return Err("'\\' or ':' in a path component");
        }
        match component {
            b"" => return Err("empty path component"),
            b"." => return Err("'.' path component"),
            b".." => return Err("'..' path component"),
            _ if component.eq_ignore_ascii_case(b".git") => {
                return Err("'.git' path component");
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(unix)]
fn os_path(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn os_path(bytes: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(bytes).ok().map(PathBuf::from)
}

fn refusal(rev: &str, path: &str, reason: &str) -> QueryError {
    QueryError::new(format!("cannot materialize '{path}' at '{rev}': {reason}"))
}

/// Parse `git ls-tree -r -z` output into entries, refusing the whole tree when
/// any entry is unsafe or has a mode this does not materialize.
fn parse_tree(stdout: &[u8], rev: &str) -> Result<Vec<Entry>, QueryError> {
    let mut entries = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    for record in stdout.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err(refusal(rev, "?", "malformed git ls-tree record"));
        };
        let raw_path = &record[tab + 1..];
        let shown = String::from_utf8_lossy(raw_path).into_owned();
        let meta = String::from_utf8_lossy(&record[..tab]).into_owned();
        let mut fields = meta.split(' ');
        let (Some(mode), Some(_type), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(refusal(rev, &shown, "malformed git ls-tree metadata"));
        };
        if let Err(reason) = check_path(raw_path) {
            refused.push((shown, reason.to_owned()));
            continue;
        }
        let kind = match mode {
            "100644" => Kind::File { executable: false },
            "100755" => Kind::File { executable: true },
            "120000" => Kind::Symlink,
            "160000" => Kind::Gitlink,
            other => {
                refused.push((shown, format!("unsupported mode {other}")));
                continue;
            }
        };
        let Some(path) = os_path(raw_path) else {
            refused.push((shown, "path is not valid UTF-8".to_owned()));
            continue;
        };
        entries.push(Entry {
            path,
            shown,
            oid: oid.to_owned(),
            kind,
        });
    }
    if let Some((path, reason)) = refused.first() {
        let more = refused.len() - 1;
        let tail = if more == 0 {
            String::new()
        } else {
            format!(" (and {more} more refused)")
        };
        return Err(refusal(rev, path, &format!("{reason}{tail}")));
    }
    Ok(entries)
}

/// Read one `cat-file --batch` reply for `expected` and stream its body to
/// `sink`, returning the byte count.
fn read_blob<R: BufRead>(
    reader: &mut R,
    expected: &str,
    sink: &mut impl Write,
) -> Result<u64, String> {
    let mut header = Vec::new();
    if reader
        .read_until(b'\n', &mut header)
        .map_err(|e| format!("could not read from git cat-file: {e}"))?
        == 0
    {
        return Err("git cat-file ended before every blob was read".to_owned());
    }
    let header = String::from_utf8_lossy(&header);
    let mut fields = header.split_whitespace();
    let (Some(oid), Some(kind)) = (fields.next(), fields.next()) else {
        return Err("git cat-file sent a malformed header".to_owned());
    };
    if oid != expected {
        return Err(format!("git cat-file answered {oid}, not {expected}"));
    }
    if kind == "missing" {
        return Err(format!("object {expected} is missing"));
    }
    let size = match (kind, fields.next().map(str::parse::<u64>)) {
        ("blob", Some(Ok(size))) => size,
        _ => return Err(format!("object {expected} is not a readable blob")),
    };
    let copied = std::io::copy(&mut reader.by_ref().take(size), sink)
        .map_err(|e| format!("could not write: {e}"))?;
    if copied != size {
        return Err("git cat-file ended inside a blob".to_owned());
    }
    let mut newline = [0u8; 1];
    reader
        .read_exact(&mut newline)
        .map_err(|e| format!("could not read from git cat-file: {e}"))?;
    Ok(size)
}

/// Write every entry under `root`: directories and files first, symlinks last,
/// so nothing is ever written through a link. An `Err` names the entry's path.
fn write_tree(stdout: impl Read, entries: &[Entry], root: &Path) -> Result<(), (String, String)> {
    let mut reader = std::io::BufReader::new(stdout);
    let mut links: Vec<(&Entry, Vec<u8>)> = Vec::new();
    for entry in entries {
        let fail = |reason: String| (entry.shown.clone(), reason);
        let destination = root.join(&entry.path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| fail(e.to_string()))?;
        }
        match entry.kind {
            Kind::Gitlink => {
                std::fs::create_dir(&destination).map_err(|e| fail(e.to_string()))?;
            }
            Kind::File { executable } => {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                let file = options
                    .open(&destination)
                    .map_err(|e| fail(e.to_string()))?;
                let mut file = std::io::BufWriter::new(file);
                read_blob(&mut reader, &entry.oid, &mut file).map_err(fail)?;
                let file = file.into_inner().map_err(|e| fail(e.to_string()))?;
                set_mode(&file, executable).map_err(|e| fail(e.to_string()))?;
            }
            Kind::Symlink => {
                let mut target = Vec::new();
                read_blob(&mut reader, &entry.oid, &mut target).map_err(fail)?;
                links.push((entry, target));
            }
        }
    }
    for (entry, target) in links {
        let fail = |reason: String| (entry.shown.clone(), reason);
        let target = os_path(&target).ok_or_else(|| fail("link target is not UTF-8".to_owned()))?;
        make_symlink(&target, &root.join(&entry.path)).map_err(|e| fail(e.to_string()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(file: &std::fs::File, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(if executable {
        0o755
    } else {
        0o644
    }))
}

#[cfg(not(unix))]
fn set_mode(_: &std::fs::File, _: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn make_symlink(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("symlinks are not supported here"))
}

fn seconds(bound: Duration) -> String {
    if bound.subsec_nanos() == 0 {
        bound.as_secs().to_string()
    } else {
        format!("{:.1}", bound.as_secs_f64())
    }
}

/// Extract the target's committed tree at `rev` into `into`, replacing
/// whatever revision was extracted there before.
///
/// The tree comes from `git ls-tree -r -z` and the blobs from one
/// `git cat-file --batch`, written to files as they arrive. Both are read-only
/// on the target's git state: there is nothing to deregister, and the working
/// tree stays exactly as the user left it. The bytes are the committed blobs:
/// `export-ignore`, `export-subst`, `eol`, `text` and `filter` attributes do not
/// apply and no smudge filter runs. Modes `100644` and `100755` become `0644` and
/// `0755`, `120000` a symlink (created last), `160000` an empty directory; any
/// other mode, or a path that could leave or alter the directory, is an error.
///
/// Both git processes run under one `bound`. On expiry or any failure the
/// child is killed and the partial extraction is removed, so a later revision
/// never reads a mix of two trees and a partial tree is never a success.
///
/// # Errors
///
/// [`QueryError`] when `into` cannot be cleared, `git` cannot be started or exits
/// non-zero, an entry is refused or cannot be written, or `bound` expires.
#[doc(hidden)]
pub fn materialize_revision(
    target: &Path,
    rev: &str,
    into: &MaterializationDir,
    bound: Duration,
) -> Result<(), QueryError> {
    let started = Instant::now();
    refuse_option_revision(target, rev)?;
    let end_of_options = git_has_end_of_options(bound, || {
        QueryError::new(format!(
            "git --version before '{}' in {}: git exceeded {} s",
            rev.replace('\0', "\\0"),
            target.display(),
            seconds(bound)
        ))
    })?;
    let id = resolve_revision(target, rev, end_of_options, bound, started)?;
    // A path that is no longer the directory this guard created (renamed and
    // replaced, perhaps by a symlink) is neither cleared nor written.
    if !into.is_ours() {
        return Err(QueryError::new(format!(
            "{} is no longer the directory lattice created; not extracting '{rev}'",
            into.path().display()
        )));
    }
    // Extracting over a tree that could not be cleared would mix two revisions.
    // The directory itself stays: it is owned, private, and the same path
    // serves both revisions.
    empty_directory(into.path()).map_err(|e| {
        QueryError::new(format!(
            "could not clear {} before extracting '{rev}': {e}",
            into.path().display()
        ))
    })?;
    let result = extract(target, rev, &id, into.path(), bound, started);
    if result.is_err() && into.is_ours() {
        let _ = empty_directory(into.path());
    }
    result
}

/// How long a read waits for data between looks at the cancel flag.
const CANCEL_TICK: Duration = Duration::from_millis(20);

/// `cat-file`'s stdout, read until it ends or `cancel` is set and nothing more
/// is waiting. A cancelled read still returns what is already buffered, so data
/// a finished git wrote is kept; once the pipe is quiet it is an error, never a
/// short success, even while a descendant of git holds the write end open.
struct Cancellable<'a> {
    inner: ChildStdout,
    cancel: &'a AtomicBool,
}

impl Read for Cancellable<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.inner.readable_within(CANCEL_TICK)? {
                return self.inner.read(buf);
            }
            if self.cancel.load(Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the git cat-file run ended with its output still open",
                ));
            }
        }
    }
}

/// Read the tree and its blobs into `into`. `bound` is the whole budget,
/// counted from `started` (which the rev resolution already spent from);
/// messages name `bound`, as configured.
fn extract(
    target: &Path,
    rev: &str,
    id: &str,
    into: &Path,
    bound: Duration,
    started: Instant,
) -> Result<(), QueryError> {
    let describe = |program: &str, error: &dyn std::fmt::Display| {
        QueryError::new(format!(
            "{program} '{rev}' in {}: {error}",
            target.display()
        ))
    };
    let timed_out =
        |program: &str| describe(program, &format!("git exceeded {} s", seconds(bound)));

    let mut ls_tree = target_git(target);
    ls_tree.args(["ls-tree", "-r", "-z", id]);
    let remaining = bound.saturating_sub(started.elapsed());
    let listing = spawn_bounded(ls_tree, remaining.max(Duration::from_millis(1)))
        .map_err(|e| QueryError::new(format!("could not run git: {e}")))?
        .wait_output()
        .map_err(|e| match e {
            BoundedError::TimedOut { .. } => timed_out("git ls-tree"),
            other => describe("git ls-tree", &other),
        })?;
    if !listing.status.success() {
        let detail = String::from_utf8_lossy(&listing.stderr);
        return Err(describe("git ls-tree", &detail.trim()));
    }
    let entries = parse_tree(&listing.stdout, rev)?;

    let remaining = bound.saturating_sub(started.elapsed());
    let mut cat_file = target_git(target);
    cat_file.args(["cat-file", "--batch"]).stdin(Stdio::piped());
    let mut child = spawn_bounded(cat_file, remaining.max(Duration::from_millis(1)))
        .map_err(|e| QueryError::new(format!("could not run git: {e}")))?;
    let stdin = child.take_stdin().expect("stdin was piped");
    let stdout = child.take_stdout().expect("stdout was piped");
    let request: String = entries
        .iter()
        .filter(|entry| entry.kind != Kind::Gitlink)
        .map(|entry| format!("{}\n", entry.oid))
        .collect();
    // Written off-thread so a git that never reads cannot block us on a full
    // pipe before the bound is polled. Not joined: the thread ends when the
    // pipe closes, which the kill at the deadline or git's exit does.
    std::thread::spawn(move || {
        let mut stdin = stdin;
        stdin.write_all(request.as_bytes())
    });

    // The reader runs beside `wait_output`, which holds the deadline: expiry
    // kills git, the pipe closes, and the reader ends.
    let cancel = AtomicBool::new(false);
    let (waited, written) = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            write_tree(
                Cancellable {
                    inner: stdout,
                    cancel: &cancel,
                },
                &entries,
                into,
            )
        });
        let waited = child.wait_output();
        // A process git started can hold the pipe open after git is gone, so
        // the reader is told to stop rather than left to wait for its EOF.
        cancel.store(true, Ordering::Release);
        (waited, reader.join())
    });
    let output = waited.map_err(|e| match e {
        BoundedError::TimedOut { .. } => timed_out("git cat-file"),
        other => describe("git cat-file", &other),
    })?;
    let written = written.map_err(|_| QueryError::new("materialization thread panicked"))?;

    // A git that exited with a code of its own is the cause of a short read;
    // a signal (SIGPIPE after the reader stopped) is the effect.
    if output.status.code().is_some_and(|code| code != 0) {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(describe("git cat-file", &detail.trim()));
    }
    if let Err((path, reason)) = written {
        return Err(refusal(rev, &path, &reason));
    }
    if !output.status.success() {
        return Err(describe("git cat-file", &output.status));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod materialize_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use super::{
        MaterializationDir, check_path, create_exclusive, materialize_revision, parse_tree,
        resolve_revision, version_has_end_of_options,
    };

    fn scratch_parent() -> std::path::PathBuf {
        let parent = MaterializationDir::new().unwrap();
        let path = parent.path().join("parent");
        std::fs::create_dir(&path).unwrap();
        // Leak the guard's removal: the caller removes `path` with its parent.
        std::mem::forget(parent);
        path
    }

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .env_remove("GIT_DIR")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    // Requirement: query/Diff materialization is bounded
    #[test]
    fn the_directory_is_exclusive_private_and_owned() {
        let parent = scratch_parent();
        let taken = parent.join("lattice-diff-taken");
        std::fs::create_dir(&taken).unwrap();
        std::fs::write(taken.join("marker"), "mine").unwrap();

        // The first name collides with the existing directory: it is skipped, not adopted.
        let mut names = ["lattice-diff-taken", "lattice-diff-fresh"].into_iter();
        let path = create_exclusive(&parent, || names.next().unwrap().to_owned()).unwrap();
        assert_eq!(path, parent.join("lattice-diff-fresh"));
        assert_eq!(mode(&path), 0o700);
        assert_eq!(
            std::fs::read_to_string(taken.join("marker")).unwrap(),
            "mine"
        );

        // `new()` names differ and are private.
        let first = MaterializationDir::new().unwrap();
        let second = MaterializationDir::new().unwrap();
        assert_ne!(first.path(), second.path());
        assert_eq!(mode(first.path()), 0o700);
        let name = first.path().file_name().unwrap().to_string_lossy();
        let suffix = name
            .strip_prefix(&format!("lattice-diff-{}-", std::process::id()))
            .unwrap();
        assert_eq!(suffix.len(), 32, "{name}");
        let (first_path, second_path) = (first.path().to_owned(), second.path().to_owned());

        // Dropping removes its own directory and nothing beside it.
        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.exists());
        assert!(taken.join("marker").exists());
        drop(second);

        // A path swapped for a symlink is not removed through.
        let victim = parent.join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("keep"), "x").unwrap();
        let guard = MaterializationDir::new_in(&parent).unwrap();
        let guarded = guard.path().to_owned();
        std::fs::remove_dir(&guarded).unwrap();
        std::os::unix::fs::symlink(&victim, &guarded).unwrap();
        drop(guard);
        assert!(victim.join("keep").exists());
        std::fs::remove_dir_all(parent.parent().unwrap()).unwrap();
    }

    // Requirement: query/Diff materialization is bounded
    #[test]
    fn a_second_materialization_keeps_the_directory_and_its_mode() {
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        run_git(&repo, &["commit", "-q", "-m", "a"]);
        std::fs::remove_file(repo.join("a.txt")).unwrap();
        std::fs::write(repo.join("b.txt"), "two").unwrap();
        run_git(&repo, &["add", "-A"]);
        run_git(&repo, &["commit", "-q", "-m", "b"]);

        let into = MaterializationDir::new().unwrap();
        let path = into.path().to_owned();
        let bound = Duration::from_secs(30);
        materialize_revision(&repo, "HEAD~", &into, bound).unwrap();
        assert!(path.join("a.txt").exists());
        materialize_revision(&repo, "HEAD", &into, bound).unwrap();
        assert_eq!(into.path(), path);
        assert!(!path.join("a.txt").exists());
        assert!(path.join("b.txt").exists());
        assert_eq!(mode(&path), 0o700);
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    // Requirement: query/Diff materialization is bounded
    #[test]
    fn a_ref_git_cannot_read_is_bounded_by_the_timeout() {
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        run_git(&repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
        let fifo = repo.join(".git").join("refs").join("heads").join("stuck");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let started = std::time::Instant::now();
        let error = resolve_revision(
            &repo,
            "stuck",
            true,
            Duration::from_secs(1),
            std::time::Instant::now(),
        )
        .unwrap_err();
        assert!(
            error.message().contains("exceeded 1 s"),
            "{}",
            error.message()
        );
        assert!(started.elapsed() < Duration::from_secs(20));
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    // Requirement: query/Diff materialization is bounded
    #[test]
    fn a_directory_renamed_and_replaced_is_neither_cleared_nor_removed() {
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        run_git(&repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
        let parent = repo.parent().unwrap().join("tmp");
        std::fs::create_dir(&parent).unwrap();
        let guard = MaterializationDir::new_in(&parent).unwrap();
        let guarded = guard.path().to_owned();
        std::fs::rename(&guarded, parent.join("moved")).unwrap();
        std::fs::create_dir(&guarded).unwrap();
        std::fs::write(guarded.join("keep"), "theirs").unwrap();

        let error =
            materialize_revision(&repo, "HEAD", &guard, Duration::from_secs(30)).unwrap_err();
        assert!(
            error.message().contains("no longer the directory"),
            "{}",
            error.message()
        );
        assert!(guarded.join("keep").exists());
        drop(guard);
        assert!(guarded.join("keep").exists());
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    // Requirement: query/Diff materialization is bounded
    #[test]
    fn a_revision_resolves_to_an_object_id_with_or_without_end_of_options() {
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        run_git(&repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
        let bound = Duration::from_secs(30);
        for end_of_options in [true, false] {
            let id = resolve_revision(
                &repo,
                "HEAD",
                end_of_options,
                bound,
                std::time::Instant::now(),
            )
            .unwrap();
            assert_eq!(id.len(), 40, "{id}");
            assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
            let missing = resolve_revision(
                &repo,
                "nonesuch",
                end_of_options,
                bound,
                std::time::Instant::now(),
            )
            .unwrap_err();
            assert!(
                missing
                    .message()
                    .contains("'nonesuch' is not a revision in"),
                "{}",
                missing.message()
            );
        }
        for bad in ["", "-x", "--output=f", "a\0b"] {
            let error =
                resolve_revision(&repo, bad, true, bound, std::time::Instant::now()).unwrap_err();
            assert!(error.message().contains("is not a revision"), "{bad:?}");
        }
        assert!(version_has_end_of_options("git version 2.43.0"));
        assert!(version_has_end_of_options("git version 2.24.0"));
        assert!(!version_has_end_of_options("git version 2.23.1"));
        assert!(!version_has_end_of_options("git version 1.9.0"));
        assert!(version_has_end_of_options("something else"));
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    // Requirement: query/Diff materialization is faithful
    #[test]
    fn modes_symlinks_and_gitlinks_are_materialized() {
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        std::fs::create_dir(repo.join("dir")).unwrap();
        std::fs::write(repo.join("dir").join("plain.txt"), "plain\n").unwrap();
        std::fs::write(repo.join("run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(repo.join("run.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::os::unix::fs::symlink("../x", repo.join("dir").join("link")).unwrap();
        run_git(&repo, &["add", "dir", "run.sh"]);
        run_git(&repo, &["commit", "-q", "-m", "a"]);
        let head = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let head = String::from_utf8(head.stdout).unwrap();
        let cacheinfo = format!("160000,{},subm", head.trim());
        run_git(&repo, &["update-index", "--add", "--cacheinfo", &cacheinfo]);
        run_git(&repo, &["commit", "-q", "-m", "b"]);

        let into = MaterializationDir::new().unwrap();
        materialize_revision(&repo, "HEAD", &into, Duration::from_secs(30)).unwrap();
        let root = into.path();
        assert_eq!(mode(&root.join("run.sh")), 0o755);
        assert_eq!(mode(&root.join("dir").join("plain.txt")), 0o644);
        assert_eq!(
            std::fs::read_to_string(root.join("dir").join("plain.txt")).unwrap(),
            "plain\n"
        );
        assert_eq!(
            std::fs::read_link(root.join("dir").join("link")).unwrap(),
            std::path::Path::new("../x")
        );
        let subm = root.join("subm");
        assert!(subm.is_dir());
        assert_eq!(std::fs::read_dir(&subm).unwrap().count(), 0);
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

    // Requirement: query/Diff materialization is faithful
    #[test]
    fn a_path_that_escapes_is_refused() {
        for bad in [
            &b".."[..],
            b"a/../b",
            b".git/config",
            b"x/.git/config",
            b".GIT/x",
            b"/abs",
            b"",
            b"a//b",
            b"./a",
        ] {
            assert!(
                check_path(bad).is_err(),
                "{} was accepted",
                String::from_utf8_lossy(bad)
            );
        }
        for good in [&b"a"[..], b"a/b.txt", b".gitattributes", b"dir/.github/x"] {
            assert!(
                check_path(good).is_ok(),
                "{}",
                String::from_utf8_lossy(good)
            );
        }
        // A tree holding one refused entry is refused whole, naming the first and counting the rest.
        let listing = b"100644 blob 0000000000000000000000000000000000000000\tok.txt\0\
                        100644 blob 0000000000000000000000000000000000000000\t../up\0\
                        100644 blob 0000000000000000000000000000000000000000\t.git/hooks\0";
        let error = parse_tree(listing, "HEAD").unwrap_err();
        assert_eq!(
            error.message(),
            "cannot materialize '../up' at 'HEAD': '..' path component (and 1 more refused)"
        );
        let mode = b"100664 blob 0000000000000000000000000000000000000000\ta\0";
        assert!(
            parse_tree(mode, "HEAD")
                .unwrap_err()
                .message()
                .contains("unsupported mode 100664")
        );
    }

    // Requirement: query/Diff materialization is bounded
    #[test]
    fn a_blob_that_cannot_be_written_is_an_error_and_leaves_no_tree() {
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        run_git(&repo, &["commit", "-q", "-m", "a"]);

        let into = MaterializationDir::new().unwrap();
        std::fs::set_permissions(into.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        // Root (CI containers) ignores the mode, so nothing would fail to write.
        if std::fs::write(into.path().join("probe"), "").is_ok() {
            std::fs::set_permissions(into.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            eprintln!("skipped: directory permissions are not enforced for this user");
            return;
        }
        let result = materialize_revision(&repo, "HEAD", &into, Duration::from_secs(30));
        std::fs::set_permissions(into.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();
        assert!(
            error
                .message()
                .starts_with("cannot materialize 'a.txt' at 'HEAD': "),
            "{}",
            error.message()
        );
        assert_eq!(std::fs::read_dir(into.path()).unwrap().count(), 0);
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }

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
        // A repository of its own: `HEAD` must resolve before the clearing is reached.
        let repo = scratch_parent();
        run_git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        run_git(&repo, &["commit", "-q", "-m", "a"]);
        let result = materialize_revision(&repo, "HEAD", &into, Duration::from_secs(30));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.unwrap_err();
        assert!(
            error.message().contains("could not clear"),
            "{}",
            error.message()
        );
        std::fs::remove_dir_all(repo.parent().unwrap()).unwrap();
    }
}

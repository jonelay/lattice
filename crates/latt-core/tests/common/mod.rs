//! Fixtures shared by the library's integration suites.
//!
//! Each suite compiles its own copy of this module, so anything a given suite
//! does not call reads as dead there. The allow is about that, not about
//! anything here being unused. Suites that run the `latt` binary live in
//! `crates/latt/tests`, the package that builds it.
#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

pub mod validation;

use latt_core::document::ingest_document;

use latt_core::graph::LatticeGraph;
use latt_core::profile::{Profile, ProfileError, load_profile};

/// A profile with one node kind and one edge kind, enough to reach validation.
///
/// Written to end in a newline so a test can append its own top-level keys.
pub const MINIMAL_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds:
  derives:
    allowed: [[req, req]]
"#;

/// Load a profile from YAML text, via a file, because `load_profile` takes a path.
pub fn profile_from(yaml: &str) -> Result<Profile, ProfileError> {
    sweep_dead_runs();
    let dir = std::env::temp_dir().join("lattice_core_tests");
    std::fs::create_dir_all(&dir).unwrap();
    // A fresh path per call. Tests run on threads, and naming the file after its
    // contents let two tests using the same profile share one path, where one
    // can read the file while the other is still truncating it.
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = dir.join(format!(
        "p{}-{}.yaml",
        run_id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, yaml).unwrap();
    load_profile(&path)
}

/// Write multiple named files into one fresh temp dir and load the first one.
///
/// Each entry is `(filename, yaml_content)`. The first file is the one
/// `load_profile` opens. The others exist so `extends:` can find them.
pub fn profile_from_files(files: &[(&str, &str)]) -> Result<Profile, ProfileError> {
    sweep_dead_runs();
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lattice_core_tests/multi-{}-{}",
        run_id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
    }
    load_profile(&dir.join(files[0].0))
}

/// Ingest a well-formed interface document, panicking if it is not one.
pub fn ingest(document: serde_json::Value) -> LatticeGraph {
    ingest_document(document).expect("document is well-formed")
}

/// The PID namespace this process runs in, as the inode `/proc/self/ns/pid`
/// names; empty where there is none to read.
fn pid_namespace() -> String {
    std::fs::read_link("/proc/self/ns/pid")
        .ok()
        .and_then(|link| {
            let link = link.to_string_lossy().into_owned();
            let (start, end) = (link.find('[')? + 1, link.rfind(']')?);
            Some(link[start..end].to_string())
        })
        .unwrap_or_default()
}

/// What a scratch name carries so a sweeper can tell whether its run is over:
/// `<pid>` alone, or `<pid>n<namespace>` where the namespace is readable.
///
/// A PID is meaningful only inside its namespace: a sandboxed run sharing this
/// temp dir cannot see our `/proc/<pid>`, and would read a live run as dead.
fn run_id() -> String {
    let namespace = pid_namespace();
    if namespace.is_empty() {
        std::process::id().to_string()
    } else {
        format!("{}n{namespace}", std::process::id())
    }
}

/// Remove the scratch files of test processes that have exited, once per process.
///
/// Scratch paths carry the creating process's ID and are not removed when a
/// test ends: a guard dropped at the end of a test would also be dropped where
/// a test passes `scratch()` by value, before the command using it has run,
/// and a failing test's files are the evidence. Sweeping by liveness keeps the
/// files of a run still in progress, in this suite or a concurrent one. A run
/// from another PID namespace is never judged: its liveness is not readable
/// here, and deleting its inputs mid-run was a reproduced failure.
fn sweep_dead_runs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if !cfg!(target_os = "linux") {
            return;
        }
        let namespace = pid_namespace();
        let dead = |name: &str, prefix: &str| {
            let Some(id) = name
                .strip_prefix(prefix)
                .and_then(|rest| rest.split(['_', '-']).next())
            else {
                return false;
            };
            let (pid, ns) = id.split_once('n').unwrap_or((id, ""));
            ns == namespace
                && pid
                    .parse::<u32>()
                    .is_ok_and(|pid| !Path::new("/proc").join(pid.to_string()).exists())
        };
        let temp = std::env::temp_dir();
        let shared = temp.join("lattice_core_tests");
        for (dir, prefixes) in [
            (temp.as_path(), &["lattice_cli_"][..]),
            (shared.as_path(), &["p", "multi-"][..]),
        ] {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if prefixes.iter().any(|prefix| dead(&name, prefix)) {
                    let path = entry.path();
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
    });
}

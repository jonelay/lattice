//! Fixtures shared by the integration suites.
//!
//! Each suite compiles its own copy of this module, so anything a given suite
//! does not call reads as dead there. The allow is about that, not about
//! anything here being unused.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use lattice_core::document::ingest_document;
use lattice_core::graph::LatticeGraph;
use lattice_core::profile::{Profile, ProfileError, load_profile};

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
    let dir = std::env::temp_dir().join("lattice_core_tests");
    std::fs::create_dir_all(&dir).unwrap();
    // A fresh path per call. Tests run on threads, and naming the file after its
    // contents let two tests using the same profile share one path — where one
    // can read the file while the other is still truncating it.
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = dir.join(format!(
        "p{}-{}.yaml",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, yaml).unwrap();
    load_profile(&path)
}

/// Write multiple named files into one fresh temp dir and load the first one.
///
/// Each entry is `(filename, yaml_content)`. The first file is the one
/// `load_profile` opens — the others exist so `extends:` can find them.
pub fn profile_from_files(files: &[(&str, &str)]) -> Result<Profile, ProfileError> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lattice_core_tests/multi-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
    }
    load_profile(&dir.join(files[0].0))
}

/// Ingest a well-formed contract document, panicking if it is not one.
pub fn ingest(document: serde_json::Value) -> LatticeGraph {
    ingest_document(&document).expect("document is well-formed")
}

pub fn binary() -> PathBuf {
    // The test binary sits in target/<profile>/deps; the CLI is two levels up.
    let mut path = std::env::current_exe().expect("the test binary has a path");
    path.pop();
    path.pop();
    path.join("lattice")
}

/// A scratch directory unique to this call, so parallel tests never share a path.
pub fn scratch() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lattice_cli_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One case's fixtures: a profile, and an adapter program behaving as described.
pub struct Case {
    pub dir: PathBuf,
    pub profile: PathBuf,
    pub adapter: PathBuf,
}

impl Case {
    /// An adapter that writes `stdout_text` and exits 0.
    pub fn emitting(stdout_text: &str) -> Self {
        Self::running(&format!("cat <<'DOC'\n{stdout_text}\nDOC"))
    }

    /// An adapter whose body is the given shell, run with `set -e` off so the
    /// case controls the exit status itself.
    pub fn running(body: &str) -> Self {
        Self::with_profile(MINIMAL_PROFILE, body)
    }

    pub fn with_profile(profile_yaml: &str, body: &str) -> Self {
        let dir = scratch();
        let profile = dir.join("profile.yaml");
        std::fs::write(&profile, profile_yaml).unwrap();

        let adapter = dir.join("adapter");
        std::fs::write(&adapter, format!("#!/bin/sh\n{body}\n")).unwrap();
        make_executable(&adapter);

        Self {
            dir,
            profile,
            adapter,
        }
    }

    pub fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(binary());
        command.args(args);
        command
            .arg("--profile")
            .arg(&self.profile)
            .arg("--adapter")
            .arg(&self.adapter)
            .arg("--target")
            .arg(&self.dir);
        command.output().expect("the lattice binary runs")
    }
}

pub fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

pub fn code(output: &Output) -> i32 {
    output.status.code().expect("the process was not signalled")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

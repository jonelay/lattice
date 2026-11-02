//! External lookup and scratch-profile guarantees, with and without native features.
mod common;

use common::{Case, MINIMAL_PROFILE, code, scratch, stderr, stdout};
use latt_core::document::{AdapterSpec, resolve_adapter};

const DOCUMENT: &str = r#"{"interface_version":"1.2","nodes":[{"id":"REQ-1","kind":"req","attrs":{},"provenance":{"file":"r.md","line":1}}]}"#;

fn emitting_case() -> Case {
    Case::running(&format!("printf '%s' '{DOCUMENT}'"))
}

#[test]
fn external_adapter_found_only_on_path_runs() {
    let case = emitting_case();
    let output = common::lattice()
        .args([
            "query",
            "counts",
            "--format",
            "json",
            "--adapter",
            "adapter",
        ])
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .current_dir(scratch())
        .env("PATH", &case.dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["nodes"]["req"], 1);
}

#[test]
fn missing_external_program_reports_spawn_failure_and_native_availability() {
    let case = emitting_case();
    let output = common::lattice()
        .args(["validate", "--adapter", "absent-adapter"])
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .env("PATH", scratch())
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    let message = stderr(&output);
    assert!(
        message.contains("could not run adapter 'absent-adapter'"),
        "{message}"
    );
    #[cfg(feature = "native-adapters")]
    assert!(
        message.contains("available native adapters: md, toml"),
        "{message}"
    );
    #[cfg(not(feature = "native-adapters"))]
    assert!(
        message.contains("feature 'native-adapters' is disabled"),
        "{message}"
    );
}

#[test]
fn profile_native_is_never_resolved_as_an_external_program() {
    let profile =
        common::profile_from(&format!("{MINIMAL_PROFILE}adapter:\n  native: adapter\n")).unwrap();
    let message = resolve_adapter(None, &profile)
        .expect_err("unknown native fails")
        .to_string();
    assert!(message.contains("profile declares adapter.native: 'adapter'"));
    #[cfg(not(feature = "native-adapters"))]
    assert!(
        message.contains("feature 'native-adapters' is disabled"),
        "{message}"
    );
    assert!(matches!(
        resolve_adapter(Some("adapter".as_ref()), &profile),
        Ok(AdapterSpec::External(_))
    ));
}

#[test]
fn native_name_resolution_matches_compiled_features() {
    let profile =
        common::profile_from(&format!("{MINIMAL_PROFILE}adapter:\n  native: md\n")).unwrap();
    let explicit = resolve_adapter(Some("md".as_ref()), &profile).unwrap();
    #[cfg(feature = "native-adapters")]
    {
        assert!(matches!(explicit, AdapterSpec::Native(_)));
        assert!(matches!(
            resolve_adapter(None, &profile),
            Ok(AdapterSpec::Native(_))
        ));
    }
    #[cfg(not(feature = "native-adapters"))]
    {
        assert!(matches!(explicit, AdapterSpec::External(_)));
        let message = resolve_adapter(None, &profile).unwrap_err().to_string();
        assert!(
            message.contains("feature 'native-adapters' is disabled"),
            "{message}"
        );
    }
}

#[cfg(unix)]
#[test]
fn external_adapter_non_utf8_path_is_preserved() {
    use std::os::unix::ffi::OsStringExt;
    let case = emitting_case();
    let program = case
        .dir
        .join(std::ffi::OsString::from_vec(b"adapter-\xff".to_vec()));
    std::fs::rename(&case.adapter, &program).unwrap();
    let output = common::lattice()
        .args(["query", "counts", "--format", "json"])
        .arg("--adapter")
        .arg(program)
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["nodes"]["req"], 1);
}

#[test]
fn external_scratch_is_removed_after_success_and_execution_errors() {
    for (body, expected) in [
        (format!("printf '%s' '{DOCUMENT}'"), 0),
        ("exit 7".to_owned(), 2),
        ("printf 'invalid json'".to_owned(), 2),
    ] {
        let case = Case::running(&format!("printf '%s' \"$2\" > \"$4/scratch-path\"\n{body}"));
        let temp = scratch();
        let output = common::lattice()
            .args(["query", "counts"])
            .arg("--adapter")
            .arg(&case.adapter)
            .arg("--profile")
            .arg(&case.profile)
            .arg("--target")
            .arg(&case.dir)
            .env("TMPDIR", &temp)
            .output()
            .unwrap();
        assert_eq!(code(&output), expected, "{}", stderr(&output));
        let path = std::fs::read_to_string(case.dir.join("scratch-path")).unwrap();
        assert!(!std::path::Path::new(&path).exists(), "leaked {path}");
        assert_eq!(std::fs::read_dir(temp).unwrap().count(), 0);
    }
}

#[test]
fn external_profile_error_names_the_users_profile() {
    // The adapter read a scratch copy the user never wrote, and it is gone
    // by the time the message is read.
    let case = Case::running("echo \"could not load profile $2: unknown field\" >&2\nexit 1");
    let output = case.run(&["validate"]);
    assert_eq!(code(&output), 2);
    let err = stderr(&output);
    assert!(
        err.contains(&format!(
            "could not load profile {}: unknown field",
            case.profile.display()
        )),
        "{err}"
    );
    assert!(!err.contains("lattice-profile-"), "{err}");
}

#[test]
fn external_scratch_is_removed_after_spawn_failure() {
    let case = emitting_case();
    let temp = scratch();
    let output = common::lattice()
        .args(["query", "counts"])
        .arg("--adapter")
        .arg(case.dir.join("missing"))
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .env("TMPDIR", &temp)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert_eq!(std::fs::read_dir(temp).unwrap().count(), 0);
}

#[test]
fn bare_adapter_name_falls_back_to_the_latt_adapter_prefixed_program_on_path() {
    // A release install has `latt-adapter-foo` on PATH beside `latt`; the user
    // types `--adapter foo`, the spelling the shim and the docs use.
    let case = emitting_case();
    std::fs::rename(&case.adapter, case.dir.join("latt-adapter-foo")).unwrap();
    let output = common::lattice()
        .args(["query", "counts", "--adapter", "foo", "--format", "json"])
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .env("PATH", &case.dir)
        .current_dir(scratch())
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["nodes"]["req"], 1);
}

#[test]
fn a_path_never_falls_back_to_the_latt_adapter_prefixed_program() {
    // `./foo` is a path the user chose; it is not retried as `./latt-adapter-foo`.
    let case = emitting_case();
    std::fs::rename(&case.adapter, case.dir.join("latt-adapter-foo")).unwrap();
    let output = common::lattice()
        .args(["query", "counts", "--adapter", "./foo"])
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .current_dir(&case.dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    let message = stderr(&output);
    assert!(
        message.contains("could not run adapter './foo'"),
        "{message}"
    );
    assert!(!message.contains("latt-adapter-foo"), "{message}");
}

#[test]
fn missing_bare_adapter_names_both_spellings_tried() {
    let case = emitting_case();
    let temp = scratch();
    let output = common::lattice()
        .args(["validate", "--adapter", "absent"])
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .env("PATH", scratch())
        .env("TMPDIR", &temp)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    let message = stderr(&output);
    assert!(
        message.contains("could not run adapter 'absent'"),
        "{message}"
    );
    assert!(message.contains("'latt-adapter-absent'"), "{message}");
    assert_eq!(std::fs::read_dir(temp).unwrap().count(), 0);
}

#[cfg(not(feature = "native-adapters"))]
#[test]
fn disabled_native_name_can_still_run_an_explicit_external_program_on_path() {
    let case = emitting_case();
    std::fs::rename(&case.adapter, case.dir.join("md")).unwrap();
    let output = common::lattice()
        .args(["query", "counts", "--adapter", "md", "--format", "json"])
        .arg("--profile")
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .env("PATH", &case.dir)
        .current_dir(scratch())
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["nodes"]["req"], 1);
}

#[cfg(not(feature = "native-adapters"))]
#[test]
fn adapter_command_names_the_disabled_feature() {
    let case = emitting_case();
    let output = common::lattice()
        .args(["adapter", "md", "--profile"])
        .arg(&case.profile)
        .arg("--target")
        .arg(&case.dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert!(output.stdout.is_empty(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("feature 'native-adapters' is disabled"),
        "{}",
        stderr(&output)
    );
}

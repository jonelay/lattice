//! Tests for native adapter dispatch: resolution, parity, and CLI integration.
#![cfg(feature = "native-adapters")]

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::{code, scratch, stderr, stdout};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

fn profiles() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("profiles")
}

/// The `adapters/<name>` shim, which runs `latt adapter <name>`: the native adapter
/// as an external program.
fn shim(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../adapters")
        .join(name)
}

/// Point the shim at the `latt` under test: an installed `latt` on PATH would
/// otherwise run in its place.
fn under_test(mut command: Command) -> Command {
    let debug = common::binary().parent().unwrap().to_owned();
    command
        .env("PATH", "/usr/bin:/bin")
        .env("CARGO_TARGET_DIR", debug.parent().unwrap());
    command
}

#[test]
fn native_md_produces_identical_output_to_its_shim() {
    let profile = profiles().join("md.yaml");
    let target = fixtures().join("mini-md");

    let native = common::lattice()
        .args(["trace", "--adapter", "md", "--format", "json"])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();

    let external = under_test(common::lattice())
        .args(["trace", "--format", "json"])
        .arg("--adapter")
        .arg(shim("md"))
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();

    assert_fixture_trace(&native, "md");
    assert_fixture_trace(&external, "md");
    assert_eq!(stdout(&native), stdout(&external));
}

#[test]
fn native_toml_produces_identical_output_to_its_shim() {
    let profile = profiles().join("toml.yaml");
    let target = fixtures().join("mini-toml");

    let native = common::lattice()
        .args(["trace", "--adapter", "toml", "--format", "json"])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();

    let external = under_test(common::lattice())
        .args(["trace", "--format", "json"])
        .arg("--adapter")
        .arg(shim("toml"))
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();

    assert_fixture_trace(&native, "toml");
    assert_fixture_trace(&external, "toml");
    assert_eq!(stdout(&native), stdout(&external));
}

#[test]
fn native_prose_xref_produces_identical_output_to_its_shim() {
    let profile = profiles().join("prose-xref.yaml");
    let target = fixtures().join("mini-prose-xref");

    let native = common::lattice()
        .args(["trace", "--adapter", "prose-xref", "--format", "json"])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();

    let external = under_test(common::lattice())
        .args(["trace", "--format", "json"])
        .arg("--adapter")
        .arg(shim("prose-xref"))
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();

    assert_fixture_trace(&native, "prose-xref");
    assert_fixture_trace(&external, "prose-xref");
    assert_eq!(stdout(&native), stdout(&external));
}

#[test]
fn native_md_validates_a_fixture() {
    let output = common::lattice()
        .args(["validate", "--adapter", "md", "--format", "plain"])
        .arg("--profile")
        .arg(profiles().join("md.yaml"))
        .arg("--target")
        .arg(fixtures().join("mini-md"))
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        1,
        "should not be a config error: {}",
        stderr(&output)
    );
}

#[test]
fn profile_declared_native_adapter_resolves() {
    let dir = scratch();
    let profile = dir.join("profile.yaml");
    std::fs::write(
        &profile,
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds:
  derives:
    allowed: [[req, req]]
adapter:
  native: md
  paths:
    files: ["*.md"]
  table:
    id_column: "ID"
    column_map:
      "Description": summary
    edge_columns:
      "Traces To": derives
"#,
    )
    .unwrap();

    let output = common::lattice()
        .args(["validate", "--format", "plain"])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(fixtures().join("mini-md"))
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        1,
        "should resolve native adapter from profile: {}",
        stderr(&output)
    );
}

#[test]
fn missing_adapter_and_no_profile_native_exits_two() {
    let dir = scratch();
    let profile = dir.join("profile.yaml");
    std::fs::write(
        &profile,
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds: {}
"#,
    )
    .unwrap();

    let output = common::lattice()
        .args(["validate", "--format", "plain"])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    let err = stderr(&output);
    assert!(
        err.contains("no adapter specified"),
        "expected guidance in error: {err}"
    );
}

#[test]
fn unknown_adapter_name_that_is_not_a_path_exits_two() {
    let dir = scratch();
    let profile = dir.join("profile.yaml");
    std::fs::write(
        &profile,
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds: {}
"#,
    )
    .unwrap();

    let output = common::lattice()
        .args([
            "validate",
            "--adapter",
            "nonexistent_adapter_xyz",
            "--format",
            "plain",
        ])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        2,
        "unresolvable adapter should exit 2: {}",
        stderr(&output)
    );
    let err = stderr(&output);
    assert!(
        err.contains("could not run adapter 'nonexistent_adapter_xyz'"),
        "should name the value: {err}"
    );
    assert!(err.contains("md"), "should list available adapters: {err}");
}

/// Nodes and edges each fixture holds; a parity check over zero of either
/// would pass for an adapter that read nothing.
fn fixture_counts(engine: &str) -> (usize, usize) {
    match engine {
        "md" => (3, 3),
        "toml" => (21, 10),
        _ => (26, 7),
    }
}

fn assert_fixture_trace(output: &std::process::Output, engine: &str) {
    // Both fixtures deliberately contain malformed records: findings are exit 1.
    assert_eq!(code(output), 1, "{}", stderr(output));
    let value: serde_json::Value = serde_json::from_str(&stdout(output)).unwrap();
    let entries = value["entries"].as_array().unwrap();
    let (nodes, edges) = fixture_counts(engine);
    assert_eq!(entries.len(), nodes);
    assert_eq!(
        entries
            .iter()
            .map(|n| n["edges"].as_array().unwrap().len())
            .sum::<usize>(),
        edges
    );
    assert!(
        value["unattachable_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "PARSE_ERROR")
    );
    if engine == "prose-xref" {
        assert!(entries.iter().any(|n| {
            n["id"] == "characters/alice.md#Alice::Relationships"
                && n["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["tgt"] == "outline/structure.md#Story Structure::Act I")
        }));
    } else if engine == "md" {
        assert_eq!(
            entries.iter().find(|n| n["id"] == "REQ-2").unwrap()["attrs"]["summary"],
            "Preserve source provenance"
        );
    } else {
        assert_eq!(value["pathways"][0]["current"], "s2");
        assert!(entries.iter().any(|n| {
            n["id"] == "alpha/one"
                && n["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["tgt"] == "beta/two")
        }));
    }
}

#[test]
fn native_interface_documents_match_shim_values() {
    use latt_core::profile::{load_profile, resolved_document};
    for engine in ["md", "toml", "prose-xref"] {
        let dir = scratch();
        let profile = load_profile(&profiles().join(format!("{engine}.yaml"))).unwrap();
        let resolved_path = dir.join("resolved.json");
        std::fs::write(&resolved_path, resolved_document(&profile).unwrap()).unwrap();
        let target = fixtures().join(format!("mini-{engine}"));
        let native = match engine {
            "md" => latt_core::adapters::md::build_document_value(&resolved_path, &target),
            "toml" => latt_core::adapters::toml::build_document_value(&resolved_path, &target),
            _ => latt_core::adapters::prose_xref::build_document_value(&resolved_path, &target),
        }
        .unwrap();
        let output = under_test(Command::new(shim(engine)))
            .arg("--profile")
            .arg(resolved_path)
            .arg("--target")
            .arg(target)
            .output()
            .unwrap();
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        let external: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(native, external);
        let (nodes, edges) = fixture_counts(engine);
        // The prose-xref fixture repeats a heading; the document declares it
        // twice and the core, not the adapter, resolves the repeat.
        let nodes = nodes + usize::from(engine == "prose-xref");
        assert_eq!(native["nodes"].as_array().unwrap().len(), nodes);
        assert_eq!(native["edges"].as_array().unwrap().len(), edges);
        assert!(!native["findings"].as_array().unwrap().is_empty());
    }
}

/// Each native adapter writes its document's struct fields in declared order and its
/// maps sorted, so the bytes do not depend on whether a host enables
/// `serde_json/preserve_order`; `verify.sh` runs this suite in both builds against one
/// committed copy. Regenerate with `LATTICE_REGEN_BASELINES=1`; the diff is the review.
#[test]
fn native_documents_are_the_same_bytes_in_every_build() {
    use latt_core::profile::{load_profile, resolved_document};
    let baselines =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-documents");
    let regen = std::env::var_os("LATTICE_REGEN_BASELINES").is_some();
    for engine in ["md", "toml", "prose-xref"] {
        let dir = scratch();
        let profile = load_profile(&profiles().join(format!("{engine}.yaml"))).unwrap();
        let resolved_path = dir.join("resolved.json");
        std::fs::write(&resolved_path, resolved_document(&profile).unwrap()).unwrap();
        let target = fixtures().join(format!("mini-{engine}"));
        let mut written = Vec::new();
        latt_core::document::write_native_document(engine, &resolved_path, &target, &mut written)
            .unwrap();
        let text = String::from_utf8(written).unwrap();
        assert!(
            !text.contains(&*dir.to_string_lossy()) && !text.contains(&*target.to_string_lossy()),
            "{engine}: the document names a machine-local path"
        );
        let baseline = baselines.join(format!("{engine}.json"));
        if regen {
            std::fs::create_dir_all(&baselines).unwrap();
            std::fs::write(&baseline, &text).unwrap();
        }
        assert_eq!(
            text,
            std::fs::read_to_string(&baseline).unwrap(),
            "{engine}: document bytes differ from {}; regenerate with LATTICE_REGEN_BASELINES=1 and review the diff",
            baseline.display()
        );
    }
}

#[test]
fn native_configuration_errors_are_exit_two_and_cleanup_scratch() {
    for engine in ["md", "toml", "prose-xref"] {
        let dir = scratch();
        let temp = scratch();
        let profile = dir.join("profile.yaml");
        // Valid core profile, but missing required adapter configuration.
        std::fs::write(
            &profile,
            format!("{}adapter:\n  native: {engine}\n", common::MINIMAL_PROFILE),
        )
        .unwrap();
        let output = common::lattice()
            .args(["validate", "--format", "json"])
            .arg("--profile")
            .arg(profile)
            .arg("--target")
            .arg(dir)
            .env("TMPDIR", &temp)
            .output()
            .unwrap();
        assert_eq!(code(&output), 2);
        let message = stderr(&output);
        assert!(
            message.contains(&format!("for adapter {engine}")),
            "{message}"
        );
        assert!(stdout(&output).is_empty());
        assert_eq!(std::fs::read_dir(temp).unwrap().count(), 0);
    }
}

#[test]
fn native_profile_error_names_the_users_profile_not_the_scratch_file() {
    // The adapter reads a scratch copy of the resolved profile; the person
    // reading the error edited `profile.yaml`, so that is the path it names,
    // keeping serde's field and position.
    let dir = scratch();
    let profile = dir.join("profile.yaml");
    std::fs::write(
        &profile,
        format!(
            "{}adapter:\n  native: md\n  paths:\n    files: ['*.md']\n  table:\n    column_map: {{}}\n",
            common::MINIMAL_PROFILE
        ),
    )
    .unwrap();
    let output = common::lattice()
        .args(["validate"])
        .arg("--profile")
        .arg(&profile)
        .arg("--target")
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    let message = stderr(&output);
    assert!(
        message.contains(&format!(
            "could not load profile {} for adapter md: missing field `id_column`",
            profile.display()
        )),
        "{message}"
    );
    assert!(!message.contains("lattice-profile-"), "{message}");
}

#[test]
fn native_unreadable_target_is_a_finding_and_cleanup_scratch() {
    for engine in ["md", "toml", "prose-xref"] {
        let temp = scratch();
        let output = common::lattice()
            .args(["trace", "--adapter", engine, "--format", "json"])
            .arg("--profile")
            .arg(profiles().join(format!("{engine}.yaml")))
            .arg("--target")
            .arg(scratch().join("missing"))
            .env("TMPDIR", &temp)
            .output()
            .unwrap();
        assert_eq!(code(&output), 1, "{}", stderr(&output));
        let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert!(value["entries"].as_array().unwrap().is_empty());
        assert!(
            value["unattachable_findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == "PARSE_ERROR")
        );
        assert_eq!(std::fs::read_dir(temp).unwrap().count(), 0);
    }
}

#[test]
fn native_query_diff_runs_both_revisions_with_explicit_and_profile_selection() {
    let dir = scratch();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        stdout(&out).trim().to_owned()
    };
    git(&["init", "-q"]);
    let rows = "| ID | Description | Status | Traces To |\n| --- | --- | --- | --- |\n| REQ-1 | first | active | |\n";
    std::fs::write(dir.join("register.md"), rows).unwrap();
    git(&["add", "register.md"]);
    git(&["commit", "-q", "-m", "first"]);
    let a = git(&["rev-parse", "HEAD"]);
    std::fs::write(
        dir.join("register.md"),
        format!("{rows}| REQ-2 | second | active | REQ-1 |\n"),
    )
    .unwrap();
    git(&["commit", "-q", "-am", "second"]);
    let b = git(&["rev-parse", "HEAD"]);
    // Uncommitted state must not replace either requested revision.
    std::fs::write(dir.join("register.md"), "uncommitted").unwrap();
    for explicit in [false, true] {
        let mut command = common::lattice();
        command
            .args(["query", "diff", &a, &b, "--format", "json"])
            .arg("--profile")
            .arg(profiles().join("md.yaml"))
            .arg("--target")
            .arg(&dir);
        if explicit {
            command.args(["--adapter", "md"]);
        }
        let output = command.output().unwrap();
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(value["nodes_added"].as_array().unwrap().len(), 1);
        assert_eq!(value["nodes_added"][0]["id"], "REQ-2");
        assert_eq!(value["edges_added"].as_array().unwrap().len(), 1);
        assert!(value["nodes_removed"].as_array().unwrap().is_empty());
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("register.md")).unwrap(),
        "uncommitted"
    );
}

/// `latt show` for `profile`, written to a scratch file: what `latt adapter` reads.
fn resolved_profile(profile: &str) -> PathBuf {
    let output = common::lattice()
        .arg("show")
        .arg("--profile")
        .arg(profiles().join(profile))
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let path = scratch().join("resolved.json");
    std::fs::write(&path, &output.stdout).unwrap();
    path
}

#[test]
fn adapter_command_prints_each_native_document() {
    for (name, profile, fixture) in [
        ("md", "md.yaml", "mini-md"),
        ("toml", "toml.yaml", "mini-toml"),
        ("prose-xref", "prose-xref.yaml", "mini-prose-xref"),
    ] {
        let output = common::lattice()
            .args(["adapter", name, "--profile"])
            .arg(resolved_profile(profile))
            .arg("--target")
            .arg(fixtures().join(fixture))
            .output()
            .unwrap();
        assert_eq!(code(&output), 0, "{name}: {}", stderr(&output));
        let document: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(document["interface_version"], "1.4", "{name}");
        let nodes = document["nodes"].as_array().unwrap();
        assert!(!nodes.is_empty(), "{name}: no nodes read from {fixture}");
    }
}

#[test]
fn adapter_command_refuses_a_yaml_profile() {
    let profile = profiles().join("md.yaml");
    let output = common::lattice()
        .args(["adapter", "md", "--profile"])
        .arg(&profile)
        .arg("--target")
        .arg(fixtures().join("mini-md"))
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert!(output.stdout.is_empty(), "{}", stdout(&output));
    let expected = format!("Error: could not load profile {}: ", profile.display());
    assert!(
        stderr(&output).starts_with(&expected),
        "{}",
        stderr(&output)
    );
}

#[test]
fn adapter_command_lists_native_adapters_for_an_unknown_name() {
    let output = common::lattice()
        .args(["adapter", "yaml", "--profile"])
        .arg(resolved_profile("md.yaml"))
        .arg("--target")
        .arg(fixtures().join("mini-md"))
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert!(output.stdout.is_empty(), "{}", stdout(&output));
    assert_eq!(
        stderr(&output),
        "Error: 'yaml' is not a native adapter; available native adapters: md, toml, prose-xref\n"
    );
}

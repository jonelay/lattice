//! Tests for native adapter dispatch: resolution, parity, and CLI integration.
#![cfg(feature = "native-adapters")]

mod common;

#[path = "common/regen.rs"]
mod regen;

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
    let baselines = std::env::var_os("LATT_TEST_NATIVE_BASELINES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-documents")
        });
    let regen = regen::regen_baselines();
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
fn native_regeneration_path_obeys_environment_matrix() {
    let binary = std::env::current_exe().unwrap();
    for (value, ci, succeeds) in [
        (None, false, false),
        (Some("0"), false, false),
        (Some("false"), false, false),
        (Some("1"), false, true),
        (Some("1"), true, false),
    ] {
        let baselines = scratch();
        for engine in ["md", "toml", "prose-xref"] {
            std::fs::write(baselines.join(format!("{engine}.json")), "sentinel").unwrap();
        }
        let mut command = Command::new(&binary);
        command.args([
            "--exact",
            "native_documents_are_the_same_bytes_in_every_build",
        ]);
        command
            .env_remove("LATTICE_REGEN_BASELINES")
            .env_remove("CI")
            .env("LATT_TEST_NATIVE_BASELINES", &baselines);
        if let Some(value) = value {
            command.env("LATTICE_REGEN_BASELINES", value);
        }
        if ci {
            command.env("CI", "1");
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.success(),
            succeeds,
            "{value:?} CI={ci}: {output:?}"
        );
        for engine in ["md", "toml", "prose-xref"] {
            let content =
                std::fs::read_to_string(baselines.join(format!("{engine}.json"))).unwrap();
            assert_eq!(
                content == "sentinel",
                !succeeds,
                "{value:?} CI={ci}: {engine}"
            );
        }
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

/// A profile for `adapter` whose register under `dir` validates clean, so the only
/// finding a symlinked directory can add is `UNFOLLOWED_LINK`. `validations` is
/// appended to the profile verbatim.
#[cfg(unix)]
fn unreached_link_fixture(adapter: &str, validations: &str) -> (PathBuf, PathBuf, &'static str) {
    let dir = scratch();
    let (paths, kinds, content, link) = match adapter {
        "md" => (
            "    files: [\"registers/*.md\"]\n  table:\n    id_column: \"ID\"\n    column_map:\n      \"Description\": \"summary\"\n",
            "node_kinds:\n  requirement:\n    id_pattern: \"^REQ-[0-9]+$\"\n    orphan_ok: true\n    attrs:\n      summary: { type: string, required: true }\nedge_kinds:\n  traces_to:\n    allowed:\n      - [requirement, requirement]\n",
            (
                "registers/requirements.md",
                "# Requirements\n\n| ID | Description |\n|----|-------------|\n| REQ-1 | One |\n",
            ),
            ".venv/lib64",
        ),
        "prose-xref" => (
            "    files: [\"**/*.md\"]\n    content_dirs: [\"ref\"]\n",
            "node_kinds:\n  document:\n    id_pattern: \"^.+\\\\.md$\"\n    orphan_ok: true\n  section:\n    id_pattern: \"^[^#]+\\\\.md#.+$\"\n    orphan_ok: true\n    attrs:\n      heading: { type: string, required: true }\nedge_kinds:\n  xref:\n    allowed:\n      - [section, section]\n",
            ("ref/notes.md", "# Notes\n"),
            "ref/.venv/lib64",
        ),
        "toml" => (
            "    files: [\"data/*.toml\"]\n  mode: directory\n  tables:\n    record:\n      kind: device\n      id_key: id\n      key_map:\n        name: name\n      edge_keys: {}\n",
            "node_kinds:\n  device:\n    id_pattern: \"^DEV-[0-9]{3}$\"\n    orphan_ok: true\n    attrs:\n      name: { type: string, required: true }\nedge_kinds:\n  references:\n    allowed:\n      - [device, device]\n",
            (
                "data/device-001.toml",
                "id = \"DEV-001\"\nname = \"Sensor A\"\n",
            ),
            ".venv/lib64",
        ),
        other => panic!("no fixture for adapter {other}"),
    };
    let profile = dir.join("profile.yaml");
    std::fs::write(
        &profile,
        format!(
            "name: unreached-link\nprofile_version: \"1.0.0\"\nadapter:\n  native: {adapter}\n  paths:\n{paths}{kinds}{validations}"
        ),
    )
    .unwrap();
    let target = dir.join("target");
    let file = target.join(content.0);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, content.1).unwrap();
    let link_path = target.join(link);
    std::fs::create_dir_all(link_path.parent().unwrap().join("lib")).unwrap();
    std::os::unix::fs::symlink("lib", &link_path).unwrap();
    (profile, target, link)
}

#[cfg(unix)]
fn validate_unreached(profile: &PathBuf, target: &PathBuf, extra: &[&str]) -> std::process::Output {
    common::lattice()
        .arg("validate")
        .args(extra)
        .arg("--profile")
        .arg(profile)
        .arg("--target")
        .arg(target)
        .output()
        .unwrap()
}

// Requirement: adapter-md/Select configured files beneath the target
// Requirement: adapter-prose-xref/Build document and section nodes from content directories
// Requirement: adapter-toml/Select configured TOML files beneath the target
#[cfg(unix)]
#[test]
fn an_unreached_symlinked_directory_does_not_fail_strict() {
    for adapter in ["md", "prose-xref", "toml"] {
        let (profile, target, link) = unreached_link_fixture(adapter, "");
        // A clean run proves nothing over an empty register: the walk must read nodes.
        let summary = common::lattice()
            .args(["summary", "--format", "json", "--profile"])
            .arg(&profile)
            .arg("--target")
            .arg(&target)
            .output()
            .unwrap();
        let counts: serde_json::Value = serde_json::from_str(&stdout(&summary)).unwrap();
        let nodes: u64 = counts["node_counts"]
            .as_object()
            .unwrap()
            .values()
            .map(|count| count.as_u64().unwrap())
            .sum();
        assert!(nodes > 0, "{adapter}: no nodes read: {}", stdout(&summary));
        let clean = validate_unreached(&profile, &target, &[]);
        assert_eq!(code(&clean), 0, "{adapter}: {}", stdout(&clean));
        let strict = validate_unreached(&profile, &target, &["--strict"]);
        assert_eq!(code(&strict), 0, "{adapter}: {}", stdout(&strict));
        let line =
            format!("INFO UNFOLLOWED_LINK {link}:0 symlinked directory '{link}' not followed");
        assert!(
            stdout(&strict).contains(&line),
            "{adapter}: {}",
            stdout(&strict)
        );
        let json = validate_unreached(&profile, &target, &["--strict", "--format", "json"]);
        assert_eq!(code(&json), 0, "{adapter}: {}", stdout(&json));
        let report: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
        let linked: Vec<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["code"] == "UNFOLLOWED_LINK")
            .map(|finding| (finding["severity"].clone(), finding["file"].clone()))
            .collect();
        assert_eq!(
            linked,
            [(serde_json::json!("info"), serde_json::json!(link))],
            "{adapter}"
        );
        std::fs::remove_dir_all(profile.parent().unwrap()).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn a_profile_can_raise_unfollowed_link_back_to_warning() {
    let (profile, target, link) = unreached_link_fixture(
        "md",
        "validations:\n  - UNFOLLOWED_LINK:\n      severity: warning\n",
    );
    let plain = validate_unreached(&profile, &target, &[]);
    assert_eq!(code(&plain), 0, "{}", stdout(&plain));
    let line = format!("WARNING UNFOLLOWED_LINK {link}:0");
    assert!(stdout(&plain).contains(&line), "{}", stdout(&plain));
    let strict = validate_unreached(&profile, &target, &["--strict"]);
    assert_eq!(code(&strict), 1, "{}", stdout(&strict));
    std::fs::remove_dir_all(profile.parent().unwrap()).unwrap();
}

/// Drop the rich format's own SGR sequences (`ESC [ digits ; m`), leaving any
/// other escape in place for the caller to find.
fn without_sgr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('\u{1b}') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let sgr = after.strip_prefix('[').and_then(|body| {
            let end = body.find(|c: char| !(c.is_ascii_digit() || c == ';'))?;
            body[end..].starts_with('m').then_some(&body[end + 1..])
        });
        match sgr {
            Some(tail) => rest = tail,
            None => {
                out.push('\u{1b}');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// Requirement: output/Untrusted text in human formats
#[test]
fn hostile_toml_id_is_escaped_in_plain_rich_and_json() {
    let profile = profiles().join("toml.yaml");
    let target = fixtures().join("hostile-toml");
    let run = |format: &str, color: &str| {
        common::lattice()
            .args(["validate", "--format", format])
            .arg("--profile")
            .arg(&profile)
            .arg("--target")
            .arg(&target)
            .env(color, "1")
            .output()
            .unwrap()
    };
    let json = run("json", "NO_COLOR");
    assert_eq!(code(&json), 1, "{}", stderr(&json));
    let golden =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hostile-toml.validate.json");
    assert_eq!(
        stdout(&json),
        std::fs::read_to_string(golden).unwrap(),
        "json bytes moved"
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    let findings = parsed["findings"].as_array().unwrap().len();
    for format in ["plain", "rich"] {
        for color in ["NO_COLOR", "FORCE_COLOR"] {
            let output = run(format, color);
            assert_eq!(code(&output), 1, "{format} {color}");
            let out = stdout(&output);
            let err = stderr(&output);
            for (stream, text) in [("stdout", &out), ("stderr", &err)] {
                let bare = without_sgr(text);
                if format == "plain" || color == "NO_COLOR" {
                    assert_eq!(
                        &bare, text,
                        "{format} {color} {stream}: SGR in uncoloured output"
                    );
                }
                assert!(
                    !bare
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t'),
                    "{format} {color} {stream}: {bare:?}"
                );
                assert!(!bare.contains('\u{202e}'), "{format} {color} {stream}");
            }
            let bare = without_sgr(&out);
            let lines = bare
                .lines()
                .filter(|line| {
                    matches!(
                        line.split_whitespace().next(),
                        Some("ERROR" | "WARNING" | "INFO" | "HINT")
                    )
                })
                .count();
            assert_eq!(lines, findings, "{format} {color}: {out}");
            assert!(
                bare.contains(r"alpha/bad\u{1b}]8;;http://evil/\u{7}link\u{1b}]8;;\u{7}\nERROR FORGED X.1 forged line\r\u{202e}txt"),
                "{format} {color}: {out}"
            );
        }
    }
}

// Requirement: output/Untrusted text in human formats
// serde_json escapes only C0; `latt adapter` escapes the rest of what a
// terminal acts on, so the document still parses to the same value.
#[test]
fn adapter_document_escapes_c1_and_bidi() {
    let target = scratch();
    let registers = target.join("registers");
    std::fs::create_dir(&registers).unwrap();
    for entry in std::fs::read_dir(fixtures().join("mini-toml/registers")).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, registers.join(path.file_name().unwrap())).unwrap();
    }
    let alpha = registers.join("alpha.toml");
    let mut text = std::fs::read_to_string(&alpha).unwrap();
    text.push_str("\n[[item]]\nid = \"c1\\u009B\\u202E\\u2029\\u007Fz\"\n");
    std::fs::write(&alpha, text).unwrap();
    let output = common::lattice()
        .args(["adapter", "toml", "--profile"])
        .arg(resolved_profile("toml.yaml"))
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let out = stdout(&output);
    assert!(
        !out.chars()
            .any(|c| matches!(c, '\u{7f}'..='\u{9f}' | '\u{202e}' | '\u{2029}')),
        "{out}"
    );
    assert!(out.contains(r"c1\u009b\u202e\u2029\u007fz"), "{out}");
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = "c1\u{9b}\u{202e}\u{2029}\u{7f}z";
    assert!(
        value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"].as_str().is_some_and(|i| i.ends_with(id))),
        "{value}"
    );
    std::fs::remove_dir_all(&target).unwrap();
}

// Requirement: adapter-contract/Adapter output is bounded
#[test]
fn an_adapter_that_never_stops_writing_exits_2() {
    let case = common::Case::running("exec yes");
    let output = case
        .command(&["validate"])
        .env("LATT_ADAPTER_MAX_OUTPUT_BYTES", "1048576")
        .output()
        .unwrap();
    let err = stderr(&output);
    assert_eq!(code(&output), 2, "{err}");
    assert!(err.contains("wrote more than 1 MiB to stdout"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

// Requirement: adapter-contract/Adapter output is bounded
#[test]
fn an_adapter_flooding_stderr_still_reports_its_exit_code() {
    let case = common::Case::running(
        "head -c 5000000 /dev/zero | tr '\\000' x >&2; echo TAILMARK >&2; exit 2",
    );
    let output = case
        .command(&["validate"])
        .env("LATT_ADAPTER_MAX_OUTPUT_BYTES", "1048576")
        .output()
        .unwrap();
    let err = stderr(&output);
    assert_eq!(code(&output), 2, "{err}");
    assert!(err.contains("exited 2"), "{err}");
    assert!(err.trim_end().ends_with("TAILMARK"), "{err}");
}

// Requirement: adapter-contract/Adapter output is bounded
#[test]
fn an_unreadable_output_limit_is_a_setup_error() {
    let case = common::Case::emitting("{}");
    for bad in ["lots", "0", "-5", ""] {
        let output = case
            .command(&["validate"])
            .env("LATT_ADAPTER_MAX_OUTPUT_BYTES", bad)
            .output()
            .unwrap();
        let err = stderr(&output);
        assert_eq!(code(&output), 2, "{bad:?}: {err}");
        assert!(
            err.contains("LATT_ADAPTER_MAX_OUTPUT_BYTES"),
            "{bad:?}: {err}"
        );
    }
}

// Requirement: adapter-contract/Adapter output is bounded
#[test]
fn an_unreadable_output_limit_is_a_setup_error_for_a_native_adapter() {
    for bad in ["lots", "0", ""] {
        let output = common::lattice()
            .args(["validate", "--adapter", "md", "--format", "plain"])
            .arg("--profile")
            .arg(profiles().join("md.yaml"))
            .arg("--target")
            .arg(fixtures().join("mini-md"))
            .env("LATT_ADAPTER_MAX_OUTPUT_BYTES", bad)
            .output()
            .unwrap();
        let err = stderr(&output);
        assert_eq!(code(&output), 2, "{bad:?}: {err}");
        assert!(
            err.contains("LATT_ADAPTER_MAX_OUTPUT_BYTES"),
            "{bad:?}: {err}"
        );
    }
}

// Requirement: adapter-contract/Adapter output is bounded
#[cfg(target_os = "linux")]
#[test]
fn an_address_space_limit_does_not_abort_an_endless_adapter() {
    let probe = Command::new("sh")
        .args(["-c", "ulimit -v 2000000"])
        .status();
    if !probe.is_ok_and(|status| status.success()) {
        eprintln!("skipped: `ulimit -v` is unavailable");
        return;
    }
    let case = common::Case::running("exec yes");
    let inner = case.command(&["validate"]);
    let mut command = Command::new("sh");
    command
        .args(["-c", "ulimit -v 2000000 && exec \"$@\"", "sh"])
        .arg(inner.get_program())
        .args(inner.get_args());
    let output = command.output().unwrap();
    // 134 is SIGABRT from a failed allocation: the default cap must stop the
    // run before the address-space limit does.
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("wrote more than"),
        "{}",
        stderr(&output)
    );
}

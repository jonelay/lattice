//! Frozen output from the pre-service CLI at 29a9e87 (same runtime as a63fc23).
mod common;
use common::{Case, code, stderr, stdout};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Command;

const PROFILE: &str = include_str!("../../latt-core/tests/fixtures/embedding/profile.yaml");
const DOCUMENT: &str = include_str!("../../latt-core/tests/fixtures/embedding/actor-a.json");

fn capture(case: &Case, args: &[&str], expected_exit: Option<i32>) -> Value {
    let out = common::lattice()
        .current_dir(&case.dir)
        .args(args)
        .arg("--profile")
        .arg(&case.profile)
        .arg("--adapter")
        .arg(&case.adapter)
        .arg("--target")
        .arg(&case.dir)
        .output()
        .expect("the lattice binary runs");
    if let Some(exit) = expected_exit {
        assert_eq!(code(&out), exit, "{args:?}: {}", stderr(&out));
    }
    let normalize = |text: String| text.replace(case.dir.to_str().unwrap(), "<TARGET>");
    json!({"exit":code(&out),"stdout":normalize(stdout(&out)),"stderr":normalize(stderr(&out))})
}

fn git(case: &Case, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(&case.dir)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn observations() -> BTreeMap<String, Value> {
    let mut actual = BTreeMap::new();
    for (scenario, severity, suppress, configured, empty) in [
        ("clean", None, false, false, false),
        ("warning", Some("warning"), false, false, false),
        ("suppressed-error", Some("error"), true, false, false),
        ("rollup-error", Some("error"), true, true, false),
        ("empty", None, false, false, true),
    ] {
        let mut doc: Value = serde_json::from_str(DOCUMENT).unwrap();
        if empty {
            doc["nodes"] = json!([]);
            doc["edges"] = json!([]);
        }
        if let Some(severity) = severity {
            doc["findings"] = json!([{"severity":severity,"code":"PARSE_ERROR","message":"visible malformed row",
                "provenance":{"file":"input.json","line":9},"node_id":null}]);
        }
        let mut profile = PROFILE.to_owned();
        if suppress || configured {
            profile.push_str("validations:\n");
        }
        if suppress {
            profile.push_str("  - SUPPRESS: {code: PARSE_ERROR}\n");
        }
        if configured {
            profile.push_str(
                "  - SUMMARY: {node_kind: assertion, status_attr: status, group_by_attr: group}\n",
            );
        }
        let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{doc}\nDOC"));
        std::fs::write(case.dir.join("empty.txt"), "").unwrap();
        for format in ["plain", "json", "rich"] {
            let operations: Vec<(&str, Vec<&str>, Option<i32>)> = vec![
                ("validate", vec!["validate"], None),
                ("strict", vec!["validate", "--strict"], None),
                ("trace", vec!["trace"], None),
                ("trace-strict", vec!["trace", "--strict"], None),
                (
                    "summary",
                    vec!["summary"],
                    // The profile's SUPPRESS applies before the CLI gates, as
                    // it does for validate: a suppressed error does not exit 1.
                    Some(if severity == Some("error") && !suppress {
                        1
                    } else {
                        0
                    }),
                ),
                ("coverage", vec!["coverage"], Some(0)),
                ("counts", vec!["query", "counts"], Some(0)),
                (
                    "counts-filter",
                    vec!["query", "counts", "--filter", "status=settled"],
                    Some(0),
                ),
                ("orphans", vec!["query", "orphans"], Some(0)),
                (
                    "reach",
                    vec!["query", "reaches", "A1", "--check-resolved"],
                    Some(if empty { 2 } else { 0 }),
                ),
                (
                    "reverse",
                    vec!["query", "reached-by", "R1"],
                    Some(if empty { 2 } else { 0 }),
                ),
                (
                    "path",
                    vec!["query", "path", "A1", "R1"],
                    Some(if empty { 2 } else { 0 }),
                ),
                (
                    "no-path",
                    vec!["query", "path", "R1", "A1"],
                    Some(if empty { 2 } else { 0 }),
                ),
                (
                    "at",
                    vec![
                        "query",
                        "at",
                        "crates/latt-core/tests/fixtures/embedding/actor-a.json",
                    ],
                    Some(if empty { 2 } else { 0 }),
                ),
                (
                    "at-dir",
                    vec!["query", "at", "crates/latt-core/tests/fixtures/embedding"],
                    Some(if empty { 2 } else { 0 }),
                ),
                ("at-empty", vec!["query", "at", "empty.txt"], Some(0)),
                ("at-missing", vec!["query", "at", "missing.txt"], Some(2)),
                ("unknown-node", vec!["query", "reaches", "RX"], Some(2)),
                (
                    "unknown-kind",
                    vec!["query", "orphans", "--kind", "missing"],
                    Some(2),
                ),
                (
                    "unknown-edge",
                    vec!["query", "reaches", "A1", "--edge-kind", "missing"],
                    Some(2),
                ),
            ];
            for (name, mut args, exit) in operations {
                args.extend(["--format", format]);
                actual.insert(
                    format!("{scenario}/{format}/{name}"),
                    capture(&case, &args, exit),
                );
            }
            let absolute = case.dir.join("empty.txt");
            actual.insert(
                format!("{scenario}/{format}/at-absolute"),
                capture(
                    &case,
                    &[
                        "query",
                        "at",
                        absolute.to_str().unwrap(),
                        "--format",
                        format,
                    ],
                    Some(0),
                ),
            );
        }
    }
    let case = Case::with_profile(
        PROFILE,
        "while [ $# -gt 0 ]; do\n  if [ \"$1\" = --target ]; then target=$2; fi\n  shift\ndone\ncat \"$target/document.json\"",
    );
    git(&case, &["init", "-q"]);
    std::fs::write(case.dir.join("document.json"), DOCUMENT).unwrap();
    git(&case, &["add", "document.json"]);
    git(&case, &["commit", "-qm", "before"]);
    let mut doc: Value = serde_json::from_str(DOCUMENT).unwrap();
    doc["nodes"][0]["attrs"]["status"] = json!("changed");
    doc["findings"] = json!([{"severity":"error","code":"PARSE_ERROR","message":"after diagnostic","provenance":{"file":"input.json","line":1}}]);
    std::fs::write(case.dir.join("document.json"), doc.to_string()).unwrap();
    git(&case, &["add", "document.json"]);
    git(&case, &["commit", "-qm", "after"]);
    for format in ["plain", "json", "rich"] {
        actual.insert(
            format!("diff/{format}"),
            capture(
                &case,
                &["query", "diff", "HEAD~1", "HEAD", "--format", format],
                Some(0),
            ),
        );
        let bad = Case::with_profile(
            &format!("{PROFILE}validations:\n  - SUMMARY: {{node_kind: assertion}}\n"),
            &format!("cat <<'DOC'\n{DOCUMENT}\nDOC"),
        );
        actual.insert(
            format!("invalid-summary/{format}"),
            capture(&bad, &["summary", "--format", format], Some(2)),
        );
        let overlay = case.dir.join("suggestions.json");
        std::fs::write(&overlay, json!({"suggestion_version":"1.0","producer":"fixture","suggestions":[{"src":"A1","tgt":"A2","kind":"uses","score":0.5,"basis":"fixture"}]}).to_string()).unwrap();
        actual.insert(
            format!("overlay/{format}"),
            capture(
                &case,
                &[
                    "validate",
                    "--suggestions",
                    overlay.to_str().unwrap(),
                    "--format",
                    format,
                ],
                None,
            ),
        );
    }
    actual
}

fn baseline_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../latt-core/tests/fixtures/embedding/cli-baseline.json")
}

// Requirement: graph-service/Preserve existing CLI and provenance contracts
#[test]
fn frozen_cli_bytes_and_exit_codes() {
    let actual = observations();
    let expected: BTreeMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(baseline_path()).unwrap()).unwrap();
    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (name, output) in actual {
        assert_eq!(output, expected[&name], "{name}");
    }
}

#[test]
#[ignore = "maintenance utility: writes a baseline candidate for manual review"]
fn capture_service_baseline() {
    let path = std::env::temp_dir().join("lattice-service-cli-baseline-candidate.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&observations()).unwrap() + "\n",
    )
    .unwrap();
    eprintln!("wrote {}", path.display());
}

//! Rule coverage keeps missing evidence and unread input beside its aggregate.
mod common;
use common::{Case, code, stderr, stdout};
use serde_json::{Value, json};

const PROFILE: &str = r#"
name: coverage
profile_version: '1.0.0'
node_kinds:
  item: {id_pattern: '.*'}
  source: {id_pattern: '.*'}
edge_kinds:
  contains: {allowed: [[source, item]]}
  evidence: {allowed: [[source, item]]}
validations:
  - SUPPRESS: {code: PARSE_ERROR}
  - COVERAGE: {target_kind: item, edge_kind: evidence}
  - COVERAGE: {target_kind: item, edge_kind: contains}
"#;

fn document() -> Value {
    let mut doc = json!({"interface_version":"1.2", "nodes":[
        {"id":"I2","kind":"item","provenance":{"file":"items","line":2}},
        {"id":"I1","kind":"item","provenance":{"file":"items","line":1}},
        {"id":"S1","kind":"source","provenance":{"file":"sources","line":1}}
    ], "edges":[
        {"src":"S1","tgt":"I1","kind":"contains"},
        {"src":"S1","tgt":"I2","kind":"contains"},
        {"src":"absent","tgt":"I1","kind":"evidence"}
    ], "findings":[{"code":"PARSE_ERROR","severity":"error","message":"unread row",
        "provenance":{"file":"broken","line":7}}]});
    for node in doc["nodes"].as_array_mut().unwrap() {
        node["attrs"] = json!({});
    }
    for edge in doc["edges"].as_array_mut().unwrap() {
        edge["provenance"] = json!({"file":"sources","line":1});
    }
    doc
}

fn run(doc: &Value, rule: Option<&str>, format: &str) -> std::process::Output {
    let case = Case::with_profile(PROFILE, &format!("cat <<'DOC'\n{doc}\nDOC"));
    let mut args = vec!["coverage", "--format", format];
    if let Some(rule) = rule {
        args.extend(["--rule", rule]);
    }
    case.run(&args)
}

// Requirement: coverage-query/Rule-specific coverage report
#[test]
fn structural_edges_do_not_hide_missing_evidence_or_suppressed_parse_errors() {
    let doc = document();
    for format in ["plain", "json", "rich"] {
        let out = run(&doc, Some("1"), format);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        assert!(stderr(&out).is_empty());
        let text = stdout(&out);
        for expected in [
            "evidence",
            "incoming",
            "I1",
            "I2",
            "PARSE_ERROR",
            "broken",
            "partial",
            "unknown",
        ] {
            assert!(
                text.contains(expected),
                "{format}: missing {expected}: {text}"
            );
        }
        if format == "json" {
            let report: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(report["total"], 2);
            assert_eq!(report["covered"], 0);
            assert_eq!(report["covered_pct"], 0.0);
            assert_eq!(
                report["missing"][0],
                json!({"id":"I1","file":"items","line":1,"state":"unknown"})
            );
            assert_eq!(report["uncertainty"]["source_total"], 1);
            assert_eq!(report["uncertainty"]["unattributed"], 1);
            assert_eq!(
                report["uncertainty"]["findings"][0]["message"],
                "unread row"
            );
        }
    }
    let legacy: Value = serde_json::from_str(&stdout(&run(&doc, None, "json"))).unwrap();
    assert_eq!(legacy["kinds"][0]["incoming_pct"], 100.0);
    let second: Value = serde_json::from_str(&stdout(&run(&doc, Some("2"), "json"))).unwrap();
    assert_eq!(second["covered"], 2);
    assert_eq!(second["missing"], json!([]));
}

// Requirement: coverage-query/Rule-specific coverage report
#[test]
fn rule_selection_and_unreadable_register_exit_two_with_diagnostics() {
    for rule in ["0", "3", "invalid"] {
        let out = run(&document(), Some(rule), "json");
        assert_eq!(code(&out), 2);
        assert!(stdout(&out).is_empty());
    }
    let mut doc = document();
    doc["nodes"] = json!([]);
    let out = run(&doc, Some("1"), "json");
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).is_empty());
    assert!(stderr(&out).contains("PARSE_ERROR"));
}

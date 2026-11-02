//! `Condition` is built through constructors and answered through `execute`; a bad
//! filter or pattern is a `FilterError`.

use latt_core::document::ingest_document;
use latt_core::profile::{Comparable, Condition, FilterError};
use latt_core::service::{self, InputContext, Report, Request, View, prepare_profile_str};
use serde_json::json;

const PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^R\\d+$"
edge_kinds: {}
"#;

/// The IDs of the orphans `filters` selects from three unconnected nodes.
fn selected(filters: &[Condition]) -> Vec<String> {
    let profile = prepare_profile_str(PROFILE, "t.yaml").unwrap();
    let node = |id: &str, attrs| {
        json!({"id": id, "kind": "req", "attrs": attrs,
               "provenance": {"file": "r.md", "line": 1}})
    };
    let graph = ingest_document(json!({
        "interface_version": "1.4",
        "nodes": [
            node("R1", json!({"status": "open", "rank": 1})),
            node("R2", json!({"status": "done", "rank": 5})),
            node("R3", json!({"status": "draft", "rank": 3, "owner": "amy"})),
        ],
    }))
    .unwrap();
    let context = InputContext::default();
    let view = View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    let response = service::execute(
        view,
        Request::Orphans {
            kind: None,
            filters,
        },
    )
    .unwrap();
    let Report::Orphans(report) = response.payload else {
        panic!("an orphans report");
    };
    report.orphans.into_iter().map(|entry| entry.id).collect()
}

fn ids(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|id| (*id).to_owned()).collect()
}

// Requirement: graph-service/Supported service shapes admit additions

#[test]
fn each_constructor_selects_through_execute() {
    let int = |n| Comparable::Int(n);
    let cases = [
        (Condition::eq("status", json!("open")), &["R1"][..]),
        (Condition::ne("status", json!("open")), &["R2", "R3"]),
        (
            Condition::one_of("status", vec![json!("open"), json!("done")]),
            &["R1", "R2"],
        ),
        (Condition::lt("rank", int(3)), &["R1"]),
        (Condition::gt("rank", int(3)), &["R2"]),
        (Condition::lte("rank", int(3)), &["R1", "R3"]),
        (Condition::gte("rank", int(3)), &["R2", "R3"]),
        (Condition::matches("status", "^d").unwrap(), &["R2", "R3"]),
        (Condition::present("owner", true), &["R3"]),
        (Condition::present("owner", false), &["R1", "R2"]),
    ];
    for (condition, expected) in cases {
        let attr = condition.attr().to_owned();
        assert_eq!(selected(&[condition]), ids(expected), "{attr}");
    }
}

#[test]
fn parse_and_construct_agree() {
    let int = |n| Comparable::Int(n);
    let pairs = [
        ("status=open", Condition::eq("status", json!("open"))),
        ("status!=open", Condition::ne("status", json!("open"))),
        ("rank<3", Condition::lt("rank", int(3))),
        ("rank>3", Condition::gt("rank", int(3))),
        ("rank<=3", Condition::lte("rank", int(3))),
        ("rank>=3", Condition::gte("rank", int(3))),
        ("status~=^d", Condition::matches("status", "^d").unwrap()),
    ];
    for (text, constructed) in pairs {
        let parsed = Condition::parse(text).unwrap();
        assert_eq!(parsed.attr(), constructed.attr(), "{text}");
        assert_eq!(selected(&[parsed]), selected(&[constructed]), "{text}");
    }
}

#[test]
fn a_bad_filter_is_a_filter_error_with_the_cli_message() {
    let message = |text| -> String {
        let error: FilterError = Condition::parse(text).unwrap_err();
        error.message().to_owned()
    };
    assert_eq!(
        message("no-operator"),
        "invalid filter 'no-operator': expected an attribute operator"
    );
    assert_eq!(message("=x"), "invalid filter '=x': empty attribute name");
    let bad_regex = message("id~=[unclosed");
    assert!(
        bad_regex.starts_with("invalid filter 'id~=[unclosed': regex parse error"),
        "{bad_regex}"
    );
}

#[test]
fn a_bad_pattern_is_a_filter_error_naming_it() {
    let error = Condition::matches("id", "[unclosed").unwrap_err();
    assert!(
        error
            .message()
            .starts_with("invalid pattern '[unclosed' for 'id': regex parse error"),
        "{error}"
    );
    assert_eq!(error.to_string(), error.message());
}

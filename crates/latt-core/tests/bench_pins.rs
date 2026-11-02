//! The benchmark harness's pinned-result gate, table-tested outside Criterion:
//! `cargo test` never compiles the bench target, so the comparison lives in a
//! module both include.

#[path = "../benches/pins/mod.rs"]
mod pins;

use pins::{Pinned, gate};
use serde_json::{Value, json};

fn pin() -> Value {
    json!({
        "findings": { "counts": { "VACANCY": 2 }, "order_fnv1a64": "aa" },
        "reach": { "count": 10, "order": [1, 2] },
        "note": null
    })
}

/// Return the refusal for one scale, asserting registration never ran.
fn refusal(expected: Option<&Value>, actual: Value) -> String {
    let mut registered = false;
    let result = gate(
        &[Pinned {
            label: "chain-10",
            expected,
            actual,
        }],
        || registered = true,
    );
    assert!(!registered, "timing was registered despite the refusal");
    result.expect_err("the gate refuses")
}

/// Parse the computed block a refusal prints for an unpinned `label`.
fn printed_block(message: &str, label: &str) -> Value {
    let marker = format!("{label}:\n\"expected\": ");
    let start = message.find(&marker).expect("the refusal prints the block") + marker.len();
    serde_json::Deserializer::from_str(&message[start..])
        .into_iter::<Value>()
        .next()
        .expect("a block follows the marker")
        .expect("the block is JSON")
}

// Requirement: development-verification/Component benchmarks produce structured development evidence
#[test]
fn matching_pins_register_the_benchmarks() {
    let expected = pin();
    let scales = [
        Pinned {
            label: "chain-10",
            expected: Some(&expected),
            actual: pin(),
        },
        Pinned {
            label: "chain-100",
            expected: Some(&expected),
            actual: pin(),
        },
    ];
    let mut registered = 0;
    gate(&scales, || registered += 1).expect("matching pins pass");
    assert_eq!(registered, 1);
}

#[test]
fn a_differing_leaf_names_scale_field_expected_and_actual() {
    let mut actual = pin();
    actual["findings"]["counts"]["VACANCY"] = json!(3);
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.starts_with("pinned fixture results changed:\n"),
        "{message}"
    );
    assert!(
        message.contains("chain-10: findings.counts.VACANCY: expected 2, actual 3"),
        "{message}"
    );
    assert_eq!(
        message.lines().count(),
        2,
        "only the changed leaf: {message}"
    );
}

#[test]
fn an_added_leaf_is_a_difference() {
    let mut actual = pin();
    actual["reach"]["tainted"] = json!(1);
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains("chain-10: reach.tainted: expected absent, actual 1"),
        "{message}"
    );
}

#[test]
fn a_removed_leaf_is_a_difference() {
    let mut actual = pin();
    actual["findings"]
        .as_object_mut()
        .unwrap()
        .remove("order_fnv1a64");
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains("chain-10: findings.order_fnv1a64: expected \"aa\", actual absent"),
        "{message}"
    );
}

#[test]
fn absence_is_not_null() {
    let mut actual = pin();
    actual.as_object_mut().unwrap().remove("note");
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains("chain-10: note: expected null, actual absent"),
        "{message}"
    );

    let mut actual = pin();
    actual["reach"]["fnv1a64"] = Value::Null;
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains("chain-10: reach.fnv1a64: expected absent, actual null"),
        "{message}"
    );
}

#[test]
fn a_leaf_replaced_by_an_object_is_a_difference() {
    let mut actual = pin();
    actual["reach"]["count"] = json!({ "open": 10 });
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains(r#"chain-10: reach.count: expected 10, actual {"open":10}"#),
        "{message}"
    );
}

#[test]
fn a_scale_without_a_pin_prints_its_computed_block() {
    let message = refusal(None, pin());
    assert!(
        message.starts_with("fixture scales have no pinned results;"),
        "{message}"
    );
    assert_eq!(printed_block(&message, "chain-10"), pin());
}

#[test]
fn a_null_expected_block_is_no_pin() {
    let message = refusal(Some(&Value::Null), pin());
    assert!(
        message.starts_with("fixture scales have no pinned results;"),
        "{message}"
    );
    assert_eq!(printed_block(&message, "chain-10"), pin());
}

#[test]
fn every_differing_leaf_is_reported() {
    let mut actual = pin();
    actual["findings"]["counts"]["VACANCY"] = json!(3);
    actual["reach"]["count"] = json!(11);
    assert_eq!(
        refusal(Some(&pin()), actual),
        "pinned fixture results changed:\n\
         chain-10: findings.counts.VACANCY: expected 2, actual 3\n\
         chain-10: reach.count: expected 10, actual 11"
    );
}

#[test]
fn a_reordered_array_is_a_difference() {
    let mut actual = pin();
    actual["reach"]["order"] = json!([2, 1]);
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains("chain-10: reach.order: expected [1,2], actual [2,1]"),
        "{message}"
    );
}

#[test]
fn an_empty_object_added_or_removed_is_a_difference() {
    let mut actual = pin();
    actual["reach"]["paths"] = json!({});
    let message = refusal(Some(&pin()), actual.clone());
    assert!(
        message.contains("chain-10: reach.paths: expected absent, actual {}"),
        "{message}"
    );

    let message = refusal(Some(&actual), pin());
    assert!(
        message.contains("chain-10: reach.paths: expected {}, actual absent"),
        "{message}"
    );
}

#[test]
fn a_null_leaf_becoming_an_object_is_a_difference() {
    let mut actual = pin();
    actual["note"] = json!({});
    let message = refusal(Some(&pin()), actual);
    assert!(
        message.contains("chain-10: note: expected null, actual {}"),
        "{message}"
    );
}

#[test]
fn a_whole_block_mismatch_names_the_root() {
    let message = refusal(Some(&json!(5)), json!({ "count": 1 }));
    assert!(
        message.contains(r#"chain-10: <root>: expected 5, actual {"count":1}"#),
        "{message}"
    );
}

#[test]
fn every_refusal_is_reported_together() {
    let mut changed = pin();
    changed["reach"]["count"] = json!(11);
    let expected = pin();
    let mut registered = false;
    let message = gate(
        &[
            Pinned {
                label: "chain-10",
                expected: None,
                actual: pin(),
            },
            Pinned {
                label: "chain-50",
                expected: None,
                actual: pin(),
            },
            Pinned {
                label: "chain-100",
                expected: Some(&expected),
                actual: changed,
            },
        ],
        || registered = true,
    )
    .expect_err("the gate refuses");
    assert!(!registered);
    assert!(
        message.starts_with("fixture scales have no pinned results;"),
        "{message}"
    );
    assert_eq!(printed_block(&message, "chain-10"), pin());
    assert_eq!(printed_block(&message, "chain-50"), pin());
    assert!(message.contains("}\nchain-50:\n"), "{message}");
    assert!(
        message.ends_with(
            "\npinned fixture results changed:\nchain-100: reach.count: expected 10, actual 11"
        ),
        "{message}"
    );
}

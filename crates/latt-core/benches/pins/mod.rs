//! The pinned-result gate, shared by the benchmark harness and the table tests in
//! `tests/bench_pins.rs`, so the comparison the harness trusts is the one tested.

use std::collections::BTreeSet;

use serde_json::Value;

/// One scale's pinned `expected` block beside the results computed for it.
pub struct Pinned<'a> {
    pub label: &'a str,
    pub expected: Option<&'a Value>,
    pub actual: Value,
}

/// Collect `scale: field: expected …, actual …` for every leaf that differs. A key on
/// one side only reads `absent` on the other, so a removed `null` leaf still differs;
/// a difference above every key is named `<root>`.
fn differences(
    scale: &str,
    path: &str,
    expected: Option<&Value>,
    actual: Option<&Value>,
    out: &mut Vec<String>,
) {
    match (expected, actual) {
        (Some(Value::Object(expected)), Some(Value::Object(actual))) => {
            let keys: BTreeSet<&String> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                let field = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                differences(scale, &field, expected.get(key), actual.get(key), out);
            }
        }
        _ if expected != actual => {
            let shown =
                |value: Option<&Value>| value.map_or_else(|| "absent".to_owned(), Value::to_string);
            let field = if path.is_empty() { "<root>" } else { path };
            out.push(format!(
                "{scale}: {field}: expected {}, actual {}",
                shown(expected),
                shown(actual)
            ));
        }
        _ => {}
    }
}

/// Run `register` only when every scale has a pin and every pin matches, so a
/// changed result refuses before any timing. Pins are edited by hand; this never
/// writes them, and a scale with no block gets its computed block in the error for
/// review.
///
/// # Errors
///
/// Return the refusal message, naming each unpinned scale's computed block and each
/// differing leaf, when the gate does not pass.
pub fn gate<T>(scales: &[Pinned<'_>], register: impl FnOnce() -> T) -> Result<T, String> {
    let mut missing = Vec::new();
    let mut mismatches = Vec::new();
    for scale in scales {
        match scale.expected {
            // A literal `"expected": null` is a scale awaiting its pin, not a pin of null.
            None | Some(Value::Null) => missing.push(format!(
                "{}:\n\"expected\": {}",
                scale.label,
                serde_json::to_string_pretty(&scale.actual).expect("computed results serialize")
            )),
            Some(expected) => differences(
                scale.label,
                "",
                Some(expected),
                Some(&scale.actual),
                &mut mismatches,
            ),
        }
    }
    let mut refusals = Vec::new();
    if !missing.is_empty() {
        refusals.push(format!(
            "fixture scales have no pinned results; review and add these blocks to fixture.json:\n{}",
            missing.join("\n")
        ));
    }
    if !mismatches.is_empty() {
        refusals.push(format!(
            "pinned fixture results changed:\n{}",
            mismatches.join("\n")
        ));
    }
    if !refusals.is_empty() {
        return Err(refusals.join("\n"));
    }
    Ok(register())
}

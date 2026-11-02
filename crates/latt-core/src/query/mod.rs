//! Ask-time graph queries: reachability, paths, orphans, counts and coverage.
//!
//! The graph stores flat node and edge vectors; every index here is built per
//! invocation from a borrowed graph and dropped with the answer. Nothing is
//! cached or written back; a stored index would be derived state at rest.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod components;
mod index;
mod layout;
mod materialize;
mod reports;
mod schema;
mod traverse;

pub use crate::algo::{betweenness_centrality, bfs_layers, bfs_reachable, scc};
use crate::profile::Condition;

pub use components::{assign_lanes, components, lane_layout};
pub use index::AdjacencyIndex;
pub use layout::graph_layout;
pub use materialize::{MaterializationDir, materialize_revision};
pub(crate) use reports::{PathError, at_in_view, path_typed, tally};
pub use reports::{at, counts, coverage, diff, orphans, path};
pub use schema::schema_graph;
pub use traverse::{reach, traverse};

/// Parse one command-line attribute filter. Unstable: [`Condition::parse`] is the
/// supported form, and this keeps the query error the hidden query functions share.
///
/// # Errors
///
/// [`QueryError`] with [`Condition::parse`]'s message.
pub fn parse_filter(value: &str) -> Result<Condition, QueryError> {
    Condition::parse(value).map_err(|error| QueryError::new(error.message()))
}

/// A question that cannot be posed: an ID or kind the run does not know.
///
/// Operational, not a finding: the caller reports it rather than an empty
/// answer. The CLI maps it to exit 2.
///
/// # Examples
///
/// ```
/// use latt_core::query::parse_filter;
///
/// let error = parse_filter("no-operator").unwrap_err();
/// assert_eq!(error.message(), error.to_string());
/// let text = error.into_message();
/// assert!(text.starts_with("invalid filter"), "{text}");
/// ```
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct QueryError {
    message: String,
}

impl QueryError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Return the explanation, the same text `Display` writes.
    ///
    /// It is for people and may change editorially; do not parse it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use latt_core::query::parse_filter;
    /// let error = parse_filter("=x").unwrap_err();
    /// assert_eq!(error.message(), "invalid filter '=x': empty attribute name");
    /// ```
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the explanation as an owned string, consuming the error.
    ///
    /// # Examples
    ///
    /// ```
    /// # use latt_core::query::parse_filter;
    /// let text: String = parse_filter("=x").unwrap_err().into_message();
    /// assert_eq!(text, "invalid filter '=x': empty attribute name");
    /// ```
    #[must_use]
    pub fn into_message(self) -> String {
        self.message
    }
}

/// Which way a reachability walk follows edges.
///
/// # Examples
///
/// ```
/// use latt_core::query::Direction;
///
/// assert_eq!(Direction::Forward.as_str(), "reaches");
/// assert_eq!(Direction::Reverse.as_str(), "reached-by");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Along outgoing edges: what does this node reach?
    Forward,
    /// Along incoming edges: what reaches this node?
    Reverse,
}

impl Direction {
    /// Return the wire-format string: `"reaches"` or `"reached-by"`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use latt_core::query::Direction;
    /// assert_eq!(Direction::Reverse.as_str(), "reached-by");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "reaches",
            Self::Reverse => "reached-by",
        }
    }
}

#[cfg(test)]
mod filter_tests {
    use serde_json::json;

    use super::parse_filter;
    use crate::profile::ConditionOp;
    use crate::profile::FILTER_OPERATORS as OPERATORS;

    #[test]
    fn parses_every_filter_operator() {
        let cases = [
            ("a=x", "eq"),
            ("a!=x", "ne"),
            ("a~=^x$", "matches"),
            ("a<1", "lt"),
            ("a>1", "gt"),
            ("a<=1", "lte"),
            ("a>=1", "gte"),
        ];
        for (input, expected) in cases {
            let condition = parse_filter(input).unwrap();
            let actual = match condition.op {
                ConditionOp::Eq(_) => "eq",
                ConditionOp::Ne(_) => "ne",
                ConditionOp::Matches(_) => "matches",
                ConditionOp::Lt(_) => "lt",
                ConditionOp::Gt(_) => "gt",
                ConditionOp::Lte(_) => "lte",
                ConditionOp::Gte(_) => "gte",
                ConditionOp::In(_) | ConditionOp::Present(_) => unreachable!(),
            };
            assert_eq!(actual, expected, "{input}");
        }
    }

    #[test]
    fn equality_filters_infer_int_bool_then_string() {
        for operator in ["=", "!="] {
            for (raw, expected) in [
                ("42", json!(42)),
                ("18446744073709551615", json!(u64::MAX)),
                ("true", json!(true)),
                ("text", json!("text")),
            ] {
                let condition = parse_filter(&format!("a{operator}{raw}")).unwrap();
                let (ConditionOp::Eq(actual) | ConditionOp::Ne(actual)) = condition.op else {
                    unreachable!()
                };
                assert_eq!(actual, expected, "{operator}{raw}");
            }
        }
    }

    #[test]
    fn ordering_filters_infer_int_then_string() {
        use crate::profile::Comparable;
        for operator in ["<", ">", "<=", ">="] {
            for (raw, expected) in [
                ("42", Comparable::Int(42)),
                ("true", Comparable::Str("true".into())),
            ] {
                let condition = parse_filter(&format!("a{operator}{raw}")).unwrap();
                let (ConditionOp::Lt(actual)
                | ConditionOp::Gt(actual)
                | ConditionOp::Lte(actual)
                | ConditionOp::Gte(actual)) = condition.op
                else {
                    unreachable!()
                };
                assert_eq!(actual, expected, "{operator}{raw}");
            }
        }
    }

    #[test]
    fn match_filter_compiles_its_regex() {
        let condition = parse_filter("status~=^act").unwrap();
        let ConditionOp::Matches(regex) = condition.op else {
            panic!("expected matches")
        };
        assert!(regex.is_match("active"));
        assert!(!regex.is_match("inactive"));
    }

    #[test]
    fn operators_are_ordered_longest_first() {
        assert!(
            OPERATORS
                .windows(2)
                .all(|pair| pair[0].len() >= pair[1].len())
        );
        for (i, long) in OPERATORS.iter().enumerate() {
            for short in &OPERATORS[..i] {
                assert!(!long.starts_with(short), "{short} would shadow {long}");
            }
        }
    }

    #[test]
    fn filter_without_an_operator_is_rejected() {
        assert!(parse_filter("status").is_err());
    }
}

#[cfg(test)]
mod filter_properties {
    use proptest::prelude::*;
    use serde_json::Value;

    use super::parse_filter;
    use crate::profile::FILTER_OPERATORS as OPERATORS;
    use crate::profile::{Comparable, ConditionOp};

    /// An attr name: no operator character, so the split can only fall after it.
    fn attr() -> impl Strategy<Value = String> {
        "[a-z_][a-z0-9_]{0,7}"
    }

    /// A value that reads as text, not a number or bool, and may hold operator characters.
    fn text_value() -> impl Strategy<Value = String> {
        "[a-z][A-Za-z0-9<>=!~_-]{0,8}".prop_filter("bool literals read as bools", |raw| {
            raw != "true" && raw != "false"
        })
    }

    proptest! {
        /// Split a filter at the attr's end whatever the value holds: `attr<op>value`
        /// parses back to that attr, that operator and that value, operator characters
        /// in the value included.
        #[test]
        fn text_filter_round_trips(
            attr in attr(),
            operator in prop::sample::select(OPERATORS.to_vec()),
            raw in text_value(),
        ) {
            let condition = parse_filter(&format!("{attr}{operator}{raw}")).unwrap();
            prop_assert_eq!(&condition.attr, &attr);
            match (operator, condition.op) {
                ("=", ConditionOp::Eq(value)) | ("!=", ConditionOp::Ne(value)) => {
                    prop_assert_eq!(value, Value::String(raw));
                }
                ("<", ConditionOp::Lt(value))
                | (">", ConditionOp::Gt(value))
                | ("<=", ConditionOp::Lte(value))
                | (">=", ConditionOp::Gte(value)) => {
                    prop_assert_eq!(value, Comparable::Str(raw));
                }
                ("~=", ConditionOp::Matches(regex)) => {
                    prop_assert_eq!(regex.as_str(), raw);
                }
                (operator, op) => prop_assert!(false, "{} parsed to {:?}", operator, op),
            }
        }

        /// Read an integer value as a number for every operator that compares one.
        #[test]
        fn integer_filter_round_trips(attr in attr(), n: i64) {
            let eq = parse_filter(&format!("{attr}={n}")).unwrap();
            let ConditionOp::Eq(value) = eq.op else {
                return Err(TestCaseError::fail(format!("{attr}={n} parsed to {:?}", eq.op)));
            };
            prop_assert_eq!(value, Value::from(n));
            let lt = parse_filter(&format!("{attr}<{n}")).unwrap();
            let ConditionOp::Lt(value) = lt.op else {
                return Err(TestCaseError::fail(format!("{attr}<{n} parsed to {:?}", lt.op)));
            };
            prop_assert_eq!(value, Comparable::Int(i128::from(n)));
        }

        /// Return a result, never panic, on any text, and name an attr that prefixes the
        /// input and holds no operator; the filter is split at char boundaries, not bytes.
        #[test]
        fn any_text_parses_or_is_refused(value in "\\PC{0,16}") {
            if let Ok(condition) = parse_filter(&value) {
                prop_assert!(!condition.attr.is_empty());
                prop_assert!(value.starts_with(&condition.attr));
                prop_assert!(OPERATORS.iter().all(|op| !condition.attr.contains(op)));
            }
        }
    }
}

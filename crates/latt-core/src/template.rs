//! The placeholder grammar node `actions` and validation `message`s share:
//! `{name}` placeholders, `{{` and `}}` for literal braces, parsed once at load
//! so a typo fails on the profile rather than on a key press or a finding.
//!
//! What a template may name depends on its subject. An action runs against a
//! node and may read the node, its location and its kind's attrs. A message
//! renders a finding: it may always read the finding, may read the node only
//! when every finding of the entry's code carries one, may read attrs only when
//! the entry names the kind they belong to, and never names a location, which
//! the finding carries itself. The `Scope` says which, and the parser refuses
//! the rest, so an unresolvable placeholder is a load error and never empty text
//! in a finding.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::graph::Node;
use crate::profile::{AttrSchema, AttrType, ProfileError};
use crate::types::Severity;

/// One piece of a template: literal text or a placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub enum Segment {
    /// Text copied as written, with `{{` and `}}` already reduced to braces.
    Literal(String),
    /// `{id}`: the node ID.
    Id,
    /// `{kind}`: the node's kind.
    Kind,
    /// `{provenance.file}`: the node's provenance file.
    ProvenanceFile,
    /// `{provenance.line}`: the node's provenance line.
    ProvenanceLine,
    /// `{attrs.<name>}`: a declared, non-list attr of the kind.
    Attr(String),
    /// `{message}`: the message the finding would otherwise carry.
    Message,
    /// `{code}`: the finding's code.
    Code,
    /// `{severity}`: the finding's severity.
    Severity,
}

/// A `run` argument or a declared `message`, parsed into segments at load.
///
/// Parsing once lets the loader reject a bad placeholder and the reader
/// substitute values from the same form.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct Template {
    /// The text as the profile wrote it.
    pub source: String,
    /// The text split into literal text and placeholders.
    pub segments: Vec<Segment>,
}

/// What a template may name, with the reason for each refusal.
pub(crate) struct Scope<'a> {
    /// `{message}`, `{code}` and `{severity}`: the template renders a finding.
    pub finding: bool,
    /// `{id}` and `{kind}`, or why the subject carries no node.
    pub node: Result<(), String>,
    /// `{provenance.file}` and `{provenance.line}`: the template hands the node's
    /// location to a program. A message never does: the finding's own file and
    /// line already say where it is, and for an attr finding they name the attr's
    /// line, not the node's.
    pub location: bool,
    /// The declared attrs `{attrs.<name>}` may name, or why none is in scope.
    pub attrs: Result<&'a BTreeMap<String, AttrSchema>, String>,
}

impl<'a> Scope<'a> {
    /// An action's scope: the node, its location and its kind's attrs, no finding.
    pub(crate) fn action(attrs: &'a BTreeMap<String, AttrSchema>) -> Self {
        Self {
            finding: false,
            node: Ok(()),
            location: true,
            attrs: Ok(attrs),
        }
    }

    fn valid(&self) -> String {
        let mut names = Vec::new();
        if self.finding {
            names.extend(["{message}", "{code}", "{severity}"]);
        }
        if self.node.is_ok() {
            names.extend(["{id}", "{kind}"]);
        }
        if self.location && self.node.is_ok() {
            names.extend(["{provenance.file}", "{provenance.line}"]);
        }
        if self.attrs.is_ok() {
            names.push("{attrs.<name>}");
        }
        names.join(", ")
    }
}

fn err<T>(message: String) -> Result<T, ProfileError> {
    Err(ProfileError::Invalid(message))
}

/// Parse `text` against `scope`, naming `where_` in every refusal.
///
/// # Errors
///
/// An unmatched brace, an unknown placeholder, a node or finding placeholder
/// the scope refuses, and an `{attrs.<name>}` the kind does not declare or
/// declares as a `list`.
pub(crate) fn parse_template(
    where_: &str,
    text: &str,
    scope: &Scope<'_>,
) -> Result<Template, ProfileError> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '}' => {
                return err(format!(
                    "{where_}: '{text}' has a '}}' with no opening '{{' (write '}}}}' for a literal brace)"
                ));
            }
            '{' => {
                let mut name_ = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    name_.push(c);
                }
                if !closed {
                    return err(format!(
                        "{where_}: '{text}' has a '{{' that is never closed (write '{{{{' for a literal brace)"
                    ));
                }
                let segment = placeholder(where_, &name_, scope)?;
                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }
                segments.push(segment);
            }
            _ => literal.push(c),
        }
    }
    if !literal.is_empty() || segments.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    Ok(Template {
        source: text.to_owned(),
        segments,
    })
}

/// Resolve one placeholder name against the scope.
fn placeholder(where_: &str, name_: &str, scope: &Scope<'_>) -> Result<Segment, ProfileError> {
    let finding = |segment: Segment| {
        if scope.finding {
            Ok(segment)
        } else {
            err(format!(
                "{where_}: placeholder '{{{name_}}}' names a finding, which an action has none of \
                 (valid: {})",
                scope.valid()
            ))
        }
    };
    let node = |segment: Segment| match &scope.node {
        Ok(()) => Ok(segment),
        Err(reason) => err(format!(
            "{where_}: placeholder '{{{name_}}}' names a node, but {reason} (valid: {})",
            scope.valid()
        )),
    };
    let location = |segment: Segment| {
        if scope.location {
            node(segment)
        } else {
            err(format!(
                "{where_}: placeholder '{{{name_}}}' names a location, which a message never \
                 repeats: the finding's own file and line say where it is (valid: {})",
                scope.valid()
            ))
        }
    };
    match name_ {
        "message" => finding(Segment::Message),
        "code" => finding(Segment::Code),
        "severity" => finding(Segment::Severity),
        "id" => node(Segment::Id),
        "kind" => node(Segment::Kind),
        "provenance.file" => location(Segment::ProvenanceFile),
        "provenance.line" => location(Segment::ProvenanceLine),
        _ => match name_.strip_prefix("attrs.") {
            Some(attr) => {
                let attrs = match &scope.attrs {
                    Ok(attrs) => attrs,
                    Err(reason) => {
                        return err(format!(
                            "{where_}: placeholder '{{{name_}}}' reads an attr, but {reason} \
                             (valid: {})",
                            scope.valid()
                        ));
                    }
                };
                match attrs.get(attr) {
                    None => err(format!(
                        "{where_}: placeholder '{{{name_}}}' names an attr \
                         the kind does not declare (declared: {:?})",
                        attrs.keys().collect::<Vec<_>>()
                    )),
                    Some(schema) if schema.kind == AttrType::List => err(format!(
                        "{where_}: placeholder '{{{name_}}}' names a list attr, \
                         which has no single argument value"
                    )),
                    Some(_) => Ok(Segment::Attr(attr.to_owned())),
                }
            }
            None => err(format!(
                "{where_}: unknown placeholder '{{{name_}}}' (valid: {})",
                scope.valid()
            )),
        },
    }
}

/// What a message renders: the finding, and its node when the graph has it.
pub(crate) struct Subject<'a> {
    /// The message the finding would otherwise carry.
    pub message: &'a str,
    pub code: &'a str,
    pub severity: Severity,
    pub node: Option<&'a Node>,
}

/// The text for a value the subject lacks: visible, never empty.
const ABSENT: &str = "<absent>";

impl Template {
    /// Render a message against `subject`. An absent node or attr renders as
    /// `<absent>`: the register may omit an attr, and silence would read as a
    /// value.
    pub(crate) fn render(&self, subject: &Subject<'_>) -> String {
        let mut out = String::new();
        let node_text =
            |f: &dyn Fn(&Node) -> String| subject.node.map_or_else(|| ABSENT.to_owned(), f);
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Message => out.push_str(subject.message),
                Segment::Code => out.push_str(subject.code),
                Segment::Severity => out.push_str(subject.severity.as_str()),
                Segment::Id => out.push_str(&node_text(&|n| n.id.clone())),
                Segment::Kind => out.push_str(&node_text(&|n| n.kind.clone())),
                // A message's scope refuses both at load (`Scope::location`), and
                // only `parse_template` builds a template; actions render in browse.
                Segment::ProvenanceFile | Segment::ProvenanceLine => {
                    unreachable!("a message template never holds a location placeholder")
                }
                Segment::Attr(attr) => {
                    let text = subject.node.and_then(|n| n.attrs.get(attr)).map_or_else(
                        || ABSENT.to_owned(),
                        |value| match value {
                            Value::Null => ABSENT.to_owned(),
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        },
                    );
                    out.push_str(&text);
                }
            }
        }
        out
    }
}

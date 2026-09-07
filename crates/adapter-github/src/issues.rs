use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::config::Config;
use crate::document::{Axis, Document, Edge, Node, Provenance};

#[derive(Debug)]
struct GithubIssue {
    number: u64,
    title: String,
    body: String,
    state: String,
    labels: Vec<String>,
    assignee: Option<String>,
    milestone: Option<Milestone>,
}

#[derive(Debug)]
struct Milestone {
    number: u64,
    title: String,
    state: String,
}

/// Map GitHub API issues into nodes, edges, axes, and parse findings.
pub(crate) fn read<'a>(
    document: &mut Document<'a>,
    config: &'a Config,
    repo: &str,
    values: Vec<Value>,
) {
    let mut milestones = BTreeMap::<u64, (String, String)>::new();
    for (index, value) in values.into_iter().enumerate() {
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("pull_request"))
        {
            continue;
        }
        let issue = match parse_issue(&value) {
            Ok(issue) => issue,
            Err(error) => {
                document.parse_error(
                    format!("issue response {}: {error}", index + 1),
                    format!("github:{repo}"),
                );
                continue;
            }
        };
        let provenance = Provenance::new(format!("github:{repo}#{}", issue.number));
        let kind = issue
            .labels
            .iter()
            .find_map(|label| config.label_map.get(label))
            .map(String::as_str)
            .or(config.default_kind.as_deref());
        let Some(kind) = kind else {
            document.parse_error(
                format!(
                    "issue #{} has no label mapped to a node kind and no default_kind",
                    issue.number
                ),
                provenance.file.clone(),
            );
            continue;
        };

        let declared_attrs = &config.attrs_by_kind[kind];
        let mut attrs = BTreeMap::new();
        insert_attr(&mut attrs, declared_attrs, "summary", issue.title.clone());
        insert_attr(&mut attrs, declared_attrs, "state", issue.state.clone());
        insert_attr(&mut attrs, declared_attrs, "labels", issue.labels.clone());
        if let Some(assignee) = issue.assignee.clone() {
            insert_attr(&mut attrs, declared_attrs, "assignee", assignee);
        }
        if let Some(milestone) = &issue.milestone {
            for axis in &config.axes {
                insert_attr(&mut attrs, declared_attrs, axis, milestone.title.clone());
            }
            milestones.insert(
                milestone.number,
                (milestone.title.clone(), milestone.state.clone()),
            );
        }

        let id = format!("#{}", issue.number);
        document.nodes.push(Node {
            id: id.clone(),
            kind,
            attrs,
            provenance: provenance.clone(),
        });
        for (pattern, edge_kind) in &config.edge_patterns {
            for captures in pattern.captures_iter(&issue.body) {
                let Some(target) = captures.get(1) else {
                    continue;
                };
                let Ok(target) = target.as_str().parse::<u64>() else {
                    document.parse_error(
                        format!(
                            "issue #{0}: edge pattern captured invalid issue number '{1}'",
                            issue.number,
                            target.as_str()
                        ),
                        provenance.file.clone(),
                    );
                    continue;
                };
                document.edges.push(Edge {
                    src: id.clone(),
                    tgt: format!("#{target}"),
                    kind: edge_kind,
                    provenance: provenance.clone(),
                });
            }
        }
    }

    add_axes(document, config, milestones);
}

fn insert_attr<T: Into<Value>>(
    attrs: &mut BTreeMap<String, Value>,
    declared: &BTreeSet<String>,
    name: &str,
    value: T,
) {
    if declared.contains(name) {
        attrs.insert(name.to_owned(), value.into());
    }
}

fn add_axes(document: &mut Document, config: &Config, milestones: BTreeMap<u64, (String, String)>) {
    if milestones.is_empty() {
        return;
    }
    let mut seen = BTreeSet::new();
    let order: Vec<String> = milestones
        .values()
        .map(|(title, _)| title.clone())
        .filter(|title| seen.insert(title.clone()))
        .collect();
    let current = milestones
        .values()
        .find(|(_, state)| state == "open")
        .or_else(|| milestones.values().next_back())
        .map(|(title, _)| title.clone())
        .expect("nonempty milestones have a current value");
    for name in &config.axes {
        document.axes.push(Axis {
            name: name.clone(),
            order: order.clone(),
            current: current.clone(),
        });
    }
}

fn parse_issue(value: &Value) -> Result<GithubIssue, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("expected an object, got {}", value_type(value)))?;
    let number = required_u64(object, "number")?;
    let title = required_string(object, "title")?.to_owned();
    let state = required_string(object, "state")?.to_owned();
    let body = optional_string(object, "body")?
        .unwrap_or_default()
        .to_owned();
    let labels = parse_named_array(object, "labels")?;
    let assignees = parse_named_array(object, "assignees")?;
    let assignee = assignees.into_iter().next();
    let milestone = parse_milestone(object.get("milestone"))?;
    Ok(GithubIssue {
        number,
        title,
        body,
        state,
        labels,
        assignee,
        milestone,
    })
}

fn parse_milestone(value: Option<&Value>) -> Result<Option<Milestone>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| "field 'milestone' must be an object or null".to_owned())?;
    Ok(Some(Milestone {
        number: required_u64(object, "number")?,
        title: required_string(object, "title")?.to_owned(),
        state: required_string(object, "state")?.to_owned(),
    }))
}

fn parse_named_array(object: &Map<String, Value>, field: &str) -> Result<Vec<String>, String> {
    let values = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("field '{field}' must be an array"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_object()
                .and_then(|item| item.get("name").or_else(|| item.get("login")))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("field '{field}' item {} has no string name", index + 1))
        })
        .collect()
}

fn required_u64(object: &Map<String, Value>, field: &str) -> Result<u64, String> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("field '{field}' must be a positive integer"))
}

fn required_string<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("field '{field}' must be a string"))
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, String> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("field '{field}' must be a string or null")),
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

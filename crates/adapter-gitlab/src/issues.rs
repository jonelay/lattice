use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use serde_json::{Value, json};

use crate::config::Config;
use crate::document::{Document, Edge, Node, Provenance};
use crate::glab::{GitLabIssue, IssueLink};

const MAX_LINK_WORKERS: usize = 8;

fn issue_file(project: &str, iid: u64) -> String {
    format!("gitlab:{project}#{iid}")
}

fn node_kind<'a>(issue: &GitLabIssue, config: &'a Config) -> &'a str {
    issue
        .labels
        .iter()
        .find_map(|label| config.label_map.get(label))
        .map_or(config.default_kind.as_str(), String::as_str)
}

fn attrs(issue: &GitLabIssue, config: &Config, kind: &str) -> BTreeMap<String, Value> {
    let declared = &config.attrs_by_kind[kind];
    let mut attrs = BTreeMap::new();
    if declared.contains("summary") {
        attrs.insert("summary".to_owned(), json!(issue.title));
    }
    if declared.contains("state") {
        attrs.insert("state".to_owned(), json!(issue.state));
    }
    if declared.contains("labels") {
        attrs.insert("labels".to_owned(), json!(issue.labels));
    }
    if let Some(assignee) = &issue.assignee
        && declared.contains("assignee")
    {
        attrs.insert("assignee".to_owned(), json!(assignee.username));
    }
    attrs
}

fn add_text_edges<'a>(
    document: &mut Document<'a>,
    project: &str,
    issue: &GitLabIssue,
    config: &'a Config,
) {
    let Some(description) = &issue.description else {
        return;
    };
    for pattern in &config.edge_patterns {
        for captures in pattern.regex.captures_iter(description) {
            let Some(target) = captures.get(1) else {
                document.parse_error(
                    format!(
                        "edge pattern '{}' matched without a target iid capture",
                        pattern.regex.as_str()
                    ),
                    issue_file(project, issue.iid),
                );
                continue;
            };
            let Ok(target_iid) = target.as_str().parse::<u64>() else {
                document.parse_error(
                    format!(
                        "edge pattern '{}' captured invalid target iid '{}'",
                        pattern.regex.as_str(),
                        target.as_str()
                    ),
                    issue_file(project, issue.iid),
                );
                continue;
            };
            document.edges.push(Edge {
                src: format!("#{}", issue.iid),
                tgt: format!("#{target_iid}"),
                kind: &pattern.kind,
                provenance: Provenance::new(issue_file(project, issue.iid)),
            });
        }
    }
}

fn add_link_edges<'a>(
    document: &mut Document<'a>,
    project: &str,
    issue: &GitLabIssue,
    links: &[IssueLink],
    local_iids: &BTreeSet<u64>,
    config: &'a Config,
) {
    for link in links {
        let (Some(source_project_id), Some(target_project_id)) =
            (issue.project_id, link.project_id)
        else {
            document.parse_error(
                format!(
                    "issue #{} links to issue #{} without enough project identity to determine whether the link is local; no edge was created",
                    issue.iid, link.iid
                ),
                issue_file(project, issue.iid),
            );
            continue;
        };
        let project_mismatch = source_project_id != target_project_id;
        if !local_iids.contains(&link.iid) || project_mismatch {
            document.external_ref(
                format!(
                    "issue #{} links to external or cross-project issue #{}; no edge was created",
                    issue.iid, link.iid
                ),
                issue_file(project, issue.iid),
            );
            continue;
        }
        let Some(mapping) = config.link_type_map.get(&link.link_type) else {
            document.parse_error(
                format!(
                    "issue #{} has link_type '{}' not declared in adapter.link_type_map",
                    issue.iid, link.link_type
                ),
                issue_file(project, issue.iid),
            );
            continue;
        };
        let current = format!("#{}", issue.iid);
        let linked = format!("#{}", link.iid);
        let (src, tgt) = if mapping.reverse {
            (linked, current)
        } else {
            (current, linked)
        };
        document.edges.push(Edge {
            src,
            tgt,
            kind: &mapping.edge_kind,
            provenance: Provenance::new(issue_file(project, issue.iid)),
        });
    }
}

/// Map GitLab's issue and link API shapes into a lattice contract document.
pub(crate) fn read<'a>(
    document: &mut Document<'a>,
    project: &str,
    config: &'a Config,
    values: Vec<Value>,
    fetch_links: impl Fn(u64) -> Result<Vec<IssueLink>, String> + Sync,
) {
    let mut issues = Vec::new();
    for (index, value) in values.into_iter().enumerate() {
        match serde_json::from_value::<GitLabIssue>(value) {
            Ok(issue) => issues.push(issue),
            Err(error) => document.parse_error(
                format!("issue response {}: {error}", index + 1),
                format!("gitlab:{project}"),
            ),
        }
    }
    let local_iids = issues.iter().map(|issue| issue.iid).collect();
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        let worker_count = issues.len().min(MAX_LINK_WORKERS);
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let sender = sender.clone();
            let issues = &issues;
            let fetch_links = &fetch_links;
            let next = &next;
            workers.push(scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(issue) = issues.get(index) else {
                        break;
                    };
                    if sender.send((index, fetch_links(issue.iid))).is_err() {
                        break;
                    }
                }
            }));
        }
        for worker in workers {
            if let Err(payload) = worker.join() {
                std::panic::resume_unwind(payload);
            }
        }
    });
    drop(sender);
    let mut links_by_issue: Vec<Option<Result<Vec<IssueLink>, String>>> =
        (0..issues.len()).map(|_| None).collect();
    for (index, result) in receiver {
        links_by_issue[index] = Some(result);
    }

    for (index, issue) in issues.into_iter().enumerate() {
        let provenance = Provenance::new(issue_file(project, issue.iid));
        let kind = node_kind(&issue, config);
        document.nodes.push(Node {
            id: format!("#{}", issue.iid),
            kind,
            attrs: attrs(&issue, config, kind),
            provenance,
        });
        add_text_edges(document, project, &issue, config);
        match links_by_issue[index]
            .take()
            .unwrap_or_else(|| Err(format!("failed to fetch links for issue #{}", issue.iid)))
        {
            Ok(links) => add_link_edges(document, project, &issue, &links, &local_iids, config),
            Err(error) => document.parse_error(error, issue_file(project, issue.iid)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EdgePattern, LinkMapping};
    use regex::Regex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn default_attrs_by_kind() -> BTreeMap<String, BTreeSet<String>> {
        BTreeMap::from([
            (
                "requirement".to_owned(),
                BTreeSet::from([
                    "summary".to_owned(),
                    "state".to_owned(),
                    "assignee".to_owned(),
                    "labels".to_owned(),
                ]),
            ),
            (
                "issue".to_owned(),
                BTreeSet::from(["summary".to_owned(), "state".to_owned()]),
            ),
        ])
    }

    fn default_link_type_map() -> BTreeMap<String, LinkMapping> {
        BTreeMap::from([
            (
                "relates_to".to_owned(),
                LinkMapping {
                    edge_kind: "relates_to".to_owned(),
                    reverse: false,
                },
            ),
            (
                "is_blocked_by".to_owned(),
                LinkMapping {
                    edge_kind: "depends_on".to_owned(),
                    reverse: false,
                },
            ),
            (
                "blocks".to_owned(),
                LinkMapping {
                    edge_kind: "depends_on".to_owned(),
                    reverse: true,
                },
            ),
        ])
    }

    #[test]
    fn maps_gitlab_fields_labels_and_description_edges() {
        let config = Config {
            project: None,
            label_map: BTreeMap::from([("requirement".to_owned(), "requirement".to_owned())]),
            default_kind: "issue".to_owned(),
            edge_patterns: vec![EdgePattern {
                regex: Regex::new(r"depends on #(\d+)").unwrap(),
                kind: "depends_on".to_owned(),
            }],
            link_type_map: default_link_type_map(),
            attrs_by_kind: default_attrs_by_kind(),
        };
        let mut document = Document::default();
        read(
            &mut document,
            "group/project",
            &config,
            vec![json!({
                "iid": 1,
                "project_id": 10,
                "title": "Validate input",
                "state": "opened",
                "labels": ["requirement"],
                "assignee": {"username": "jone"},
                "description": "depends on #2"
            })],
            |_| Ok(vec![]),
        );
        assert_eq!(document.nodes[0].id, "#1");
        assert_eq!(document.nodes[0].kind, "requirement");
        assert_eq!(document.nodes[0].attrs["assignee"], "jone");
        assert_eq!(document.edges[0].tgt, "#2");
    }

    #[test]
    fn malformed_issue_is_reported_without_losing_valid_issues() {
        let config = Config {
            project: None,
            label_map: BTreeMap::new(),
            default_kind: "issue".to_owned(),
            edge_patterns: vec![],
            link_type_map: default_link_type_map(),
            attrs_by_kind: default_attrs_by_kind(),
        };
        let values = vec![
            json!({"iid": "bad"}),
            json!({
                "iid": 2,
                "project_id": 10,
                "title": "Valid issue",
                "state": "opened",
                "labels": []
            }),
        ];
        let mut document = Document::default();

        read(&mut document, "group/project", &config, values, |_| {
            Ok(vec![])
        });

        assert_eq!(document.nodes.len(), 1);
        assert_eq!(document.nodes[0].id, "#2");
        assert_eq!(document.issues.len(), 1);
        assert_eq!(document.issues[0].code, "PARSE_ERROR");
    }

    #[test]
    fn cross_project_link_is_reported_without_creating_an_edge() {
        let config = Config {
            project: None,
            label_map: BTreeMap::new(),
            default_kind: "issue".to_owned(),
            edge_patterns: vec![],
            link_type_map: default_link_type_map(),
            attrs_by_kind: default_attrs_by_kind(),
        };
        let values = vec![
            json!({
                "iid": 1,
                "project_id": 10,
                "title": "Source",
                "state": "opened",
                "labels": []
            }),
            json!({
                "iid": 2,
                "project_id": 10,
                "title": "Local target",
                "state": "opened",
                "labels": []
            }),
        ];
        let mut document = Document::default();

        read(&mut document, "group/project", &config, values, |iid| {
            Ok(if iid == 1 {
                vec![IssueLink {
                    iid: 2,
                    project_id: Some(20),
                    link_type: "relates_to".to_owned(),
                }]
            } else {
                vec![]
            })
        });

        assert!(document.edges.is_empty());
        assert_eq!(document.issues.len(), 1);
        assert_eq!(document.issues[0].severity, "info");
        assert_eq!(document.issues[0].code, "EXTERNAL_REF");
    }

    #[test]
    fn missing_project_identity_is_reported_without_creating_an_edge() {
        let config = Config {
            project: None,
            label_map: BTreeMap::new(),
            default_kind: "issue".to_owned(),
            edge_patterns: vec![],
            link_type_map: default_link_type_map(),
            attrs_by_kind: default_attrs_by_kind(),
        };

        for (source_project_id, target_project_id) in [(None, Some(10)), (Some(10), None)] {
            let values = vec![
                json!({
                    "iid": 1,
                    "project_id": source_project_id,
                    "title": "Source",
                    "state": "opened",
                    "labels": []
                }),
                json!({
                    "iid": 2,
                    "project_id": 10,
                    "title": "Local IID collision",
                    "state": "opened",
                    "labels": []
                }),
            ];
            let mut document = Document::default();

            read(&mut document, "group/project", &config, values, |iid| {
                Ok(if iid == 1 {
                    vec![IssueLink {
                        iid: 2,
                        project_id: target_project_id,
                        link_type: "relates_to".to_owned(),
                    }]
                } else {
                    vec![]
                })
            });

            assert!(document.edges.is_empty());
            assert_eq!(document.issues.len(), 1);
            assert_eq!(document.issues[0].code, "PARSE_ERROR");
        }
    }

    #[test]
    fn link_fetches_are_bounded_and_failures_stay_with_their_issues() {
        let config = Config {
            project: None,
            label_map: BTreeMap::new(),
            default_kind: "issue".to_owned(),
            edge_patterns: vec![],
            link_type_map: default_link_type_map(),
            attrs_by_kind: default_attrs_by_kind(),
        };
        let values = (1..=16)
            .map(|iid| {
                json!({
                    "iid": iid,
                    "project_id": 10,
                    "title": format!("Issue {iid}"),
                    "state": "opened",
                    "labels": []
                })
            })
            .collect();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let first_wave = Arc::new(Barrier::new(MAX_LINK_WORKERS));
        let mut document = Document::default();

        read(&mut document, "group/project", &config, values, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let first_wave = Arc::clone(&first_wave);
            move |iid| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                if iid <= MAX_LINK_WORKERS as u64 {
                    first_wave.wait();
                }
                active.fetch_sub(1, Ordering::SeqCst);
                if iid == 7 {
                    Err("issue #7 link request failed".to_owned())
                } else {
                    Ok(vec![])
                }
            }
        });

        assert_eq!(document.nodes.len(), 16);
        assert_eq!(peak.load(Ordering::SeqCst), MAX_LINK_WORKERS);
        assert_eq!(document.issues.len(), 1);
        assert_eq!(document.issues[0].code, "PARSE_ERROR");
        assert_eq!(document.issues[0].provenance.file, "gitlab:group/project#7");
        assert!(document.issues[0].message.contains("issue #7"));
    }
}

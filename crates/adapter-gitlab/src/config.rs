use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) project: Option<String>,
    pub(crate) label_map: BTreeMap<String, String>,
    pub(crate) default_kind: String,
    pub(crate) edge_patterns: Vec<EdgePattern>,
    pub(crate) link_type_map: BTreeMap<String, LinkMapping>,
    pub(crate) attrs_by_kind: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone)]
pub(crate) struct LinkMapping {
    pub(crate) edge_kind: String,
    pub(crate) reverse: bool,
}

#[derive(Debug)]
pub(crate) struct EdgePattern {
    pub(crate) regex: Regex,
    pub(crate) kind: String,
}

#[derive(Debug, Deserialize)]
struct NodeKind {
    #[serde(default)]
    attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Profile {
    resolved_schema: String,
    adapter: Adapter,
    #[serde(default)]
    node_kinds: BTreeMap<String, NodeKind>,
    #[serde(default)]
    edge_kinds: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Adapter {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    label_map: BTreeMap<String, String>,
    default_kind: String,
    #[serde(default)]
    edge_patterns: BTreeMap<String, String>,
    #[serde(default)]
    link_type_map: BTreeMap<String, RawLinkMapping>,
}

#[derive(Debug, Deserialize)]
struct RawLinkMapping {
    edge_kind: String,
    #[serde(default)]
    reverse: bool,
}

/// Read the GitLab adapter section from a core-resolved profile document.
pub(crate) fn load(path: &Path) -> Result<Config, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
    let profile: Profile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
    if profile.resolved_schema != "1" {
        return Err(format!(
            "could not load profile {}: not a resolved profile document",
            path.display()
        ));
    }
    if profile.adapter.default_kind.is_empty() {
        return Err(format!(
            "could not load profile {}: adapter.default_kind must not be empty",
            path.display()
        ));
    }
    if profile.adapter.project.as_deref() == Some("") {
        return Err(format!(
            "could not load profile {}: adapter.project must not be empty",
            path.display()
        ));
    }

    for kind in profile
        .adapter
        .label_map
        .values()
        .chain(std::iter::once(&profile.adapter.default_kind))
    {
        if !profile.node_kinds.contains_key(kind) {
            return Err(format!(
                "could not load profile {}: adapter references undeclared node kind '{kind}'",
                path.display()
            ));
        }
    }

    let mut edge_patterns = Vec::with_capacity(profile.adapter.edge_patterns.len());
    for (pattern, kind) in profile.adapter.edge_patterns {
        if kind.is_empty() {
            return Err(format!(
                "could not load profile {}: edge kind for pattern '{pattern}' must not be empty",
                path.display()
            ));
        }
        if !profile.edge_kinds.contains_key(&kind) {
            return Err(format!(
                "could not load profile {}: adapter references undeclared edge kind '{kind}'",
                path.display()
            ));
        }
        let regex = Regex::new(&pattern).map_err(|error| {
            format!(
                "could not load profile {}: invalid adapter.edge_patterns regex '{pattern}': {error}",
                path.display()
            )
        })?;
        if regex.captures_len() < 2 {
            return Err(format!(
                "could not load profile {}: adapter.edge_patterns regex '{pattern}' needs a capture group for the target iid",
                path.display()
            ));
        }
        edge_patterns.push(EdgePattern { regex, kind });
    }

    let link_type_map = if profile.adapter.link_type_map.is_empty() {
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
    } else {
        profile
            .adapter
            .link_type_map
            .into_iter()
            .map(|(key, raw)| {
                (
                    key,
                    LinkMapping {
                        edge_kind: raw.edge_kind,
                        reverse: raw.reverse,
                    },
                )
            })
            .collect()
    };

    let attrs_by_kind = profile
        .node_kinds
        .into_iter()
        .map(|(kind, node)| (kind, node.attrs.into_keys().collect()))
        .collect();

    Ok(Config {
        project: profile.adapter.project,
        label_map: profile.adapter.label_map,
        default_kind: profile.adapter.default_kind,
        edge_patterns,
        link_type_map,
        attrs_by_kind,
    })
}

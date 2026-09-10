use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) repo: Option<String>,
    pub(crate) label_map: BTreeMap<String, String>,
    pub(crate) default_kind: Option<String>,
    pub(crate) edge_patterns: Vec<(Regex, String)>,
    pub(crate) attrs_by_kind: BTreeMap<String, BTreeSet<String>>,
    pub(crate) pathways: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ResolvedProfile {
    resolved_schema: String,
    adapter: Adapter,
    node_kinds: BTreeMap<String, NodeKind>,
    #[serde(default)]
    edge_kinds: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pathways: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Adapter {
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    label_map: BTreeMap<String, String>,
    #[serde(default)]
    default_kind: Option<String>,
    #[serde(default)]
    edge_patterns: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct NodeKind {
    #[serde(default)]
    attrs: BTreeMap<String, serde_json::Value>,
}

impl Config {
    /// Load the adapter namespace from a core-resolved profile document.
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
        let profile: ResolvedProfile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
        if profile.resolved_schema != "1" {
            return Err(format!(
                "could not load profile {}: not a resolved profile document",
                path.display()
            ));
        }

        if let Some(repo) = profile.adapter.repo.as_deref()
            && !valid_repo_slug(repo)
        {
            return Err(format!(
                "could not load profile {}: adapter.repo must be an owner/repo slug",
                path.display()
            ));
        }

        for kind in profile
            .adapter
            .label_map
            .values()
            .chain(profile.adapter.default_kind.iter())
        {
            if !profile.node_kinds.contains_key(kind) {
                return Err(format!(
                    "could not load profile {}: adapter references undeclared node kind '{kind}'",
                    path.display()
                ));
            }
        }

        let mut edge_patterns = Vec::new();
        for (pattern, kind) in profile.adapter.edge_patterns {
            if !profile.edge_kinds.contains_key(&kind) {
                return Err(format!(
                    "could not load profile {}: adapter references undeclared edge kind '{kind}'",
                    path.display()
                ));
            }
            let regex = Regex::new(&pattern).map_err(|error| {
                format!(
                    "could not load profile {}: invalid adapter edge pattern '{pattern}': {error}",
                    path.display()
                )
            })?;
            if regex.captures_len() < 2 {
                return Err(format!(
                    "could not load profile {}: adapter edge pattern '{pattern}' needs a capture group",
                    path.display()
                ));
            }
            edge_patterns.push((regex, kind));
        }

        let attrs_by_kind = profile
            .node_kinds
            .into_iter()
            .map(|(kind, node)| (kind, node.attrs.into_keys().collect()))
            .collect();
        Ok(Self {
            repo: profile.adapter.repo,
            label_map: profile.adapter.label_map,
            default_kind: profile.adapter.default_kind,
            edge_patterns,
            attrs_by_kind,
            pathways: profile.pathways,
        })
    }
}

fn valid_repo_slug(repo: &str) -> bool {
    let mut parts = repo.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty()
    )
}

#[cfg(test)]
mod tests {
    use super::valid_repo_slug;

    #[test]
    fn repo_slug_has_exactly_two_nonempty_parts() {
        assert!(valid_repo_slug("owner/repo"));
        assert!(!valid_repo_slug("repo"));
        assert!(!valid_repo_slug("owner/repo/extra"));
    }
}

use std::path::Path;
use std::process::{Command, Output};

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct User {
    pub(crate) username: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct GitLabIssue {
    pub(crate) iid: u64,
    #[serde(default)]
    pub(crate) project_id: Option<u64>,
    pub(crate) title: String,
    pub(crate) state: String,
    pub(crate) labels: Vec<String>,
    #[serde(default)]
    pub(crate) assignee: Option<User>,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct IssueLink {
    pub(crate) iid: u64,
    #[serde(default)]
    pub(crate) project_id: Option<u64>,
    pub(crate) link_type: String,
}

fn git(target: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(target)
        .args(args)
        .output()
}

fn project_from_remote(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/');
    let path = if let Some(rest) = remote.strip_prefix("git@gitlab.com:") {
        rest
    } else if let Some(rest) = remote.strip_prefix("ssh://git@gitlab.com/") {
        rest
    } else if let Some(rest) = remote.strip_prefix("https://gitlab.com/") {
        rest
    } else {
        remote.strip_prefix("http://gitlab.com/")?
    };
    let project = path.strip_suffix(".git").unwrap_or(path);
    (project.contains('/') && !project.starts_with('/') && !project.ends_with('/'))
        .then(|| project.to_owned())
}

/// Derive a GitLab namespace/project path from the target's origin remote.
pub(crate) fn discover_project(target: &Path) -> Result<String, String> {
    let output = git(target, &["config", "--get", "remote.origin.url"])
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "could not read origin remote for '{}': {}",
            target.display(),
            stderr.trim()
        ));
    }
    let remote = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("origin remote is not valid UTF-8: {error}"))?;
    project_from_remote(remote).ok_or_else(|| {
        format!(
            "origin remote '{}' is not a supported gitlab.com URL",
            remote.trim()
        )
    })
}

fn encode_project(project: &str) -> String {
    let mut encoded = String::new();
    for byte in project.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn api<T: for<'de> Deserialize<'de>>(endpoint: &str, paginate: bool) -> Result<T, String> {
    let mut command = Command::new("glab");
    command.arg("api").arg(endpoint);
    if paginate {
        command.arg("--paginate");
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run glab api {endpoint}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "glab api {endpoint} failed with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("glab api {endpoint} returned malformed JSON: {error}"))
}

/// Fetch every raw issue, with pagination delegated to glab.
pub(crate) fn issues(project: &str) -> Result<Vec<Value>, String> {
    api(
        &format!("projects/{}/issues", encode_project(project)),
        true,
    )
}

/// Fetch links declared for one project-scoped issue iid.
pub(crate) fn links(project: &str, iid: u64) -> Result<Vec<IssueLink>, String> {
    api(
        &format!("projects/{}/issues/{iid}/links", encode_project(project)),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_gitlab_remotes() {
        assert_eq!(
            project_from_remote("git@gitlab.com:group/sub/project.git\n").as_deref(),
            Some("group/sub/project")
        );
        assert_eq!(
            project_from_remote("https://gitlab.com/group/project.git").as_deref(),
            Some("group/project")
        );
        assert_eq!(
            project_from_remote("git@github.com:group/project.git"),
            None
        );
    }

    #[test]
    fn encodes_the_complete_project_path_as_one_api_identifier() {
        assert_eq!(encode_project("group/sub project"), "group%2Fsub%20project");
    }
}

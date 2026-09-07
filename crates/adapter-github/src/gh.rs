use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

use crate::document::Document;

fn command(program: &str, target: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program)
        .current_dir(target)
        .args(args)
        .output()
}

/// Derive the GitHub repository slug from the target's origin remote.
pub(crate) fn derive_repo(document: &mut Document, target: &Path) -> Option<String> {
    let output = match command("git", target, &["config", "--get", "remote.origin.url"]) {
        Ok(output) => output,
        Err(error) => {
            document.parse_error(format!("failed to run git: {error}"), ".");
            return None;
        }
    };
    if !output.status.success() {
        document.parse_error(
            format!(
                "could not read origin remote for '{}': {}",
                target.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            ".",
        );
        return None;
    }
    let remote = match std::str::from_utf8(&output.stdout) {
        Ok(remote) => remote.trim(),
        Err(error) => {
            document.parse_error(format!("origin remote is not valid UTF-8: {error}"), ".");
            return None;
        }
    };
    match repo_from_remote(remote) {
        Some(repo) => Some(repo),
        None => {
            document.parse_error(
                format!("origin remote '{remote}' is not a GitHub repository URL"),
                ".",
            );
            None
        }
    }
}

fn repo_from_remote(remote: &str) -> Option<String> {
    let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = remote.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = remote.strip_prefix("http://github.com/") {
        path
    } else {
        remote.strip_prefix("ssh://git@github.com/")?
    };
    let slug = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let mut parts = slug.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repo), None) if !owner.is_empty() && !repo.is_empty() => {
            Some(format!("{owner}/{repo}"))
        }
        _ => None,
    }
}

/// Fetch every issue and retain malformed JSON as a parse finding.
pub(crate) fn fetch_issues(
    document: &mut Document,
    target: &Path,
    repo: &str,
) -> Option<Vec<Value>> {
    let endpoint = format!("repos/{repo}/issues");
    let output = match command("gh", target, &["api", &endpoint, "--paginate", "-q", ".[]"]) {
        Ok(output) => output,
        Err(error) => {
            document.parse_error(
                format!("failed to run gh: {error}"),
                format!("github:{repo}"),
            );
            return None;
        }
    };
    if !output.status.success() {
        document.parse_error(
            format!(
                "gh api failed for {repo}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            format!("github:{repo}"),
        );
        return None;
    }

    let mut issues = Vec::new();
    let stream = serde_json::Deserializer::from_slice(&output.stdout).into_iter::<Value>();
    for value in stream {
        match value {
            Ok(Value::Array(values)) => issues.extend(values),
            Ok(value) => issues.push(value),
            Err(error) => {
                document.parse_error(
                    format!("malformed response from gh api: {error}"),
                    format!("github:{repo}"),
                );
                break;
            }
        }
    }
    Some(issues)
}

#[cfg(test)]
mod tests {
    use super::repo_from_remote;

    #[test]
    fn parses_common_github_remotes() {
        assert_eq!(
            repo_from_remote("https://github.com/test-org/test-repo.git"),
            Some("test-org/test-repo".into())
        );
        assert_eq!(
            repo_from_remote("git@github.com:test-org/test-repo.git"),
            Some("test-org/test-repo".into())
        );
    }
}

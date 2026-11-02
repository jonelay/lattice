use std::path::Path;
use std::process::Command;
use std::time::Duration;

use latt_adapter::process::{BoundedError, run_bounded};

/// How long the provenance panel waits for `git log` before giving up.
pub const FETCH_BOUND: Duration = Duration::from_secs(10);

/// What one provenance fetch produced.
pub enum LogFetch {
    /// Commits touching the line, possibly none.
    Commits(Vec<GitCommit>),
    /// `git log` did not answer within its bound and was killed.
    TimedOut,
}

pub struct GitCommit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// Lane placement for one commit in a newest-first log.
pub struct CommitRow {
    pub lane: usize,
    /// Lanes joined to this commit by an edge (children above, extra parents below).
    pub linked_lanes: Vec<usize>,
    /// Unrelated lanes passing through this row.
    pub open_lanes: Vec<usize>,
}

impl CommitRow {
    pub fn width(&self) -> usize {
        let linked = self.linked_lanes.iter().chain(&self.open_lanes).copied();
        linked.fold(self.lane, usize::max) + 1
    }
}

/// Run `git log -L<start>,<end>:<file>` under `bound` and parse its output.
/// `--parents` rewrites parent hashes to the filtered set so lanes never dangle.
/// A git that fails is no commits; only expiry is reported distinctly.
pub fn parse_git_log(
    git: &Path,
    target_dir: &str,
    file: &str,
    start: usize,
    end: usize,
    bound: Duration,
) -> LogFetch {
    let range = format!("-L{start},{end}:{file}");
    let mut command = Command::new(git);
    command.args([
        "-C",
        target_dir,
        "log",
        "--no-patch",
        "--parents",
        "--format=%H %P%n%an%n%ai%n%s",
        &range,
    ]);
    match run_bounded(command, bound) {
        Ok(output) if output.status.success() => {
            LogFetch::Commits(parse_log_output(&String::from_utf8_lossy(&output.stdout)))
        }
        Err(BoundedError::TimedOut { .. }) => LogFetch::TimedOut,
        _ => LogFetch::Commits(Vec::new()),
    }
}

fn parse_log_output(text: &str) -> Vec<GitCommit> {
    let mut commits = Vec::new();
    let mut lines = text.lines().filter(|l| !l.is_empty());
    while let Some(hashes) = lines.next() {
        let mut hashes = hashes.split_whitespace().map(str::to_owned);
        let hash = hashes.next().unwrap_or_default();
        let parents = hashes.collect();
        let author = lines.next().unwrap_or("").to_owned();
        let date = lines.next().unwrap_or("").to_owned();
        let subject = lines.next().unwrap_or("").to_owned();
        commits.push(GitCommit {
            hash,
            parents,
            author,
            date,
            subject,
        });
    }
    commits
}

/// Assign lanes by a single newest-first scan: each active lane holds the
/// hash it expects next; a commit takes the first lane expecting it.
pub fn commit_lanes(commits: &[GitCommit]) -> Vec<CommitRow> {
    let mut active: Vec<Option<&str>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for c in commits {
        let child_lanes = lanes_holding(&active, &c.hash);
        let lane = match child_lanes.first() {
            Some(&l) => l,
            None => free_lane(&mut active),
        };
        let mut linked: Vec<usize> = child_lanes.into_iter().filter(|&l| l != lane).collect();
        for &l in &linked {
            active[l] = None;
        }
        active[lane] = None;
        let open: Vec<usize> = (0..active.len()).filter(|&i| active[i].is_some()).collect();
        for p in &c.parents {
            if let Some(l) = lanes_holding(&active, p).first() {
                linked.push(*l);
            } else if active[lane].is_none() {
                active[lane] = Some(p);
            } else {
                let l = free_lane(&mut active);
                active[l] = Some(p);
                linked.push(l);
            }
        }
        while active.last() == Some(&None) {
            active.pop();
        }
        rows.push(CommitRow {
            lane,
            linked_lanes: linked,
            open_lanes: open,
        });
    }
    rows
}

fn lanes_holding(active: &[Option<&str>], hash: &str) -> Vec<usize> {
    (0..active.len())
        .filter(|&i| active[i] == Some(hash))
        .collect()
}

fn free_lane(active: &mut Vec<Option<&str>>) -> usize {
    if let Some(i) = active.iter().position(Option::is_none) {
        return i;
    }
    active.push(None);
    active.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_output() {
        let text = "\
def456abc789012345678901234567890abcdef01 abc123def456789012345678901234567890abcd
Bob
2025-03-16 11:00:00 -0700
Fix bug in parser

abc123def456789012345678901234567890abcd
Alice
2025-03-15 10:00:00 -0700
Initial commit
";
        let commits = parse_log_output(text);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "def456abc789012345678901234567890abcdef01");
        assert_eq!(
            commits[0].parents,
            vec!["abc123def456789012345678901234567890abcd"]
        );
        assert_eq!(commits[0].author, "Bob");
        assert_eq!(commits[0].date, "2025-03-16 11:00:00 -0700");
        assert_eq!(commits[0].subject, "Fix bug in parser");
        assert_eq!(commits[1].author, "Alice");
        assert!(commits[1].parents.is_empty());
        assert_eq!(commits[1].subject, "Initial commit");
    }

    fn commit(hash: &str, parents: &[&str]) -> GitCommit {
        GitCommit {
            hash: hash.to_owned(),
            parents: parents.iter().map(|p| (*p).to_owned()).collect(),
            author: String::new(),
            date: String::new(),
            subject: String::new(),
        }
    }

    #[test]
    fn lanes_linear_chain_stays_in_lane_zero() {
        let commits = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        let rows = commit_lanes(&commits);
        assert!(rows.iter().all(|r| r.lane == 0));
        assert!(rows.iter().all(|r| r.linked_lanes.is_empty()));
        assert!(rows.iter().all(|r| r.open_lanes.is_empty()));
        assert!(rows.iter().all(|r| r.width() == 1));
    }

    #[test]
    fn lanes_merge_opens_and_closes_side_lane() {
        let commits = [
            commit("m", &["a", "b"]),
            commit("a", &["r"]),
            commit("b", &["r"]),
            commit("r", &[]),
        ];
        let rows = commit_lanes(&commits);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(
            rows[0].linked_lanes,
            vec![1],
            "second parent forks to lane 1"
        );
        assert_eq!(rows[1].lane, 0);
        assert_eq!(rows[1].open_lanes, vec![1], "b's lane passes through a");
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[2].linked_lanes, vec![0], "b rejoins r in lane 0");
        assert_eq!(rows[2].open_lanes, vec![0]);
        assert_eq!(rows[3].lane, 0);
        assert!(rows[3].linked_lanes.is_empty());
        assert_eq!(rows.iter().map(CommitRow::width).max(), Some(2));
    }

    #[test]
    fn lanes_unrelated_roots_get_fresh_lanes() {
        let commits = [commit("x", &[]), commit("y", &[])];
        let rows = commit_lanes(&commits);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 0, "closed lane is reused");
        assert!(rows[1].open_lanes.is_empty());
    }

    #[test]
    fn parse_empty_output() {
        let commits = parse_log_output("");
        assert!(commits.is_empty());
    }

    fn make_git_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lattice-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed: {out:?}");
            out
        };
        run(&["init", "-b", "main"]);
        run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ]);
        std::fs::write(dir.join("reqs.md"), "line1\nline2\nline3\n").unwrap();
        run(&["add", "reqs.md"]);
        run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add reqs",
        ]);
        dir
    }

    fn log(dir: &str, file: &str, line: usize) -> Vec<GitCommit> {
        match parse_git_log(Path::new("git"), dir, file, line, line, FETCH_BOUND) {
            LogFetch::Commits(commits) => commits,
            LogFetch::TimedOut => panic!("a local git log does not time out"),
        }
    }

    #[test]
    fn parse_git_log_real_repo() {
        let dir = make_git_repo("real");
        let d = dir.to_str().unwrap();

        let commits = log(d, "reqs.md", 1);
        assert!(!commits.is_empty(), "should find commits touching line 1");
        assert_eq!(commits[0].subject, "add reqs");

        let empty = log(d, "nonexistent.txt", 1);
        assert!(empty.is_empty(), "nonexistent file returns empty");

        let zero = log(d, "reqs.md", 0);
        assert!(zero.is_empty(), "line 0 fails against a real repo");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_incomplete_record() {
        let text = "abc123\nAlice\n";
        let commits = parse_log_output(text);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].hash, "abc123");
        assert!(commits[0].parents.is_empty());
        assert_eq!(commits[0].author, "Alice");
        assert_eq!(commits[0].date, "");
        assert_eq!(commits[0].subject, "");
    }
}

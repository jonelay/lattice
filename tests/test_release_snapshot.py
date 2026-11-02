from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest


SCRIPT = Path(__file__).parent.parent / "tools" / "release-snapshot.sh"

RULES = """\
# comment
+ Cargo.toml
+ docs
- docs/private
+ openspec/specs
- openspec/changes
- notes
"""


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
        cwd=repo,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


# CI runners carry no git identity, and commit-tree needs one.
IDENTITY = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.invalid",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.invalid",
}


def snapshot(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, **IDENTITY},
    )


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    files = {
        "Cargo.toml": '[workspace.package]\nversion = "0.8.0"\n',
        "docs/guide.md": "guide\n",
        "docs/private/secret.md": "secret\n",
        "openspec/specs/cli/spec.md": "spec\n",
        "openspec/changes/x/tasks.md": "tasks\n",
        "notes/sessions/NEXT.md": "next\n",
        "tools/snapshot-files": RULES + "+ tools\n",
    }
    for name, text in files.items():
        (tmp_path / name).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / name).write_text(text)
    git(tmp_path, "init", "-q", "-b", "dev")
    git(tmp_path, "add", ".")
    git(tmp_path, "commit", "-q", "-m", "development")
    git(tmp_path, "checkout", "-q", "--orphan", "public")
    git(tmp_path, "rm", "-rq", "--cached", ".")
    git(tmp_path, "commit", "-q", "--allow-empty", "-m", "previous snapshot")
    git(tmp_path, "checkout", "-q", "-f", "dev")
    return tmp_path


# Requirement: development-verification/GitHub receives release snapshots only
def test_the_snapshot_holds_only_shipped_paths_on_the_previous_snapshot(repo: Path) -> None:
    before = git(repo, "status", "--porcelain")
    result = snapshot(repo, "dev", "public")
    assert result.returncode == 0, result.stderr
    commit = result.stdout.strip()
    assert git(repo, "ls-tree", "-r", "--name-only", commit).splitlines() == [
        "Cargo.toml",
        "docs/guide.md",
        "openspec/specs/cli/spec.md",
        "tools/snapshot-files",
    ]
    assert git(repo, "rev-parse", f"{commit}^@") == git(repo, "rev-parse", "public")
    assert git(repo, "log", "-1", "--format=%s", commit) == "lattice 0.8.0"
    # Nothing outside the object store moved.
    assert git(repo, "status", "--porcelain") == before
    assert git(repo, "rev-parse", "public") != commit


# Requirement: development-verification/GitHub receives release snapshots only
def test_the_same_revision_builds_the_same_tree(repo: Path) -> None:
    first = snapshot(repo, "dev", "public", "m").stdout.strip()
    second = snapshot(repo, "dev", "public", "m").stdout.strip()
    assert git(repo, "rev-parse", f"{first}^{{tree}}") == git(repo, "rev-parse", f"{second}^{{tree}}")


# Requirement: development-verification/GitHub receives release snapshots only
def test_an_unclassified_path_is_named_and_nothing_is_built(repo: Path) -> None:
    (repo / "PROPOSAL.md").write_text("proposal\n")
    (repo / "docs" / "private" / "more.md").write_text("more\n")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "new paths")
    result = snapshot(repo, "dev", "public")
    assert result.returncode == 2
    assert result.stdout == ""
    assert "release-snapshot: unclassified: PROPOSAL.md" in result.stderr.splitlines()
    # A withheld directory covers a file added under it.
    assert "more.md" not in result.stderr


@pytest.mark.parametrize(
    "args", [[], ["dev", "public", "m", "extra"], ["nonexistent"], ["dev", "nonexistent"]]
)
def test_bad_usage_exits_two(repo: Path, args: list[str]) -> None:
    assert snapshot(repo, *args).returncode == 2


# Requirement: development-verification/GitHub receives release snapshots only
def test_a_revision_without_rules_exits_two(repo: Path) -> None:
    result = snapshot(repo, "public", "public")
    assert result.returncode == 2
    assert "no tools/snapshot-files" in result.stderr

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import git, resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "gitlab.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "gitlab"
ADAPTER_BIN = REPO_ROOT / "target" / "debug" / "adapter-gitlab"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "mini-gitlab"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


@pytest.fixture
def mini_gitlab_repo(tmp_path: Path) -> Path:
    repo = tmp_path / "mini-gitlab"
    shutil.copytree(FIXTURE, repo)
    git(repo, "init", "-q", "-b", "main")
    git(repo, "remote", "add", "origin", "https://gitlab.com/test-group/test-project.git")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "mini GitLab fixture")
    return repo


def run_adapter(
    target: Path, tmp_path: Path, *, extra_env: dict[str, str] | None = None
) -> tuple[subprocess.CompletedProcess[str], dict]:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    env = os.environ.copy()
    env["PATH"] = f"{target}{os.pathsep}{env['PATH']}"
    env.update(extra_env or {})
    result = subprocess.run(
        [
            str(ADAPTER_BIN),
            "--profile",
            str(resolved),
            "--target",
            str(target),
        ],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.stdout, result.stderr
    return result, json.loads(result.stdout)


@pytest.fixture
def document(mini_gitlab_repo: Path, tmp_path: Path) -> dict:
    result, doc = run_adapter(mini_gitlab_repo, tmp_path)
    assert result.returncode == 0, result.stderr
    return doc


def nodes(document: dict) -> dict[str, dict]:
    return {node["id"]: node for node in document["nodes"]}


def edge_set(document: dict, kind: str) -> set[tuple[str, str]]:
    return {
        (edge["src"], edge["tgt"])
        for edge in document["edges"]
        if edge["kind"] == kind
    }


def test_contract_version_and_all_issue_nodes(document: dict):
    assert document["contract_version"] == "1.1"
    assert set(nodes(document)) == {"#1", "#2", "#3", "#4"}


def test_labels_choose_node_kinds_and_unmapped_labels_default(document: dict):
    by_id = nodes(document)
    assert by_id["#1"]["kind"] == "requirement"
    assert by_id["#2"]["kind"] == "requirement"
    assert by_id["#3"]["kind"] == "bug"
    assert by_id["#4"]["kind"] == "issue"


def test_gitlab_attrs_use_title_username_and_plain_string_labels(document: dict):
    attrs = nodes(document)["#1"]["attrs"]
    assert attrs == {
        "assignee": "jone",
        "labels": ["requirement"],
        "state": "opened",
        "summary": "System shall handle input validation",
    }
    bug_attrs = nodes(document)["#3"]["attrs"]
    assert "labels" not in bug_attrs, "bug kind does not declare labels"
    assert bug_attrs["summary"] == "Invalid input bypasses the validator"


def test_description_patterns_and_issue_links_become_edges(document: dict):
    assert ("#1", "#2") in edge_set(document, "depends_on")
    assert ("#3", "#1") in edge_set(document, "relates_to")
    assert ("#2", "#1") in edge_set(document, "relates_to")


def test_provenance_names_gitlab_project_and_iid(document: dict):
    by_id = nodes(document)
    for iid in range(1, 5):
        assert by_id[f"#{iid}"]["provenance"] == {
            "file": f"gitlab:test-group/test-project#{iid}",
            "line": 0,
        }


def test_glab_api_failure_is_parse_error_with_zero_exit(
    mini_gitlab_repo: Path, tmp_path: Path
):
    result, doc = run_adapter(
        mini_gitlab_repo, tmp_path, extra_env={"FAKE_GLAB_ERROR": "1"}
    )
    assert result.returncode == 0, result.stderr
    assert not doc["nodes"]
    assert len(doc["issues"]) == 1
    assert doc["issues"][0]["code"] == "PARSE_ERROR"
    assert "simulated GitLab API failure" in doc["issues"][0]["message"]


def test_malformed_api_response_is_parse_error_with_zero_exit(
    mini_gitlab_repo: Path, tmp_path: Path
):
    result, doc = run_adapter(
        mini_gitlab_repo, tmp_path, extra_env={"FAKE_GLAB_MALFORMED": "1"}
    )
    assert result.returncode == 0, result.stderr
    assert not doc["nodes"]
    assert [issue["code"] for issue in doc["issues"]] == ["PARSE_ERROR"]


def test_missing_glab_is_parse_error_with_zero_exit(tmp_path: Path):
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["project"] = "test-group/test-project"
    resolved.write_text(json.dumps(profile))
    result = subprocess.run(
        [
            str(ADAPTER_BIN),
            "--profile",
            str(resolved),
            "--target",
            str(tmp_path),
        ],
        capture_output=True,
        text=True,
        env={"PATH": str(tmp_path)},
    )
    assert result.returncode == 0, result.stderr
    doc = json.loads(result.stdout)
    assert [issue["code"] for issue in doc["issues"]] == ["PARSE_ERROR"]
    assert "could not run glab" in doc["issues"][0]["message"]


@needs_core
def test_gate_runs_through_lattice_validate(mini_gitlab_repo: Path):
    env = os.environ.copy()
    env["PATH"] = f"{mini_gitlab_repo}{os.pathsep}{env['PATH']}"
    result = subprocess.run(
        [
            str(CORE_BIN),
            "validate",
            "--profile",
            str(PROFILE_PATH),
            "--adapter",
            str(ADAPTER_PATH),
            "--target",
            str(mini_gitlab_repo),
            "--format",
            "json",
        ],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 0, f"exit {result.returncode}: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    assert not [finding for finding in findings if finding["severity"] == "error"]

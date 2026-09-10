from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from conftest import git, resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "github.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "github"
ADAPTER_BIN = REPO_ROOT / "target" / "debug" / "adapter-github"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "mini-github"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


def _environment(*, fail: bool = False) -> dict[str, str]:
    env = os.environ.copy()
    env["PATH"] = f"{FIXTURE}{os.pathsep}{env['PATH']}"
    if fail:
        env["FAKE_GH_ERROR"] = "1"
    return env


@pytest.fixture(scope="module")
def mini_github_repo(tmp_path_factory) -> Path:
    repo = tmp_path_factory.mktemp("mini-github") / "repo"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    git(repo, "remote", "add", "origin", "https://github.com/test-org/test-repo")
    return repo


def _run_adapter(target: Path, tmp_path: Path, *, fail: bool = False) -> dict:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    result = subprocess.run(
        [
            str(ADAPTER_PATH),
            "--profile",
            str(resolved),
            "--target",
            str(target),
        ],
        capture_output=True,
        text=True,
        env=_environment(fail=fail),
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


@pytest.fixture(scope="module")
def document(mini_github_repo, tmp_path_factory) -> dict:
    return _run_adapter(
        mini_github_repo, tmp_path_factory.mktemp("github-profile")
    )


def _nodes(document: dict) -> dict[str, dict]:
    return {node["id"]: node for node in document["nodes"]}


def test_interface_version_is_1_2(document):
    assert document["interface_version"] == "1.2"


def test_labels_select_node_kinds(document):
    nodes = _nodes(document)
    assert set(nodes) == {"#1", "#2", "#3", "#4"}
    assert nodes["#1"]["kind"] == "requirement"
    assert nodes["#2"]["kind"] == "requirement"
    assert nodes["#3"]["kind"] == "bug"


def test_default_kind_handles_unmapped_labels(document):
    assert _nodes(document)["#4"]["kind"] == "issue"


def test_body_references_become_edges(document):
    edges = {(edge["src"], edge["tgt"], edge["kind"]) for edge in document["edges"]}
    assert ("#1", "#2", "depends_on") in edges
    assert ("#3", "#1", "traces_to") in edges


def test_attrs_are_populated_and_filtered_by_kind(document):
    nodes = _nodes(document)
    assert nodes["#1"]["attrs"] == {
        "assignee": "octavia",
        "labels": ["requirement", "planning"],
        "state": "open",
        "summary": "Define the adapter contract",
    }
    assert nodes["#3"]["attrs"] == {
        "state": "open",
        "summary": "Fix pagination",
    }


def test_provenance_uses_public_github_location(document):
    for number, node in _nodes(document).items():
        assert node["provenance"] == {
            "file": f"github:test-org/test-repo{number}",
            "line": 0,
        }


def test_pull_requests_are_skipped_without_a_parse_error(document):
    assert "#5" not in _nodes(document)
    assert not document["findings"]


def test_gh_api_failure_is_reported(mini_github_repo, tmp_path):
    document = _run_adapter(mini_github_repo, tmp_path, fail=True)
    assert document["nodes"] == []
    assert len(document["findings"]) == 1
    assert document["findings"][0]["code"] == "PARSE_ERROR"
    assert "fixture authentication failure" in document["findings"][0]["message"]


def test_missing_gh_is_parse_error_with_zero_exit(tmp_path):
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["repo"] = "test-org/test-repo"
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
        env={"PATH": str(tmp_path / "missing")},
    )
    assert result.returncode == 0, result.stderr
    document = json.loads(result.stdout)
    assert [issue["code"] for issue in document["findings"]] == ["PARSE_ERROR"]
    assert "failed to run gh" in document["findings"][0]["message"]


def test_malformed_json_from_gh_is_parse_error(tmp_path):
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["repo"] = "test-org/test-repo"
    resolved.write_text(json.dumps(profile))
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    fake_gh = fake_bin / "gh"
    fake_gh.write_text("#!/bin/sh\nprintf 'not valid JSON\\n'\n")
    fake_gh.chmod(0o755)
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
        env={"PATH": str(fake_bin)},
    )
    assert result.returncode == 0, result.stderr
    document = json.loads(result.stdout)
    assert [issue["code"] for issue in document["findings"]] == ["PARSE_ERROR"]


@needs_core
def test_gate_runs_through_lattice_validate(mini_github_repo, tmp_path):
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    adapter_result = subprocess.run(
        [
            str(ADAPTER_PATH),
            "--profile",
            str(resolved),
            "--target",
            str(mini_github_repo),
        ],
        capture_output=True,
        text=True,
        env=_environment(),
    )
    assert adapter_result.returncode == 0, adapter_result.stderr

    result = subprocess.run(
        [
            str(CORE_BIN),
            "validate",
            "--profile",
            str(PROFILE_PATH),
            "--adapter",
            str(ADAPTER_PATH),
            "--target",
            str(mini_github_repo),
            "--format",
            "json",
        ],
        capture_output=True,
        text=True,
        env=_environment(),
    )
    assert result.returncode == 0, f"exit {result.returncode}: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    assert findings == []

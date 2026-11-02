from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

import pytest

from conftest import git, resolve_profile, uncomment_actions

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "entomologist.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "entomologist"
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)

FULL = "aa" * 16
NOSTATE = "bb" * 16
NODESC = "cc" * 16
NOAUTHOR = "dd" * 16
BADSTATE = "ee" * 16
BADDESC = "ff" * 16
DANGLING = "deadbeef" * 4
COMMENT = "cafe" * 8
ALL_ISSUES = {FULL, NOSTATE, NODESC, NOAUTHOR, BADSTATE, BADDESC}


class ContractDoc:
    """Thin wrapper over a parsed interface document for test assertions."""

    def __init__(self, doc: dict):
        self._doc = doc
        self._nodes = {n["id"]: n for n in doc.get("nodes", [])}

    def has_node(self, nid: str) -> bool:
        return nid in self._nodes

    def node_ids(self) -> set[str]:
        return set(self._nodes)

    def node_data(self, nid: str) -> dict:
        return self._nodes[nid]

    def edges(self, kind: str) -> set[tuple[str, str]]:
        return {
            (e["src"], e["tgt"])
            for e in self._doc.get("edges", [])
            if e["kind"] == kind
        }

    def issues_by_code(self, code: str) -> list[dict]:
        return [i for i in self._doc.get("findings", []) if i["code"] == code]

    @property
    def all_issues(self) -> list[dict]:
        return self._doc.get("findings", [])


def _run_adapter(
    target: Path, tmp_path: Path, profile: Path = PROFILE_PATH
) -> ContractDoc:
    resolved = resolve_profile(profile, tmp_path)
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved),
         "--target", str(target)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    return ContractDoc(json.loads(result.stdout))


@pytest.fixture(scope="module")
def graph(mini_ent_repo, tmp_path_factory):
    tmp = tmp_path_factory.mktemp("resolve")
    return _run_adapter(mini_ent_repo, tmp)


# --- the branch, not the worktree -----------------------------------------

# Requirement: adapter-entomologist/Read the register from the data branch, not the worktree
def test_gate_the_profile_with_its_actions_uncommented_still_reads(
    mini_ent_repo, tmp_path
):
    profile = uncomment_actions(PROFILE_PATH, tmp_path)
    doc = _run_adapter(mini_ent_repo, tmp_path, profile)
    assert doc.node_ids(), "the adapter read the register"


def test_branch_local_preferred(mini_ent_repo, tmp_path):
    clone = tmp_path / "clone"
    git(tmp_path, "clone", "-q", str(mini_ent_repo), str(clone))
    extra = "0123456789abcdef0123456789abcdef"
    (clone / extra).mkdir()
    (clone / extra / "description").write_text("Only on the local branch\n")
    (clone / extra / "author").write_text("Mira Voss\n")
    git(clone, "add", extra)
    git(clone, "commit", "-q", "-m", "local-only issue")
    built = _run_adapter(clone, tmp_path)
    assert built.has_node(extra)


def test_branch_remote_fallback(mini_ent_repo, tmp_path):
    clone = tmp_path / "clone"
    git(tmp_path, "clone", "-q", str(mini_ent_repo), str(clone))
    git(clone, "checkout", "-q", "--detach")
    git(clone, "branch", "-D", "entomologist-data")
    built = _run_adapter(clone, tmp_path)
    assert built.node_ids() == ALL_ISSUES


def test_branch_absent_is_reported(tmp_path):
    git(tmp_path, "init", "-q")
    built = _run_adapter(tmp_path, tmp_path)
    errs = built.issues_by_code("PARSE_ERROR")
    assert len(errs) == 1
    assert "entomologist-data" in errs[0]["message"]
    assert not built.node_ids()


def test_branch_tag_with_the_name_is_never_read(tmp_path):
    git(tmp_path, "init", "-q", "-b", "main")
    git(tmp_path, "commit", "-q", "--allow-empty", "-m", "x")
    git(tmp_path, "tag", "entomologist-data")
    built = _run_adapter(tmp_path, tmp_path)
    errs = built.issues_by_code("PARSE_ERROR")
    assert len(errs) == 1
    assert "entomologist-data" in errs[0]["message"]
    assert not built.node_ids()


def test_branch_target_in_no_repo_is_reported(tmp_path):
    built = _run_adapter(tmp_path, tmp_path)
    errs = built.issues_by_code("PARSE_ERROR")
    assert len(errs) == 1
    assert "repository" in errs[0]["message"]
    assert "not a git repository" in errs[0]["message"], "git's own reason is kept"
    assert not built.node_ids()


def test_branch_nested_target_does_not_read_the_ancestor(mini_ent_repo, tmp_path):
    nested = mini_ent_repo / FULL
    built = _run_adapter(nested, tmp_path)
    errs = built.issues_by_code("PARSE_ERROR")
    assert len(errs) == 1
    assert "root" in errs[0]["message"]
    assert not built.node_ids()


def test_branch_listing_failure_is_reported(tmp_path):
    git(tmp_path, "init", "-q", "-b", "main")
    git(tmp_path, "commit", "-q", "--allow-empty", "-m", "x")
    ref = tmp_path / ".git" / "refs" / "heads" / "entomologist-data"
    ref.parent.mkdir(parents=True, exist_ok=True)
    ref.write_text("1" * 40 + "\n")
    built = _run_adapter(tmp_path, tmp_path)
    errs = built.issues_by_code("PARSE_ERROR")
    assert len(errs) == 1
    assert "ls-tree" in errs[0]["message"]
    assert not built.node_ids()


def test_corrupt_object_reports_git_failure(mini_ent_repo, tmp_path):
    repo = tmp_path / "repo"
    shutil.copytree(mini_ent_repo, repo)
    oid = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", f"entomologist-data:{FULL}/description"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    loose = repo / ".git" / "objects" / oid[:2] / oid[2:]
    loose.chmod(0o644)
    loose.write_bytes(b"not a zlib stream")
    built = _run_adapter(repo, tmp_path)
    failures = [
        e for e in built.issues_by_code("PARSE_ERROR")
        if e["message"].startswith("git cat-file --batch reported:")
    ]
    assert len(failures) == 1, built.all_issues
    assert "inflate" in failures[0]["message"] or "unpack" in failures[0]["message"]


# --- issue nodes ----------------------------------------------------------

# Requirement: adapter-entomologist/Build one issue node per issue directory
def test_issue_node_per_directory(graph):
    assert graph.node_ids() == ALL_ISSUES
    assert graph.node_data(FULL)["kind"] == "issue"


def test_issue_node_full_fields(graph):
    attrs = graph.node_data(FULL)["attrs"]
    assert attrs["summary"] == "Fold duplicate axis declarations in the loader"
    assert attrs["author"] == "Mira Voss"
    assert attrs["state"] == "inprogress"
    assert attrs["assignee"] == "rowan"
    assert attrs["tags"] == ["loader", "parser"]


def test_issue_node_tags_are_unescaped(tmp_path):
    """ent escapes a tag's "," as ",0" and "/" as ",1" in its filename."""
    repo = tmp_path / "repo"
    tags = repo / FULL / "tags"
    tags.mkdir(parents=True)
    for name in ("ui,1v0.1", "a,0b", "plain", "bad,9", "trailing,"):
        (tags / name).write_text("")
    (repo / FULL / "state").write_text("new")
    git(repo, "init", "-q", "-b", "entomologist-data")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "escaped tags")
    built = _run_adapter(repo, tmp_path)
    assert built.node_data(FULL)["attrs"]["tags"] == ["a,b", "plain", "ui/v0.1"]
    errors = sorted(e["message"] for e in built.issues_by_code("PARSE_ERROR"))
    assert len(errors) == 2, errors
    assert "bad,9" in errors[0] and "trailing," in errors[1], errors


def test_issue_node_description_absent_emits_without_summary(graph):
    attrs = graph.node_data(NODESC)["attrs"]
    assert "summary" not in attrs
    assert attrs["author"] == "Mira Voss"
    assert not [
        i for i in graph.issues_by_code("PARSE_ERROR")
        if f"{NODESC}/description" in i["message"]
    ]


# --- state ----------------------------------------------------------------

# Requirement: adapter-entomologist/State is passed through, with ent's own default
def test_state_missing_file_defaults_to_new(graph):
    assert graph.node_data(NOSTATE)["attrs"]["state"] == "new"


def test_state_unknown_value_passes_through(graph):
    assert graph.node_data(BADSTATE)["attrs"]["state"] == "paused"
    assert not [
        i for i in graph.all_issues if BADSTATE in (i.get("node_id") or "")
    ]


# --- edges ----------------------------------------------------------------

# Requirement: adapter-entomologist/Dependencies become depends_on edges
def test_edge_dependency_becomes_depends_on(graph):
    assert (FULL, NOSTATE) in graph.edges("depends_on")


def test_edge_to_absent_issue_is_still_emitted(graph):
    assert (FULL, DANGLING) in graph.edges("depends_on")
    assert not graph.has_node(DANGLING)


# --- recognized-unrepresented vs unrecognized -----------------------------

# Requirement: adapter-entomologist/Recognized-but-unrepresented content is deliberate, unrecognized content is reported
def test_unread_comments_pass_in_silence(graph):
    for field in ("author", "creation_time", "description"):
        assert not [
            i for i in graph.all_issues
            if f"comments/{COMMENT}/{field}" in i["message"]
        ]


def test_unread_stray_comment_file_is_reported(graph):
    errs = [
        i for i in graph.issues_by_code("PARSE_ERROR")
        if f"comments/{COMMENT}/attachment" in i["message"]
    ]
    assert len(errs) == 1


def test_unread_unrecognized_field_is_reported(graph):
    errs = [
        i for i in graph.issues_by_code("PARSE_ERROR")
        if f"{NODESC}/priority" in i["message"]
    ]
    assert len(errs) == 1
    assert graph.node_data(NODESC)["attrs"]["state"] == "backlog"


# --- unreadable content ---------------------------------------------------

# Requirement: adapter-entomologist/Unreadable content is reported, never raised
def test_unreadable_description_reported_not_raised(graph):
    errs = [
        i for i in graph.issues_by_code("PARSE_ERROR")
        if f"{BADDESC}/description" in i["message"]
    ]
    assert len(errs) == 1
    assert "summary" not in graph.node_data(BADDESC)["attrs"]
    assert graph.has_node(FULL)


# --- the gate -------------------------------------------------------------

# Requirement: adapter-entomologist/Serve as the branch-read gate
EXPECTED_EDGES = {(FULL, NOSTATE), (FULL, DANGLING)}
EXPECTED_ADAPTER_ISSUES = sorted([
    ("PARSE_ERROR", f"entomologist-data:{FULL}/comments/{COMMENT}/attachment"),
    ("PARSE_ERROR", f"entomologist-data:{NODESC}/priority"),
    ("PARSE_ERROR", f"entomologist-data:{BADDESC}/description"),
])
EXPECTED_FINDINGS = sorted(EXPECTED_ADAPTER_ISSUES + [
    ("ATTR_REQUIRED", f"entomologist-data:{NODESC}"),
    ("ATTR_REQUIRED", f"entomologist-data:{NOAUTHOR}"),
    ("ATTR_REQUIRED", f"entomologist-data:{BADDESC}"),
    ("ATTR_ENUM", f"entomologist-data:{BADSTATE}/state"),
    ("VACANCY", f"entomologist-data:{FULL}/dependencies/{DANGLING}"),
])


def test_gate_document_is_exact(mini_ent_repo, tmp_path):
    """A partial read hides in a non-zero count; only exactness catches it."""
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved),
         "--target", str(mini_ent_repo)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    doc = json.loads(result.stdout)
    assert {n["id"] for n in doc["nodes"]} == ALL_ISSUES
    assert {(e["src"], e["tgt"]) for e in doc["edges"]} == EXPECTED_EDGES
    assert sorted(
        (i["code"], i["provenance"]["file"]) for i in doc["findings"]
    ) == EXPECTED_ADAPTER_ISSUES


@needs_core
def test_gate_run_exits_1_with_the_exact_finding_multiset(mini_ent_repo):
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(mini_ent_repo),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode == 1, f"exit {result.returncode}: {result.stderr}"
    findings = sorted(
        (f["code"], f["file"]) for f in json.loads(result.stdout)["findings"]
    )
    assert findings == EXPECTED_FINDINGS


def test_git_hangs_is_a_parse_error(mini_ent_repo, tmp_path):
    """A git that never answers rev-parse is unreadable input, not a crash."""
    shims = tmp_path / "shims"
    shims.mkdir()
    shim = shims / "git"
    shim.write_text(
        "#!/bin/sh\n"
        'for arg in "$@"; do [ "$arg" = rev-parse ] && exec sleep 30; done\n'
        f'exec {shutil.which("git")} "$@"\n'
    )
    shim.chmod(0o755)
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"] = {"command_timeout_seconds": 1}
    resolved.write_text(json.dumps(profile))
    env = os.environ.copy()
    env["PATH"] = f"{shims}{os.pathsep}{env['PATH']}"
    started = time.monotonic()
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(mini_ent_repo)],
        capture_output=True, text=True, env=env,
    )
    assert time.monotonic() - started < 10, "the bound, not the sleep, ended the run"
    assert result.returncode == 0, result.stderr
    doc = ContractDoc(json.loads(result.stdout))
    assert doc.node_ids() == set()
    assert [issue["code"] for issue in doc.all_issues] == ["PARSE_ERROR"]
    message = doc.all_issues[0]["message"]
    assert "git rev-parse" in message and "1 s" in message, message


def _path_without_git(tmp_path: Path) -> dict[str, str]:
    """An environment whose PATH holds what the shim needs, and no git."""
    bin_dir = tmp_path / "bin-without-git"
    bin_dir.mkdir()
    (bin_dir / "dirname").symlink_to(shutil.which("dirname"))
    env = os.environ.copy()
    env["PATH"] = str(bin_dir)
    return env


# Requirement: adapter-entomologist/Every git call the adapter runs is bounded
def test_a_git_that_cannot_be_run_is_a_broken_setup(mini_ent_repo, tmp_path):
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(mini_ent_repo)],
        capture_output=True, text=True, env=_path_without_git(tmp_path),
    )
    assert result.returncode != 0, result.stdout
    assert "git" in result.stderr, result.stderr


@needs_core
# Requirement: adapter-entomologist/Every git call the adapter runs is bounded
def test_a_git_that_cannot_be_run_makes_validate_exit_2(mini_ent_repo, tmp_path):
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(mini_ent_repo)],
        capture_output=True, text=True, env=_path_without_git(tmp_path),
    )
    assert result.returncode == 2, f"exit {result.returncode}: {result.stdout}{result.stderr}"


def test_a_leading_byte_order_mark_is_encoding_not_content(tmp_path):
    repo = tmp_path / "repo"
    (repo / FULL).mkdir(parents=True)
    (repo / FULL / "description").write_bytes("﻿Summary line\n\nBody\n".encode())
    (repo / FULL / "author").write_bytes("﻿Mira Voss\n".encode())
    (repo / FULL / "state").write_text("new")
    git(repo, "init", "-q", "-b", "entomologist-data")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "byte order marks")
    built = _run_adapter(repo, tmp_path)
    attrs = built.node_data(FULL)["attrs"]
    assert attrs["summary"] == "Summary line"
    assert attrs["author"] == "Mira Voss"
    assert built.all_issues == []


@pytest.mark.parametrize("value", ["30", -5, 1.5])
def test_a_mistyped_command_timeout_exits_2(mini_ent_repo, tmp_path, value):
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"] = {"command_timeout_seconds": value}
    resolved.write_text(json.dumps(profile))
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(mini_ent_repo)],
        capture_output=True, text=True,
    )
    assert result.returncode == 2, result.stdout
    assert "could not load profile" in result.stderr, result.stderr


def test_git_hangs_in_cat_file_keeps_the_listed_nodes(mini_ent_repo, tmp_path, graph):
    """An object exchange that never answers loses the content, not the register."""
    shims = tmp_path / "shims"
    shims.mkdir()
    shim = shims / "git"
    shim.write_text(
        "#!/bin/sh\n"
        'for arg in "$@"; do [ "$arg" = cat-file ] && exec sleep 30; done\n'
        f'exec {shutil.which("git")} "$@"\n'
    )
    shim.chmod(0o755)
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"] = {"command_timeout_seconds": 1}
    resolved.write_text(json.dumps(profile))
    env = os.environ.copy()
    env["PATH"] = f"{shims}{os.pathsep}{env['PATH']}"
    started = time.monotonic()
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(mini_ent_repo)],
        capture_output=True, text=True, env=env,
    )
    assert time.monotonic() - started < 10, "the bound, not the sleep, ended the run"
    assert result.returncode == 0, result.stderr
    doc = ContractDoc(json.loads(result.stdout))
    assert doc.node_ids() == ALL_ISSUES
    assert doc.edges("depends_on") == graph.edges("depends_on") != set()
    for nid in ALL_ISSUES:
        attrs = doc.node_data(nid).get("attrs", {})
        assert not {"summary", "author", "assignee"} & set(attrs), (nid, attrs)
    exchange = [
        issue["message"] for issue in doc.all_issues if "cat-file" in issue["message"]
    ]
    assert exchange == ["git cat-file --batch exceeded 1 s"], doc.all_issues
    assert not [
        issue for issue in doc.all_issues if "could not read object" in issue["message"]
    ], doc.all_issues


# Requirement: adapter-contract/Resolved profile document
def test_a_profile_without_the_resolved_marker_exits_2(tmp_path: Path) -> None:
    """A marker-less document is named as unresolved, not as a missing serde field."""
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    del profile["resolved_schema"]
    resolved.write_text(json.dumps(profile))
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(tmp_path)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2, result.stdout
    expected = f"could not load profile {resolved}: not a resolved profile document"
    assert expected in result.stderr, result.stderr

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from conftest import git, resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "entomologist.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "entomologist"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"

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
    """Thin wrapper over a parsed contract document for test assertions."""

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
        return [i for i in self._doc.get("issues", []) if i["code"] == code]

    @property
    def all_issues(self) -> list[dict]:
        return self._doc.get("issues", [])


def _run_adapter(target: Path, tmp_path: Path) -> ContractDoc:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
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

# Requirement: Read the register from the data branch, not the worktree
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


# --- issue nodes ----------------------------------------------------------

# Requirement: Build one issue node per issue directory
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


def test_issue_node_description_absent_emits_without_summary(graph):
    attrs = graph.node_data(NODESC)["attrs"]
    assert "summary" not in attrs
    assert attrs["author"] == "Mira Voss"
    assert not [
        i for i in graph.issues_by_code("PARSE_ERROR")
        if f"{NODESC}/description" in i["message"]
    ]


# --- state ----------------------------------------------------------------

# Requirement: State is passed through, with ent's own default
def test_state_missing_file_defaults_to_new(graph):
    assert graph.node_data(NOSTATE)["attrs"]["state"] == "new"


def test_state_unknown_value_passes_through(graph):
    assert graph.node_data(BADSTATE)["attrs"]["state"] == "paused"
    assert not [
        i for i in graph.all_issues if BADSTATE in (i.get("node_id") or "")
    ]


# --- edges ----------------------------------------------------------------

# Requirement: Dependencies become depends_on edges
def test_edge_dependency_becomes_depends_on(graph):
    assert (FULL, NOSTATE) in graph.edges("depends_on")


def test_edge_to_absent_issue_is_still_emitted(graph):
    assert (FULL, DANGLING) in graph.edges("depends_on")
    assert not graph.has_node(DANGLING)


# --- recognized-unrepresented vs unrecognized -----------------------------

# Requirement: Recognized-but-unrepresented content is deliberate, unrecognized content is reported
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

# Requirement: Unreadable content is reported, never raised
def test_unreadable_description_reported_not_raised(graph):
    errs = [
        i for i in graph.issues_by_code("PARSE_ERROR")
        if f"{BADDESC}/description" in i["message"]
    ]
    assert len(errs) == 1
    assert "summary" not in graph.node_data(BADDESC)["attrs"]
    assert graph.has_node(FULL)


# --- the gate -------------------------------------------------------------

# Requirement: Serve as the branch-read gate
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
    ("ATTR_ENUM", f"entomologist-data:{BADSTATE}"),
    ("DANGLING_REF", f"entomologist-data:{FULL}/dependencies/{DANGLING}"),
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
        (i["code"], i["provenance"]["file"]) for i in doc["issues"]
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

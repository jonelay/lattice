"""``docs/json.md`` lists every key the binary writes, and nothing it does not.

Each ``text`` fence under a command heading is one key path per line. The test
runs that command over a fixture, walks the JSON it wrote, and fails naming the
command and the key when the output carries a path the page does not list, or
the page lists a path (not marked ``?``) the output does not carry. A ``<name>``
segment on the page matches any key, for the kinds, attrs and statuses a
profile supplies. A ``<*>`` segment matches any key and everything beneath it,
for a block whose shape the profile's author owns.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"
PAGE = REPO_ROOT / "docs" / "json.md"
FIXTURES = REPO_ROOT / "tests" / "fixtures"
TOML = ["--profile", "profiles/toml.yaml", "--adapter", "toml",
        "--target", "tests/fixtures/mini-toml"]

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


def documented_sections() -> dict[str, list[str]]:
    """Heading → the key-path lines of the ``text`` fence under it."""
    sections: dict[str, list[str]] = {}
    heading = None
    in_fence = False
    for line in PAGE.read_text().splitlines():
        if line.startswith("## "):
            heading = line[3:].strip()
            continue
        if line.strip() == "```text":
            in_fence = True
            continue
        if in_fence and line.strip() == "```":
            in_fence = False
            continue
        if in_fence and line.strip():
            assert heading, f"a fence before any heading: {line}"
            sections.setdefault(heading, []).append(line.split()[0])
    return sections


class Documented:
    """The page's paths for one section: which are optional, which are placeholders."""

    def __init__(self, lines: list[str]) -> None:
        self.optional: set[str] = set()
        self.paths: list[str] = []
        for raw in lines:
            path = raw.rstrip("?")
            if raw.endswith("?"):
                self.optional.add(path)
            self.paths.append(path)
        self.known = set(self.paths)
        # Every ancestor of a documented path is documented by implication
        # (``a.b`` says ``a`` exists), so a page may spell only the leaves.
        for path in list(self.known):
            for ancestor in ancestors(path):
                self.known.add(ancestor)

    def match(self, parent: str, key: str) -> str | None:
        """The documented path an observed key maps to, or None."""
        literal = join(parent, key)
        if literal in self.known:
            return literal
        for candidate in self.known:
            head, _, last = candidate.rpartition(".")
            if head == parent and last.startswith("<") and last.endswith(">"):
                return candidate
        return None


def join(parent: str, key: str) -> str:
    return f"{parent}.{key}" if parent else key


def ancestors(path: str) -> list[str]:
    """``a[].b.c`` → ``a[].b``, ``a[]``, ``a``."""
    found = []
    while path:
        if path.endswith("[]"):
            path = path[:-2]
        else:
            head, dot, _ = path.rpartition(".")
            path = head if dot else ""
        if path:
            found.append(path)
    return found


def observe(value, parent: str, doc: Documented, seen: set[str], unknown: set[str]) -> None:
    """Walk the output, recording the documented path of every key met."""
    if isinstance(value, dict):
        for key, child in value.items():
            path = doc.match(parent, key)
            if path is None:
                unknown.add(join(parent, key))
                continue
            seen.add(path)
            if not path.endswith(".<*>"):
                observe(child, path, doc, seen, unknown)
    elif isinstance(value, list):
        if value:
            seen.add(parent + "[]")
        for item in value:
            observe(item, parent + "[]", doc, seen, unknown)


def check(section: str, payload) -> None:
    doc = Documented(documented_sections()[section])
    seen: set[str] = set()
    unknown: set[str] = set()
    observe(payload, "", doc, seen, unknown)
    assert not unknown, f"{section}: output carries keys docs/json.md does not list: {sorted(unknown)}"
    missing = []
    for path in doc.paths:
        if path in doc.optional or path in seen:
            continue
        # An empty array still carries its key; a placeholder names keys the
        # profile or register supplies, which a fixture may leave empty; and a
        # child of an optional or empty parent is only expected once the parent
        # was seen. A top-level path is always expected.
        if path.endswith("[]") and path[:-2] in seen:
            continue
        if path.rpartition(".")[2].startswith("<"):
            continue
        parents = ancestors(path)
        if parents and parents[0] not in seen:
            continue
        missing.append(path)
    assert not missing, f"{section}: docs/json.md lists keys the output does not carry: {missing}"


def run(args: list[str], cwd: Path = REPO_ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(CORE_BIN), *args], cwd=cwd, capture_output=True, text=True, timeout=300
    )


def payload_of(result: subprocess.CompletedProcess[str]):
    assert result.returncode in (0, 1), result.stderr
    return json.loads(result.stdout)


STDOUT_SECTIONS = {
    "validate": ["validate", *TOML, "--format", "json"],
    "trace": ["trace", *TOML, "--format", "json"],
    "summary": ["summary", *TOML, "--format", "json"],
    "coverage": ["coverage", *TOML, "--format", "json"],
    "describe": ["describe", *TOML, "--format", "json"],
    "query reaches, query reached-by": ["query", "reaches", "alpha/two", *TOML, "--format", "json"],
    "query path": ["query", "path", "alpha/two", "alpha", *TOML, "--format", "json"],
    "query orphans": ["query", "orphans", *TOML, "--format", "json"],
    "query counts": ["query", "counts", *TOML, "--format", "json"],
    "query at": ["query", "at", "registers/alpha.toml", *TOML, "--format", "json"],
    "fuse": ["fuse", "--manifest", "tests/fixtures/mini-fuse/fuse-native.yaml", "--format", "json"],
}


def test_every_section_on_the_page_is_exercised() -> None:
    sections = set(documented_sections())
    covered = set(STDOUT_SECTIONS) | {
        "summary with a SUMMARY block", "query diff", "Adapter findings on stderr",
    }
    assert sections == covered, (sections ^ covered)


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_opening_example_is_actual_output() -> None:
    # The excerpt is trimmed, never edited: each finding it shows is one the
    # stated command writes, value for value.
    text = PAGE.read_text(encoding="utf-8")
    assert "`latt validate " + " ".join(TOML) + " --format json`" in text
    example = json.loads(text.split("```json\n", 1)[1].split("```", 1)[0])
    live = payload_of(run(["validate", *TOML, "--format", "json"]))["findings"]
    assert example["findings"], "the opening example shows no findings"
    for finding in example["findings"]:
        assert finding in live, f"docs/json.md example is not in the output: {finding}"


@needs_core
@pytest.mark.parametrize("section", list(STDOUT_SECTIONS), ids=list(STDOUT_SECTIONS))
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_stdout_keys_match_the_page(section: str) -> None:
    check(section, payload_of(run(STDOUT_SECTIONS[section])))


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_reached_by_keys_match_the_page() -> None:
    check("query reaches, query reached-by",
          payload_of(run(["query", "reached-by", "alpha/two", *TOML, "--format", "json"])))


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_reach_direction_and_tainted_match_the_page() -> None:
    for direction in ("reaches", "reached-by"):
        payload = payload_of(run(["query", direction, "alpha/two", *TOML, "--check-resolved",
                                  "--format", "json"]))
        check("query reaches, query reached-by", payload)
        assert payload["direction"] == direction
        assert all(isinstance(n["tainted"], bool) for n in payload["nodes"]), payload


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_query_at_path_level_findings_match_the_page() -> None:
    payload = payload_of(run(["query", "at", "malformed.md", "--profile", "profiles/md.yaml",
                              "--target", "tests/fixtures/mini-md", "--format", "json"]))
    assert payload["findings"], "mini-md's malformed.md carries findings attached to no node"
    check("query at", payload)


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_configured_summary_keys_match_the_page(tmp_path: Path) -> None:
    profile = (REPO_ROOT / "profiles" / "md.yaml").read_text()
    profile += (
        "validations:\n"
        "  - SUMMARY:\n"
        "      node_kind: requirement\n"
        "      status_attr: status\n"
        "      group_by_attr: file\n"
    )
    (tmp_path / "p.yaml").write_text(profile)
    result = run(["summary", "--profile", str(tmp_path / "p.yaml"), "--adapter", "md",
                  "--target", "tests/fixtures/mini-md", "--format", "json"])
    check("summary with a SUMMARY block", payload_of(result))


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_stderr_envelope_keys_match_the_page() -> None:
    result = run(["query", "counts", *TOML, "--format", "json"])
    assert result.returncode == 0, result.stderr
    assert result.stderr.strip(), "mini-toml carries adapter findings"
    check("Adapter findings on stderr", json.loads(result.stderr))


def git(repo: Path, *args: str) -> None:
    env = {
        **os.environ,
        "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid",
        "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null",
    }
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, env=env)


@needs_core
# Requirement: development-verification/Verify documentation and supported feature configurations
def test_diff_keys_match_the_page(tmp_path: Path) -> None:
    """Two commits of ``mini-md``: one row changes, one is added with an edge."""
    repo = tmp_path / "repo"
    shutil.copytree(FIXTURES / "mini-md", repo)
    git(repo, "init", "-q")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "a")
    register = repo / "requirements.md"
    text = register.read_text()
    assert re.search(r"^\| REQ-1 \|", text, re.M), "mini-md declares REQ-1 in requirements.md"
    text = text.replace("| done |", "| draft |", 1)
    text += "| REQ-9 | Added later | draft | REQ-1 |\n"
    register.write_text(text)
    git(repo, "commit", "-qam", "b")
    result = run(["query", "diff", "HEAD~1", "HEAD", "--profile",
                  str(REPO_ROOT / "profiles" / "md.yaml"), "--adapter", "md",
                  "--target", str(repo), "--format", "json"])
    payload = payload_of(result)
    assert payload["nodes_added"] and payload["nodes_changed"] and payload["edges_added"], payload
    check("query diff", payload)

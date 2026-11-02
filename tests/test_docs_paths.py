"""The shipped docs name only files a release install holds.

An install carries the paths ``tools/install-files`` lists, so a docs page that
points at ``openspec/specs/…`` or ``tests/fixtures/…`` sends an installed reader
nowhere. Outside fenced blocks, the test fails on an inline code span that is
exactly a tracked repository path outside the install, and on a relative link
that leaves the install or names no file. A ``#fragment`` on a link to a
Markdown page, or on a bare ``#fragment``, must name a heading of that page,
slugged as GitHub and GitLab slug it. A link to this repository on GitHub
must pin the tag of the workspace version and name a tracked path, so it
resolves to the text the install shipped with; a link to a release page
must name that tag too.

``CHANGELOG.md`` is exempt: its entries record what changed in the repository.

Where a page used to point at a contract file instead, it now states the fact,
and the test holds the two together: ``docs/profiles.md`` lists the codes whose
findings always carry a node, which must be the ``node`` column of
``contracts/codes.tsv``; ``docs/adapters.md`` lists the interface document's keys,
which must be the adapter-contract spec's closed-key table.
"""
from __future__ import annotations

import re
import subprocess
import tomllib
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SHIPPED = [
    line for line in (REPO_ROOT / "tools" / "install-files").read_text().splitlines()
    if line and not line.startswith("#")
]
PAGES = [REPO_ROOT / "README.md", REPO_ROOT / "AGENTS.md", *sorted((REPO_ROOT / "docs").rglob("*.md")),
         *sorted((REPO_ROOT / "skills").rglob("*.md"))]
GITHUB_BLOB = re.compile(r"https://github\.com/jonelay/lattice/blob/([^/]+)/([^)#\s]+)")
GITHUB_RELEASE = re.compile(r"https://github\.com/jonelay/lattice/releases/tag/([^/)#\s]+)")
# Links an installed reader is not expected to follow, each with why.
EXEMPT: dict[tuple[str, str], str] = {
    ("README.md", "CONTRIBUTING.md"): "for contributors, who read it in a checkout or on GitHub;"
    " the install withholds it, and a tag-pinned URL goes stale on main",
}


def tracked() -> set[str]:
    out = subprocess.run(["git", "ls-files"], cwd=REPO_ROOT, check=True,
                         capture_output=True, text=True).stdout.split()
    paths = set(out)
    for path in out:
        parts = path.split("/")
        paths.update("/".join(parts[:i]) for i in range(1, len(parts)))
    return paths


TRACKED = tracked()


def slug(heading: str) -> str:
    """Return the anchor GitHub and GitLab give a heading's text."""
    text = re.sub(r"`|\*\*|\[([^\]]*)\]\([^)]*\)", r"\1", heading.strip().lower())
    return re.sub(r"[^\w\- ]", "", text).replace(" ", "-")


def anchors(text: str) -> set[str]:
    """Return the anchors a Markdown page's headings define, numbering repeats."""
    seen: dict[str, int] = {}
    out = set()
    for line in prose(text).splitlines():
        if m := re.match(r"#{1,6}\s+(.+?)\s*#*$", line):
            base = slug(m.group(1))
            n = seen.get(base, 0)
            seen[base] = n + 1
            out.add(base if n == 0 else f"{base}-{n}")
    return out


def shipped(rel: str) -> bool:
    """True when a repository-relative path lies inside the install."""
    return any(rel == s or rel.startswith(s + "/") for s in SHIPPED)


def prose(text: str) -> str:
    """Return the text with fenced blocks blanked, keeping line numbers."""
    out, fenced = [], False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            fenced = not fenced
            out.append("")
        else:
            out.append("" if fenced else line)
    return "\n".join(out)


def problems(page: Path, text: str | None = None) -> list[str]:
    rel_page = page.relative_to(REPO_ROOT).as_posix()
    version = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    text = page.read_text() if text is None else text
    found = []
    for n, line in enumerate(prose(text).splitlines(), 1):
        where = f"{rel_page}:{n}"
        spans = re.findall(r"(`+)(.+?)\1", line)
        for _, span in spans:
            path = span.strip().rstrip("/")
            if "/" in path and path in TRACKED and not shipped(path):
                found.append(f"{where}: `{span}` is not in an install")
        bare = re.sub(r"(`+).+?\1", "", line)
        for target in re.findall(r"\]\(([^)\s]+)\)", bare):
            if m := GITHUB_BLOB.match(target):
                ref, path = m.groups()
                if ref != f"v{version}":
                    found.append(f"{where}: {target} pins {ref}, not v{version}")
                if path not in TRACKED:
                    found.append(f"{where}: {target} names no tracked path")
                continue
            if m := GITHUB_RELEASE.match(target):
                if m.group(1) != f"v{version}":
                    found.append(f"{where}: {target} pins {m.group(1)}, not v{version}")
                continue
            if "://" in target or target.startswith("mailto:"):
                continue
            file, _, fragment = target.partition("#")
            if not file:
                if fragment not in anchors(text):
                    found.append(f"{where}: {target} names no heading on this page")
                continue
            if (rel_page, file) in EXEMPT:
                continue
            resolved = (page.parent / file).resolve()
            try:
                rel = resolved.relative_to(REPO_ROOT).as_posix()
            except ValueError:
                found.append(f"{where}: {target} leaves the repository")
                continue
            if not resolved.exists():
                found.append(f"{where}: {target} names no file")
            elif not shipped(rel):
                found.append(f"{where}: {target} is not in an install")
            elif fragment and resolved.suffix == ".md" and fragment not in anchors(resolved.read_text()):
                found.append(f"{where}: {target} names no heading in {rel}")
    return found


@pytest.mark.parametrize("page", PAGES, ids=lambda p: p.relative_to(REPO_ROOT).as_posix())
def test_shipped_page_names_only_installed_paths(page: Path) -> None:
    assert problems(page) == []


def test_the_guard_catches_an_unshipped_path() -> None:
    text = ("# Probe\n"
            "See `openspec/specs/validation/spec.md` and [x](../tests/conftest.py).\n"
            "Also [y](https://github.com/jonelay/lattice/blob/main/README.md)"
            " and [u](https://github.com/jonelay/lattice/releases/tag/v0.1.0).\n"
            "And [z](cli.md#no-such-heading), [w](#probe) and [v](#nowhere).\n")
    found = problems(REPO_ROOT / "docs" / "probe.md", text)
    assert len(found) == 6, found


def test_node_placeholder_codes_match_the_codes_contract() -> None:
    rows = [line.split("\t") for line in
            (REPO_ROOT / "contracts" / "codes.tsv").read_text().splitlines()]
    expected = sorted(r[0] for r in rows if len(r) == 4 and r[3] == "yes")
    text = (REPO_ROOT / "docs" / "profiles.md").read_text()
    m = re.search(r"The codes whose findings always carry a node are\n(.+?)\. ", text, re.S)
    assert m, "docs/profiles.md no longer lists the codes whose findings carry a node"
    assert sorted(re.findall(r"`([A-Z_]+)`", m.group(1))) == expected


def key_table(text: str, after: str) -> dict[str, set[str]]:
    """Return object → backticked keys from the first Object/Keys table after `after`."""
    rows: dict[str, set[str]] = {}
    for line in text.split(after, 1)[1].splitlines():
        if m := re.match(r"\| (document|node|edge|pathway|finding|provenance) \| (.+) \|$", line):
            rows[m.group(1)] = set(re.findall(r"`([a-z_]+)`", m.group(2)))
        elif rows and not line.startswith("|"):
            break
    return rows


def test_adapter_contract_keys_match_the_spec() -> None:
    spec = key_table((REPO_ROOT / "openspec/specs/adapter-contract/spec.md").read_text(),
                     "### Requirement: Document keys are closed")
    page = key_table((REPO_ROOT / "docs/adapters.md").read_text(), "### The contract")
    legacy = {"contract_version", "issues"}  # pre-1.2 names the page explains in prose
    assert page and page == {obj: keys - legacy for obj, keys in spec.items()}

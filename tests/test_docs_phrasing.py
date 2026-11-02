"""The shipped prose avoids three contrastive sentence shapes.

``<x> is not a <y>``, ``<x> is <y>, not <z>`` and ``<x>, never <y>.`` define a
thing by what it is not; the docs state what it is. The rules are the declaude
skill's banned shapes, carried here so the guard runs where the skill is not
installed. A sentence is matched whole across wrapped lines; fenced code, inline
code and quotations are left out, and ``negative-identity`` exempts a condition
("when stdout is not a TTY", "a value that is not a table").

The pages are the docs an install ships (with ``AGENTS.md`` and ``skills/``), ``CONTRIBUTING.md``, ``CLAUDE.md``, latt-eval's README, and the
unreleased part of ``CHANGELOG.md``: sections of a published release record what
was written then.
"""
from __future__ import annotations

import re
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
RELEASED = re.compile(r"^## \[0\.7\.1\]", re.M)

BANNED = [
    ("negative-identity",
     re.compile(r"\b(?:is|are|was|were)\s+not\s+an?\b|\b(?:isn't|aren't|wasn't|weren't)\s+an?\b", re.I)),
    ("not-contrast", re.compile(r"\b(?:is|are|was|were)\b[^.;:]*?,\s+not\b", re.I)),
    ("never-tail", re.compile(r",\s+never\b", re.I)),
]
CONDITION = re.compile(
    r"\b(?:that|which|who|whose|when|whenever|if|unless|whether|while|where)\b(?:\s+\S+)?\s*$", re.I)
SENTENCE_END = re.compile(r"(?<=[.!?])\s+(?=[A-Z`*\[(])")
QUOTED = re.compile(r"`[^`]*`|\"[^\"]*\"|“[^”]*”")


def pages() -> list[Path]:
    found = [REPO_ROOT / "README.md", REPO_ROOT / "AGENTS.md", REPO_ROOT / "CONTRIBUTING.md", REPO_ROOT / "CHANGELOG.md",
             REPO_ROOT / "CLAUDE.md", REPO_ROOT / "crates/latt-eval/README.md",
             *sorted((REPO_ROOT / "docs").rglob("*.md")),
             *sorted((REPO_ROOT / "skills").rglob("*.md"))]
    # The release snapshot withholds these two, and its CI runs this guard too.
    withheld = {REPO_ROOT / "CLAUDE.md", REPO_ROOT / "crates/latt-eval/README.md"}
    return [page for page in found if page not in withheld or page.exists()]


def prose(path: Path) -> str:
    """Return the page's text, cut at the first published release for the changelog."""
    text = path.read_text()
    if path.name == "CHANGELOG.md":
        text = text[:m.start()] if (m := RELEASED.search(text)) else text
    return text


def sentences(text: str):
    """Yield (first line number, sentence) for each sentence of each prose paragraph."""
    para: list[str] = []
    start, fenced = 0, False
    for n, line in enumerate([*text.splitlines(), ""], 1):
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            line = ""
        block = re.match(r"\s*(?:[-*+]\s|\d+[.)]\s|\|)", line)
        if fenced or not line.strip() or block:
            if para:
                for sent in SENTENCE_END.split(QUOTED.sub("X", " ".join(para))):
                    yield start, sent
            para = []
            if fenced or not line.strip():
                continue
        if not para:
            start = n
        para.append(line.strip())


def violations(text: str) -> list[tuple[int, str, str]]:
    out = []
    for n, sent in sentences(text):
        for name, rx in BANNED:
            m = rx.search(sent)
            if not m or (name == "negative-identity" and CONDITION.search(sent[:m.start()])):
                continue
            out.append((n, name, sent))
    return out


@pytest.mark.parametrize("text,expected", [
    ("A dangling endpoint is not a node.", ["negative-identity"]),
    ("It is a list of arguments,\nnever a shell string.", ["never-tail"]),
    ("Unknown keys are rejected, not ignored.", ["not-contrast"]),
    ("It exits 2 when stdout is not a TTY.", []),
    ("A value that is not a table is reported.", []),
    ("The `x is not a y` form, and \"a, never b\" in quotes.", []),
    ("```\nfoo is not a bar\n```", []),
    ("Unknown keys are rejected.", []),
    ("- It is a tool.\n- Keys are rejected, not ignored.\n- It is fine.", ["not-contrast"]),
])
def test_rules_match_known_sentences(text: str, expected: list[str]) -> None:
    assert [name for _, name, _ in violations(text)] == expected


@pytest.mark.parametrize("page", pages(), ids=lambda p: str(p.relative_to(REPO_ROOT)))
def test_page_avoids_banned_shapes(page: Path) -> None:
    found = [f"{page.relative_to(REPO_ROOT)}:{n} {name}: {sent[:120]}"
             for n, name, sent in violations(prose(page))]
    assert not found, "\n".join(found)

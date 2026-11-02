from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from conftest import resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "prose-xref.yaml"
ADAPTER_BIN = REPO_ROOT / "adapters" / "prose-xref"
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "mini-prose-xref"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


def _run_adapter(target: Path, tmp_path: Path) -> dict:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
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
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


@pytest.fixture(scope="module")
def document(tmp_path_factory) -> dict:
    return _run_adapter(FIXTURE, tmp_path_factory.mktemp("prose-xref-profile"))


def _core_validate(profile: Path, target: Path) -> tuple[int, list[dict]]:
    result = subprocess.run(
        [
            str(CORE_BIN),
            "validate",
            "--profile",
            str(profile),
            "--adapter",
            str(ADAPTER_BIN),
            "--target",
            str(target),
            "--format",
            "json",
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode in (0, 1), result.stderr
    return result.returncode, json.loads(result.stdout)["findings"]


def _fixture_copy(tmp_path: Path) -> Path:
    target = tmp_path / "target"
    shutil.copytree(FIXTURE, target)
    return target


def _nodes(document: dict) -> dict[str, dict]:
    return {node["id"]: node for node in document["nodes"]}


def _edges(document: dict) -> list[dict]:
    return document["edges"]


def _findings(document: dict) -> list[dict]:
    return document["findings"]


# --- Structure ---


def test_interface_version(document):
    assert document["interface_version"] == "1.4"


def test_document_nodes(document):
    nodes = _nodes(document)
    doc_nodes = {nid for nid, n in nodes.items() if n["kind"] == "document"}
    assert "characters/alice.md" in doc_nodes
    assert "characters/bob.md" in doc_nodes
    assert "outline/structure.md" in doc_nodes
    assert "ref/notes.md" in doc_nodes


def test_section_nodes(document):
    nodes = _nodes(document)
    section_ids = {nid for nid, n in nodes.items() if n["kind"] == "section"}
    assert "characters/alice.md#Alice::Background" in section_ids
    assert "characters/alice.md#Alice::Relationships" in section_ids
    assert "characters/bob.md#Bob::Personality" in section_ids
    assert "outline/structure.md#Story Structure::Act I" in section_ids


def test_duplicate_headings_under_different_parents(document):
    """Headings with the same text under different parents get distinct node IDs."""
    nodes = _nodes(document)
    section_ids = {nid for nid, n in nodes.items() if n["kind"] == "section"}
    assert "characters/bob.md#Bob::Early Life::Motivation" in section_ids
    assert "characters/bob.md#Bob::Career::Motivation" in section_ids
    # Both exist — no duplicate-node collision.
    motivation_nodes = [nid for nid in section_ids if nid.endswith("::Motivation")]
    assert len(motivation_nodes) == 2


def test_fenced_code_headings_excluded(document):
    nodes = _nodes(document)
    section_ids = {nid for nid, n in nodes.items() if n["kind"] == "section"}
    assert not any("Not A Heading" in sid for sid in section_ids)


# --- Edges ---


def test_cross_file_xref(document):
    """alice.md → bob.md#Bob::Personality."""
    edges = _edges(document)
    cross = [e for e in edges if e["tgt"] == "characters/bob.md#Bob::Personality"]
    assert len(cross) == 1
    assert cross[0]["kind"] == "xref"


def test_self_ref_xref(document):
    """alice.md § Background → alice.md#Alice::Background."""
    edges = _edges(document)
    self_refs = [
        e for e in edges
        if e["tgt"] == "characters/alice.md#Alice::Background"
        and "characters/alice.md" in e["src"]
    ]
    assert len(self_refs) == 1


def test_carry_forward_xref(document):
    """alice.md: `outline/structure.md` § `Act I` and § `Act II`.

    The second § has no file prefix — it should carry forward to
    outline/structure.md.
    """
    edges = _edges(document)
    act_ii = [e for e in edges if e["tgt"] == "outline/structure.md#Story Structure::Act II"]
    assert len(act_ii) == 1


def test_valid_crossref_from_structure(document):
    """structure.md → alice.md#Alice::Relationships."""
    edges = _edges(document)
    cross = [e for e in edges if e["tgt"] == "characters/alice.md#Alice::Relationships"]
    assert len(cross) == 1


def test_edge_count_nonzero(document):
    """Scope check: adapter found a nontrivial number of edges."""
    assert len(_edges(document)) >= 4


# --- Findings ---


def test_ambiguous_ref(document):
    findings = _findings(document)
    ambiguous = [f for f in findings if f["code"] == "AMBIGUOUS_REF"]
    assert len(ambiguous) == 1
    assert "Duplicate Section" in ambiguous[0]["message"]


def test_bare_undelimited_parse_error(document):
    findings = _findings(document)
    parse_errors = [f for f in findings if f["code"] == "PARSE_ERROR"]
    bare = [p for p in parse_errors if "bare §" in p["message"].lower()]
    assert len(bare) == 1


def test_unresolved_ref_becomes_edge(document):
    """A ref to a nonexistent heading emits an edge (the core fires VACANCY)."""
    edges = _edges(document)
    unresolved = [
        e for e in edges
        if e["tgt"] == "characters/alice.md#Nonexistent Heading"
    ]
    assert len(unresolved) == 1


# --- Unreadable input ---


def test_invalid_utf8_line_is_reported_and_the_rest_of_the_file_read(tmp_path):
    target = _fixture_copy(tmp_path)
    (target / "ref" / "bad.md").write_bytes(b"# Bad\n\xff\xfe\n## Gone caf\xe9\n")
    (target / "ref" / "pointer.md").write_text(
        "# Pointer\n\nSee `ref/bad.md` \u00a7 `Bad` here.\n"
    )
    document = _run_adapter(target, tmp_path)
    findings = _findings(document)
    unreadable = [f for f in findings if f["provenance"]["file"] == "ref/bad.md"]
    assert [(f["code"], f["provenance"]["line"]) for f in unreadable] == [
        ("PARSE_ERROR", 2), ("PARSE_ERROR", 3)]
    assert r"so it was not read: ## Gone caf\xE9" in unreadable[1]["message"]
    assert "ref/bad.md" in _nodes(document)
    assert "characters/alice.md" in _nodes(document)
    assert not any("Gone" in node for node in _nodes(document))
    pointer = [f for f in findings if f["provenance"]["file"] == "ref/pointer.md"]
    assert pointer == []


def test_dangling_symlink_matching_a_glob_is_reported(tmp_path):
    target = _fixture_copy(tmp_path)
    (target / "ref" / "gone.md").symlink_to("missing.md")
    (target / "ref" / "gone.txt").symlink_to("missing.txt")
    (target / "ref" / "pointer.md").write_text(
        "# Pointer\n\nSee `ref/gone.md` § `Gone` here.\n"
    )
    document = _run_adapter(target, tmp_path)
    findings = _findings(document)
    dangling = [f for f in findings if f["provenance"]["file"].startswith("ref/gone")]
    assert [(f["code"], f["provenance"]["file"]) for f in dangling] == [
        ("PARSE_ERROR", "ref/gone.md")
    ]
    assert "characters/alice.md" in _nodes(document)
    pointer = [f for f in findings if f["provenance"]["file"] == "ref/pointer.md"]
    assert [f["message"] for f in pointer] == ["target file unreadable: ref/gone.md"]


def test_non_utf8_path_matching_a_glob_is_reported(tmp_path):
    target = _fixture_copy(tmp_path)
    for name in (b"bad\xff.md", b"bad\xfe.md"):
        (target / "ref" / os.fsdecode(name)).write_text("# Bad\n")
    document = _run_adapter(target, tmp_path)
    bad = [
        f for f in _findings(document) if f["provenance"]["file"] == "ref/bad\ufffd.md"
    ]
    assert [f["code"] for f in bad] == ["PARSE_ERROR", "PARSE_ERROR"]
    assert all("path is not valid UTF-8" in f["message"] for f in bad)
    assert ["\\xFE" in bad[0]["message"], "\\xFF" in bad[1]["message"]] == [True, True]
    assert "ref/bad\ufffd.md" not in _nodes(document)
    assert "characters/alice.md" in _nodes(document)


def test_unclosed_fence_is_reported(tmp_path):
    target = _fixture_copy(tmp_path)
    (target / "ref" / "open.md").write_text(
        "# Open\n\n- example:\n\n    ```\n    See § `Open`\n\n# Hidden\n"
    )
    document = _run_adapter(target, tmp_path)
    unclosed = [f for f in _findings(document) if f["provenance"]["file"] == "ref/open.md"]
    assert [(f["code"], f["provenance"]["line"]) for f in unclosed] == [("PARSE_ERROR", 5)]
    assert "never closed" in unclosed[0]["message"]
    assert "ref/open.md#Open" in _nodes(document)
    assert "ref/open.md#Hidden" not in _nodes(document)


def test_missing_content_directory_is_reported(tmp_path):
    target = _fixture_copy(tmp_path)
    shutil.rmtree(target / "worldbuilding")
    document = _run_adapter(target, tmp_path)
    missing = [f for f in _findings(document) if f["provenance"]["file"] == "worldbuilding"]
    assert len(missing) == 1
    assert missing[0]["code"] == "PARSE_ERROR"
    assert missing[0]["message"] == (
        "content directory 'worldbuilding' does not exist or is not a directory"
    )
    assert "characters/alice.md" in _nodes(document)
    assert "outline/structure.md" in _nodes(document)


@pytest.mark.parametrize("kind", ["missing", "file"])
def test_non_directory_target_is_one_finding(tmp_path, kind):
    target = tmp_path / "target"
    if kind == "file":
        target.write_text("# Not a directory\n")
    document = _run_adapter(target, tmp_path)
    findings = _findings(document)
    assert len(findings) == 1
    assert findings[0]["code"] == "PARSE_ERROR"
    assert "is not a directory" in findings[0]["message"]
    assert document["nodes"] == []


@needs_core
def test_missing_directory_severity_follows_profile_override(tmp_path):
    target = tmp_path / "target"
    (target / "characters").mkdir(parents=True)
    (target / "characters" / "a.md").write_text("# A\n\nPlain text.\n")

    code, findings = _core_validate(PROFILE_PATH, target)
    assert code == 1
    assert sorted(f["file"] for f in findings if f["code"] == "PARSE_ERROR") == [
        "outline", "ref", "worldbuilding",
    ]

    profile = yaml.safe_load(PROFILE_PATH.read_text())
    profile["validations"] = [{"PARSE_ERROR": {"severity": "warning"}}]
    downgraded = tmp_path / "downgraded.yaml"
    downgraded.write_text(yaml.safe_dump(profile))
    code, findings = _core_validate(downgraded, target)
    assert code == 0
    parse_errors = [f for f in findings if f["code"] == "PARSE_ERROR"]
    assert len(parse_errors) == 3
    assert {f["severity"] for f in parse_errors} == {"warning"}


# --- Heading matching ---


def _matching_target(tmp_path: Path) -> Path:
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "ref" / "stages.md").write_text(
        "# Stages\n\n"
        "## *Emphasized*\n\n"
        "## Plain\n\n"
        "## Stage 8: The Mixing Frontier [working]\n\n"
        "## Stage 9 (draft)\n\n"
        "## Stage 10 [held]\n\n"
        "## Stage 11 — Close\n\n"
        "## Twin: One\n\n"
        "## Twin (Two)\n\n"
        "## Refs\n\n"
        "See \u00a7 `Emphasized` and \u00a7 `**Plain**`.\n\n"
        "See \u00a7 `Stage 8`, \u00a7 `Stage 9`, \u00a7 `Stage 10` and \u00a7 `Stage 11`.\n\n"
        "See \u00a7 `Twin`.\n\n"
        "See \u00a7 `Stage`.\n"
    )
    return target


def test_reference_matches_heading_without_emphasis(tmp_path):
    targets = [e["tgt"] for e in _edges(_run_adapter(_matching_target(tmp_path), tmp_path))]
    assert "ref/stages.md#Stages::*Emphasized*" in targets
    assert "ref/stages.md#Stages::Plain" in targets


def test_reference_matches_heading_prefix_before_qualifier(tmp_path):
    targets = [e["tgt"] for e in _edges(_run_adapter(_matching_target(tmp_path), tmp_path))]
    for heading in (
        "Stage 8: The Mixing Frontier [working]",
        "Stage 9 (draft)",
        "Stage 10 [held]",
        "Stage 11 — Close",
    ):
        assert f"ref/stages.md#Stages::{heading}" in targets
    # A prefix that stops short of a separator is not a match.
    assert "ref/stages.md#Stage" in targets


def test_two_qualified_candidates_are_ambiguous(tmp_path):
    document = _run_adapter(_matching_target(tmp_path), tmp_path)
    ambiguous = [f for f in _findings(document) if f["code"] == "AMBIGUOUS_REF"]
    assert [f["message"] for f in ambiguous] == [
        "heading 'Twin' matches 2 sections in ref/stages.md"
    ]
    assert not any("Twin" in e["tgt"] for e in _edges(document))


def test_exact_heading_beats_qualified_candidates(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "ref" / "arcs.md").write_text(
        "# Arcs\n\n"
        "## Arc\n\n"
        "## Arc: One\n\n"
        "## Arc (Two)\n\n"
        "## Effects\n\n"
        "## Effects [working]\n\n"
        "## Left\n\n### Same\n\n"
        "## Right\n\n### Same\n\n"
        "## Refs\n\n"
        "See § `Arc`, § `Effects` and § `Same`.\n"
    )
    document = _run_adapter(target, tmp_path)
    targets = [e["tgt"] for e in _edges(document)]
    assert "ref/arcs.md#Arcs::Arc" in targets
    assert "ref/arcs.md#Arcs::Effects" in targets
    ambiguous = [f["message"] for f in _findings(document) if f["code"] == "AMBIGUOUS_REF"]
    assert ambiguous == ["heading 'Same' matches 2 sections in ref/arcs.md"]


# --- Profile ---


@needs_core
def test_shipped_profile_section_ids(tmp_path):
    """The shipped profile loads through the core and fits section IDs."""
    profile = yaml.safe_load(PROFILE_PATH.read_text())
    assert profile["profile_version"] == "1.1.0"
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "characters" / "a.md").write_text(
        "# Top\n\n## C# Notes\n\n### Issue #42\n"
    )
    (target / "ref" / "x#y.md").write_text("# Hash File\n")
    code, findings = _core_validate(PROFILE_PATH, target)
    id_format = sorted(f["node_id"] for f in findings if f["code"] == "ID_FORMAT")
    assert id_format == ["ref/x#y.md#Hash File"], findings
    assert code == 1
    # The fixture writes no `§` reference, so the declared edge kind is empty.
    assert [f for f in findings if f["code"] not in ("ID_FORMAT", "EMPTY_KIND")] == []


# --- Gate test (end-to-end through the core) ---


def test_sibling_duplicate_headings_are_two_node_records(document):
    """Sibling headings with the same text are two records; the core resolves them."""
    duplicates = [
        n for n in document["nodes"]
        if n["id"] == "characters/bob.md#Bob::Duplicate Section"
    ]
    assert [n["provenance"]["line"] for n in duplicates] == [7, 11]


GATE_FINDINGS = sorted([
    (
        "PARSE_ERROR", "characters/bob.md", 11,
        "duplicate node 'characters/bob.md#Bob::Duplicate Section': "
        "first at characters/bob.md:7",
        "characters/bob.md#Bob::Duplicate Section", "error",
    ),
    (
        "VACANCY", "outline/structure.md", 19,
        "edge 'outline/structure.md#Story Structure::References'->"
        "'characters/alice.md#Nonexistent Heading' (kind 'xref'): "
        "target 'characters/alice.md#Nonexistent Heading' does not exist",
        "outline/structure.md#Story Structure::References", "error",
    ),
    (
        "AMBIGUOUS_REF", "outline/structure.md", 21,
        "heading 'Duplicate Section' matches 2 sections in characters/bob.md",
        None, "warning",
    ),
    (
        "PARSE_ERROR", "outline/structure.md", 23,
        "bare \u00a7 without backtick delimiters: \u00a7 Bare Reference here",
        None, "error",
    ),
], key=repr)


@needs_core
def test_gate(document):
    """Run `latt validate` on the fixture: exactly the expected findings."""
    # Zero findings over zero edges would pass vacuously; the fixture has edges.
    assert len(_edges(document)) >= 5
    code, findings = _core_validate(PROFILE_PATH, FIXTURE)
    assert code == 1
    actual = sorted(
        (
            (f["code"], f["file"], f["line"], f["message"], f["node_id"], f["severity"])
            for f in findings
        ),
        key=repr,
    )
    assert actual == GATE_FINDINGS


def test_carry_forward_after_unresolved_file_is_not_retargeted(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "ref" / "a.md").write_text("# A\n\n## X\n\n## Z\n")
    (target / "ref" / "notes.md").write_text(
        "# Notes\n\n"
        "See `a.md` § `X`.\n\n"
        "See `gone.md` § `Y` / § `Z`.\n"
    )
    document = _run_adapter(target, tmp_path)
    assert "ref/a.md#A::Z" not in {e["tgt"] for e in _edges(document)}
    missing = [
        f["message"] for f in _findings(document)
        if f["code"] == "PARSE_ERROR" and f["provenance"]["line"] == 5
    ]
    assert missing == ["target file not found: gone.md"] * 2


def test_wrapped_reference_names_the_path_on_the_previous_line(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "ref" / "a.md").write_text("# A\n\n## X\n")
    (target / "ref" / "notes.md").write_text(
        "# Notes\n\n"
        "Reflowed: `a.md`\n"
        "\u00a7 `X`.\n\n"
        "Missing: \u00a7 `Y`.\n"
    )
    document = _run_adapter(target, tmp_path)
    # The edge stays, so the core still reports the VACANCY it names.
    assert "ref/notes.md#X" in {e["tgt"] for e in _edges(document)}
    wrapped = [f for f in _findings(document) if f["code"] == "WRAPPED_REF"]
    assert [(f["severity"], f["provenance"]["line"]) for f in wrapped] == [("info", 4)]
    assert "`a.md`" in wrapped[0]["message"]
    assert "previous line" in wrapped[0]["message"]


def test_a_markdown_link_names_the_file_of_the_reference(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "ref" / "a.md").write_text("# A\n\n## Alpha\n\n## Beta\n")
    (target / "ref" / "notes.md").write_text(
        "# Notes\n\n## Alpha\n\n"
        "See [a](a.md) § `Alpha` and § `Beta`.\n"
        "Also [the A file](./a.md#alpha) §`Alpha`.\n"
    )
    document = _run_adapter(target, tmp_path)
    assert _findings(document) == []
    assert [(e["tgt"], e["provenance"]["line"]) for e in _edges(document)] == [
        ("ref/a.md#A::Alpha", 5),
        ("ref/a.md#A::Beta", 5),
        ("ref/a.md#A::Alpha", 6),
    ]


def test_a_double_section_sign_reads_every_listed_heading(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "ref" / "a.md").write_text("# A\n\n## Alpha\n\n## Beta\n\n## Gamma\n")
    (target / "ref" / "notes.md").write_text(
        "# Notes\n\nSee [a](a.md) §§ `Alpha`, `Beta` and `Gamma`.\n"
    )
    document = _run_adapter(target, tmp_path)
    assert _findings(document) == []
    assert [e["tgt"] for e in _edges(document)] == [
        "ref/a.md#A::Alpha",
        "ref/a.md#A::Beta",
        "ref/a.md#A::Gamma",
    ]


def test_a_leading_byte_order_mark_is_encoding_not_content(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE, target)
    plain = _run_adapter(target, tmp_path)
    for path in target.rglob("*.md"):
        path.write_bytes(b"\xef\xbb\xbf" + path.read_bytes())
    assert _run_adapter(target, tmp_path) == plain


# --- Nothing matched, and paths that leave the register ---


@pytest.mark.parametrize("contents", ["empty", "other-extension"])
def test_no_matching_files_is_an_info_finding_naming_the_patterns(tmp_path, contents):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    if contents == "other-extension":
        (target / "ref" / "notes.markdown").write_text("# Notes\n")
    document = _run_adapter(target, tmp_path)
    assert document["nodes"] == []
    [finding] = _findings(document)
    assert (finding["code"], finding["severity"]) == ("NO_MATCHING_FILES", "info")
    message = finding["message"]
    assert "**/*.md" in message and "worldbuilding" in message, message


def test_a_path_past_the_register_root_is_outside_the_register(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    # A real file outside the target: reaching it would read past the register.
    (tmp_path / "outside.md").write_text("# Outside\n\n## X\n")
    (target / "ref" / "notes.md").write_text(
        "# Notes\n\n"
        "See `../../outside.md` § `X`.\n\n"
        "See `../../../outside.md` § `X`.\n"
    )
    document = _run_adapter(target, tmp_path)
    assert _edges(document) == []
    messages = [
        f["message"] for f in _findings(document) if f["code"] == "PARSE_ERROR"
    ]
    assert messages == [
        "target outside the register: ../../outside.md",
        "target outside the register: ../../../outside.md",
    ]


def test_a_path_that_climbs_and_returns_still_resolves(tmp_path):
    target = tmp_path / "target"
    for name in ("characters", "outline", "ref", "worldbuilding"):
        (target / name).mkdir(parents=True)
    (target / "outline" / "o.md").write_text("# O\n\n## X\n")
    (target / "ref" / "notes.md").write_text("# Notes\n\nSee `../outline/o.md` § `X`.\n")
    document = _run_adapter(target, tmp_path)
    assert [e["tgt"] for e in _edges(document)] == ["outline/o.md#O::X"]
    assert _findings(document) == []


# Requirement: adapter-contract/Resolved profile document
def test_a_profile_without_the_resolved_marker_exits_2(tmp_path: Path) -> None:
    """A marker-less document is named as unresolved, not as a missing serde field."""
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    del profile["resolved_schema"]
    resolved.write_text(json.dumps(profile))
    result = subprocess.run(
        [str(ADAPTER_BIN), "--profile", str(resolved), "--target", str(tmp_path)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2, result.stdout
    expected = f"could not load profile {resolved}: not a resolved profile document"
    assert expected in result.stderr, result.stderr


# Requirement: adapter-contract/Document order is significant
@needs_core
def test_a_repeated_sections_xrefs_are_not_read(tmp_path: Path) -> None:
    target = tmp_path / "target"
    (target / "ref").mkdir(parents=True)
    (target / "ref" / "a.md").write_text(
        "# Top\n\n## Same\n\nSee § `Other`.\n\n## Same\n\nSee § `Third`.\n"
        "\n## Other\n\n## Third\n"
    )
    document = _run_adapter(target, tmp_path)
    sections = [node["id"] for node in document["nodes"]]
    by_target = {edge["tgt"]: edge["declared_by"] for edge in document["edges"]}
    assert sections[by_target["ref/a.md#Top::Other"]] == "ref/a.md#Top::Same"
    assert by_target["ref/a.md#Top::Third"] == sections.index("ref/a.md#Top::Same", 3)

    result = subprocess.run(
        [str(CORE_BIN), "trace", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_BIN), "--target", str(target),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode != 2, result.stderr
    entries = {e["id"]: e for e in json.loads(result.stdout)["entries"]}
    same = entries["ref/a.md#Top::Same"]
    assert [edge["tgt"] for edge in same["edges"]] == ["ref/a.md#Top::Other"]
    assert any(
        finding["message"].endswith("; its 1 edge was not read")
        for finding in same["findings"]
    ), same["findings"]

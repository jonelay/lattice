from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "md.yaml"
MULTI_PROFILE_PATH = REPO_ROOT / "profiles" / "md-multi.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "md"
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"
FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-md"
MULTI_FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-md-multi"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


def _run_adapter(target: Path, tmp_path: Path) -> dict:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    return _run_adapter_with_profile(target, resolved)


def _run_adapter_with_profile(target: Path, profile: Path) -> dict:
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(profile),
         "--target", str(target)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


@pytest.fixture
def document(tmp_path: Path) -> dict:
    return _run_adapter(FIXTURE_PATH, tmp_path)


@pytest.fixture
def multikind_document(tmp_path: Path) -> dict:
    resolved = resolve_profile(MULTI_PROFILE_PATH, tmp_path)
    return _run_adapter_with_profile(MULTI_FIXTURE_PATH, resolved)


def test_valid_nodes_have_profile_mapped_attributes(document: dict) -> None:
    nodes = {node["id"]: node for node in document["nodes"]}
    assert set(nodes) == {"REQ-1", "REQ-2", "REQ-3"}
    assert nodes["REQ-1"]["kind"] == "requirement"
    assert nodes["REQ-2"]["attrs"] == {
        "summary": "Preserve source provenance",
        "status": "active",
    }


def test_edge_columns_emit_comma_separated_references(document: dict) -> None:
    assert {
        (edge["src"], edge["tgt"], edge["kind"])
        for edge in document["edges"]
    } == {
        ("REQ-2", "REQ-1", "traces_to"),
        ("REQ-3", "REQ-1", "traces_to"),
        ("REQ-3", "REQ-2", "traces_to"),
    }


def test_short_rows_are_reported(document: dict) -> None:
    errors = [
        issue for issue in document["findings"]
        if issue["code"] == "PARSE_ERROR" and issue["provenance"]["line"] == 5
    ]
    assert len(errors) == 1
    assert "fewer columns" in errors[0]["message"]


def test_empty_ids_are_reported(document: dict) -> None:
    errors = [
        issue for issue in document["findings"]
        if issue["code"] == "PARSE_ERROR" and issue["provenance"]["line"] == 6
    ]
    assert len(errors) == 1
    assert "empty ID" in errors[0]["message"]


def test_provenance_has_source_line_numbers(document: dict) -> None:
    nodes = {node["id"]: node for node in document["nodes"]}
    assert nodes["REQ-1"]["provenance"] == {
        "file": "requirements.md",
        "line": 5,
    }
    req3_edges = [edge for edge in document["edges"] if edge["src"] == "REQ-3"]
    assert {edge["provenance"]["line"] for edge in req3_edges} == {7}


def test_interface_version_is_1_4(document: dict) -> None:
    assert document["interface_version"] == "1.4"


def _utf8_errors(document: dict, file: str) -> list[dict]:
    return [
        issue for issue in document["findings"]
        if issue["code"] == "PARSE_ERROR" and issue["provenance"]["file"] == file
        and "UTF-8" in issue["message"]
    ]


def test_non_utf8_file_is_reported_and_other_files_survive(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "invalid.md").write_bytes(b"\xff\xfe not UTF-8")
    document = _run_adapter(target, tmp_path)
    errors = _utf8_errors(document, "invalid.md")
    assert len(errors) == 1
    assert errors[0]["provenance"]["line"] == 1
    assert {node["id"] for node in document["nodes"]} == {
        "REQ-1", "REQ-2", "REQ-3"
    }


def test_non_utf8_row_withholds_only_itself_shown_as_written(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    path = target / "requirements.md"
    path.write_bytes(path.read_bytes().replace(
        b"| REQ-2 | Preserve source", b"| REQ-2 | Pr\xE9serve source"))
    document = _run_adapter(target, tmp_path)
    errors = _utf8_errors(document, "requirements.md")
    assert len(errors) == 1
    assert errors[0]["provenance"]["line"] == 6
    assert "column 13" in errors[0]["message"]
    assert r"| REQ-2 | Pr\xE9serve source provenance | active | REQ-1 |" in errors[0]["message"]
    assert {node["id"] for node in document["nodes"]} == {"REQ-1", "REQ-3"}


def test_non_utf8_table_header_withholds_its_table_and_says_so(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    path = target / "requirements.md"
    path.write_bytes(path.read_bytes().replace(b"| Description |", b"| Descr\xedption |", 1))
    document = _run_adapter(target, tmp_path)
    lines = sorted(e["provenance"]["line"] for e in _utf8_errors(document, "requirements.md"))
    assert lines == [3, 3]
    withheld = [e for e in _utf8_errors(document, "requirements.md") if "3 rows" in e["message"]]
    assert len(withheld) == 1
    assert not {"REQ-1", "REQ-2", "REQ-3"} & {node["id"] for node in document["nodes"]}


def test_non_utf8_heading_withholds_the_tables_it_dispatches(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(MULTI_FIXTURE_PATH, target)
    path = target / "register.md"
    path.write_bytes(path.read_bytes().replace(b"## Domain 01", b"## Doma\xefn 01"))
    resolved = resolve_profile(MULTI_PROFILE_PATH, tmp_path)
    document = _run_adapter_with_profile(target, resolved)
    errors = _utf8_errors(document, "register.md")
    assert [e["provenance"]["line"] for e in errors] == [3, 5]
    assert "heading at line 3" in errors[1]["message"]
    ids = {node["id"] for node in document["nodes"]}
    assert "REQ-101" not in ids
    assert "REQ-201" in ids


def test_dangling_symlink_matching_a_glob_is_reported(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "gone.md").symlink_to("missing.md")
    (target / "gone.txt").symlink_to("missing.txt")
    document = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in document["findings"]
        if issue["provenance"]["file"].startswith("gone")
    ]
    assert [(issue["code"], issue["provenance"]["file"]) for issue in errors] == [
        ("PARSE_ERROR", "gone.md")
    ]
    assert {node["id"] for node in document["nodes"]} == {
        "REQ-1", "REQ-2", "REQ-3"
    }


def test_non_utf8_path_matching_a_glob_is_reported(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    for name in (b"bad\xff.md", b"bad\xfe.md"):
        (target / os.fsdecode(name)).write_text("# Empty\n")
    document = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in document["findings"]
        if issue["provenance"]["file"] == "bad\ufffd.md"
    ]
    assert [issue["code"] for issue in errors] == ["PARSE_ERROR", "PARSE_ERROR"]
    assert all("path is not valid UTF-8" in issue["message"] for issue in errors)
    assert ["\\xFE" in errors[0]["message"], "\\xFF" in errors[1]["message"]] == [True, True]
    assert {node["id"] for node in document["nodes"]} == {
        "REQ-1", "REQ-2", "REQ-3"
    }


def test_backslash_in_a_file_name_is_matched(tmp_path: Path) -> None:
    target = tmp_path / "register"
    target.mkdir()
    table = "| ID | Title |\n|----|-------|\n| REQ-9 | Slashed |\n"
    (target / "back\\slash.md").write_text(table)
    for run in (target, target / "back\\slash.md"):
        document = _run_adapter(run, tmp_path)
        assert [node["id"] for node in document["nodes"]] == ["REQ-9"]
        assert [n["provenance"]["file"] for n in document["nodes"]] == ["back\\slash.md"]


def test_separator_of_another_width_is_reported(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "narrow.md").write_text(
        "| ID | Title | Parent |\n|---|---|\n| REQ-9 | lost | |\n"
    )
    document = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in document["findings"]
        if issue["provenance"]["file"] == "narrow.md"
    ]
    assert [(e["code"], e["provenance"]["line"]) for e in errors] == [
        ("PARSE_ERROR", 1)
    ]
    assert "3 columns but its separator row has 2" in errors[0]["message"]
    assert "REQ-9" not in {node["id"] for node in document["nodes"]}


def test_edge_columns_do_not_split_on_a_comma_inside_a_link(tmp_path: Path) -> None:
    target = tmp_path / "register"
    target.mkdir()
    (target / "links.md").write_text(
        "| ID | Description | Status | Traces To |\n|---|---|---|---|\n"
        "| REQ-1 | a | done | [REQ-2, REQ-3](http://x/a,b), REQ-4 |\n"
    )
    document = _run_adapter(target, tmp_path)
    assert document["findings"] == []
    assert [edge["tgt"] for edge in document["edges"]] == [
        "[REQ-2, REQ-3](http://x/a,b)",
        "REQ-4",
    ]


def _emphasis_document(tmp_path: Path, strip: bool | None) -> dict:
    target = tmp_path / "register"
    target.mkdir()
    (target / "asvs.md").write_text(
        "| ID | Description | Status | Traces To |\n|---|---|---|---|\n"
        "| **1.1.1** | **bold** | done | **1.1.2**, `1.1.3`, [1.1.4](x.md) |\n"
        "| **`1.1.2`** | b | done | |\n"
        "| **** | c | done | |\n"
    )
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    data = json.loads(resolved.read_text())
    if strip is not None:
        data["adapter"]["table"]["strip_emphasis"] = strip
    resolved.write_text(json.dumps(data))
    return _run_adapter_with_profile(target, resolved)


def test_strip_emphasis_strips_ids_and_targets_and_keeps_attrs(tmp_path: Path) -> None:
    document = _emphasis_document(tmp_path, True)
    assert [node["id"] for node in document["nodes"]] == ["1.1.1", "1.1.2", "****"]
    assert document["nodes"][0]["attrs"]["summary"] == "**bold**"
    assert [edge["tgt"] for edge in document["edges"]] == ["1.1.2", "1.1.3", "[1.1.4](x.md)"]


@pytest.mark.parametrize("strip", [None, False])
def test_strip_emphasis_off_keeps_cells_as_written(tmp_path: Path, strip: bool | None) -> None:
    document = _emphasis_document(tmp_path, strip)
    assert [node["id"] for node in document["nodes"]] == ["**1.1.1**", "**`1.1.2`**", "****"]
    assert [edge["tgt"] for edge in document["edges"]] == [
        "**1.1.2**",
        "`1.1.3`",
        "[1.1.4](x.md)",
    ]


def test_a_blockquoted_table_is_read(tmp_path: Path) -> None:
    target = tmp_path / "register"
    target.mkdir()
    (target / "quoted.md").write_text(
        "> **Note**\n>\n> | ID | Description | Status | Traces To |\n> |---|---|---|---|\n"
        "> | REQ-1 | a | done | |\n> | REQ-2 | b | done | REQ-1 |\n"
    )
    document = _run_adapter(target, tmp_path)
    assert document["findings"] == []
    assert [(n["id"], n["provenance"]["line"]) for n in document["nodes"]] == [
        ("REQ-1", 5), ("REQ-2", 6)
    ]
    assert [(e["src"], e["tgt"]) for e in document["edges"]] == [("REQ-2", "REQ-1")]


def test_fenced_tables_are_not_read_and_an_unclosed_fence_is_reported(
    tmp_path: Path,
) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "fenced.md").write_text(
        "```\n| ID | Description | Status | Traces To |\n|---|---|---|---|\n"
        "| REQ-80 | example | draft | |\n```\n\n"
        "```\n| ID | Description | Status | Traces To |\n|---|---|---|---|\n"
        "| REQ-81 | swallowed | draft | |\n"
    )
    document = _run_adapter(target, tmp_path)
    ids = {node["id"] for node in document["nodes"]}
    assert not {"REQ-80", "REQ-81"} & ids
    errors = [
        issue for issue in document["findings"]
        if issue["provenance"]["file"] == "fenced.md"
    ]
    assert [(e["code"], e["provenance"]["line"]) for e in errors] == [
        ("PARSE_ERROR", 7)
    ]
    assert "never closed" in errors[0]["message"]


def test_file_without_tables_emits_no_nodes_or_issues(tmp_path: Path) -> None:
    document = _run_adapter(FIXTURE_PATH / "notes.md", tmp_path)
    assert document["nodes"] == []
    assert document["findings"] == []


@pytest.mark.parametrize("kind", ["directory", "file"])
def test_no_matching_files_is_an_info_finding_naming_the_patterns(
    tmp_path: Path, kind: str
) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "notes.markdown").write_text("| ID |\n|----|\n| REQ-1 |\n")
    if kind == "file":
        target = target / "notes.markdown"
    document = _run_adapter(target, tmp_path)
    assert document["nodes"] == []
    [finding] = document["findings"]
    assert (finding["code"], finding["severity"]) == ("NO_MATCHING_FILES", "info")
    assert "*.md" in finding["message"], finding["message"]


@needs_core
def test_no_matching_files_prints_one_info_line_through_validate(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "notes.markdown").write_text("# Notes\n")
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--target", str(target)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    lines = [line for line in result.stdout.splitlines() if "NO_MATCHING_FILES" in line]
    assert len(lines) == 1, result.stdout


def test_missing_target_is_reported_with_exit_zero(tmp_path: Path) -> None:
    document = _run_adapter(tmp_path / "missing", tmp_path)
    assert document["nodes"] == []
    assert len(document["findings"]) == 1
    assert document["findings"][0]["code"] == "PARSE_ERROR"


def test_missing_id_column_is_reported(tmp_path: Path) -> None:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["table"]["id_column"] = "Missing ID"
    resolved.write_text(json.dumps(profile))

    document = _run_adapter_with_profile(
        FIXTURE_PATH / "requirements.md", resolved
    )

    assert document["nodes"] == []
    errors = [
        issue for issue in document["findings"]
        if issue["code"] == "PARSE_ERROR" and "Missing ID" in issue["message"]
    ]
    assert len(errors) == 1
    assert errors[0]["provenance"] == {"file": "requirements.md", "line": 3}


def test_missing_mapped_column_is_reported_without_dropping_valid_mappings(
    tmp_path: Path,
) -> None:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["table"]["column_map"]["Missing Attribute"] = "missing"
    resolved.write_text(json.dumps(profile))

    document = _run_adapter_with_profile(
        FIXTURE_PATH / "requirements.md", resolved
    )

    errors = [
        issue for issue in document["findings"]
        if issue["code"] == "PARSE_ERROR"
        and "Missing Attribute" in issue["message"]
    ]
    assert len(errors) == 1
    assert errors[0]["provenance"] == {"file": "requirements.md", "line": 3}
    assert {node["id"] for node in document["nodes"]} == {
        "REQ-1", "REQ-2", "REQ-3"
    }
    assert {
        (edge["src"], edge["tgt"], edge["kind"])
        for edge in document["edges"]
    } == {
        ("REQ-2", "REQ-1", "traces_to"),
        ("REQ-3", "REQ-1", "traces_to"),
        ("REQ-3", "REQ-2", "traces_to"),
    }


@needs_core
def test_gate_validate_reports_input_errors() -> None:
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(FIXTURE_PATH),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode == 1, f"exit {result.returncode}: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    parse_errors = [finding for finding in findings if finding["code"] == "PARSE_ERROR"]
    assert {(finding["file"], finding["line"]) for finding in parse_errors} == {
        ("malformed.md", 5),
        ("malformed.md", 6),
    }


def test_multikind_dispatches_each_heading_to_the_correct_kind(
    multikind_document: dict,
) -> None:
    nodes = {node["id"]: node for node in multikind_document["nodes"]}
    assert {node["kind"] for node in nodes.values()} == {
        "requirement", "stakeholder",
    }
    assert nodes["REQ-101"]["kind"] == "requirement"
    assert nodes["REQ-201"]["kind"] == "requirement"
    assert nodes["STK-1"] == {
        "id": "STK-1",
        "kind": "stakeholder",
        "attrs": {"name": "Operations", "role": "Operator"},
        "provenance": {"file": "register.md", "line": 19},
    }


def test_multikind_regex_heading_matches_multiple_sections(
    multikind_document: dict,
) -> None:
    requirement_ids = {
        node["id"] for node in multikind_document["nodes"]
        if node["kind"] == "requirement"
    }
    assert requirement_ids == {"REQ-101", "REQ-201"}


def test_multikind_unmatched_heading_emits_an_issue_and_no_nodes(
    multikind_document: dict,
) -> None:
    assert "APP-1" not in {
        node["id"] for node in multikind_document["nodes"]
    }
    issues = [
        issue for issue in multikind_document["findings"]
        if issue["code"] == "PARSE_ERROR" and "Appendix" in issue["message"]
    ]
    assert len(issues) == 1
    assert issues[0]["severity"] == "error"
    assert issues[0]["provenance"] == {"file": "register.md", "line": 23}


@needs_core
def test_multikind_gate_reports_unmatched_heading() -> None:
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(MULTI_PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(MULTI_FIXTURE_PATH),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode == 1, f"exit {result.returncode}: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    unmatched = [
        finding for finding in findings
        if finding["code"] == "PARSE_ERROR" and "Appendix" in finding["message"]
    ]
    assert len(unmatched) == 1


def test_a_leading_byte_order_mark_is_encoding_not_content(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "first.md").write_text(
        "| ID | Description | Status | Traces To |\n|---|---|---|---|\n"
        "| REQ-70 | on line three | draft | |\n"
    )
    plain = _run_adapter(target, tmp_path)
    assert "REQ-70" in {node["id"] for node in plain["nodes"]}
    for path in target.rglob("*.md"):
        path.write_bytes(b"\xef\xbb\xbf" + path.read_bytes())
    assert _run_adapter(target, tmp_path) == plain


SOURCES_PROFILE_PATH = REPO_ROOT / "profiles" / "md-sources.yaml"
SOURCES_FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-md-sources"

# The fixture's stale rows: REGISTER.md line -> node ID.
STALE_ROWS = {
    19: "crates/demo/tests/schema.rs::loads_old_profile",
    20: "crates/demo/src/lib.rs::unit::splits",
    28: "tests/test_store.py::test_old_put",
    29: "tests/test_store.py::Outer::Inner::test_deep",
    37: "pkg/store/store_test.go::TestOldPut",
    44: "docs/removed.md",
}


def _validate_sources(*extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(SOURCES_PROFILE_PATH),
         "--target", str(SOURCES_FIXTURE_PATH), "--format", "json", *extra],
        capture_output=True, text=True,
    )


# Requirement: source-existence/Declared source addresses resolve against the target
# Requirement: source-existence/An address is a path and an optional symbol chain
# Requirement: source-existence/A file address names a regular file inside the target
# Requirement: source-existence/Unreadable source is reported, not passed
def test_sources_stale_rows_are_reported_and_live_rows_are_silent(
    tmp_path: Path,
) -> None:
    resolved = resolve_profile(SOURCES_PROFILE_PATH, tmp_path)
    document = _run_adapter_with_profile(SOURCES_FIXTURE_PATH, resolved)
    assert len(document["nodes"]) == 15
    assert len(document["edges"]) == 11
    missing = [f for f in document["findings"] if f["code"] == "SOURCE_MISSING"]
    assert {
        (f["provenance"]["file"], f["provenance"]["line"]): f["node_id"]
        for f in missing
    } == {("REGISTER.md", line): node for line, node in STALE_ROWS.items()}
    assert all(f["severity"] == "warning" for f in missing)
    others = [f for f in document["findings"] if f["code"] != "SOURCE_MISSING"]
    assert others == [{
        "severity": "warning",
        "code": "PARSE_ERROR",
        "message": "could not read python source 'tests/test_broken.py': "
                   "unterminated string",
        "provenance": {"file": "tests/test_broken.py", "line": 5},
        "node_id": None,
    }]


# Requirement: adapter-md/Resolve declared source addresses
@needs_core
def test_sources_gate_exits_zero_and_strict_promotes_only_the_new_findings() -> None:
    result = _validate_sources()
    assert result.returncode == 0, f"exit {result.returncode}: {result.stderr}"
    strict = _validate_sources("--strict")
    assert strict.returncode == 1, f"exit {strict.returncode}: {strict.stderr}"
    errors = [
        f for f in json.loads(strict.stdout)["findings"] if f["severity"] == "error"
    ]
    assert sorted((f["code"], f["file"], f["line"]) for f in errors) == sorted(
        [("SOURCE_MISSING", "REGISTER.md", line) for line in STALE_ROWS]
        + [("PARSE_ERROR", "tests/test_broken.py", 5)]
    )


# Differential tests: each scanner against the authority for its language.
# Every address the authority lists is written as a row of a scratch register,
# beside a copy of the files it names, and must resolve.

GO_CORPUS = REPO_ROOT / "tests" / "fixtures" / "go-corpus"
GO_LIST = GO_CORPUS / "go-list.txt"


def _unresolved(
    tmp_path: Path, id_names: str, ids: list[str], files: set[Path], root: Path,
) -> tuple[set[str], list[dict]]:
    """Check *ids* under *id_names*; return unresolved IDs and other findings."""
    target = tmp_path / "target"
    for file in files:
        copy = target / file.relative_to(root)
        copy.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(file, copy)
    rows = "".join(f"| {node_id} |\n" for node_id in ids)
    (target / "REGISTER.md").write_text(f"| ID |\n| --- |\n{rows}", encoding="utf-8")
    profile = tmp_path / "profile.yaml"
    profile.write_text(
        "name: differential\nprofile_version: '1.0.0'\n"
        "adapter:\n  native: md\n  paths: { files: ['REGISTER.md'] }\n"
        "  table: { id_column: 'ID', column_map: {}, edge_columns: {} }\n"
        f"node_kinds:\n  test: {{ id_pattern: '.*', id_names: {id_names}, "
        "orphan_ok: true }\nedge_kinds: {}\n",
        encoding="utf-8",
    )
    document = _run_adapter_with_profile(target, resolve_profile(profile, tmp_path))
    assert len(document["nodes"]) == len(ids)
    missing = {
        f["node_id"] for f in document["findings"] if f["code"] == "SOURCE_MISSING"
    }
    others = [f for f in document["findings"] if f["code"] != "SOURCE_MISSING"]
    return missing, others


def _libtest_listing() -> dict[Path, list[str]]:
    """Map each integration-test source file to the names libtest lists for it."""
    build = subprocess.run(
        ["cargo", "test", "--workspace", "--tests", "--no-run", "--locked",
         "--message-format", "json"],
        capture_output=True, text=True, cwd=REPO_ROOT,
    )
    assert build.returncode == 0, build.stderr[-2000:]
    listing: dict[Path, list[str]] = {}
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if (message.get("reason") != "compiler-artifact"
                or not message.get("executable")
                or message["target"]["kind"] != ["test"]):
            continue
        listed = subprocess.run(
            [message["executable"], "--list", "--format", "terse"],
            capture_output=True, text=True, check=True,
        ).stdout
        listing[Path(message["target"]["src_path"])] = [
            entry.removesuffix(": test")
            for entry in listed.splitlines() if entry.endswith(": test")
        ]
    return listing


# Requirement: source-existence/Languages define where a definition counts
# CI's python-check image carries no toolchain; tools/verify.sh always does,
# so this runs on every pre-push gate.
@pytest.mark.skipif(shutil.which("cargo") is None, reason="needs cargo for libtest --list")
def test_differential_rust_scanner_resolves_every_libtest_name(tmp_path: Path) -> None:
    listing = _libtest_listing()
    assert len(listing) > 10, "no integration-test binaries were found"
    ids, out_of_line = [], 0
    for source, names in listing.items():
        rel = source.relative_to(REPO_ROOT).as_posix()
        for name in names:
            # A name through an out-of-line module (`mod common;`) lives in
            # another file, which an address names directly.
            head = name.split("::")[0]
            if "::" in name and not _defines_inline_mod(source, head):
                out_of_line += 1
                continue
            ids.append(f"{rel}::{name}")
    assert out_of_line == 0, "an out-of-line module now holds tests; recount"
    missing, others = _unresolved(tmp_path, "cargo", ids, set(listing), REPO_ROOT)
    assert others == []
    assert missing == set(), sorted(missing)[:20]


def _defines_inline_mod(source: Path, name: str) -> bool:
    return f"mod {name} {{" in source.read_text(encoding="utf-8")


# Requirement: source-existence/Languages define where a definition counts
def test_differential_python_scanner_resolves_every_ast_definition(
    tmp_path: Path,
) -> None:
    import ast

    from adapters._python_tests import iter_test_holders

    files = sorted((REPO_ROOT / "tests").glob("*.py"))
    ids = []
    for file in files:
        rel = file.relative_to(REPO_ROOT).as_posix()
        tree = ast.parse(file.read_text(encoding="utf-8"), filename=rel)
        for holder, tests in iter_test_holders(tree):
            for test in tests:
                ids.append(f"{rel}::{holder.name}::{test.name}" if holder
                           else f"{rel}::{test.name}")
    assert len(ids) > 100, "the AST found too few tests to compare"
    missing, others = _unresolved(tmp_path, "pytest", ids, set(files), REPO_ROOT)
    assert others == []
    assert missing == set(), sorted(missing)[:20]


def _go_list() -> list[str]:
    """Run `go test -list` per corpus package; one `_test.go` per package."""
    ids = []
    for package in sorted(path for path in GO_CORPUS.iterdir() if path.is_dir()):
        (test_file,) = package.glob("*_test.go")
        # -vet=off: from Go 1.24 `go test` vets example names, and the corpus's
        # ExamplePlain names no identifier on purpose.
        listed = subprocess.run(
            ["go", "test", "-vet=off", "-list", ".*", f"./{package.name}"],
            capture_output=True, text=True, check=True, cwd=GO_CORPUS,
        ).stdout
        rel = test_file.relative_to(GO_CORPUS).as_posix()
        ids += [f"{rel}::{name}" for name in listed.splitlines()
                if name and not name.startswith(("ok ", "?"))]
    return ids


# Requirement: source-existence/Languages define where a definition counts
def test_differential_go_scanner_resolves_every_listed_test(tmp_path: Path) -> None:
    if os.environ.get("LATTICE_REGEN_BASELINES") == "1":
        GO_LIST.write_text("\n".join(_go_list()) + "\n", encoding="utf-8")
    pinned = GO_LIST.read_text(encoding="utf-8").split()
    if shutil.which("go"):
        assert _go_list() == pinned, "go-list.txt is stale; regenerate it"
    files = set(GO_CORPUS.rglob("*_test.go"))
    missing, others = _unresolved(tmp_path, "gotest", pinned, files, GO_CORPUS)
    assert others == []
    assert missing == set(), sorted(missing)


# Requirement: source-existence/Languages define where a definition counts
def test_differential_go_decoys_do_not_resolve(tmp_path: Path) -> None:
    decoys = [
        "alpha/alpha_test.go::TestInComment",
        "alpha/alpha_test.go::TestInBlockComment",
        "alpha/alpha_test.go::TestInRawString",
        "alpha/alpha_test.go::TestMethod",
        "alpha/alpha_test.go::TestSubtest",
        "beta/beta_test.go::TestInString",
    ]
    files = set(GO_CORPUS.rglob("*_test.go"))
    missing, _ = _unresolved(tmp_path, "gotest", decoys, files, GO_CORPUS)
    assert missing == set(decoys)


# Requirement: source-existence/An unknown form is a broken setup
def test_sources_unknown_form_exits_2_without_a_document(tmp_path: Path) -> None:
    (tmp_path / "r.md").write_text("| ID |\n| --- |\n| t.py::x |\n", encoding="utf-8")
    profile = tmp_path / "profile.json"
    profile.write_text(json.dumps({
        "resolved_schema": "1",
        "adapter": {"paths": {"files": ["*.md"]},
                    "table": {"id_column": "ID", "column_map": {}, "edge_columns": {}}},
        "node_kinds": {"test": {"id_names": "jest"}},
    }), encoding="utf-8")
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(profile), "--target", str(tmp_path)],
        capture_output=True, text=True,
    )
    assert result.returncode == 2
    assert result.stdout == ""
    assert "node kind 'test'" in result.stderr and "'jest'" in result.stderr


# Requirement: adapter-contract/Resolved profile document
# Requirement: adapter-md/Load the resolved profile configuration
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


# Requirement: adapter-contract/Document order is significant
@needs_core
def test_a_duplicate_rows_edges_are_not_read(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "r.md").write_text(
        "| ID | Description | Traces To |\n|----|----|----|\n"
        "| REQ-1 | first | REQ-2 |\n"
        "| REQ-1 | second | REQ-3, REQ-4 |\n"
        "| REQ-2 | two | |\n| REQ-3 | three | |\n| REQ-4 | four | |\n"
    )
    document = _run_adapter(target, tmp_path)
    assert [edge["declared_by"] for edge in document["edges"]] == [0, 1, 1]

    result = subprocess.run(
        [str(CORE_BIN), "trace", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(target),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode != 2, result.stderr
    entries = {e["id"]: e for e in json.loads(result.stdout)["entries"]}
    assert [edge["tgt"] for edge in entries["REQ-1"]["edges"]] == ["REQ-2"]
    assert [
        finding["message"] for finding in entries["REQ-1"]["findings"]
        if finding["message"].startswith("duplicate node")
    ] == ["duplicate node 'REQ-1': first at r.md:3; its 2 edges were not read"]

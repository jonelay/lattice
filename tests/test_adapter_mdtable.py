from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "mdtable.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "mdtable"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"
FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-mdtable"

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
        issue for issue in document["issues"]
        if issue["code"] == "PARSE_ERROR" and issue["provenance"]["line"] == 5
    ]
    assert len(errors) == 1
    assert "fewer columns" in errors[0]["message"]


def test_empty_ids_are_reported(document: dict) -> None:
    errors = [
        issue for issue in document["issues"]
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


def test_contract_version_is_1_1(document: dict) -> None:
    assert document["contract_version"] == "1.1"


def test_non_utf8_file_is_reported_and_other_files_survive(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "invalid.md").write_bytes(b"\xff\xfe not UTF-8")
    document = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in document["issues"]
        if issue["code"] == "PARSE_ERROR"
        and issue["provenance"]["file"] == "invalid.md"
    ]
    assert len(errors) == 1
    assert "UTF-8" in errors[0]["message"]
    assert {node["id"] for node in document["nodes"]} == {
        "REQ-1", "REQ-2", "REQ-3"
    }


def test_file_without_tables_emits_no_nodes_or_issues(tmp_path: Path) -> None:
    document = _run_adapter(FIXTURE_PATH / "notes.md", tmp_path)
    assert document["nodes"] == []
    assert document["issues"] == []


def test_missing_target_is_reported_with_exit_zero(tmp_path: Path) -> None:
    document = _run_adapter(tmp_path / "missing", tmp_path)
    assert document["nodes"] == []
    assert len(document["issues"]) == 1
    assert document["issues"][0]["code"] == "PARSE_ERROR"


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
        issue for issue in document["issues"]
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
        issue for issue in document["issues"]
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

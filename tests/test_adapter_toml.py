from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "toml.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "toml"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"
FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-toml"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


def _run_adapter(target: Path, tmp_path: Path) -> dict:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    return _run_adapter_with_profile(target, resolved)


def _run_adapter_with_profile(target: Path, profile: Path) -> dict:
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(profile), "--target", str(target)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


@pytest.fixture
def document(tmp_path: Path) -> dict:
    return _run_adapter(FIXTURE_PATH, tmp_path)


def test_contract_and_gate_build_nodes_and_edges(document: dict) -> None:
    assert document["contract_version"] == "1.1"
    assert document["nodes"], "the gate proves nothing over an empty graph"
    assert document["edges"], "no cross-reference was checked"


def test_header_nodes_and_native_attributes(document: dict) -> None:
    nodes = {node["id"]: node for node in document["nodes"]}
    assert nodes["alpha"]["kind"] == "register"
    assert nodes["alpha"]["attrs"] == {
        "id": "reg.alpha",
        "register_version": 0,
        "status": "draft-3",
    }


def test_qualified_item_nodes_dispatch_from_tables(document: dict) -> None:
    nodes = {node["id"]: node for node in document["nodes"]}
    assert nodes["alpha/one"]["kind"] == "item"
    assert nodes["alpha/one"]["attrs"] == {"id": "one", "stage": "s1"}
    assert "beta/one" in nodes
    # A missing optional header is reported, but independent table rows survive.
    assert "noregister/unreachable" in nodes


def test_multi_kind_dispatch_in_standing_fixture(document: dict) -> None:
    nodes = {node["id"]: node for node in document["nodes"]}
    assert nodes["alpha/context"]["kind"] == "memo"
    assert nodes["alpha/context"]["attrs"] == {"text": "background for the alpha register"}
    assert nodes["alpha/one"]["kind"] == "item"


def test_string_and_array_edge_keys_expand(document: dict) -> None:
    edges = {
        (edge["src"], edge["tgt"], edge["kind"])
        for edge in document["edges"]
    }
    assert ("alpha/one", "beta", "belongs_to") in edges
    assert ("alpha/one", "beta/two", "references") in edges
    assert ("wrongshape/string_refs", "beta/two", "references") in edges
    assert ("dangling/overdue", "alpha/absent", "references") in edges


def test_axis_is_read_from_configured_dot_paths(document: dict) -> None:
    assert document["axes"] == [
        {"name": "stage", "order": ["s1", "s2", "s3"], "current": "s2"}
    ]


def test_parse_error_locations_cover_each_bad_fixture(document: dict) -> None:
    locations = {
        issue["provenance"]["file"]
        for issue in document["issues"]
        if issue["code"] == "PARSE_ERROR"
    }
    assert locations == {
        "registers/baditem.toml",
        "registers/broken.toml",
        "registers/noregister.toml",
        "registers/unread.toml",
        "registers/wrongshape.toml",
    }


def test_row_error_names_index_and_unread_table(document: dict) -> None:
    messages = [issue["message"] for issue in document["issues"]]
    assert any("baditem.toml" in message and "row 1" in message for message in messages)
    assert any("unread.toml" in message and "'note'" in message for message in messages)


def test_wrong_edge_shape_has_node_context(document: dict) -> None:
    errors = [
        issue
        for issue in document["issues"]
        if issue["node_id"] == "wrongshape/numeric_register"
    ]
    assert len(errors) == 1
    assert "'register'" in errors[0]["message"]


def test_missing_target_is_reported_with_exit_zero(tmp_path: Path) -> None:
    document = _run_adapter(tmp_path / "missing", tmp_path)
    assert document["nodes"] == []
    assert [issue["code"] for issue in document["issues"]] == ["PARSE_ERROR"]


def test_malformed_file_does_not_stop_later_files(document: dict) -> None:
    node_ids = {node["id"] for node in document["nodes"]}
    assert "dangling/overdue" in node_ids
    assert "wrongshape/numeric_register" in node_ids


def test_header_can_use_a_key_based_id(tmp_path: Path) -> None:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["header"]["id"] = "id"
    profile["adapter"]["paths"]["files"] = ["alpha.toml"]
    resolved.write_text(json.dumps(profile))
    document = _run_adapter_with_profile(FIXTURE_PATH / "registers" / "alpha.toml", resolved)
    assert "reg.alpha" in {node["id"] for node in document["nodes"]}


def test_invalid_and_partial_axes_are_reported(tmp_path: Path) -> None:
    target = tmp_path / "target"
    shutil.copytree(FIXTURE_PATH, target)
    index = target / "registers" / "index.toml"
    index.write_text(index.read_text().replace('current_stage = "s2"', 'current_stage = "s9"'))
    invalid = _run_adapter(target, tmp_path)
    assert invalid["axes"] == []
    assert len([issue for issue in invalid["issues"] if issue["code"] == "AXIS_INVALID"]) == 1

    index.write_text(index.read_text().replace('current_stage = "s9"', ""))
    partial = _run_adapter(target, tmp_path)
    errors = [issue for issue in partial["issues"] if issue["code"] == "AXIS_INVALID"]
    assert len(errors) == 1
    assert "current_stage" in errors[0]["message"]


def test_profile_without_tables_exits_two(tmp_path: Path) -> None:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["tables"] = {}
    resolved.write_text(json.dumps(profile))
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(FIXTURE_PATH)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2
    assert "adapter.tables" in result.stderr


@pytest.mark.parametrize("files", [[], [""]])
def test_profile_without_a_usable_file_glob_exits_two(
    tmp_path: Path, files: list[str]
) -> None:
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["paths"]["files"] = files
    resolved.write_text(json.dumps(profile))
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolved), "--target", str(FIXTURE_PATH)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2
    assert "adapter.paths.files" in result.stderr


def test_multiple_tables_dispatch_and_prefix_can_be_absent(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "rows.toml").write_text(
        '[[item]]\nid = "one"\nstage = "s1"\n\n'
        '[[note]]\nid = "N1"\ntext = "remember"\n'
    )
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["paths"]["files"] = ["*.toml"]
    profile["adapter"].pop("id_prefix")
    profile["adapter"].pop("header")
    profile["adapter"].pop("axis")
    profile["adapter"]["tables"]["note"] = {
        "kind": "note",
        "id_key": "id",
        "key_map": {"text": "text"},
        "edge_keys": {},
    }
    resolved.write_text(json.dumps(profile))

    document = _run_adapter_with_profile(target, resolved)
    nodes = {node["id"]: node for node in document["nodes"]}
    assert nodes["one"]["kind"] == "item"
    assert nodes["N1"]["kind"] == "note"
    assert nodes["N1"]["attrs"] == {"text": "remember"}
    assert document["issues"] == []


def test_scalar_mapping_preserves_types_and_rejects_nested_values(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "types.toml").write_text(
        '[[item]]\nid = "typed"\nactive = true\nscore = 1.5\n'
        'when = 1979-05-27T07:32:00Z\nvalues = [1, 2]\n'
        'nested = { key = "value" }\n'
    )
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"]["paths"]["files"] = ["*.toml"]
    profile["adapter"].pop("header")
    profile["adapter"].pop("axis")
    profile["adapter"]["tables"]["item"]["key_map"] = {
        "active": "active",
        "score": "score",
        "when": "when",
        "values": "values",
        "nested": "nested",
    }
    resolved.write_text(json.dumps(profile))

    document = _run_adapter_with_profile(target, resolved)
    assert document["nodes"][0]["attrs"] == {
        "active": True,
        "score": 1.5,
        "when": "1979-05-27T07:32:00Z",
    }
    errors = [issue for issue in document["issues"] if issue["code"] == "PARSE_ERROR"]
    assert len(errors) == 2
    assert {issue["node_id"] for issue in errors} == {"types/typed"}


@needs_core
def test_gate_runs_through_core_with_expected_findings() -> None:
    result = subprocess.run(
        [
            str(CORE_BIN),
            "validate",
            "--profile",
            str(PROFILE_PATH),
            "--adapter",
            str(ADAPTER_PATH),
            "--target",
            str(FIXTURE_PATH),
            "--format",
            "json",
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 1, f"exit {result.returncode}: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    parse_locations = {
        finding["file"] for finding in findings if finding["code"] == "PARSE_ERROR"
    }
    assert parse_locations == {
        "registers/baditem.toml",
        "registers/broken.toml",
        "registers/noregister.toml",
        "registers/unread.toml",
        "registers/wrongshape.toml",
    }
    dangling = {
        finding["message"].split("'")[1]: finding["severity"]
        for finding in findings
        if finding["code"] == "DANGLING_REF"
    }
    assert dangling["dangling/not_yet"] == "info"
    assert dangling["dangling/overdue"] == "error"

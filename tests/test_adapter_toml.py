from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import resolve_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "toml.yaml"
DIRECTORY_PROFILE_PATH = REPO_ROOT / "profiles" / "toml-directory.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "toml"
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"
FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-toml"
DIRECTORY_FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-toml-directory"

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


def _directory_profile(tmp_path: Path, **adapter_updates: object) -> Path:
    resolved = resolve_profile(DIRECTORY_PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    profile["adapter"].update(adapter_updates)
    resolved.write_text(json.dumps(profile))
    return resolved


@pytest.fixture
def document(tmp_path: Path) -> dict:
    return _run_adapter(FIXTURE_PATH, tmp_path)


def test_contract_and_gate_build_nodes_and_edges(document: dict) -> None:
    assert document["interface_version"] == "1.4"
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


def test_pathway_is_read_from_configured_dot_paths(document: dict) -> None:
    assert document["pathways"] == [
        {"name": "stage", "order": ["s1", "s2", "s3"], "current": "s2"}
    ]


def test_parse_error_locations_cover_each_bad_fixture(document: dict) -> None:
    locations = {
        issue["provenance"]["file"]
        for issue in document["findings"]
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
    messages = [issue["message"] for issue in document["findings"]]
    assert any("baditem.toml" in message and "row 1" in message for message in messages)
    assert any("unread.toml" in message and "'note'" in message for message in messages)


def test_wrong_edge_shape_has_node_context(document: dict) -> None:
    errors = [
        issue
        for issue in document["findings"]
        if issue["node_id"] == "wrongshape/numeric_register"
    ]
    assert len(errors) == 1
    assert "'register'" in errors[0]["message"]


def test_missing_target_is_reported_with_exit_zero(tmp_path: Path) -> None:
    document = _run_adapter(tmp_path / "missing", tmp_path)
    assert document["nodes"] == []
    assert [issue["code"] for issue in document["findings"]] == ["PARSE_ERROR"]


def test_dangling_symlink_matching_a_glob_is_reported(
    tmp_path: Path, document: dict
) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    (target / "registers" / "gone.toml").symlink_to("missing.toml")
    (target / "registers" / "gone.txt").symlink_to("missing.txt")
    linked = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in linked["findings"]
        if "gone" in issue["provenance"]["file"]
    ]
    assert [(issue["code"], issue["provenance"]["file"]) for issue in errors] == [
        ("PARSE_ERROR", "registers/gone.toml")
    ]
    assert linked["nodes"] == document["nodes"]


def test_each_mapped_attr_says_where_it_was_written(document: dict) -> None:
    # The node sits at its [[item]] header; each attr at its key's own line, so a
    # finding about one value lands on that value (interface 1.4 attr_provenance).
    text = (FIXTURE_PATH / "registers" / "beta.toml").read_text().splitlines()
    line = {needle: text.index(needle) + 1 for needle in ('id = "two"', 'stage = "s2"')}
    two = next(n for n in document["nodes"]
               if n["provenance"]["file"] == "registers/beta.toml" and n["attrs"].get("id") == "two")
    assert two["provenance"]["line"] == line['id = "two"'] - 1
    assert two["attr_provenance"] == {
        "id": {"file": "registers/beta.toml", "line": line['id = "two"']},
        "stage": {"file": "registers/beta.toml", "line": line['stage = "s2"']},
    }


@needs_core
def test_an_attr_type_finding_lands_on_the_key(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    path = target / "registers" / "beta.toml"
    path.write_text(path.read_text().replace('stage = "s2"', "stage = 2"))
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH), "--adapter", "toml",
         "--target", str(target), "--format", "json"],
        capture_output=True, text=True,
    )
    findings = [f for f in json.loads(result.stdout)["findings"] if f["code"] == "ATTR_TYPE"]
    stage_line = path.read_text().splitlines().index("stage = 2") + 1
    assert [(f["file"], f["line"]) for f in findings] == [("registers/beta.toml", stage_line)]
    assert findings[0]["message"].endswith("got int 2")


def test_non_utf8_content_names_its_line_as_written(tmp_path: Path) -> None:
    # TOML is defined over UTF-8, so the file is not TOML and is read as nothing,
    # as for a syntax error; the finding shows where and what, not a byte offset.
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    path = target / "registers" / "alpha.toml"
    path.write_bytes(path.read_bytes().replace(b'status = "draft-3"', b'status = "dr\xe4ft-3"'))
    document = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in document["findings"]
        if issue["provenance"]["file"] == "registers/alpha.toml"
    ]
    assert [(e["code"], e["provenance"]["line"]) for e in errors] == [("PARSE_ERROR", 6)]
    assert "column 13" in errors[0]["message"]
    assert r'status = "dr\xE4ft-3"' in errors[0]["message"]
    assert "was not read" in errors[0]["message"]
    assert not any(
        node["provenance"]["file"] == "registers/alpha.toml" for node in document["nodes"]
    )


def test_non_utf8_path_matching_a_glob_is_reported(
    tmp_path: Path, document: dict
) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    for name in (b"bad\xff.toml", b"bad\xfe.toml"):
        (target / "registers" / os.fsdecode(name)).write_text("")
    renamed = _run_adapter(target, tmp_path)
    errors = [
        issue for issue in renamed["findings"]
        if issue["provenance"]["file"] == "registers/bad\ufffd.toml"
    ]
    assert [issue["code"] for issue in errors] == ["PARSE_ERROR", "PARSE_ERROR"]
    assert all("path is not valid UTF-8" in issue["message"] for issue in errors)
    assert ["\\xFE" in errors[0]["message"], "\\xFF" in errors[1]["message"]] == [True, True]
    assert renamed["nodes"] == document["nodes"]


def test_non_utf8_paths_alone_are_not_also_no_matching_files(tmp_path: Path) -> None:
    target = tmp_path / "register"
    (target / "registers").mkdir(parents=True)
    for name in (b"bad\xff.toml", b"bad\xfe.toml"):
        (target / "registers" / os.fsdecode(name)).write_text("")
    renamed = _run_adapter(target, tmp_path)
    assert renamed["nodes"] == []
    # The profile's pathway source is unread too; that finding is its own.
    errors = [f for f in renamed["findings"] if f["code"] != "PATHWAY_INVALID"]
    assert [(issue["code"], issue["provenance"]["file"]) for issue in errors] == [
        ("PARSE_ERROR", "registers/bad�.toml"),
        ("PARSE_ERROR", "registers/bad�.toml"),
    ]
    assert all("path is not valid UTF-8" in issue["message"] for issue in errors)


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


def test_invalid_and_partial_pathways_are_reported(tmp_path: Path) -> None:
    target = tmp_path / "target"
    shutil.copytree(FIXTURE_PATH, target)
    index = target / "registers" / "index.toml"
    index.write_text(index.read_text().replace('current_stage = "s2"', 'current_stage = "s9"'))
    invalid = _run_adapter(target, tmp_path)
    assert invalid["pathways"] == []
    assert len([issue for issue in invalid["findings"] if issue["code"] == "PATHWAY_INVALID"]) == 1

    index.write_text(index.read_text().replace('current_stage = "s9"', ""))
    partial = _run_adapter(target, tmp_path)
    errors = [issue for issue in partial["findings"] if issue["code"] == "PATHWAY_INVALID"]
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
    profile["adapter"].pop("pathway")
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
    assert document["findings"] == []


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
    profile["adapter"].pop("pathway")
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
        "values": [1, 2],
    }
    errors = [issue for issue in document["findings"] if issue["code"] == "PARSE_ERROR"]
    assert len(errors) == 1
    assert {issue["node_id"] for issue in errors} == {"types/typed"}


def test_directory_mode_reads_each_file_as_one_record(tmp_path: Path) -> None:
    document = _run_adapter_with_profile(
        DIRECTORY_FIXTURE_PATH, _directory_profile(tmp_path)
    )
    nodes = {node["id"]: node for node in document["nodes"]}
    assert set(nodes) == {"DEV-001", "DEV-002", "DEV-003"}
    assert nodes["DEV-001"]["kind"] == "device"
    assert nodes["DEV-001"]["attrs"] == {"name": "Sensor A"}
    assert nodes["DEV-001"]["provenance"]["file"] == "records/device-001.toml"


def test_directory_mode_emits_edges(tmp_path: Path) -> None:
    document = _run_adapter_with_profile(
        DIRECTORY_FIXTURE_PATH, _directory_profile(tmp_path)
    )
    edges = {
        (edge["src"], edge["tgt"], edge["kind"])
        for edge in document["edges"]
    }
    assert edges == {
        ("DEV-001", "DEV-002", "references"),
        ("DEV-003", "DEV-001", "references"),
        ("DEV-003", "DEV-002", "references"),
    }


def test_a_dotted_edge_key_follows_nested_tables(tmp_path: Path) -> None:
    target = tmp_path / "register"
    (target / "records").mkdir(parents=True)
    (target / "records" / "a.toml").write_text(
        'id = "DEV-001"\nname = "a"\n[people]\nleads = ["DEV-002"]\n'
        '[access.teams]\nx = "y"\n'
    )
    (target / "records" / "b.toml").write_text(
        'id = "DEV-002"\nname = "b"\npeople = "not a table"\n'
    )
    profile = _directory_profile(
        tmp_path,
        tables={"record": {
            "kind": "device", "id_key": "id", "key_map": {"name": "name"},
            "edge_keys": {
                "people.leads": "references",
                "access.teams": "references",
                "absent.key": "references",
            },
        }},
    )
    document = _run_adapter_with_profile(target, profile)
    assert [(e["src"], e["tgt"]) for e in document["edges"]] == [
        ("DEV-001", "DEV-002")
    ]
    messages = sorted(
        (f["provenance"]["file"], f["message"]) for f in document["findings"]
    )
    assert [file for file, _ in messages] == ["records/a.toml", "records/b.toml"]
    assert "'access.teams' of type table" in messages[0][1]
    assert "'people.leads'" in messages[1][1] and "'people'" in messages[1][1]
    assert "string" in messages[1][1]


def test_a_mixed_edge_array_keeps_its_strings(tmp_path: Path) -> None:
    # rust-lang/team lists members as names and `{ github = ... }` tables in one
    # array; dropping the whole key lost every name beside the tables.
    target = tmp_path / "register"
    (target / "records").mkdir(parents=True)
    (target / "records" / "a.toml").write_text(
        'id = "DEV-001"\n[people]\n'
        'members = ["DEV-002", { github = "x", roles = [] }, "DEV-003", 7]\n'
    )
    profile = _directory_profile(
        tmp_path,
        tables={"record": {
            "kind": "device", "id_key": "id", "key_map": {},
            "edge_keys": {"people.members": "references"},
        }},
    )
    document = _run_adapter_with_profile(target, profile)
    assert [(e["src"], e["tgt"]) for e in document["edges"]] == [
        ("DEV-001", "DEV-002"), ("DEV-001", "DEV-003")
    ]
    messages = [(f["code"], f["message"]) for f in document["findings"]]
    assert messages == [
        ("PARSE_ERROR", "records/a.toml: item 'DEV-001' has edge key 'people.members' entry 2 "
         'of 4: { github = "x", roles = [] } is a table, not a target string; it was not read'),
        ("PARSE_ERROR", "records/a.toml: item 'DEV-001' has edge key 'people.members' entry 4 "
         "of 4: 7 is an integer, not a target string; it was not read"),
    ]


def test_a_dotted_key_map_key_follows_nested_tables(tmp_path: Path) -> None:
    target = tmp_path / "register"
    (target / "records").mkdir(parents=True)
    (target / "records" / "a.toml").write_text(
        'id = "DEV-001"\n"meta.owner" = "literal"\n[status]\nstate = "live"\n'
    )
    (target / "records" / "b.toml").write_text(
        'id = "DEV-002"\nstatus = "flat"\n[meta]\nowner = "nested"\n'
    )
    profile = _directory_profile(
        tmp_path,
        tables={"record": {
            "kind": "device", "id_key": "id",
            "key_map": {"status.state": "state", "meta.owner": "owner"},
            "edge_keys": {},
        }},
    )
    document = _run_adapter_with_profile(target, profile)
    attrs = {n["id"]: n["attrs"] for n in document["nodes"]}
    assert attrs["DEV-001"] == {"state": "live", "owner": "literal"}
    assert attrs["DEV-002"] == {"owner": "nested"}
    [finding] = document["findings"]
    assert finding["code"] == "PARSE_ERROR"
    assert finding["provenance"]["file"] == "records/b.toml"
    message = finding["message"]
    assert "'status.state'" in message and "'status'" in message, message
    assert "string" in message, message


def test_directory_mode_with_id_prefix(tmp_path: Path) -> None:
    profile = _directory_profile(
        tmp_path,
        id_prefix="file_stem",
        paths={"files": ["records/device-001.toml"]},
    )
    document = _run_adapter_with_profile(DIRECTORY_FIXTURE_PATH, profile)
    assert [node["id"] for node in document["nodes"]] == ["device-001/DEV-001"]


@pytest.mark.parametrize("kind", ["directory", "file"])
def test_no_matching_files_is_an_info_finding_naming_the_patterns(
    tmp_path: Path, kind: str
) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "notes.md").write_text("[register]\nid = 'R-1'\n")
    if kind == "file":
        target = target / "notes.md"
    document = _run_adapter(target, tmp_path)
    assert document["nodes"] == []
    # The profile's pathway source is unread too; that finding is its own.
    [finding] = [f for f in document["findings"] if f["code"] != "PATHWAY_INVALID"]
    assert (finding["code"], finding["severity"]) == ("NO_MATCHING_FILES", "info")
    assert "registers/*.toml" in finding["message"], finding["message"]


def test_directory_mode_zero_files_emits_info_issue(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    document = _run_adapter_with_profile(target, _directory_profile(tmp_path))
    assert document["nodes"] == []
    assert [
        (issue["code"], issue["severity"])
        for issue in document["findings"]
    ] == [("NO_MATCHING_FILES", "info")]


def test_directory_mode_malformed_file_continues(tmp_path: Path) -> None:
    target = tmp_path / "target"
    shutil.copytree(DIRECTORY_FIXTURE_PATH, target)
    (target / "records" / "device-broken.toml").write_text('id = "unterminated')
    document = _run_adapter_with_profile(target, _directory_profile(tmp_path))
    assert {node["id"] for node in document["nodes"]} == {
        "DEV-001",
        "DEV-002",
        "DEV-003",
    }
    errors = [
        issue for issue in document["findings"] if issue["code"] == "PARSE_ERROR"
    ]
    assert len(errors) == 1
    assert errors[0]["provenance"]["file"] == "records/device-broken.toml"


def test_directory_mode_rejects_header_config(tmp_path: Path) -> None:
    profile = _directory_profile(
        tmp_path,
        header={
            "table": "metadata",
            "kind": "device",
            "id": "file_stem",
            "key_map": {},
        },
    )
    result = subprocess.run(
        [
            str(ADAPTER_PATH),
            "--profile",
            str(profile),
            "--target",
            str(DIRECTORY_FIXTURE_PATH),
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2
    assert "adapter.header" in result.stderr
    assert "directory" in result.stderr


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
        if finding["code"] == "VACANCY"
    }
    assert dangling["dangling/not_yet"] == "info"
    assert dangling["dangling/overdue"] == "error"


def test_a_leading_byte_order_mark_is_encoding_not_content(tmp_path: Path) -> None:
    target = tmp_path / "register"
    shutil.copytree(FIXTURE_PATH, target)
    plain = _run_adapter(target, tmp_path)
    for path in target.rglob("*.toml"):
        path.write_bytes(b"\xef\xbb\xbf" + path.read_bytes())
    assert _run_adapter(target, tmp_path) == plain


# Requirement: adapter-contract/Resolved profile document
# Requirement: adapter-toml/Load the resolved TOML profile configuration
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


def _trace(profile: Path, target: Path) -> dict[str, dict]:
    result = subprocess.run(
        [str(CORE_BIN), "trace", "--profile", str(profile),
         "--adapter", str(ADAPTER_PATH), "--target", str(target),
         "--format", "json"],
        capture_output=True,
        text=True,
    )
    assert result.returncode != 2, result.stderr
    return {entry["id"]: entry for entry in json.loads(result.stdout)["entries"]}


# Requirement: adapter-contract/Document order is significant
@needs_core
def test_a_duplicate_rows_edges_are_not_read(tmp_path: Path) -> None:
    target = tmp_path / "target"
    (target / "registers").mkdir(parents=True)
    (target / "registers" / "reg.toml").write_text(
        '[[item]]\nid = "A"\nrefs = "X"\n\n[[item]]\nid = "A"\nrefs = "Y"\n\n'
        '[[item]]\nid = "X"\n\n[[item]]\nid = "Y"\n'
    )
    document = _run_adapter(target, tmp_path)
    assert [edge["declared_by"] for edge in document["edges"]] == [0, 1]

    entries = _trace(PROFILE_PATH, target)
    assert [edge["tgt"] for edge in entries["reg/A"]["edges"]] == ["X"]
    duplicates = [
        finding["message"] for finding in entries["reg/A"]["findings"]
        if finding["message"].startswith("duplicate node")
    ]
    assert duplicates == [
        "duplicate node 'reg/A': first at registers/reg.toml:1; its 1 edge was not read"
    ]


HELIX = (
    '[[language]]\nname = "rust"\ngrammar = "rust"\nfile-types = ["rs"]\n\n'
    '[[language]]\nname = "ada"\nfile-types = ["adb", "ads"]\n\n'
    '[[language]]\nname = "yaml-ish"\ngrammar = "yaml"\n\n'
    '[[grammar]]\nname = "rust"\n\n'
    '[[grammar]]\nname = "ada"\n\n'
    '[[grammar]]\nname = "yaml"\n'
)


def _helix_profile(tmp_path: Path, **adapter_updates: object) -> Path:
    """A two-table profile whose kinds share names, as Helix's languages.toml does."""
    resolved = resolve_profile(PROFILE_PATH, tmp_path)
    profile = json.loads(resolved.read_text())
    adapter = profile["adapter"]
    adapter["paths"]["files"] = ["*.toml"]
    for key in ("id_prefix", "header", "pathway"):
        adapter.pop(key)
    adapter["tables"] = {
        "language": {
            "kind": "language",
            "id_key": "name",
            "id_prefix": "kind",
            "key_map": {"file-types": "file_types"},
            "edge_keys": {
                "grammar": {"kind": "uses_grammar", "table": "grammar", "fallback": "name"},
            },
        },
        "grammar": {"kind": "grammar", "id_key": "name", "id_prefix": "kind"},
    }
    adapter.update(adapter_updates)
    resolved.write_text(json.dumps(profile))
    return resolved


def _helix_target(tmp_path: Path) -> Path:
    target = tmp_path / "target"
    target.mkdir()
    (target / "languages.toml").write_text(HELIX)
    return target


def test_kind_prefixed_tables_share_names_and_edges_reach_the_target_table(
    tmp_path: Path,
) -> None:
    document = _run_adapter_with_profile(_helix_target(tmp_path), _helix_profile(tmp_path))
    nodes = {node["id"]: node["kind"] for node in document["nodes"]}
    assert nodes == {
        "language/rust": "language", "language/ada": "language",
        "language/yaml-ish": "language",
        "grammar/rust": "grammar", "grammar/ada": "grammar", "grammar/yaml": "grammar",
    }
    edges = {(edge["src"], edge["tgt"], edge["kind"]) for edge in document["edges"]}
    assert edges == {
        ("language/rust", "grammar/rust", "uses_grammar"),
        ("language/ada", "grammar/ada", "uses_grammar"),  # the fallback key
        ("language/yaml-ish", "grammar/yaml", "uses_grammar"),
    }
    assert document["findings"] == []


def test_a_kind_prefix_follows_the_file_stem(tmp_path: Path) -> None:
    profile = _helix_profile(tmp_path, id_prefix="file_stem")
    document = _run_adapter_with_profile(_helix_target(tmp_path), profile)
    ids = {node["id"] for node in document["nodes"]}
    assert "languages/language/rust" in ids and "languages/grammar/rust" in ids
    edges = {(edge["src"], edge["tgt"]) for edge in document["edges"]}
    assert ("languages/language/ada", "languages/grammar/ada") in edges


def test_an_array_of_scalars_maps_to_a_list_attr(tmp_path: Path) -> None:
    document = _run_adapter_with_profile(_helix_target(tmp_path), _helix_profile(tmp_path))
    attrs = {node["id"]: node["attrs"] for node in document["nodes"]}
    assert attrs["language/ada"] == {"file_types": ["adb", "ads"]}
    assert attrs["language/yaml-ish"] == {}


def test_each_unread_list_entry_is_its_own_warning_shown_as_written(tmp_path: Path) -> None:
    # A subset of the list would be a value the register never declared, so the
    # attr is rejected whole; each entry it could not read is a finding of its own,
    # shown as written at its line, so the user sees the gap itself.
    target = tmp_path / "target"
    target.mkdir()
    (target / "languages.toml").write_text(
        '[[language]]\nname = "x"\nfile-types = ["a", { glob = "b" }, "c"]\n'  # 1-3
        '[[language]]\nname = "y"\nfile-types = [{ glob = "d" }]\n'  # 4-6
        '[[language]]\nname = "w"\nfile-types = [["e"], "f"]\n'  # 7-9
        '[[language]]\nname = "u"\nfile-types = [\n  { glob = "g" },\n  "h",\n'
        '  [\n    "i",\n    "j",\n  ],\n]\n'  # 10-19
        '[[language]]\nname = "z"\nfile-types = { glob = "k" }\n'  # 20-22
        '[[language]]\nname = "v"\nfile-types = ["l", inf]\n'  # 23-25
    )
    document = _run_adapter_with_profile(target, _helix_profile(tmp_path))
    attrs = {node["id"]: node["attrs"] for node in document["nodes"]}
    assert all(attrs[f"language/{name}"] == {} for name in "xywuzv")
    unread = [
        (f["node_id"], f["provenance"]["line"], f["message"])
        for f in document["findings"]
        if f["code"] == "UNREAD_ENTRIES"
    ]
    assert {f["severity"] for f in document["findings"] if f["code"] == "UNREAD_ENTRIES"} == {
        "warning"
    }
    assert [(node, line) for node, line, _ in unread] == [
        ("language/x", 3),
        ("language/y", 6),
        ("language/w", 9),
        ("language/u", 13),
        ("language/u", 15),
    ]
    messages = [message for *_, message in unread]
    assert messages[0] == (
        "languages.toml: item 'language/x' has mapped key 'file-types' entry 2 of 3: "
        '{ glob = "b" } is a table, not read; the attr is not set'
    )
    assert 'entry 1 of 1: { glob = "d" } is a table' in messages[1]
    assert 'entry 1 of 2: ["e"] is an array' in messages[2]
    # A multi-line entry is shown on one line.
    assert 'entry 3 of 3: [ "i", "j", ] is an array' in messages[4]
    # A whole value that is a table cannot be an attr at all: the profile's mapping
    # is wrong, and that stays an error. So does a scalar JSON cannot hold, which
    # must not drop out of the list in silence.
    errors = [f for f in document["findings"] if f["code"] == "PARSE_ERROR"]
    assert [(e["node_id"], e["provenance"]["line"]) for e in errors] == [
        ("language/z", 22),
        ("language/v", 25),
    ], [e["message"] for e in errors]


def test_each_unread_edge_entry_is_its_own_error_and_the_strings_are_edges(
    tmp_path: Path,
) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "languages.toml").write_text(
        '[[language]]\nname = "x"\ngrammar = [\n  "g",\n  { name = "h" },\n  7,\n]\n'
        '[[grammar]]\nname = "g"\n'
    )
    document = _run_adapter_with_profile(target, _helix_profile(tmp_path))
    assert [(e["tgt"], e["provenance"]["line"]) for e in document["edges"]] == [("grammar/g", 4)]
    errors = [
        (f["severity"], f["provenance"]["line"], f["message"])
        for f in document["findings"]
        if f["code"] == "PARSE_ERROR"
    ]
    assert errors == [
        (
            "error",
            5,
            "languages.toml: item 'language/x' has edge key 'grammar' entry 2 of 3: "
            '{ name = "h" } is a table, not a target string; it was not read',
        ),
        (
            "error",
            6,
            "languages.toml: item 'language/x' has edge key 'grammar' entry 3 of 3: "
            "7 is an integer, not a target string; it was not read",
        ),
    ]

def test_an_edge_entry_naming_an_unknown_table_exits_two(tmp_path: Path) -> None:
    profile = _helix_profile(tmp_path)
    data = json.loads(profile.read_text())
    data["adapter"]["tables"]["language"]["edge_keys"]["grammar"]["table"] = "grammars"
    profile.write_text(json.dumps(data))
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(profile), "--target", str(tmp_path)],
        capture_output=True, text=True,
    )
    assert result.returncode == 2
    assert "grammars" in result.stderr


def test_nodes_edges_and_findings_carry_the_line_they_were_written_on(tmp_path: Path) -> None:
    # A row is at its header, an edge at its target, a finding at its value; only a
    # record that is the whole file stays at line 0.
    target = tmp_path / "target"
    target.mkdir()
    (target / "languages.toml").write_text(
        "# languages\n"
        "[[language]]\n"
        'name = "x"\n'
        'grammar = "g"\n'
        "file-types = [\n"
        '  "a",\n'
        '  { glob = "b" },\n'
        "]\n"
        "\n"
        "[[grammar]]\n"
        'name = "g"\n'
    )
    document = _run_adapter_with_profile(target, _helix_profile(tmp_path))
    lines = {node["id"]: node["provenance"]["line"] for node in document["nodes"]}
    assert lines == {"language/x": 2, "grammar/g": 10}
    assert [edge["provenance"]["line"] for edge in document["edges"]] == [4]
    # The finding is at the entry it could not read, not at the array.
    assert [f["provenance"]["line"] for f in document["findings"]] == [7]


def _shaped_profile(tmp_path: Path, key_map: dict, edge: dict | None = None) -> Path:
    """The Helix profile with the language table's key_map, and its grammar edge, replaced."""
    profile_path = _helix_profile(tmp_path)
    profile = json.loads(profile_path.read_text())
    language = profile["adapter"]["tables"]["language"]
    language["key_map"] = key_map
    if edge is not None:
        language["edge_keys"]["grammar"] = edge
    profile_path.write_text(json.dumps(profile))
    return profile_path


def _languages(tmp_path: Path, text: str) -> Path:
    target = tmp_path / "target"
    target.mkdir()
    (target / "languages.toml").write_text(text)
    return target


def test_declared_shapes_keep_what_they_claim_and_report_the_rest(tmp_path: Path) -> None:
    # Each attr is "the entries of this shape", so an entry no shape claims costs
    # no claimed attr; it is reported where it is, as written.
    target = _languages(
        tmp_path,
        '[[language]]\nname = "x"\n'
        'file-types = ["a", { glob = "b" }, "c", { regex = "r" }, ["n"]]\n'  # 1-3
        '[[language]]\nname = "y"\nfile-types = ["d"]\n'  # 4-6
        '[[language]]\nname = "e"\nfile-types = []\n',  # 7-9
    )
    profile = _shaped_profile(
        tmp_path, {"file-types": {"scalars": "file_types", "tables": {"glob": "file_globs"}}}
    )
    document = _run_adapter_with_profile(target, profile)
    attrs = {node["id"]: node["attrs"] for node in document["nodes"]}
    assert attrs["language/x"] == {"file_types": ["a", "c"], "file_globs": ["b"]}
    assert attrs["language/y"] == {"file_types": ["d"], "file_globs": []}
    assert attrs["language/e"] == {"file_types": [], "file_globs": []}
    unread = [
        (f["severity"], f["node_id"], f["provenance"]["line"], f["message"])
        for f in document["findings"]
        if f["code"] == "UNREAD_ENTRIES"
    ]
    assert unread == [
        ("warning", "language/x", 3,
         "languages.toml: item 'language/x' has mapped key 'file-types' entry 4 of 5: "
         '{ regex = "r" } is a table no declared shape claims, not read'),
        ("warning", "language/x", 3,
         "languages.toml: item 'language/x' has mapped key 'file-types' entry 5 of 5: "
         '["n"] is an array no declared shape claims, not read'),
    ]
    assert not [f for f in document["findings"] if f["code"] == "PARSE_ERROR"]


def test_shapes_may_fill_one_attr_and_an_unreadable_claim_withholds_only_its_attr(
    tmp_path: Path,
) -> None:
    target = _languages(
        tmp_path,
        '[[language]]\nname = "s"\n'
        'language-servers = ["a", { name = "b", except-features = ["f"] }, "c"]\n'  # 1-3
        '[[language]]\nname = "w"\nlanguage-servers = ["d", { name = ["e"] }]\n'  # 4-6
        '[[language]]\nname = "v"\nlanguage-servers = ["g", inf]\n'  # 7-9
        '[[language]]\nname = "z"\nlanguage-servers = "h"\n'  # 10-12
        '[[language]]\nname = "t"\nlanguage-servers = [{ name = inf }]\n',  # 13-15
    )
    profile = _shaped_profile(
        tmp_path,
        {"language-servers": {"scalars": "servers", "tables": {"name": "servers"}}},
    )
    document = _run_adapter_with_profile(target, profile)
    attrs = {node["id"]: node["attrs"] for node in document["nodes"]}
    # Bare names and named tables are one list, in the order written.
    assert attrs["language/s"] == {"servers": ["a", "b", "c"]}
    assert attrs["language/w"] == attrs["language/v"] == attrs["language/z"] == {}
    assert attrs["language/t"] == {}
    errors = [
        (f["node_id"], f["provenance"]["line"], f["message"])
        for f in document["findings"]
        if f["code"] == "PARSE_ERROR"
    ]
    assert errors == [
        ("language/w", 6,
         "languages.toml: item 'language/w' has mapped key 'language-servers' entry 2 of 2: "
         "{ name = [\"e\"] } has 'name' of type array, expected a scalar; 'servers' is not set"),
        ("language/v", 9,
         "languages.toml: item 'language/v' has mapped key 'language-servers' entry 2 of 2: "
         "inf is a non-finite float, which JSON cannot hold; 'servers' is not set"),
        ("language/z", 12,
         "languages.toml: item 'language/z' has mapped key 'language-servers' of type "
         "string, expected an array for its declared shapes"),
        ("language/t", 15,
         "languages.toml: item 'language/t' has mapped key 'language-servers' entry 1 of 1: "
         "{ name = inf } has 'name' that is a non-finite float, which JSON cannot hold; "
         "'servers' is not set"),
    ]
    assert not [f for f in document["findings"] if f["code"] == "UNREAD_ENTRIES"]


def test_an_entry_key_makes_a_tables_field_the_edge_target(tmp_path: Path) -> None:
    target = _languages(
        tmp_path,
        '[[language]]\nname = "x"\ngrammar = ["g", { name = "h" }, { other = "q" }]\n'  # 1-3
        '[[language]]\nname = "y"\ngrammar = { name = "i" }\n'  # 4-6
        '[[grammar]]\nname = "g"\n[[grammar]]\nname = "h"\n[[grammar]]\nname = "i"\n',
    )
    profile = _shaped_profile(
        tmp_path,
        {},
        {"kind": "uses_grammar", "table": "grammar", "fallback": "name", "entry_key": "name"},
    )
    document = _run_adapter_with_profile(target, profile)
    assert [(e["src"], e["tgt"]) for e in document["edges"]] == [
        ("language/x", "grammar/g"),
        ("language/x", "grammar/h"),
        ("language/y", "grammar/i"),
    ]
    assert [(f["code"], f["provenance"]["line"], f["message"]) for f in document["findings"]] == [
        ("PARSE_ERROR", 3,
         "languages.toml: item 'language/x' has edge key 'grammar' entry 3 of 3: "
         "{ other = \"q\" } is a table, not a target string or a table with 'name'; "
         "it was not read"),
    ]


@pytest.mark.parametrize(
    ("key_map", "edge", "named"),
    [
        ({"file-types": {"scalar": "x"}}, None, "scalar"),
        ({"file-types": {}}, None, "declares no shape"),
        ({"file-types": {"tables": {}}}, None, "declares no shape"),
        ({"file-types": {"tables": {"glob": " "}}}, None, "must not be empty"),
        ({}, {"kind": "uses_grammar", "entry_key": ""}, "must not be empty"),
    ],
)
def test_a_malformed_shape_declaration_exits_two(
    tmp_path: Path, key_map: dict, edge: dict | None, named: str
) -> None:
    profile = _shaped_profile(tmp_path, key_map, edge)
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(profile), "--target", str(tmp_path)],
        capture_output=True, text=True,
    )
    assert result.returncode == 2, result.stdout
    assert named in result.stderr, result.stderr

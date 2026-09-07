from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from adapters import tomlreg
from conftest import load_yaml_profile as load_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "tomlreg.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "tomlreg"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


@pytest.fixture(scope="module")
def profile():
    return load_profile(str(PROFILE_PATH))


@pytest.fixture(scope="module")
def mini_target(request):
    return request.path.parent / "fixtures" / "mini-tomlreg"


@pytest.fixture(scope="module")
def graph(profile, mini_target):
    return tomlreg.build_graph(profile, mini_target)


def _codes(graph, code):
    return [i for i in graph.adapter_issues if i.code == code]


def _edges(graph, kind):
    return {(s, t) for s, t, k, _ in graph.iter_edges() if k == kind}


# --- register nodes -------------------------------------------------------

# Requirement: Build register nodes from the register table
def test_register_node_per_file(graph):
    for stem in ("alpha", "beta", "dangling", "unread"):
        assert graph.has_node(stem), f"missing register node {stem}"
    assert graph.node_data("alpha")["kind"] == "register"


def test_register_node_attrs_carried(graph):
    attrs = graph.node_data("alpha")["attrs"]
    assert attrs["id"] == "reg.alpha"
    assert attrs["register_version"] == 0
    assert attrs["status"] == "draft-3"


def test_register_node_absent_table_is_reported_and_skipped(graph):
    errs = [i for i in _codes(graph, "PARSE_ERROR") if "noregister.toml" in i.message]
    assert len(errs) == 1
    assert not graph.has_node("noregister")
    # The file's own row must not reach the graph either: the file has no
    # identity, so nothing in it can be qualified.
    assert not graph.has_node("noregister/unreachable")


# --- item nodes -----------------------------------------------------------

# Requirement: Build item nodes with register-qualified IDs
def test_item_node_id_is_register_qualified(graph):
    assert graph.has_node("alpha/one")
    assert graph.node_data("alpha/one")["kind"] == "item"
    assert graph.node_data("alpha/one")["attrs"]["id"] == "one"


def test_item_node_same_slug_in_two_registers(graph):
    assert graph.has_node("alpha/one")
    assert graph.has_node("beta/one")


def test_item_node_without_id_reported_by_file_and_index(graph):
    errs = [i for i in _codes(graph, "PARSE_ERROR") if "baditem.toml" in i.message]
    assert len(errs) == 1
    assert "row 1" in errs[0].message
    assert graph.has_node("baditem/has_an_id")


# --- edges ----------------------------------------------------------------

# Requirement: Resolve cross-register references as edges
def test_edge_refs_become_references(graph):
    assert ("alpha/one", "beta/two") in _edges(graph, "references")


def test_edge_register_key_becomes_belongs_to(graph):
    assert ("alpha/one", "beta") in _edges(graph, "belongs_to")


def test_edge_to_absent_item_is_still_emitted(graph):
    # Resolving it is the core's job. An adapter that dropped it would hide
    # the finding rather than report it.
    assert ("dangling/overdue", "alpha/absent") in _edges(graph, "references")
    assert not graph.has_node("alpha/absent")


# --- the ordering axis ----------------------------------------------------

# Requirement: Read the ordering axis from the register
def test_axis_attached_from_the_register(graph):
    axis = graph.axis("stage")
    assert axis is not None
    assert axis.order == ["s1", "s2", "s3"]
    assert axis.current == "s2"


def test_axis_absent_when_the_register_declares_none(profile, tmp_path):
    (tmp_path / "registers").mkdir()
    (tmp_path / "registers" / "index.toml").write_text(
        '[register]\nid = "reg.index"\nregister_version = 0\nstatus = "d"\n'
    )
    built = tomlreg.build_graph(profile, tmp_path)
    assert built.axis("stage") is None
    assert not _codes(built, "AXIS_INVALID")


def test_axis_current_not_in_order_is_reported(profile, tmp_path):
    (tmp_path / "registers").mkdir()
    (tmp_path / "registers" / "index.toml").write_text(
        '[register]\nid = "reg.index"\nregister_version = 0\nstatus = "d"\n'
        'stage_order = ["s1", "s2"]\ncurrent_stage = "s9"\n'
    )
    built = tomlreg.build_graph(profile, tmp_path)
    assert built.axis("stage") is None
    assert len(_codes(built, "AXIS_INVALID")) == 1


def test_axis_half_declared_pair_is_reported(profile, tmp_path):
    (tmp_path / "registers").mkdir()
    (tmp_path / "registers" / "index.toml").write_text(
        '[register]\nid = "reg.index"\nregister_version = 0\nstatus = "d"\n'
        'stage_order = ["s1", "s2"]\n'
    )
    built = tomlreg.build_graph(profile, tmp_path)
    assert built.axis("stage") is None
    errs = _codes(built, "AXIS_INVALID")
    assert len(errs) == 1
    assert "current_stage" in errs[0].message


# --- unreadable input -----------------------------------------------------

# Requirement: Unreadable TOML is reported, never raised
def test_malformed_file_is_reported_not_raised(graph):
    errs = [i for i in _codes(graph, "PARSE_ERROR") if "broken.toml" in i.message]
    assert len(errs) == 1


def test_malformed_file_does_not_stop_the_others(graph):
    # broken.toml sorts before the files after it; they must still be read.
    assert graph.has_node("dangling")
    assert graph.has_node("wrongshape")


# --- unread tables --------------------------------------------------------

# Requirement: Report unread row-bearing tables
def test_unread_row_bearing_table_reported(graph):
    errs = [i for i in _codes(graph, "PARSE_ERROR") if "unread.toml" in i.message]
    assert len(errs) == 1
    assert "'note'" in errs[0].message


def test_unread_check_ignores_a_plain_metadata_table(graph):
    # alpha.toml's [projection] is a table, not an array of tables.
    assert not [
        i for i in _codes(graph, "PARSE_ERROR") if "projection" in i.message
    ]


# --- fields of the wrong shape --------------------------------------------

# Requirement: Report a recognized field carrying the wrong shape
def test_wrong_shape_refs_is_reported(graph):
    errs = [
        i for i in _codes(graph, "PARSE_ERROR")
        if "wrongshape/string_refs" in (i.node_id or "")
    ]
    assert len(errs) == 1
    assert "'refs'" in errs[0].message


def test_wrong_shape_refs_emits_no_character_edges(graph):
    # Iterating the string would yield one edge per character.
    assert not [
        (s, t) for s, t in _edges(graph, "references") if s == "wrongshape/string_refs"
    ]
    assert graph.has_node("wrongshape/string_refs")


def test_wrong_shape_register_key_is_reported(graph):
    errs = [
        i for i in _codes(graph, "PARSE_ERROR")
        if "wrongshape/numeric_register" in (i.node_id or "")
    ]
    assert len(errs) == 1
    assert graph.has_node("wrongshape/numeric_register")


# --- the standing gate ----------------------------------------------------

# Requirement: Serve as the standing non-markdown gate
def test_gate_builds_nodes_and_edges(graph):
    nodes = list(graph.iter_nodes())
    edges = list(graph.iter_edges())
    assert nodes, "the gate proves nothing over an empty graph"
    # The edge count is the load-bearing half: zero dangling references over
    # zero edges is satisfied by an adapter that built no edges at all.
    assert edges, "no edges built, so no cross-reference was actually checked"


@needs_core
def test_gate_runs_through_the_core(mini_target):
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(mini_target),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode != 2, f"core could not run: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    assert findings, "the fixture carries known faults; none were reported"


# Requirement: Axis severity resolution
@needs_core
def test_gate_axis_demotes_a_finding_not_yet_due(mini_target):
    """The bound axis must actually change a severity, or the binding is inert."""
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(mini_target),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode != 2, f"core could not run: {result.stderr}"
    dangling = {
        i["message"].split("'")[1]: i["severity"]
        for i in json.loads(result.stdout)["findings"]
        if i["code"] == "DANGLING_REF"
    }
    # s3 is past the register's current stage, so this one is not due yet.
    assert dangling["dangling/not_yet"] == "info"
    # s2 is the current stage, so this one stands.
    assert dangling["dangling/overdue"] == "error"


# Requirement: Serve as the standing non-markdown gate
def test_gate_adapter_exits_zero_on_malformed_input(mini_target, tmp_path):
    from conftest import resolve_profile

    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile",
         str(resolve_profile(PROFILE_PATH, tmp_path)),
         "--target", str(mini_target)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    document = json.loads(result.stdout)
    assert [i for i in document["issues"] if i["code"] == "PARSE_ERROR"]

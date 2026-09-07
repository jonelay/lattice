from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from adapters import openspec
from conftest import load_yaml_profile as load_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "openspec.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "openspec"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


@pytest.fixture(scope="module")
def profile():
    return load_profile(str(PROFILE_PATH))


@pytest.fixture(scope="module")
def mini_target(request):
    return request.path.parent / "fixtures" / "mini-openspec"


@pytest.fixture(scope="module")
def graph(profile, mini_target):
    return openspec.build_graph(profile, mini_target)


def _codes(graph, code):
    return [i for i in graph.adapter_issues if i.code == code]


def _edges(graph, kind):
    return {(s, t) for s, t, k, _ in graph.iter_edges() if k == kind}


# --- capability and requirement nodes -------------------------------------

# Requirement: Build capability and requirement nodes from the register
def test_capability_node_per_spec_file(graph):
    for name in ("alpha", "beta"):
        assert graph.has_node(name), f"missing capability node {name}"
    assert graph.node_data("alpha")["kind"] == "capability"


def test_requirement_node_per_heading(graph):
    for title in ("Alpha parses input", "Alpha reports errors", "Beta holds state"):
        assert graph.has_node(title), f"missing requirement node {title}"
    assert graph.node_data("Beta holds state")["kind"] == "requirement"


def test_contains_edge_from_capability_to_requirement(graph):
    contains = _edges(graph, "contains")
    assert ("alpha", "Alpha parses input") in contains
    assert ("alpha", "Alpha reports errors") in contains
    assert ("beta", "Beta holds state") in contains


def test_requirement_node_carries_its_title_as_an_attr(graph):
    for node_id, data in graph.iter_nodes():
        if data["kind"] == "requirement":
            assert data["attrs"]["title"] == node_id


def test_scenarios_are_counted_not_emitted(graph):
    assert graph.node_data("Beta holds state")["attrs"]["scenarios"] == 3
    assert graph.node_data("Alpha parses input")["attrs"]["scenarios"] == 2
    kinds = {data["kind"] for _, data in graph.iter_nodes()}
    assert "scenario" not in kinds


# --- test nodes -----------------------------------------------------------

# Requirement: Build test nodes from the cited test files
def test_node_per_test_function_with_attrs(graph):
    node = graph.node_data("crates/lattice-core/tests/basic.rs::parses_a")
    assert node["kind"] == "test"
    assert node["attrs"] == {
        "file": "crates/lattice-core/tests/basic.rs", "function": "parses_a",
    }


def test_identity_is_path_qualified(graph):
    assert graph.has_node("tests/test_items.py::test_uncited")
    assert graph.has_node("tests/test_more.py::test_uncited")


def test_identity_is_class_qualified_for_a_method(graph):
    # test_items.py declares test_beta_state both at top level and in a class;
    # both must reach the graph as distinct nodes.
    assert graph.has_node("tests/test_items.py::test_beta_state")
    assert graph.has_node("tests/test_items.py::TestGrouped::test_beta_state")


def test_uncited_test_is_present_with_no_edges(graph):
    for node_id in ("crates/lattice-core/tests/basic.rs::early_uncited",
                    "tests/test_items.py::test_uncited"):
        assert graph.has_node(node_id)
        assert not [
            (s, t) for s, t in _edges(graph, "verifies") if s == node_id
        ]


def test_non_test_function_is_not_a_node(graph):
    assert not graph.has_node(
        "crates/lattice-core/tests/basic.rs::helper_not_a_test"
    )


def test_attribute_between_test_marker_and_fn_still_binds(graph):
    # parses_b carries #[ignore] between #[test] and fn.
    assert graph.has_node("crates/lattice-core/tests/basic.rs::parses_b")


def test_async_test_function_is_discovered(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: Solo stands\n"
        "async def test_a():\n    pass\n",
    )
    built = openspec.build_graph(profile, target)
    assert built.has_node("tests/test_s.py::test_a")
    assert ("tests/test_s.py::test_a", "Solo stands") in _edges(built, "verifies")


def test_a_unicode_line_break_in_a_string_does_not_break_the_scan(profile, tmp_path):
    """`str.splitlines` also cuts at U+2028, desynchronizing line numbers from
    the parsed tree's and splitting a string literal mid-file."""
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: Solo stands\n"
        "def test_a():\n    s = 'x y'\n"
        "# Requirement: Solo reports\n"
        "def test_b():\n    pass\n",
    )
    built = openspec.build_graph(profile, target)
    assert not [i for i in _codes(built, "PARSE_ERROR")
                if "syntax error" in i.message]
    verifies = _edges(built, "verifies")
    assert ("tests/test_s.py::test_a", "Solo stands") in verifies
    assert ("tests/test_s.py::test_b", "Solo reports") in verifies
    assert ("tests/test_s.py::test_b", "Solo stands") not in verifies


def test_a_def_inside_a_string_literal_is_not_a_test(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "def test_real():\n"
        "    fixture = '''\n"
        "def test_phantom():\n"
        "    pass\n"
        "'''\n",
    )
    built = openspec.build_graph(profile, target)
    assert built.has_node("tests/test_s.py::test_real")
    assert not built.has_node("tests/test_s.py::test_phantom")


# --- citations ------------------------------------------------------------

# Requirement: A citation is a section header binding the tests that follow it
def test_citation_binds_the_group_beneath_it(graph):
    verifies = _edges(graph, "verifies")
    rs = "crates/lattice-core/tests/basic.rs"
    assert (f"{rs}::parses_a", "Alpha parses input") in verifies
    assert (f"{rs}::parses_b", "Alpha parses input") in verifies
    assert (f"{rs}::reports", "Alpha reports errors") in verifies
    assert (f"{rs}::reports", "Alpha parses input") not in verifies


def test_citation_scope_crosses_no_file_boundary(graph):
    # test_more.py holds no citation; the last one in test_items.py must not
    # leak into it.
    assert not [
        (s, t) for s, t in _edges(graph, "verifies")
        if s == "tests/test_more.py::test_uncited"
    ]


def test_dangling_citation_edge_is_still_emitted(graph):
    assert (
        "tests/test_items.py::test_dangling", "No such requirement"
    ) in _edges(graph, "verifies")
    assert not graph.has_node("No such requirement")


def _stack_target(tmp_path, test_rel, test_text):
    (tmp_path / "openspec" / "specs" / "solo").mkdir(parents=True)
    (tmp_path / "openspec" / "specs" / "solo" / "spec.md").write_text(
        "### Requirement: Solo stands\n### Requirement: Solo reports\n"
    )
    path = tmp_path / test_rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(test_text)
    return tmp_path


def test_stacked_citations_bind_the_group_to_every_title(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: Solo stands\n"
        "# Requirement: Solo reports\n"
        "def test_a():\n    pass\n"
        "def test_b():\n    pass\n",
    )
    verifies = _edges(openspec.build_graph(profile, target), "verifies")
    for fn in ("test_a", "test_b"):
        assert (f"tests/test_s.py::{fn}", "Solo stands") in verifies
        assert (f"tests/test_s.py::{fn}", "Solo reports") in verifies


def test_stacked_edges_carry_their_own_citation_line(profile, tmp_path):
    target = _stack_target(
        tmp_path, "crates/lattice-core/tests/s.rs",
        "// Requirement: Solo stands\n"
        "// Requirement: Solo reports\n"
        "#[test]\nfn a() {}\n",
    )
    built = openspec.build_graph(profile, target)
    lines = {
        tgt: data["provenance"].line
        for src, tgt, kind, data in built.iter_edges()
        if kind == "verifies" and src == "crates/lattice-core/tests/s.rs::a"
    }
    assert lines == {"Solo stands": 1, "Solo reports": 2}


def test_blank_line_breaks_the_stack(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: Solo stands\n"
        "\n"
        "# Requirement: Solo reports\n"
        "def test_a():\n    pass\n",
    )
    verifies = _edges(openspec.build_graph(profile, target), "verifies")
    assert ("tests/test_s.py::test_a", "Solo reports") in verifies
    assert ("tests/test_s.py::test_a", "Solo stands") not in verifies


def test_repeated_title_in_a_stack_emits_two_edges(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: Solo stands\n"
        "# Requirement: Solo stands\n"
        "def test_a():\n    pass\n",
    )
    built = openspec.build_graph(profile, target)
    edges = [
        (src, tgt) for src, tgt, kind, _ in built.iter_edges()
        if kind == "verifies"
    ]
    assert edges.count(("tests/test_s.py::test_a", "Solo stands")) == 2


# Requirement: One requirement title per citation line
def test_compound_citation_is_one_unsplit_title(graph):
    assert (
        "tests/test_items.py::test_compound",
        "Alpha parses input / Alpha reports errors",
    ) in _edges(graph, "verifies")


# --- unreadable input -----------------------------------------------------

# Requirement: Unreadable input is reported, never dropped
def test_undecodable_spec_file_is_reported_not_raised(graph):
    errs = [i for i in _codes(graph, "PARSE_ERROR") if "spec.md" in i.message]
    assert len(errs) == 1
    assert "broken" in errs[0].provenance.file


def test_undecodable_spec_file_does_not_stop_the_others(graph):
    # broken/ sorts between alpha/ and beta/; beta must still be read.
    assert graph.has_node("beta")
    assert graph.has_node("Beta holds state")


def test_unresolvable_spec_dir_is_reported_and_the_tests_still_read(profile, tmp_path):
    (tmp_path / "tests").mkdir()
    (tmp_path / "tests" / "test_only.py").write_text("def test_alone():\n    pass\n")
    (tmp_path / "crates" / "lattice-core" / "tests").mkdir(parents=True)
    (tmp_path / "crates" / "lattice-core" / "tests" / "t.rs").write_text(
        "#[test]\nfn alone() {}\n"
    )
    built = openspec.build_graph(profile, tmp_path)
    errs = [i for i in _codes(built, "PARSE_ERROR") if "spec directory" in i.message]
    assert len(errs) == 1
    assert built.has_node("tests/test_only.py::test_alone")


def test_empty_test_glob_is_reported(profile, tmp_path):
    (tmp_path / "openspec" / "specs" / "solo").mkdir(parents=True)
    (tmp_path / "openspec" / "specs" / "solo" / "spec.md").write_text(
        "### Requirement: Solo stands\n"
    )
    built = openspec.build_graph(profile, tmp_path)
    globs = [i for i in _codes(built, "PARSE_ERROR") if "matched no file" in i.message]
    assert len(globs) == 2, [i.message for i in built.adapter_issues]
    assert built.has_node("Solo stands")


def test_unparseable_python_file_is_reported_not_guessed(profile, tmp_path):
    from adapters._types import Severity

    target = _stack_target(
        tmp_path, "tests/test_bad.py",
        "def test_a(:\n    pass\n",
    )
    (tmp_path / "tests" / "test_good.py").write_text(
        "def test_b():\n    pass\n"
    )
    built = openspec.build_graph(profile, target)
    errs = [i for i in _codes(built, "PARSE_ERROR") if "syntax error" in i.message]
    assert len(errs) == 1, [i.message for i in built.adapter_issues]
    assert errs[0].severity == Severity.WARNING
    assert not built.has_node("tests/test_bad.py::test_a")
    assert built.has_node("tests/test_good.py::test_b")


# Requirement: Run as a stdlib-only contract program
def test_adapter_program_exits_zero_on_malformed_input(mini_target, tmp_path):
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


# --- the standing gate ----------------------------------------------------

# Requirement: Lattice audits its own register
def test_gate_builds_nodes_and_edges(graph):
    nodes = list(graph.iter_nodes())
    edges = list(graph.iter_edges())
    assert nodes, "the gate proves nothing over an empty graph"
    # The edge count is the load-bearing half: zero dangling references over
    # zero edges is satisfied by an adapter that built no edges at all.
    assert edges, "no edges built, so no cross-reference was actually checked"


# Requirement: Run as a stdlib-only contract program
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


# Requirement: Coverage is visible and never exit-affecting
@needs_core
def test_gate_dangling_citation_is_an_error_and_coverage_a_hint(mini_target):
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(mini_target),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode == 1, f"dangling citations must fail the run: {result.stderr}"
    findings = json.loads(result.stdout)["findings"]
    dangling = [f for f in findings if f["code"] == "DANGLING_REF"]
    assert dangling and all(f["severity"] == "error" for f in dangling)
    coverage = [f for f in findings if f["code"] == "COVERAGE"]
    assert coverage and all(f["severity"] == "hint" for f in coverage)


# --- body text -------------------------------------------------------------


def _body_target(tmp_path, spec_text, test_rel="tests/test_b.py", test_text=""):
    (tmp_path / "openspec" / "specs" / "solo").mkdir(parents=True)
    (tmp_path / "openspec" / "specs" / "solo" / "spec.md").write_text(spec_text)
    path = tmp_path / test_rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(test_text)
    return tmp_path


def _body(graph, node_id):
    return graph.node_data(node_id)["attrs"].get("body")


# Requirement: Build capability and requirement nodes from the register
def test_requirement_body_runs_to_the_next_heading(profile, tmp_path):
    target = _body_target(
        tmp_path,
        "### Requirement: One\n"
        "The system SHALL do a thing.\n"
        "\n"
        "#### Scenario: It happens\n"
        "- **WHEN** asked\n"
        "- **THEN** it does\n"
        "\n"
        "### Requirement: Two\n"
        "Unrelated prose.\n",
    )
    body = _body(openspec.build_graph(profile, target), "One")

    assert "The system SHALL do a thing." in body
    assert "#### Scenario: It happens" in body
    assert "- **THEN** it does" in body
    assert "Unrelated prose" not in body


def test_requirement_body_excludes_a_verification_pointer_in_either_form(
    profile, tmp_path
):
    target = _body_target(
        tmp_path,
        "### Requirement: Plain\n"
        "Prose one.\n"
        "Verified by: `cargo test --test alpha`\n"
        "\n"
        "### Requirement: Bold\n"
        "Prose two.\n"
        "**Verified by:** `.venv/bin/python -m pytest tests/test_x.py`\n",
    )
    built = openspec.build_graph(profile, target)

    assert "Verified by" not in _body(built, "Plain")
    assert "Prose one." in _body(built, "Plain")
    assert "Verified by" not in _body(built, "Bold")
    assert "Prose two." in _body(built, "Bold")


def test_requirement_with_no_body_carries_no_body_attr(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: Empty\n### Requirement: Next\nProse.\n"
    )
    assert _body(openspec.build_graph(profile, target), "Empty") is None


# Requirement: Build test nodes from the cited test files
def test_python_test_body_stops_at_the_dedent(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "tests/test_b.py",
        "def test_a():\n"
        "    first = 1\n"
        "    assert first\n"
        "\n"
        "def test_b():\n"
        "    assert other_thing\n",
    )
    body = _body(openspec.build_graph(profile, target), "tests/test_b.py::test_a")

    assert "first = 1" in body
    assert "other_thing" not in body


def test_rust_test_body_stops_at_its_closing_brace(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/lattice-core/tests/b.rs",
        "#[test]\nfn a() {\n    let mine = 1;\n}\n"
        "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    body = _body(
        openspec.build_graph(profile, target), "crates/lattice-core/tests/b.rs::a"
    )

    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_one_line_rust_test_absorbs_nothing(profile, tmp_path):
    """The column-0-brace heuristic runs straight past `fn a() {}` into the
    functions after it, attaching another test's code as this node's body."""
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/lattice-core/tests/b.rs",
        "#[test]\nfn a() {}\n"
        "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    built = openspec.build_graph(profile, target)

    assert _body(built, "crates/lattice-core/tests/b.rs::a") is None
    assert "theirs" in _body(built, "crates/lattice-core/tests/b.rs::b")


def test_an_indented_closing_brace_still_bounds_the_body(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/lattice-core/tests/b.rs",
        "mod inner {\n"
        "    #[test]\n    fn a() {\n        let mine = 1;\n    }\n"
        "    #[test]\n    fn b() {\n        let theirs = 2;\n    }\n"
        "}\n",
    )
    body = _body(
        openspec.build_graph(profile, target), "crates/lattice-core/tests/b.rs::a"
    )

    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_brace_in_a_string_does_not_end_the_body(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/lattice-core/tests/b.rs",
        '#[test]\nfn a() {\n    let s = "}";\n    let mine = 1;\n}\n'
        "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    body = _body(
        openspec.build_graph(profile, target), "crates/lattice-core/tests/b.rs::a"
    )

    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_unclosed_rust_test_body_is_reported_not_guessed(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/lattice-core/tests/b.rs",
        "#[test]\nfn a() {\n    let never_closed = 1;\n",
    )
    built = openspec.build_graph(profile, target)

    assert _body(built, "crates/lattice-core/tests/b.rs::a") is None
    assert _codes(built, "PARSE_ERROR"), "unresolvable bounds must be reported"

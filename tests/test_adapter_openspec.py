from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from adapters import openspec
from adapters._types import Severity
from conftest import load_yaml_profile as load_profile

REPO_ROOT = Path(__file__).parent.parent
PROFILE_PATH = REPO_ROOT / "profiles" / "openspec.yaml"
ADAPTER_PATH = REPO_ROOT / "adapters" / "openspec"
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"

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

# Requirement: adapter-openspec/Build capability and requirement nodes from the register
def test_capability_node_per_spec_file(graph):
    for name in ("alpha", "beta"):
        assert graph.has_node(name), f"missing capability node {name}"
    assert graph.node_data("alpha")["kind"] == "capability"


def test_requirement_node_per_heading(graph):
    for title in ("alpha/Alpha parses input", "alpha/Alpha reports errors", "beta/Beta holds state"):
        assert graph.has_node(title), f"missing requirement node {title}"
    assert graph.node_data("beta/Beta holds state")["kind"] == "requirement"


def test_same_title_in_two_capabilities_is_two_requirements(graph):
    # Titled alone, the second node collided with the first: its attrs were
    # lost and its contains edge landed on the other capability's requirement.
    alpha = graph.node_data("alpha/Shared title")
    beta = graph.node_data("beta/Shared title")
    assert alpha["attrs"]["scenarios"] == 1
    assert beta["attrs"]["scenarios"] == 2
    assert "Alpha's requirement" in alpha["attrs"]["body"]
    assert "Beta's requirement" in beta["attrs"]["body"]
    contains = _edges(graph, "contains")
    assert ("alpha", "alpha/Shared title") in contains
    assert ("beta", "beta/Shared title") in contains


def test_contains_edge_from_capability_to_requirement(graph):
    contains = _edges(graph, "contains")
    assert ("alpha", "alpha/Alpha parses input") in contains
    assert ("alpha", "alpha/Alpha reports errors") in contains
    assert ("beta", "beta/Beta holds state") in contains


def test_requirement_node_carries_its_title_as_an_attr(graph):
    for node_id, data in graph.iter_nodes():
        if data["kind"] == "requirement":
            assert node_id.split("/", 1)[1] == data["attrs"]["title"]


def test_scenarios_are_counted_not_emitted(graph):
    assert graph.node_data("beta/Beta holds state")["attrs"]["scenarios"] == 3
    assert graph.node_data("alpha/Alpha parses input")["attrs"]["scenarios"] == 2
    kinds = {data["kind"] for _, data in graph.iter_nodes()}
    assert "scenario" not in kinds


# --- test nodes -----------------------------------------------------------

# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_node_per_test_function_with_attrs(graph):
    node = graph.node_data("crates/latt-core/tests/basic.rs::parses_a")
    assert node["kind"] == "test"
    assert node["attrs"] == {
        "file": "crates/latt-core/tests/basic.rs", "function": "parses_a",
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
    for node_id in ("crates/latt-core/tests/basic.rs::early_uncited",
                    "tests/test_items.py::test_uncited"):
        assert graph.has_node(node_id)
        assert not [
            (s, t) for s, t in _edges(graph, "verifies") if s == node_id
        ]


def test_non_test_function_is_not_a_node(graph):
    assert not graph.has_node(
        "crates/latt-core/tests/basic.rs::helper_not_a_test"
    )


def test_attribute_between_test_marker_and_fn_still_binds(graph):
    # parses_b carries #[ignore] between #[test] and fn.
    assert graph.has_node("crates/latt-core/tests/basic.rs::parses_b")


# Requirement: adapter-openspec/Build capability and requirement nodes from the register
def test_a_section_heading_ends_the_requirement_above(profile, tmp_path):
    # WebComPy's main specs carry `### MODIFIED:` sections; their scenarios were
    # counted onto the requirement above, and ones before any requirement were lost.
    target = _stack_target(tmp_path, "tests/test_s.py", "")
    (target / "openspec" / "specs" / "solo" / "spec.md").write_text(
        "## Requirements\n"
        "#### Scenario: before any requirement\n"
        "### Requirement: Solo stands\n"
        "#### Scenario: its own\n"
        "### MODIFIED: Something else\n"
        "#### Scenario: not its own\n"
        "#### Scenario: nor this\n"
        "### Requirement: Solo reports\n"
        "#### Scenario: its own too\n"
    )
    built = openspec.build_graph(profile, target)
    assert built.node_data("solo/Solo stands")["attrs"]["scenarios"] == 1
    assert built.node_data("solo/Solo reports")["attrs"]["scenarios"] == 1
    stray = sorted(
        (issue.provenance.line, issue.message) for issue in _codes(built, "PARSE_ERROR")
        if issue.provenance.file.endswith("spec.md")
    )
    assert [line for line, _ in stray] == [1, 5]
    assert "1 scenario belongs" in stray[0][1] and "## Requirements" in stray[0][1]
    assert "2 scenarios" in stray[1][1] and "### MODIFIED: Something else" in stray[1][1]


def test_async_test_function_is_discovered(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\n"
        "async def test_a():\n    pass\n",
    )
    built = openspec.build_graph(profile, target)
    assert built.has_node("tests/test_s.py::test_a")
    assert ("tests/test_s.py::test_a", "solo/Solo stands") in _edges(built, "verifies")


def test_a_unicode_line_break_in_a_string_does_not_break_the_scan(profile, tmp_path):
    """`str.splitlines` also cuts at U+2028, desynchronizing line numbers from
    the parsed tree's and splitting a string literal mid-file."""
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\n"
        "def test_a():\n    s = 'x y'\n"
        "# Requirement: solo/Solo reports\n"
        "def test_b():\n    pass\n",
    )
    built = openspec.build_graph(profile, target)
    assert not [i for i in _codes(built, "PARSE_ERROR")
                if "syntax error" in i.message]
    verifies = _edges(built, "verifies")
    assert ("tests/test_s.py::test_a", "solo/Solo stands") in verifies
    assert ("tests/test_s.py::test_b", "solo/Solo reports") in verifies
    assert ("tests/test_s.py::test_b", "solo/Solo stands") not in verifies


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

# Requirement: adapter-openspec/A citation is a section header binding the tests that follow it
def test_citation_binds_the_group_beneath_it(graph):
    verifies = _edges(graph, "verifies")
    rs = "crates/latt-core/tests/basic.rs"
    assert (f"{rs}::parses_a", "alpha/Alpha parses input") in verifies
    assert (f"{rs}::parses_b", "alpha/Alpha parses input") in verifies
    assert (f"{rs}::reports", "alpha/Alpha reports errors") in verifies
    assert (f"{rs}::reports", "alpha/Alpha parses input") not in verifies


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


# Requirement: adapter-openspec/A citation is a section header binding the tests that follow it
def test_a_citation_inside_a_python_string_is_not_a_citation(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\n"
        'example = """\n'
        "# Requirement: solo/Solo reports\n"
        '"""\n'
        "def test_a():\n    pass\n",
    )
    verifies = _edges(openspec.build_graph(profile, target), "verifies")
    assert verifies == {("tests/test_s.py::test_a", "solo/Solo stands")}


# Requirement: adapter-openspec/A citation is a section header binding the tests that follow it
def test_rust_text_inside_a_string_or_block_comment_is_not_code(profile, tmp_path):
    rs = "crates/latt-core/tests/t.rs"
    target = _stack_target(
        tmp_path, rs,
        "// Requirement: solo/Solo stands\n"
        'const EXAMPLE: &str = r#"\n'
        "// Requirement: solo/Solo reports\n"
        "#[test]\nfn fake() {}\n"
        '"#;\n'
        "/*\n#[test]\nfn commented() {}\n*/\n"
        "#[test]\nfn real() {}\n",
    )
    built = openspec.build_graph(profile, target)
    assert built.has_node(f"{rs}::real")
    assert not built.has_node(f"{rs}::fake")
    assert not built.has_node(f"{rs}::commented")
    assert _edges(built, "verifies") == {(f"{rs}::real", "solo/Solo stands")}


def test_stacked_citations_bind_the_group_to_every_title(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\n"
        "# Requirement: solo/Solo reports\n"
        "def test_a():\n    pass\n"
        "def test_b():\n    pass\n",
    )
    verifies = _edges(openspec.build_graph(profile, target), "verifies")
    for fn in ("test_a", "test_b"):
        assert (f"tests/test_s.py::{fn}", "solo/Solo stands") in verifies
        assert (f"tests/test_s.py::{fn}", "solo/Solo reports") in verifies


def test_stacked_edges_carry_their_own_citation_line(profile, tmp_path):
    target = _stack_target(
        tmp_path, "crates/latt-core/tests/s.rs",
        "// Requirement: solo/Solo stands\n"
        "// Requirement: solo/Solo reports\n"
        "#[test]\nfn a() {}\n",
    )
    built = openspec.build_graph(profile, target)
    lines = {
        tgt: data["provenance"].line
        for src, tgt, kind, data in built.iter_edges()
        if kind == "verifies" and src == "crates/latt-core/tests/s.rs::a"
    }
    assert lines == {"solo/Solo stands": 1, "solo/Solo reports": 2}


def test_blank_line_breaks_the_stack(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\n"
        "\n"
        "# Requirement: solo/Solo reports\n"
        "def test_a():\n    pass\n",
    )
    verifies = _edges(openspec.build_graph(profile, target), "verifies")
    assert ("tests/test_s.py::test_a", "solo/Solo reports") in verifies
    assert ("tests/test_s.py::test_a", "solo/Solo stands") not in verifies


def test_repeated_title_in_a_stack_emits_two_edges(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\n"
        "# Requirement: solo/Solo stands\n"
        "def test_a():\n    pass\n",
    )
    built = openspec.build_graph(profile, target)
    edges = [
        (src, tgt) for src, tgt, kind, _ in built.iter_edges()
        if kind == "verifies"
    ]
    assert edges.count(("tests/test_s.py::test_a", "solo/Solo stands")) == 2


# Requirement: adapter-openspec/One requirement title per citation line
def test_compound_citation_is_one_unsplit_title(graph):
    assert (
        "tests/test_items.py::test_compound",
        "alpha/Alpha parses input / alpha/Alpha reports errors",
    ) in _edges(graph, "verifies")


# --- unreadable input -----------------------------------------------------

# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_undecodable_spec_file_is_reported_not_raised(graph):
    errs = [i for i in _codes(graph, "PARSE_ERROR") if "UTF-8" in i.message]
    assert [(Path(i.provenance.file).parent.name, i.provenance.line) for i in errs] == [
        ("broken", 1), ("broken", 1)]
    assert errs[0].message.endswith(r"so it was not read: ### Requirement: Broken \xFF\xFE title")
    assert errs[1].message.startswith("the requirement at line 1 was not read")


def test_undecodable_spec_file_does_not_stop_the_others(graph):
    # broken/ sorts between alpha/ and beta/; beta must still be read.
    assert graph.has_node("beta")
    assert graph.has_node("beta/Beta holds state")


def test_unresolvable_spec_dir_is_reported_and_the_tests_still_read(profile, tmp_path):
    (tmp_path / "tests").mkdir()
    (tmp_path / "tests" / "test_only.py").write_text("def test_alone():\n    pass\n")
    (tmp_path / "crates" / "latt-core" / "tests").mkdir(parents=True)
    (tmp_path / "crates" / "latt-core" / "tests" / "t.rs").write_text(
        "#[test]\nfn alone() {}\n"
    )
    built = openspec.build_graph(profile, tmp_path)
    errs = [i for i in _codes(built, "PARSE_ERROR") if "spec directory" in i.message]
    assert len(errs) == 1
    assert built.has_node("tests/test_only.py::test_alone")


def test_spec_dir_holding_no_spec_file_is_reported(profile, tmp_path):
    # A readable directory with no `<capability>/spec.md` builds no capability;
    # unreported, a wrong spec_dir reads as an empty, clean register.
    (tmp_path / "openspec" / "specs" / "notes").mkdir(parents=True)
    (tmp_path / "openspec" / "specs" / "notes" / "README.md").write_text("# Notes\n")
    built = openspec.build_graph(profile, tmp_path)
    found = [i for i in _codes(built, "PARSE_ERROR") if "holds no" in i.message]
    assert [(i.severity, i.message) for i in found] == [(
        Severity.WARNING,
        f"spec directory '{tmp_path / 'openspec' / 'specs'}' holds no "
        "'<capability>/spec.md' file",
    )]
    assert not [i for i, d in built.iter_nodes() if d["kind"] == "capability"]


def test_empty_test_glob_is_reported(profile, tmp_path):
    (tmp_path / "openspec" / "specs" / "solo").mkdir(parents=True)
    (tmp_path / "openspec" / "specs" / "solo" / "spec.md").write_text(
        "### Requirement: Solo stands\n"
    )
    built = openspec.build_graph(profile, tmp_path)
    globs = [i for i in _codes(built, "PARSE_ERROR") if "matched no file" in i.message]
    # One finding per test glob in the profile.
    assert len(globs) == 3, [i.message for i in built.adapter_issues]
    assert built.has_node("solo/Solo stands")


@pytest.fixture
def locked(tmp_path):
    """Make directories unreadable for one test, restoring them after it."""
    if os.geteuid() == 0:
        # Mode bits do not bind root, so the directory stays readable and the
        # test asserts nothing; the CI image runs pytest as root.
        pytest.skip("directory permissions do not apply to root")
    dirs: list[Path] = []

    def lock(path: Path) -> Path:
        path.chmod(0)
        dirs.append(path)
        return path

    yield lock
    for path in dirs:
        path.chmod(0o700)


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_unreadable_capability_directory_is_reported(profile, tmp_path, locked):
    target = _stack_target(tmp_path, "tests/test_s.py", "def test_s():\n    pass\n")
    specs = target / "openspec" / "specs"
    (specs / "locked").mkdir()
    (specs / "locked" / "spec.md").write_text("### Requirement: Hidden\n")
    (specs / "open").mkdir()
    (specs / "open" / "spec.md").write_text("### Requirement: Seen\n")
    locked(specs / "locked")
    built = openspec.build_graph(profile, target)
    errs = [i for i in _codes(built, "PARSE_ERROR") if "locked" in i.provenance.file]
    assert len(errs) == 1, [i.message for i in built.adapter_issues]
    assert built.has_node("open/Seen")


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_unreadable_directory_under_a_recursive_test_glob_is_reported(tmp_path, locked):
    from adapters._profile import load_profile as load_resolved
    from conftest import resolve_profile

    resolved = json.loads(resolve_profile(PROFILE_PATH, tmp_path).read_text())
    resolved["adapter"]["paths"]["tests"] = [{"glob": "suite/**/test_*.py", "dialect": "python"}]
    (tmp_path / "resolved.json").write_text(json.dumps(resolved))
    target = tmp_path / "target"
    (target / "openspec" / "specs" / "solo").mkdir(parents=True)
    (target / "openspec" / "specs" / "solo" / "spec.md").write_text("### Requirement: Solo\n")
    (target / "suite" / "open").mkdir(parents=True)
    (target / "suite" / "open" / "test_o.py").write_text("def test_o():\n    pass\n")
    (target / "suite" / "shut").mkdir()
    (target / "suite" / "shut" / "test_x.py").write_text("def test_x():\n    pass\n")
    locked(target / "suite" / "shut")
    built = openspec.build_graph(load_resolved(tmp_path / "resolved.json"), target)
    errs = [i for i in _codes(built, "PARSE_ERROR") if "shut" in i.provenance.file]
    assert len(errs) == 1, [i.message for i in built.adapter_issues]
    assert built.has_node("suite/open/test_o.py::test_o")


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


# Requirement: adapter-openspec/Run as a stdlib-only interface program
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
    assert [i for i in document["findings"] if i["code"] == "PARSE_ERROR"]


def _profile_with_tests(tmp_path, tests) -> Path:
    """Write the shipped profile as YAML with its test globs replaced."""
    import yaml

    data = yaml.safe_load(PROFILE_PATH.read_text())
    data["adapter"]["paths"]["tests"] = tests
    path = tmp_path / "custom.yaml"
    path.write_text(yaml.safe_dump(data))
    return path


# Requirement: adapter-openspec/Run as a stdlib-only interface program
def test_unknown_dialect_is_one_line_and_exit_two(mini_target, tmp_path):
    from conftest import resolve_profile

    custom = _profile_with_tests(
        tmp_path, [{"glob": "web/*.ts", "dialect": "go"}])
    result = subprocess.run(
        [str(ADAPTER_PATH), "--profile", str(resolve_profile(custom, tmp_path)),
         "--target", str(mini_target)],
        capture_output=True, text=True,
    )
    assert result.returncode == 2
    assert result.stdout == ""
    assert "Traceback" not in result.stderr
    lines = result.stderr.strip().split("\n")
    assert len(lines) == 1, result.stderr
    assert "'go'" in lines[0] and "'rust'" in lines[0] and "'python'" in lines[0]
    assert "lattice-profile-" not in lines[0]


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_glob_whose_files_hold_no_test_item_is_reported(mini_target, tmp_path):
    custom = _profile_with_tests(
        tmp_path, [{"glob": "web/*.ts", "dialect": "rust"}])
    built = openspec.build_graph(load_profile(custom), mini_target)
    found = [i for i in _codes(built, "PARSE_ERROR") if "none held" in i.message]
    assert len(found) == 1, [i.message for i in built.adapter_issues]
    assert found[0].message == (
        "1 file matched test glob 'web/*.ts' and none held a rust test item")
    assert found[0].severity.name == "WARNING"
    # The .ts file itself is not reported: per-file silence is a helper module.
    assert not [i for i in built.adapter_issues if "alpha.test.ts" in i.message]
    assert not [n for n in built.iter_nodes() if n[1]["kind"] == "test"], "no test node from a .ts"


def test_a_helper_module_beside_real_tests_is_silent(graph):
    # The shipped profile's `tests/*.py` glob matches conftest-shaped helpers
    # too; the glob yielded tests, so it is not reported.
    assert not [i for i in graph.adapter_issues if "none held" in i.message]


@needs_core
def test_core_relays_the_unknown_dialect_line_without_a_traceback(mini_target, tmp_path):
    bad = _profile_with_tests(
        tmp_path, [{"glob": "web/*.ts", "dialect": "go"}])
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(bad),
         "--adapter", str(ADAPTER_PATH), "--target", str(mini_target)],
        capture_output=True, text=True,
    )
    assert result.returncode == 2, result.stderr
    assert "Traceback" not in result.stderr
    assert "typescript" in result.stderr


# --- the standing gate ----------------------------------------------------

# Requirement: adapter-openspec/Lattice audits its own register
def test_gate_builds_nodes_and_edges(graph):
    nodes = list(graph.iter_nodes())
    edges = list(graph.iter_edges())
    assert nodes, "the gate proves nothing over an empty graph"
    # The edge count is the load-bearing half: zero dangling references over
    # zero edges is satisfied by an adapter that built no edges at all.
    assert edges, "no edges built, so no cross-reference was actually checked"


# Requirement: adapter-openspec/Run as a stdlib-only interface program
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


# Requirement: adapter-openspec/Coverage is visible and never exit-affecting
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
    dangling = [f for f in findings if f["code"] == "VACANCY"]
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


# Requirement: adapter-openspec/Build capability and requirement nodes from the register
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
    body = _body(openspec.build_graph(profile, target), "solo/One")

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

    assert "Verified by" not in _body(built, "solo/Plain")
    assert "Prose one." in _body(built, "solo/Plain")
    assert "Verified by" not in _body(built, "solo/Bold")
    assert "Prose two." in _body(built, "solo/Bold")


def test_requirement_with_no_body_carries_no_body_attr(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: Empty\n### Requirement: Next\nProse.\n"
    )
    assert _body(openspec.build_graph(profile, target), "solo/Empty") is None


# Requirement: adapter-openspec/Build test nodes from the cited test files
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
        "crates/latt-core/tests/b.rs",
        "#[test]\nfn a() {\n    let mine = 1;\n}\n"
        "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    body = _body(
        openspec.build_graph(profile, target), "crates/latt-core/tests/b.rs::a"
    )

    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_one_line_rust_test_absorbs_nothing(profile, tmp_path):
    """The column-0-brace heuristic runs straight past `fn a() {}` into the
    functions after it, attaching another test's code as this node's body."""
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/latt-core/tests/b.rs",
        "#[test]\nfn a() {}\n"
        "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    built = openspec.build_graph(profile, target)

    assert _body(built, "crates/latt-core/tests/b.rs::a") is None
    assert "theirs" in _body(built, "crates/latt-core/tests/b.rs::b")


def test_an_indented_closing_brace_still_bounds_the_body(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/latt-core/tests/b.rs",
        "mod inner {\n"
        "    #[test]\n    fn a() {\n        let mine = 1;\n    }\n"
        "    #[test]\n    fn b() {\n        let theirs = 2;\n    }\n"
        "}\n",
    )
    body = _body(
        openspec.build_graph(profile, target), "crates/latt-core/tests/b.rs::a"
    )

    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_brace_in_a_string_does_not_end_the_body(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/latt-core/tests/b.rs",
        '#[test]\nfn a() {\n    let s = "}";\n    let mine = 1;\n}\n'
        "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    body = _body(
        openspec.build_graph(profile, target), "crates/latt-core/tests/b.rs::a"
    )

    assert "let mine = 1;" in body
    assert "theirs" not in body


def _rust_body_of_a(profile, tmp_path, test_text):
    """The body of `a` when `b` follows it, with any unbounded-body issues raised."""
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/latt-core/tests/b.rs",
        test_text + "#[test]\nfn b() {\n    let theirs = 2;\n}\n",
    )
    built = openspec.build_graph(profile, target)
    unbounded = [
        issue for issue in _codes(built, "PARSE_ERROR")
        if "could not find the end" in issue.message
    ]
    return _body(built, "crates/latt-core/tests/b.rs::a"), unbounded


def test_a_string_continued_across_lines_does_not_end_the_body(profile, tmp_path):
    """The shape `constraint_multiple_entries` in the self-audit takes: a
    `format!` string whose lines end in `\\`, so its `{{` / `}}` / `\\"` sit on
    lines that open no quote of their own. Per-line lexing counts them as
    code and never finds the function's end."""
    body, errors = _rust_body_of_a(
        profile, tmp_path,
        "#[test]\n"
        "fn a() {\n"
        "    let yaml = format!(\n"
        '        "{PREFIX}validations:\\n\\\n'
        "         \\x20 - CONSTRAINT: {{present: true}}\\n\\\n"
        '         \\x20 - REJECT: {{eq: \\"blocked\\"}}\\n"\n'
        "    );\n"
        "    let mine = 1;\n"
        "}\n",
    )

    assert errors == []
    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_raw_string_spanning_lines_does_not_end_the_body(profile, tmp_path):
    body, errors = _rust_body_of_a(
        profile, tmp_path,
        "#[test]\n"
        "fn a() {\n"
        '    let s = r#"a "quoted" }\n'
        "}}}\n"
        '"#;\n'
        "    let mine = 1;\n"
        "}\n",
    )

    assert errors == []
    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_block_comment_spanning_lines_does_not_end_the_body(profile, tmp_path):
    body, errors = _rust_body_of_a(
        profile, tmp_path,
        "#[test]\n"
        "fn a() {\n"
        "    /* closes } here\n"
        "       and /* nests */ }\n"
        "    */\n"
        "    let mine = 1;\n"
        "}\n",
    )

    assert errors == []
    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_a_brace_char_literal_does_not_end_the_body(profile, tmp_path):
    body, errors = _rust_body_of_a(
        profile, tmp_path,
        "#[test]\n"
        "fn a() {\n"
        "    let close = '}';\n"
        "    let quote = '\"';\n"
        "    let escaped = '\\'';\n"
        "    let lifetime: &'static str = \"}\";\n"
        "    let mine = 1;\n"
        "}\n",
    )

    assert errors == []
    assert "let mine = 1;" in body
    assert "theirs" not in body


def test_unclosed_rust_test_body_is_reported_not_guessed(profile, tmp_path):
    target = _body_target(
        tmp_path, "### Requirement: One\nProse.\n",
        "crates/latt-core/tests/b.rs",
        "#[test]\nfn a() {\n    let never_closed = 1;\n",
    )
    built = openspec.build_graph(profile, target)

    assert _body(built, "crates/latt-core/tests/b.rs::a") is None
    assert _codes(built, "PARSE_ERROR"), "unresolvable bounds must be reported"


# Requirement: adapter-openspec/Build capability and requirement nodes from the register
def test_a_leading_byte_order_mark_does_not_hide_the_first_requirement(
    profile, tmp_path
):
    target = _body_target(tmp_path, "")
    (target / "openspec" / "specs" / "solo" / "spec.md").write_bytes(
        b"\xef\xbb\xbf### Requirement: First\nThe system SHALL x.\n"
    )
    graph = openspec.build_graph(profile, target)

    assert _body(graph, "solo/First") is not None


def _utf8_issues(graph) -> list:
    return [i for i in graph.adapter_issues if "UTF-8" in i.message]


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_a_line_that_is_not_utf8_withholds_only_its_requirement(profile, tmp_path):
    target = _body_target(tmp_path, "")
    (target / "openspec" / "specs" / "solo" / "spec.md").write_bytes(
        b"# Solo\n\n### Requirement: First\nThe system SHALL x.\n\n"
        b"### Requirement: Second\nThe system SHALL caf\xe9.\n#### Scenario: s\n\n"
        b"### Requirement: Third\nThe system SHALL z.\n"
    )
    graph = openspec.build_graph(profile, target)

    ids = {nid for nid, _ in graph.iter_nodes()}
    assert {"solo/First", "solo/Third"} <= ids
    assert not any(nid.startswith("solo/Second") for nid in ids)
    assert "caf" not in _body(graph, "solo/First")
    issues = _utf8_issues(graph)
    assert [(i.code, i.provenance.line) for i in issues] == [("PARSE_ERROR", 7), ("PARSE_ERROR", 6)]
    assert issues[0].message == (
        r"line is not valid UTF-8 at column 21, so it was not read: The system SHALL caf\xE9."
    )
    assert "requirement at line 6 was not read" in issues[1].message
    assert not any("/home" in i.message or str(tmp_path) in i.message for i in issues)


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_an_unreadable_requirement_heading_does_not_fold_into_the_one_above(profile, tmp_path):
    target = _body_target(tmp_path, "")
    (target / "openspec" / "specs" / "solo" / "spec.md").write_bytes(
        b"### Requirement: First\nThe system SHALL x.\n#### Scenario: a\n\n"
        b"### Requirement: Caf\xe9\nThe system SHALL y.\n#### Scenario: b\n"
    )
    graph = openspec.build_graph(profile, target)

    assert graph.node_data("solo/First")["attrs"]["scenarios"] == 1
    assert "SHALL y" not in _body(graph, "solo/First")
    assert [i.provenance.line for i in _utf8_issues(graph)] == [5, 5]


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_a_test_file_that_is_not_utf8_names_its_line_as_written(profile, tmp_path):
    # Source code must be UTF-8, so the file is read as nothing, as a syntax
    # error would be; the finding says where and what, not a byte offset.
    target = _body_target(tmp_path, "### Requirement: A\nThe system SHALL a.\n")
    (target / "tests" / "test_b.py").write_bytes(
        b"def test_a():\n    assert 'caf\xe9'\n")
    graph = openspec.build_graph(profile, target)

    issues = _utf8_issues(graph)
    assert [i.provenance.line for i in issues] == [2]
    assert r"    assert 'caf\xE9'" in issues[0].message
    assert "column 16" in issues[0].message
    assert not any(nid.startswith("tests/test_b.py") for nid, _ in graph.iter_nodes())


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_a_test_file_that_vanishes_after_failing_to_decode_is_reported(
    profile, tmp_path, monkeypatch,
):
    # The bytes are reread to show each bad line; a file gone by then is still
    # register input failing, so it is a finding, not an exception.
    target = _body_target(tmp_path, "### Requirement: A\nThe system SHALL a.\n")
    vanishing = target / "tests" / "test_b.py"
    vanishing.write_bytes(b"def test_a():\n    assert 'caf\xe9'\n")
    read_bytes = Path.read_bytes

    def gone(self):
        if self == vanishing:
            raise FileNotFoundError(2, "No such file or directory", str(self))
        return read_bytes(self)

    monkeypatch.setattr(Path, "read_bytes", gone)
    graph = openspec.build_graph(profile, target)

    issues = [i for i in graph.adapter_issues if "could not read" in i.message]
    assert [(i.severity, i.code, i.provenance.file) for i in issues] == [
        (Severity.ERROR, "PARSE_ERROR", str(vanishing))]
    assert not any(nid.startswith("tests/test_b.py") for nid, _ in graph.iter_nodes())


def _trace_entries(target: Path) -> dict[str, dict]:
    result = subprocess.run(
        [str(CORE_BIN), "trace", "--profile", str(PROFILE_PATH),
         "--adapter", str(ADAPTER_PATH), "--target", str(target),
         "--format", "json"],
        capture_output=True, text=True,
    )
    assert result.returncode != 2, f"core could not run: {result.stderr}"
    return {e["id"]: e for e in json.loads(result.stdout)["entries"]}


def _unread(entry: dict) -> list[str]:
    return [f["message"] for f in entry["findings"]
            if f["message"].startswith("duplicate node")]


# Requirement: adapter-contract/Document order is significant
@needs_core
def test_a_repeated_requirement_titles_contains_edge_is_not_read(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\ndef test_a():\n    pass\n",
    )
    (target / "openspec" / "specs" / "solo" / "spec.md").write_text(
        "### Requirement: Solo stands\n### Requirement: Solo stands\n"
    )
    built = openspec.build_graph(profile, target)
    contains = [d.get("declared_by") for s, t, k, d in built.iter_edges() if k == "contains"]
    assert contains == [1, 2], "each contains edge names its own requirement entry"

    entries = _trace_entries(target)
    assert [e["tgt"] for e in entries["solo"]["edges"]] == ["solo/Solo stands"]
    assert _unread(entries["solo/Solo stands"]) == [
        "duplicate node 'solo/Solo stands': first at openspec/specs/solo/spec.md:1; "
        "its 1 edge was not read"
    ]
    # The citing test is its own entry, so its verifies edge is still read.
    assert [e["tgt"] for e in entries["tests/test_s.py::test_a"]["edges"]] == [
        "solo/Solo stands"
    ]


# Requirement: adapter-contract/Document order is significant
@needs_core
def test_a_repeated_test_functions_verifies_edges_are_not_read(profile, tmp_path):
    target = _stack_target(
        tmp_path, "tests/test_s.py",
        "# Requirement: solo/Solo stands\ndef test_a():\n    pass\n\n"
        "# Requirement: solo/Solo reports\ndef test_a():\n    pass\n",
    )
    entries = _trace_entries(target)
    test_a = entries["tests/test_s.py::test_a"]
    assert [e["tgt"] for e in test_a["edges"]] == ["solo/Solo stands"]
    assert _unread(test_a) == [
        "duplicate node 'tests/test_s.py::test_a': first at tests/test_s.py:2; "
        "its 1 edge was not read"
    ]


# --- TypeScript dialect and spec-only profiles ----------------------------

def _ts_graph(tmp_path, test_text, rel="web/a.test.ts"):
    """Build a one-capability register with one TypeScript test file."""
    target = _stack_target(tmp_path / "t", rel, test_text)
    custom = _profile_with_tests(tmp_path, [{"glob": "web/**/*.ts", "dialect": "typescript"}])
    return openspec.build_graph(load_profile(custom), target)


def _test_ids(built):
    return {n[0] for n in built.iter_nodes() if n[1]["kind"] == "test"}


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_typescript_tests_are_named_by_their_describe_path(tmp_path):
    built = _ts_graph(tmp_path, (
        "// Requirement: solo/Solo stands\n"
        'describe("parser", () => {\n'
        '  it("reads x", () => {\n'
        "    expect(1).toBe(1);\n"
        "  });\n"
        "  describe('errors', () => {\n"
        "    test.skip('reports y', async () => {\n"
        "      await go();\n"
        "    });\n"
        "  });\n"
        "  it.each([1, 2])('adds %i', (n) => {\n"
        "    expect(n).toBeTruthy();\n"
        "  });\n"
        "});\n"
        'test("top", () => expect(2).toBe(2));\n'
    ))
    ids = {
        "web/a.test.ts::parser::reads x",
        "web/a.test.ts::parser::errors::reports y",
        "web/a.test.ts::parser::adds %i",
        "web/a.test.ts::top",
    }
    assert _test_ids(built) == ids
    assert _edges(built, "verifies") == {(i, "solo/Solo stands") for i in ids}
    assert not _codes(built, "PARSE_ERROR"), [i.message for i in built.adapter_issues]


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_a_typescript_name_reused_under_two_describes_is_two_tests(tmp_path):
    built = _ts_graph(tmp_path, (
        'describe("a", () => {\n  it("works", () => {});\n});\n'
        'describe("b", () => {\n  it("works", () => {});\n});\n'
    ))
    assert _test_ids(built) == {"web/a.test.ts::a::works", "web/a.test.ts::b::works"}


# Requirement: adapter-openspec/A citation is a section header binding the tests that follow it
def test_typescript_text_inside_a_template_literal_or_comment_is_not_code(tmp_path):
    built = _ts_graph(tmp_path, (
        "// Requirement: solo/Solo stands\n"
        "const fixture = `\n"
        "// Requirement: solo/Solo reports\n"
        'it("fake", () => {});\n'
        "${'x'}\n"
        "`;\n"
        "/*\n"
        'test("also fake", () => {});\n'
        "*/\n"
        'it("real", () => {});\n'
    ))
    assert _test_ids(built) == {"web/a.test.ts::real"}
    assert _edges(built, "verifies") == {("web/a.test.ts::real", "solo/Solo stands")}


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_typescript_test_body_stops_at_its_closing_brace(tmp_path):
    built = _ts_graph(tmp_path, (
        'it("a", () => {\n'
        '  const mine = "}";\n'
        "  const tpl = `}${1}`;\n"
        "});\n"
        'it("b", () => {\n  const theirs = 2;\n});\n'
    ))
    body = _body(built, "web/a.test.ts::a")
    assert "const mine" in body and "const tpl" in body
    assert "theirs" not in body


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_an_unclosed_typescript_test_body_is_reported_not_guessed(tmp_path):
    built = _ts_graph(tmp_path, 'it("a", () => {\n  const mine = 1;\n')
    assert "web/a.test.ts::a" in _test_ids(built)
    assert _body(built, "web/a.test.ts::a") is None
    found = [i for i in _codes(built, "PARSE_ERROR") if "'a'" in i.message]
    assert len(found) == 1 and found[0].severity.name == "WARNING"


# Requirement: adapter-openspec/Build test nodes from the cited test files
@pytest.mark.parametrize("tests", [[], None])
def test_a_spec_only_profile_reads_the_specs_and_no_tests(mini_target, tmp_path, tests):
    import yaml

    data = yaml.safe_load(PROFILE_PATH.read_text())
    if tests is None:
        del data["adapter"]["paths"]["tests"]
    else:
        data["adapter"]["paths"]["tests"] = tests
    path = tmp_path / "spec-only.yaml"
    path.write_text(yaml.safe_dump(data))
    built = openspec.build_graph(load_profile(path), mini_target)
    kinds = [n[1]["kind"] for n in built.iter_nodes()]
    assert "capability" in kinds and "requirement" in kinds
    assert "test" not in kinds
    assert not [i for i in built.adapter_issues if "test glob" in i.message]


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_a_quote_inside_a_typescript_regex_does_not_open_a_string(tmp_path):
    built = _ts_graph(tmp_path, (
        'it("a", () => {\n'
        "  const q = dir.replace(/'/g, x);\n"
        "  expect(out).not.toMatch(/if \\[ -d \"/);\n"
        "  const half = total / 2;\n"
        "});\n"
        'it("b", () => {\n  const theirs = 2;\n});\n'
    ))
    assert _test_ids(built) == {"web/a.test.ts::a", "web/a.test.ts::b"}
    body = _body(built, "web/a.test.ts::a")
    assert "const half" in body and "theirs" not in body


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_typescript_modifier_calls_and_multi_line_tables_are_read(tmp_path):
    built = _ts_graph(tmp_path, (
        "it.skipIf(process.platform === 'win32')('skips on windows', () => {\n"
        "  run();\n"
        "});\n"
        "it.each([\n"
        "  ['a', 1],\n"
        "  ['b', 2],\n"
        "])('maps %s', (name, n) => {\n"
        "  expect(n).toBeTruthy();\n"
        "});\n"
        "it(\n"
        "  'wraps its name',\n"
        "  async () => {\n"
        "    await go();\n"
        "  },\n"
        ");\n"
    ))
    assert _test_ids(built) == {
        "web/a.test.ts::skips on windows",
        "web/a.test.ts::maps %s",
        "web/a.test.ts::wraps its name",
    }
    assert "await go();" in _body(built, "web/a.test.ts::wraps its name")
    assert "expect(n)" in _body(built, "web/a.test.ts::maps %s")


# Requirement: adapter-openspec/Unreadable input is reported, never dropped
def test_a_typescript_test_with_no_literal_name_is_reported(tmp_path):
    built = _ts_graph(tmp_path, (
        "for (const name of names) {\n"
        "  it(name, () => {\n"
        "    run(name);\n"
        "  });\n"
        "}\n"
        "const test = 5;\n"
    ))
    assert _test_ids(built) == set()
    found = [i for i in _codes(built, "PARSE_ERROR") if "no literal name" in i.message]
    assert len(found) == 1, [i.message for i in built.adapter_issues]
    assert found[0].severity.name == "WARNING"
    assert found[0].provenance.line == 2


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_the_shipped_profile_admits_every_dialects_test_ids(tmp_path):
    import re

    import yaml

    pattern = yaml.safe_load(PROFILE_PATH.read_text())["node_kinds"]["test"]["id_pattern"]
    built = _ts_graph(tmp_path, (
        'describe("parser", () => {\n  it("reads x: a value", () => {});\n});\n'
    ))
    ids = _test_ids(built) | {"tests/test_a.py::TestX::test_y", "crates/x/tests/a.rs::b"}
    assert all(re.search(pattern, i) for i in ids), [i for i in ids if not re.search(pattern, i)]


# Requirement: adapter-openspec/Build test nodes from the cited test files
def test_a_typescript_each_table_of_any_length_is_read(tmp_path):
    rows = "".join(f"  ['row{n}', {n}],\n" for n in range(120))
    built = _ts_graph(tmp_path, f"it.each([\n{rows}])('maps %s', (name, n) => {{\n  go(n);\n}});\n")
    assert _test_ids(built) == {"web/a.test.ts::maps %s"}
    assert _body(built, "web/a.test.ts::maps %s") == "go(n);"

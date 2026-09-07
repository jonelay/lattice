from __future__ import annotations

import importlib.machinery
import importlib.util
import json
import subprocess
from pathlib import Path
from types import ModuleType

import pytest

REPO_ROOT = Path(__file__).parent.parent
COMPOSE_PATH = REPO_ROOT / "tools" / "lattice-compose"
CORE_BIN = REPO_ROOT / "target" / "debug" / "lattice"
FIXTURE_PATH = REPO_ROOT / "tests" / "fixtures" / "mini-program"

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


def _load_compose() -> ModuleType:
    loader = importlib.machinery.SourceFileLoader("lattice_compose", str(COMPOSE_PATH))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    assert spec is not None
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def compose_module() -> ModuleType:
    return _load_compose()


def _entry(
    node_id: str,
    kind: str,
    *,
    edges: dict[str, list[str]] | None = None,
    findings: list[dict] | None = None,
    file: str = "register.md",
) -> dict:
    return {
        "id": node_id,
        "kind": kind,
        "attrs": {"title": node_id},
        "provenance": {"file": file, "line": 3},
        "edges": edges or {},
        "findings": findings or [],
    }


def _run(name: str, entries: list[dict], unattachable: list[dict] | None = None) -> dict:
    return {
        "source": {"name": name},
        "payload": {
            "header": {"profile": name},
            "entries": entries,
            "unattachable_findings": unattachable or [],
        },
        "exit_code": 0,
    }


def _profile(
    allowed: list[tuple[str, str]], validations: list[dict] | None = None
) -> dict:
    return {
        "edge_kinds": {"links": {"allowed": allowed}},
        "validations": validations or [],
    }


def _write_manifest(tmp_path: Path, sources: str) -> Path:
    manifest = tmp_path / "program.yaml"
    manifest.write_text(
        "manifest_version: '1.0.0'\n"
        "program: sample\n"
        "program_version: '2.0.0'\n"
        "program_profile: profile.yaml\n"
        f"sources:\n{sources}"
    )
    return manifest


def test_manifest_loads_in_order_and_resolves_paths(compose_module, tmp_path: Path) -> None:
    manifest_path = _write_manifest(
        tmp_path,
        "  - {name: one, profile: one.yaml, adapter: bin/one, target: ../one}\n"
        "  - {name: two, profile: two.yaml, adapter: bin/two, target: ../two}\n",
    )

    manifest = compose_module.load_manifest(manifest_path)

    assert [source["name"] for source in manifest["sources"]] == ["one", "two"]
    assert manifest["program_profile"] == (tmp_path / "profile.yaml").resolve()
    assert manifest["sources"][0]["adapter"] == (tmp_path / "bin/one").resolve()


@pytest.mark.parametrize(
    "sources, message",
    [
        ("", "non-empty list"),
        (
            "  - {name: same, profile: a, adapter: a, target: a}\n"
            "  - {name: same, profile: b, adapter: b, target: b}\n",
            "duplicate source name 'same'",
        ),
        ("  - {name: incomplete, profile: a, adapter: a}\n", "'target'"),
    ],
)
def test_manifest_validation(compose_module, tmp_path: Path, sources: str, message: str) -> None:
    with pytest.raises(compose_module.ComposeError, match=message):
        compose_module.load_manifest(_write_manifest(tmp_path, sources))


def test_manifest_missing_is_an_error(compose_module, tmp_path: Path) -> None:
    with pytest.raises(compose_module.ComposeError, match="could not load manifest"):
        compose_module.load_manifest(tmp_path / "missing.yaml")


@pytest.mark.parametrize("field", ["program_profile", "profile", "adapter", "target"])
def test_manifest_rejects_absolute_paths(
    compose_module, tmp_path: Path, capsys, field: str
) -> None:
    absolute = str(tmp_path / "absolute")
    program_profile = absolute if field == "program_profile" else "profile.yaml"
    source_paths = {
        key: absolute if field == key else key
        for key in ("profile", "adapter", "target")
    }
    manifest = tmp_path / "absolute.yaml"
    manifest.write_text(
        "manifest_version: '1.0.0'\n"
        "program: sample\n"
        "program_version: '2.0.0'\n"
        f"program_profile: {program_profile}\n"
        "sources:\n"
        "  - name: source\n"
        f"    profile: {source_paths['profile']}\n"
        f"    adapter: {source_paths['adapter']}\n"
        f"    target: {source_paths['target']}\n"
    )

    assert compose_module.main([str(manifest)]) == 2
    captured = capsys.readouterr()
    assert captured.out == ""
    assert f"'{field}' must be a relative path" in captured.err


def test_program_profile_loads_allowed_pairs(compose_module, tmp_path: Path) -> None:
    path = tmp_path / "profile.yaml"
    path.write_text(
        "edge_kinds:\n"
        "  derives:\n"
        "    allowed: [[compliance/clause, product/requirement]]\n"
        "validations:\n"
        "  - DANGLING_REF: {severity: warning}\n"
    )

    profile = compose_module.load_program_profile(path)

    assert profile["edge_kinds"]["derives"]["allowed"] == [
        ("compliance/clause", "product/requirement")
    ]
    assert profile["validations"][0]["DANGLING_REF"]["severity"] == "warning"


def test_program_profile_rejects_node_kinds(compose_module, tmp_path: Path) -> None:
    path = tmp_path / "profile.yaml"
    path.write_text("node_kinds: {}\nedge_kinds: {}\n")
    with pytest.raises(compose_module.ComposeError, match="must not declare 'node_kinds'"):
        compose_module.load_program_profile(path)


def test_program_profile_requires_qualified_allowed_kinds(compose_module, tmp_path: Path) -> None:
    path = tmp_path / "profile.yaml"
    path.write_text("edge_kinds:\n  links:\n    allowed: [[local, other/kind]]\n")
    with pytest.raises(compose_module.ComposeError, match="source-qualified"):
        compose_module.load_program_profile(path)


def test_source_run_uses_manifest_order_and_exact_cli(compose_module) -> None:
    commands = []

    def runner(command, **kwargs):
        commands.append((command, kwargs))
        payload = {"entries": [], "unattachable_findings": []}
        return subprocess.CompletedProcess(command, len(commands) - 1, json.dumps(payload), "")

    manifest = {
        "sources": [
            {"name": "one", "profile": Path("p1"), "adapter": Path("a1"), "target": Path("t1")},
            {"name": "two", "profile": Path("p2"), "adapter": Path("a2"), "target": Path("t2")},
        ]
    }
    healthy, failures = compose_module.run_sources(
        manifest, lattice_bin="/bin/lattice", runner=runner
    )

    assert not failures
    assert [run["source"]["name"] for run in healthy] == ["one", "two"]
    assert commands[0][0] == [
        "/bin/lattice", "trace", "--format", "json", "--profile", "p1",
        "--adapter", "a1", "--target", "t1",
    ]
    assert commands[0][1] == {"capture_output": True, "text": True}


def test_source_failure_runs_remaining_and_keeps_healthy_findings(compose_module) -> None:
    calls = []
    healthy_payload = {
        "entries": [
            _entry(
                "R-1",
                "req",
                findings=[{
                    "code": "LOCAL",
                    "severity": "warning",
                    "message": "local issue",
                    "file": "r.md",
                    "line": 2,
                }],
            )
        ],
        "unattachable_findings": [],
    }

    def runner(command, **_kwargs):
        calls.append(command[-1])
        if command[-1] == "bad-target":
            return subprocess.CompletedProcess(command, 2, "", "adapter broke")
        return subprocess.CompletedProcess(command, 0, json.dumps(healthy_payload), "")

    manifest = {
        "sources": [
            {"name": "bad", "profile": "p", "adapter": "a", "target": "bad-target"},
            {"name": "good", "profile": "p", "adapter": "a", "target": "good-target"},
        ]
    }
    healthy, failures = compose_module.run_sources(manifest, "lattice", runner)
    findings = compose_module.collect_source_findings(healthy)

    assert calls == ["bad-target", "good-target"]
    assert len(failures) == 1
    assert findings[0]["code"] == "LOCAL"
    assert findings[0]["source"] == "good"
    assert compose_module.exit_code(findings + compose_module.failure_findings(failures), False) == 2


def test_merge_qualifies_kinds_reconstructs_edges_and_preserves_findings(compose_module) -> None:
    attached = {
        "code": "LOCAL",
        "severity": "hint",
        "message": "kept",
        "file": "a.md",
        "line": 3,
    }
    runs = [
        _run("alpha", [_entry("A-1", "req", edges={"links": ["B-1"]}, findings=[attached])]),
        _run("beta", [_entry("B-1", "spec", file="b.md")]),
    ]

    merged = compose_module.merge_runs(runs)
    source_findings = compose_module.collect_source_findings(runs)

    assert [node["kind"] for node in merged["nodes"]] == ["alpha/req", "beta/spec"]
    assert merged["edges"][0] == {
        "src": "A-1",
        "tgt": "B-1",
        "kind": "links",
        "source": "alpha",
        "source_kind": "alpha/req",
        "provenance": {"source": "alpha", "file": "register.md", "line": 3},
    }
    assert source_findings[0]["source"] == "alpha"
    assert source_findings[0]["node_id"] == "A-1"


def test_duplicate_detection_reports_every_location(compose_module) -> None:
    runs = [
        _run("one", [_entry("DUP", "req", file="one.md")]),
        _run("two", [_entry("DUP", "spec", file="two.md")]),
        _run("three", [_entry("DUP", "test", file="three.md")]),
    ]

    duplicate = compose_module.merge_runs(runs)["findings"][0]

    assert duplicate["code"] == "CROSS_SOURCE_DUPLICATE_ID"
    assert duplicate["sources"] == ["one", "two", "three"]
    assert [location["file"] for location in duplicate["locations"]] == [
        "one.md", "two.md", "three.md"
    ]


def test_resolution_marks_an_unambiguous_edge(compose_module) -> None:
    merged = compose_module.merge_runs([
        _run("left", [_entry("L-1", "req", edges={"links": ["R-1"]})]),
        _run("right", [_entry("R-1", "spec")]),
    ])

    findings = compose_module.validate_cross_source(
        merged, _profile([("left/req", "right/spec")])
    )

    assert findings == []
    assert merged["edges"][0]["target_kind"] == "right/spec"
    assert merged["edges"][0]["target_source"] == "right"


def test_resolution_reports_dangling_with_program_severity(compose_module) -> None:
    merged = compose_module.merge_runs([
        _run("left", [_entry("L-1", "req", edges={"links": ["missing"]})])
    ])
    profile = _profile(
        [("left/req", "right/spec")],
        [{"DANGLING_REF": {"severity": "warning"}}],
    )

    findings = compose_module.validate_cross_source(merged, profile)

    assert [(finding["code"], finding["severity"]) for finding in findings] == [
        ("DANGLING_REF", "warning")
    ]
    assert findings[0]["source"] == "left"


def test_resolution_reports_all_ambiguous_targets(compose_module) -> None:
    merged = compose_module.merge_runs([
        _run("left", [_entry("L-1", "req", edges={"links": ["X"]})]),
        _run("right", [_entry("X", "spec", file="right.md")]),
        _run("legacy", [_entry("X", "spec", file="legacy.md")]),
    ])
    profile = _profile([
        ("left/req", "right/spec"),
        ("left/req", "legacy/spec"),
    ])

    ambiguous = compose_module.validate_cross_source(merged, profile)[0]

    assert ambiguous["code"] == "AMBIGUOUS_CROSS_REF"
    assert ambiguous["sources"] == ["right", "legacy"]
    assert [location["file"] for location in ambiguous["locations"]] == [
        "right.md", "legacy.md"
    ]


def test_coverage_checks_resolved_cross_source_edges(compose_module) -> None:
    merged = compose_module.merge_runs([
        _run("left", [_entry("L-1", "req", edges={"links": ["R-1"]})]),
        _run("right", [_entry("R-1", "spec"), _entry("R-2", "spec")]),
    ])
    profile = _profile(
        [("left/req", "right/spec")],
        [{"COVERAGE": {
            "target_kind": "right/spec",
            "edge_kind": "links",
            "severity": "hint",
        }}],
    )

    findings = compose_module.validate_cross_source(merged, profile)

    coverage = [finding for finding in findings if finding["code"] == "COVERAGE"]
    assert [(finding["node_id"], finding["severity"]) for finding in coverage] == [
        ("R-2", "hint")
    ]
    assert coverage[0]["state"] == "unverified"


def test_phase_findings_keep_attached_and_unattachable_source_provenance(compose_module) -> None:
    finding = {"code": "PARSE_ERROR", "severity": "error", "message": "bad", "file": "x", "line": 1}
    runs = [_run("source", [_entry("N", "req", findings=[finding])], [finding])]

    findings = compose_module.collect_source_findings(runs)

    assert len(findings) == 2
    assert all(item["source"] == "source" for item in findings)
    assert findings[0]["node_id"] == "N"
    assert "node_id" not in findings[1]


def test_output_format_is_json_serializable_and_entry_independent(compose_module) -> None:
    manifest = {
        "program": "sample",
        "program_version": "1.2.3",
        "manifest_version": "1.0.0",
    }
    payload = compose_module.build_output(
        manifest,
        [{"id": "N", "kind": "source/req"}],
        [{"src": "N", "tgt": "X", "kind": "links"}],
        [{"code": "DANGLING_REF", "severity": "error", "source": "source"}],
    )

    decoded = json.loads(json.dumps(payload))

    assert set(decoded) == {"header", "nodes", "edges", "axes", "findings"}
    assert decoded["header"]["program"] == "sample"
    assert decoded["nodes"][0]["kind"] == "source/req"
    assert decoded["findings"][0]["source"] == "source"


@pytest.mark.parametrize(
    "findings, could_run, expected",
    [
        ([], True, 0),
        ([{"severity": "hint"}, {"severity": "warning"}], True, 0),
        ([{"severity": "error"}], True, 1),
        ([], False, 2),
        ([{"severity": "error"}], False, 2),
    ],
)
def test_exit_code_logic(compose_module, findings, could_run, expected) -> None:
    assert compose_module.exit_code(findings, could_run) == expected


@needs_core
def test_end_to_end_composes_existing_mini_fixtures() -> None:
    result = subprocess.run(
        [str(COMPOSE_PATH), str(FIXTURE_PATH / "program.yaml")],
        capture_output=True,
        text=True,
    )

    assert result.returncode == 1, result.stderr
    payload = json.loads(result.stdout)
    kinds = {node["kind"] for node in payload["nodes"]}
    assert "toml/item" in kinds
    assert "markdown/requirement" in kinds
    assert payload["edges"], "the composition gate must exercise real edges"
    cross_dangling = [
        finding for finding in payload["findings"]
        if finding["code"] == "DANGLING_REF"
        and finding.get("source") == "markdown"
        and "allowed source kind" in finding["message"]
    ]
    assert cross_dangling

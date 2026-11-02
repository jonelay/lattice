from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator


ROOT = Path(__file__).parent.parent
COLLECTOR_PATH = ROOT / "tools" / "benchmark-revisions.py"
GENERATOR_PATH = ROOT / "tools" / "generate-e2e-fixture.py"
REPORT_SCHEMA_PATH = ROOT / "benchmarks" / "e2e" / "report.schema.json"
COMPONENT_SCHEMA_PATH = ROOT / "benchmarks" / "component" / "report.schema.json"
COMPONENT_FIXTURE_PATH = ROOT / "crates" / "latt-core" / "benches" / "fixture.json"
COMPONENT_TREES = ROOT / "tests" / "fixtures" / "mini-criterion"
COMPONENT_HARNESS_PATHS = [
    ROOT / "crates" / "latt-core" / "benches" / "core.rs",
    ROOT / "crates" / "latt-core" / "benches" / "adapters" / "mod.rs",
]
COMPONENT_BODIES_PATH = ROOT / "benchmarks" / "component" / "bodies.json"
# Withheld from the GitHub release snapshot: the records name the runners they ran on.
RECORDS = ROOT / "notes" / "perf-0.8"
needs_records = pytest.mark.skipif(not RECORDS.exists(), reason="retained benchmark records not in the tree")
SPEC = importlib.util.spec_from_file_location("benchmark_revisions", COLLECTOR_PATH)
assert SPEC is not None and SPEC.loader is not None
collector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collector)


def test_generator_builds_declared_chain(tmp_path: Path) -> None:
    manifest = tmp_path / "scenario.json"
    manifest.write_text(
        json.dumps(
            {
                "generator": "deterministic-forward-chain-v1",
                "node_count": 3,
                "edge_count": 2,
            }
        )
    )
    target = tmp_path / "fixture"
    subprocess.run([sys.executable, GENERATOR_PATH, manifest, target], check=True)
    assert target.joinpath("register.toml").read_text() == (
        '[[item]]\nid = "n00000"\nrefs = "n00001"\n\n'
        '[[item]]\nid = "n00001"\nrefs = "n00002"\n\n'
        '[[item]]\nid = "n00002"\n\n'
    )


def test_finding_summary_requires_structured_findings() -> None:
    output = json.dumps(
        {
            "findings": [
                {"code": "UNTRACED", "severity": "info"},
                {"code": "UNTRACED", "severity": "info"},
            ]
        }
    ).encode()
    assert collector.finding_summary(output) == {
        "finding_count": 2,
        "by_code": {"UNTRACED": 2},
        "by_severity": {"info": 2},
    }
    with pytest.raises(RuntimeError, match="findings array"):
        collector.finding_summary(b"[]")


def test_inconsistent_repeated_output_is_rejected() -> None:
    first = {
        "exit_status": 0,
        "output_bytes": 10,
        "output_sha256": "one",
        "findings": {"finding_count": 1},
    }
    second = {**first, "output_sha256": "two"}
    with pytest.raises(RuntimeError, match="inconsistent output"):
        collector.ensure_consistent([first, second], "main")


def test_three_revisions_report_every_pair() -> None:
    def revision(ref: str, wall: float, rss: float) -> dict[str, object]:
        return {
            "ref": ref,
            "output": {"sha256": "same"},
            "measurements": {
                "wall_time_nanoseconds": {"summary": {"median": wall}},
                "peak_rss_kibibytes": {"summary": {"median": rss}},
            },
        }

    records = collector.comparisons(
        [
            revision("v0.7.1", 100.0, 100.0),
            revision("main-commit", 90.0, 110.0),
            revision("v0.8.0-rc.1", 80.0, 105.0),
        ]
    )

    assert [(record["baseline_ref"], record["candidate_ref"]) for record in records] == [
        ("v0.7.1", "main-commit"),
        ("v0.7.1", "v0.8.0-rc.1"),
        ("main-commit", "v0.8.0-rc.1"),
    ]


def test_release_qualification_requires_exact_main_and_candidate_tag(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    main_commit = "a" * 40
    monkeypatch.setattr(collector, "run_text", lambda _args: "b" * 40)
    monkeypatch.setattr(collector, "resolve_commit", lambda _ref: "b" * 40)

    collector.validate_release_qualification_refs(
        ["v0.7.1", main_commit, "v0.8.0-rc.1"]
    )
    with pytest.raises(RuntimeError, match="full 40-character commit SHA"):
        collector.validate_release_qualification_refs(
            ["v0.7.1", "main", "v0.8.0-rc.1"]
        )
    with pytest.raises(RuntimeError, match="must be v0.8.0-rc.N"):
        collector.validate_release_qualification_refs(
            ["v0.7.1", main_commit, "release-candidate"]
        )


def test_alias_refs_keep_independent_sample_buckets(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    built = [
        {
            "ref": "main-commit",
            "commit": "same",
            "binary": str(tmp_path / "main"),
            "adapter": str(tmp_path / "main-adapter"),
        },
        {
            "ref": "v0.8.0-rc.1",
            "commit": "same",
            "binary": str(tmp_path / "candidate"),
            "adapter": str(tmp_path / "candidate-adapter"),
        },
    ]

    sample_dirs: list[Path] = []

    def fake_measure(
        binary: Path,
        _adapter: Path,
        _fixture: Path,
        scratch: Path,
        expected_exit_status: int,
    ) -> dict[str, str]:
        assert expected_exit_status == 0
        sample_dirs.append(scratch)
        return {"binary": binary.name}

    monkeypatch.setattr(collector, "measure", fake_measure)
    samples = collector.collect_measurements(built, 0, 2, tmp_path, tmp_path, 0)

    assert samples == [
        [{"binary": "main"}, {"binary": "main"}],
        [{"binary": "candidate"}, {"binary": "candidate"}],
    ]
    assert len(sample_dirs) == len(set(sample_dirs))


def test_resolve_commit_falls_back_to_remote_tracking_branch(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    attempted: list[str] = []

    def fake_run_text(args: list[str], **_kwargs: object) -> str:
        attempted.append(args[-1])
        if args[-1] == "refs/remotes/origin/main^{commit}":
            return "0123456789abcdef"
        raise RuntimeError("missing")

    monkeypatch.setattr(collector, "run_text", fake_run_text)

    assert collector.resolve_commit("main") == "0123456789abcdef"
    assert attempted == ["main^{commit}", "refs/remotes/origin/main^{commit}"]


def test_measure_uses_wait4_without_an_external_time_program(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    profile = tmp_path / "profile.yaml"
    profile.write_text("profile")
    monkeypatch.setattr(collector, "PROFILE", profile)
    fixture = tmp_path / "fixture"
    fixture.mkdir()
    output = json.dumps({"findings": []}) + "\n"
    binary = tmp_path / "latt"
    binary.write_text(f"#!/bin/sh\nprintf '%s' '{output}'\n")
    binary.chmod(0o755)
    adapter = tmp_path / "latt-adapter-toml"
    adapter.write_text("#!/bin/sh\nexit 0\n")
    adapter.chmod(0o755)

    sample = collector.measure(binary, adapter, fixture, tmp_path / "sample", 0)

    assert sample["exit_status"] == 0
    assert sample["peak_rss_kibibytes"] > 0
    assert sample["findings"]["finding_count"] == 0


def test_measure_rejects_an_unexpected_exit_status(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    profile = tmp_path / "profile.yaml"
    profile.write_text("profile")
    monkeypatch.setattr(collector, "PROFILE", profile)
    fixture = tmp_path / "fixture"
    fixture.mkdir()
    binary = tmp_path / "latt"
    binary.write_text("#!/bin/sh\nprintf '{\"findings\": []}\\n'\nexit 2\n")
    binary.chmod(0o755)
    adapter = tmp_path / "latt-adapter-toml"
    adapter.write_text("#!/bin/sh\nexit 0\n")
    adapter.chmod(0o755)

    with pytest.raises(RuntimeError, match="returned 2, expected 0"):
        collector.measure(binary, adapter, fixture, tmp_path / "sample", 0)


def test_generated_report_matches_the_versioned_schema(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fixture = tmp_path / "register.toml"
    fixture.write_text("fixture")
    commit = "a" * 40
    digest = "b" * 64
    summary = {
        "minimum": 1,
        "median": 1,
        "mean": 1,
        "maximum": 1,
        "population_standard_deviation": 0.0,
    }
    output = {
        "exit_status": 0,
        "bytes": 2,
        "sha256": digest,
        "findings": {"finding_count": 0, "by_code": {}, "by_severity": {}},
    }
    revisions = [
        {
            "ref": ref,
            "commit": commit,
            "version": "lattice 0.7.1",
            "binary_sha256": digest,
            "adapter_sha256": digest,
            "release_build": {"locked": True, "cargo_profile": "release"},
            "toolchain": {"rustc": "rustc 1.95.0"},
            "preflight": E2E_PREFLIGHT,
            "output": output,
            "measurements": {
                "wall_time_nanoseconds": {"samples": [1], "summary": summary},
                "peak_rss_kibibytes": {"samples": [1], "summary": summary},
            },
        }
        for ref in ("v0.7.1", "main")
    ]

    def fake_run_text(args: list[str], **_kwargs: object) -> str:
        if args[:2] == ["git", "status"]:
            return ""
        if args[:2] == ["git", "rev-parse"]:
            return commit
        if args[0] == "rustc":
            return "rustc 1.95.0"
        raise AssertionError(args)

    monkeypatch.setattr(collector, "run_text", fake_run_text)
    monkeypatch.setattr(collector, "cpu_model", lambda: "test cpu")
    monkeypatch.setattr(collector, "runner_class", lambda: "local")
    monkeypatch.setattr(collector, "execution_identity", lambda: {"provider": "local"})
    scenario = json.loads((ROOT / "benchmarks" / "e2e" / "scenario.json").read_text())
    report = collector.build_report(scenario, 0, 1, fixture, revisions)
    schema = json.loads(REPORT_SCHEMA_PATH.read_text())

    Draft202012Validator.check_schema(schema)
    Draft202012Validator(schema).validate(report)


@needs_records
def test_retained_records_have_known_schema_versions() -> None:
    component_schema = json.loads(COMPONENT_SCHEMA_PATH.read_text())
    paths = sorted(RECORDS.glob("*.json"))
    assert paths, "no retained record to check"
    for path in paths:
        record = json.loads(path.read_text())
        if path.name.startswith("component-"):
            Draft202012Validator(component_schema).validate(record)
            continue
        assert record["schema_version"] in ("1.0", "1.1", "1.2"), path.name
        assert record["scenario"]["schema_version"] in ("1.0", "1.1", "1.2"), path.name
        if record["schema_version"] == "1.0":
            assert all("toolchain" not in revision for revision in record["revisions"])


@needs_records
def test_component_schema_accepts_a_named_scale_label_and_keeps_2_0() -> None:
    validator = Draft202012Validator(json.loads(COMPONENT_SCHEMA_PATH.read_text()))
    retained = json.loads(next(RECORDS.glob("component-*.json")).read_text())
    assert retained["schema_version"] == "2.0"
    validator.validate(retained)

    revised = json.loads(json.dumps(retained))
    revised["schema_version"] = "2.1"
    measurement = next(iter(revised["measurements"].values()))
    revised["measurements"]["document/parse/defects-1024-nodes"] = measurement
    validator.validate(revised)

    revised["measurements"]["document/parse/Defects 1024"] = measurement
    assert not validator.is_valid(revised)


@needs_records
def test_component_schema_requires_body_and_timing_from_2_2() -> None:
    validator = Draft202012Validator(json.loads(COMPONENT_SCHEMA_PATH.read_text()))
    record = json.loads(next(RECORDS.glob("component-*.json")).read_text())
    record["schema_version"] = "2.2"
    assert not validator.is_valid(record)
    timing = {"setup": "none", "input_drop": "borrowed", "output_drop": "timed"}
    for measurement in record["measurements"].values():
        measurement["body"] = "1"
        measurement["timing"] = timing
    validator.validate(record)

    measurement = next(iter(record["measurements"].values()))
    measurement["timing"] = {**timing, "output_drop": "sometimes"}
    assert not validator.is_valid(record)


def normalize_component(
    tree: Path, output: Path, *args: str, manifest: Path = COMPONENT_FIXTURE_PATH
) -> subprocess.CompletedProcess[str]:
    if shutil.which("cargo") is None:
        pytest.skip("cargo is not installed")
    return subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "latt-core",
            "--example",
            "benchmark_report",
            "--locked",
            "--",
            str(tree),
            str(manifest),
            str(output),
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


def component_record(tree: Path, output: Path, *args: str) -> dict[str, object]:
    result = normalize_component(tree, output, *args)
    assert result.returncode == 0, result.stderr
    record = json.loads(output.read_text())
    schema = json.loads(COMPONENT_SCHEMA_PATH.read_text())
    Draft202012Validator.check_schema(schema)
    Draft202012Validator(schema).validate(record)
    return record


def test_component_saved_tree_records_baseline_and_changes(tmp_path: Path) -> None:
    record = component_record(
        COMPONENT_TREES / "saved", tmp_path / "out.json", "--mode", "saved", "--baseline", "main"
    )
    assert record["schema_version"] == "2.2"
    assert record["baseline"] == {"requested": "main", "mode": "saved"}
    measurements = record["measurements"]
    manifest = json.loads(COMPONENT_FIXTURE_PATH.read_text())
    labels = [scale["label"] for scale in manifest["scales"]]
    assert "defects-1024-nodes" in labels
    assert "rules-4096-nodes" in labels
    assert "located-16384-nodes" in labels
    adapter_ids = {
        f"adapter/{scale['adapter']}/{scale['label']}" for scale in manifest["adapter_scales"]
    }
    assert len(measurements) == 9 * len(labels) + len(adapter_ids) == 48
    assert sorted(id for id, m in measurements.items() if m["change"]) == [
        "document/parse/256-nodes",
        "validation/validate/16384-nodes",
    ]
    assert measurements["document/parse/256-nodes"]["throughput"] == {
        "basis": "nodes",
        "elements": 256,
    }
    assert measurements["query/counts/256-nodes"]["throughput"] == {
        "basis": "nodes",
        "elements": 256,
    }
    assert measurements["service/counts/defects-1024-nodes"]["throughput"] == {
        "basis": "nodes",
        "elements": 1048,
    }
    assert measurements["query/reach/defects-1024-nodes"]["throughput"] == {
        "basis": "nodes",
        "elements": 1048,
    }
    assert measurements["validation/validate/rules-4096-nodes"]["throughput"] == {
        "basis": "nodes",
        "elements": 4096,
    }
    assert measurements["document/ingest/256-nodes"]["timing"] == {
        "setup": "parse-document",
        "input_drop": "timed",
        "output_drop": "untimed",
    }
    assert measurements["query/reach/256-nodes"]["timing"] == {
        "setup": "none",
        "input_drop": "borrowed",
        "output_drop": "timed",
    }


BODY_START = re.compile(r"^\s*// body: ((?:[a-z]+/[a-z-]+ ?)+)\s*$")
BODY_END = "// body-end"


def timed_bodies(*sources: tuple[str, str]) -> dict[str, str]:
    """Hash each operation's marked harness source.

    A body is every region between `// body: <group>/<function>` and `// body-end`
    in the harness files; an operation may own several (its timed closure and the
    helper it shares with the pin gate), and one region may name several
    operations that share a closure. Comment lines are dropped and whitespace runs
    collapsed, so rewording a comment or reindenting does not count as a change.
    """
    regions: dict[str, list[str]] = {}
    for name, source in sources:
        current: list[str] | None = None
        for number, line in enumerate(source.splitlines(), 1):
            stripped = line.strip()
            start = BODY_START.match(line)
            if start:
                assert current is None, f"{name}:{number} opens a body inside {' '.join(current)}"
                current = start.group(1).split()
                for operation in current:
                    regions.setdefault(operation, [])
            elif stripped == BODY_END:
                assert current is not None, f"{name}:{number} closes no open body"
                current = None
            elif current is not None and stripped and not stripped.startswith("//"):
                for operation in current:
                    regions[operation].append(stripped)
        assert current is None, f"{name} leaves {' '.join(current)} open"
    return {
        operation: hashlib.sha256(" ".join(" ".join(lines).split()).encode()).hexdigest()
        for operation, lines in regions.items()
    }


def test_component_timed_bodies_match_their_versions(tmp_path: Path) -> None:
    record = component_record(COMPONENT_TREES / "plain", tmp_path / "out.json")
    declared: dict[str, str] = {}
    for id, measurement in record["measurements"].items():
        operation = id.rsplit("/", 1)[0]
        assert declared.setdefault(operation, measurement["body"]) == measurement["body"]
    hashes = timed_bodies(*((path.name, path.read_text()) for path in COMPONENT_HARNESS_PATHS))
    history = json.loads(COMPONENT_BODIES_PATH.read_text())
    assert sorted(hashes) == sorted(declared) == sorted(history)
    for operation, entries in history.items():
        versions = [entry["body"] for entry in entries]
        assert len(set(versions)) == len(versions), f"{operation} repeats a body version"
        current = entries[-1]
        assert current["body"] == declared[operation], (
            f"{operation} declares body {declared[operation]} in operations/mod.rs, "
            f"bodies.json lists {current['body']} last"
        )
        assert current["sha256"] == hashes[operation], (
            f"{operation}'s timed body changed: bump its body in "
            "crates/latt-core/benches/operations/mod.rs and append "
            f'{{"body": "<new>", "sha256": "{hashes[operation]}"}} to {COMPONENT_BODIES_PATH.name}'
        )


def test_component_body_markers_are_balanced() -> None:
    with pytest.raises(AssertionError, match="opens a body inside"):
        timed_bodies(("t.rs", "// body: query/counts\n// body: query/reach\n"))
    with pytest.raises(AssertionError, match="closes no open body"):
        timed_bodies(("t.rs", "// body-end\n"))
    with pytest.raises(AssertionError, match="leaves query/counts open"):
        timed_bodies(("t.rs", "// body: query/counts\n"))
    commented = timed_bodies(("t.rs", "// body: a/b\n  x(1);\n  // why\n// body-end\n"))
    assert commented == timed_bodies(("t.rs", "// body: a/b\nx(1);\n// body-end\n"))


def test_component_plain_tree_records_no_comparison(tmp_path: Path) -> None:
    record = component_record(COMPONENT_TREES / "plain", tmp_path / "out.json")
    assert record["baseline"] == {"requested": None, "mode": "none"}
    assert all(m["change"] is None for m in record["measurements"].values())

    fallback = component_record(
        COMPONENT_TREES / "plain", tmp_path / "fallback.json", "--baseline", "main"
    )
    assert fallback["baseline"] == {"requested": "main", "mode": "none"}


def test_component_compared_mode_requires_change_where_covered(tmp_path: Path) -> None:
    record = component_record(
        COMPONENT_TREES / "saved", tmp_path / "out.json", "--mode", "compared", "--baseline", "main"
    )
    assert record["baseline"] == {"requested": "main", "mode": "compared"}
    assert record["measurements"]["document/parse/256-nodes"]["change"] is not None
    assert record["measurements"]["query/reach/256-nodes"]["change"] is None
    # A scale added after the baseline was saved: chain IDs still compare, its own record none.
    assert record["measurements"]["validation/validate/16384-nodes"]["change"] is not None
    assert all(
        m["change"] is None
        for id, m in record["measurements"].items()
        if id.endswith(("/defects-1024-nodes", "/rules-4096-nodes", "/located-16384-nodes"))
    )

    tree = tmp_path / "tree"
    shutil.copytree(COMPONENT_TREES / "saved", tree)
    shutil.rmtree(tree / "document" / "parse" / "256-nodes" / "change")
    result = normalize_component(tree, tmp_path / "out.json", "--mode", "compared", "--baseline", "main")
    assert result.returncode == 2
    assert "change/estimates.json is missing although the baseline covers" in result.stderr

    result = normalize_component(COMPONENT_TREES / "plain", tmp_path / "out.json", "--mode", "saved")
    assert result.returncode == 2
    assert "needs --baseline" in result.stderr


def test_component_missing_fence_file_is_an_error(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    shutil.copytree(COMPONENT_TREES / "plain", tree)
    fence_file = tree / "query" / "reach" / "16384-nodes" / "new" / "tukey.json"
    fence_file.unlink()
    result = normalize_component(tree, tmp_path / "out.json")
    assert result.returncode == 2
    assert str(fence_file) in result.stderr

    tree = tmp_path / "dispersion"
    shutil.copytree(COMPONENT_TREES / "plain", tree)
    estimates_file = tree / "output" / "json-findings" / "256-nodes" / "new" / "estimates.json"
    estimates = json.loads(estimates_file.read_text())
    del estimates["std_dev"]
    estimates_file.write_text(json.dumps(estimates))
    result = normalize_component(tree, tmp_path / "out.json")
    assert result.returncode == 2
    assert "std_dev" in result.stderr


def copy_run(tree: Path, source: str, target: str, full_id: str | None = None) -> Path:
    """Copy one run's `new/` under another directory, renaming its ID when given."""
    destination = tree / target / "new"
    shutil.copytree(tree / source / "new", destination)
    if full_id is not None:
        benchmark_file = destination / "benchmark.json"
        benchmark = json.loads(benchmark_file.read_text())
        benchmark["full_id"] = full_id
        benchmark_file.write_text(json.dumps(benchmark))
    return destination


def test_component_rejects_an_id_the_run_does_not_declare(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    shutil.copytree(COMPONENT_TREES / "plain", tree)
    copy_run(tree, "query/reach/256-nodes", "query/legacy/256-nodes", "query/legacy/256-nodes")
    result = normalize_component(tree, tmp_path / "out.json")
    assert result.returncode == 2
    assert "undeclared benchmark IDs: query/legacy/256-nodes" in result.stderr
    assert not (tmp_path / "out.json").exists()


def test_component_rejects_a_duplicate_full_id(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    shutil.copytree(COMPONENT_TREES / "plain", tree)
    copy = copy_run(tree, "query/reach/256-nodes", "query/reach/256-nodes-again")
    result = normalize_component(tree, tmp_path / "out.json")
    assert result.returncode == 2
    assert "query/reach/256-nodes is recorded twice" in result.stderr
    assert str(copy) in result.stderr


def test_component_rejects_throughput_other_than_the_scale_nodes(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    shutil.copytree(COMPONENT_TREES / "plain", tree)
    benchmark_file = tree / "query" / "reach" / "16384-nodes" / "new" / "benchmark.json"
    benchmark = json.loads(benchmark_file.read_text())
    benchmark["throughput"] = {"Elements": 256}
    benchmark_file.write_text(json.dumps(benchmark))
    result = normalize_component(tree, tmp_path / "out.json")
    assert result.returncode == 2
    assert "records 256 elements but scale 16384-nodes has 16384 nodes" in result.stderr


def test_component_rejects_a_repeated_scale_label(tmp_path: Path) -> None:
    manifest = json.loads(COMPONENT_FIXTURE_PATH.read_text())
    manifest["scales"].append({**manifest["scales"][0]})
    path = tmp_path / "fixture.json"
    path.write_text(json.dumps(manifest))
    result = normalize_component(COMPONENT_TREES / "plain", tmp_path / "out.json", manifest=path)
    assert result.returncode == 2
    assert "fixture scale label 256-nodes is declared twice" in result.stderr


def test_component_command_publishes_only_the_current_run(tmp_path: Path) -> None:
    """A run that omits an ID fails though an older run of it is on disk; baselines stay."""
    cargo = shutil.which("cargo")
    if cargo is None:
        pytest.skip("cargo is not installed")
    criterion = tmp_path / "criterion"
    shutil.copytree(COMPONENT_TREES / "saved", criterion)
    baseline = criterion / "document" / "parse" / "256-nodes" / "main" / "estimates.json"
    assert baseline.is_file()
    # Stands in for `cargo bench`: writes this run's results, every ID but one.
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    stub = bin_dir / "cargo"
    stub.write_text(
        "#!/bin/sh\n"
        'if [ "$1" = bench ]; then\n'
        f'    cd "{COMPONENT_TREES / "plain"}" || exit 1\n'
        "    for run in $(/usr/bin/find . -type d -name new); do\n"
        '        [ "$run" = ./query/reach/256-nodes/new ] && continue\n'
        '        mkdir -p "$CRITERION_HOME/$run" && cp "$run"/* "$CRITERION_HOME/$run/" || exit 1\n'
        "    done\n"
        "    exit 0\n"
        "fi\n"
        f'exec "{cargo}" "$@"\n'
    )
    stub.chmod(0o755)
    result = subprocess.run(
        ["sh", str(ROOT / "tools" / "benchmark.sh")],
        env={**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "LATTICE_CRITERION_DIR": str(criterion)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2, result.stderr
    assert "missing benchmark IDs: query/reach/256-nodes" in result.stderr
    assert baseline.is_file()
    assert not (criterion / "document" / "parse" / "256-nodes" / "change").exists()


@pytest.mark.parametrize("name", ["new", "change"])
def test_component_command_refuses_a_baseline_it_would_delete(name: str) -> None:
    result = subprocess.run(
        ["sh", str(ROOT / "tools" / "benchmark.sh"), "--save-baseline", name],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert "usage:" in result.stderr


def test_component_adapter_operations_run_only_at_their_adapter_scales(tmp_path: Path) -> None:
    manifest = json.loads(COMPONENT_FIXTURE_PATH.read_text())
    graph_labels = {scale["label"] for scale in manifest["scales"]}
    adapter_scales = {scale["label"]: scale for scale in manifest["adapter_scales"]}
    assert {scale["adapter"] for scale in adapter_scales.values()} == {"toml", "md", "prose-xref"}
    assert not graph_labels & adapter_scales.keys(), "a label names two runs"

    record = component_record(COMPONENT_TREES / "plain", tmp_path / "out.json")
    assert record["fixture"]["adapter_scales"] == manifest["adapter_scales"]
    by_label: dict[str, list[str]] = {}
    for id in record["measurements"]:
        group, function, label = id.split("/")
        by_label.setdefault(label, []).append(f"{group}/{function}")
    for label, scale in adapter_scales.items():
        # One adapter operation per adapter scale, and no graph operation there.
        assert by_label[label] == [f"adapter/{scale['adapter']}"], label
        measurement = record["measurements"][f"adapter/{scale['adapter']}/{label}"]
        assert measurement["throughput"] == {"basis": "nodes", "elements": scale["nodes"]}
    for label in graph_labels:
        assert not any(op.startswith("adapter/") for op in by_label[label]), label


def test_component_missing_adapter_run_or_scale_is_an_error(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    shutil.copytree(COMPONENT_TREES / "plain", tree)
    shutil.rmtree(tree / "adapter" / "md")
    result = normalize_component(tree, tmp_path / "out.json")
    assert result.returncode == 2
    assert "missing benchmark IDs: adapter/md/md-32768-rows" in result.stderr

    manifest = json.loads(COMPONENT_FIXTURE_PATH.read_text())
    manifest["adapter_scales"] = [s for s in manifest["adapter_scales"] if s["adapter"] != "md"]
    unscaled = tmp_path / "fixture.json"
    unscaled.write_text(json.dumps(manifest))
    result = normalize_component(COMPONENT_TREES / "plain", tmp_path / "out.json", manifest=unscaled)
    assert result.returncode == 2
    assert "declares no scale for adapter/md" in result.stderr

    del manifest["adapter_scales"]
    unscaled.write_text(json.dumps(manifest))
    result = normalize_component(COMPONENT_TREES / "plain", tmp_path / "out.json", manifest=unscaled)
    assert result.returncode == 2
    assert "adapter_scales must be an array" in result.stderr


@pytest.mark.parametrize(
    ("present", "found"),
    [(["latt", "lattice"], "latt"), (["lattice"], "lattice"), (["latt"], "latt"), ([], "latt")],
)
def test_a_revision_before_the_rename_is_found_by_its_old_name(
    tmp_path: Path, present: list[str], found: str
) -> None:
    (tmp_path / "release").mkdir()
    for name in present:
        (tmp_path / "release" / name).write_text("")
    assert collector.built(tmp_path, "latt", "lattice") == tmp_path / "release" / found


def test_a_revision_without_the_toml_binary_runs_latt_adapter(tmp_path: Path) -> None:
    release = tmp_path / "release"
    release.mkdir()
    binary = release / "latt"
    binary.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
    binary.chmod(0o755)

    adapter = collector.toml_adapter(tmp_path, binary)

    result = subprocess.run(
        [str(adapter), "--profile", "p.json", "--target", "t"],
        capture_output=True, text=True, check=True,
    )
    assert result.stdout.splitlines() == ["adapter", "toml", "--profile", "p.json", "--target", "t"]


def test_a_revision_with_the_toml_binary_runs_it(tmp_path: Path) -> None:
    release = tmp_path / "release"
    release.mkdir()
    (release / "latt-adapter-toml").write_text("")
    assert collector.toml_adapter(tmp_path, release / "latt") == release / "latt-adapter-toml"


E2E_SCENARIO = json.loads((ROOT / "benchmarks" / "e2e" / "scenario.json").read_text())
E2E_PREFLIGHT = {
    "command": "summary",
    "exit_status": 0,
    "node_counts": {"item": 16384},
    "edge_counts": {"references": 16383},
    "finding_counts": [
        {"code": "UNREFERENCED", "severity": "info", "count": 1},
        {"code": "UNTRACED", "severity": "info", "count": 1},
    ],
}
E2E_RECORDS = sorted((ROOT / "notes").glob("perf-*/e2e-*.json"))
needs_e2e_records = pytest.mark.skipif(not E2E_RECORDS, reason="retained benchmark records not in the tree")


def summary_stub(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, payload: object) -> Path:
    """Write a `latt` stub that prints `payload` as its summary JSON."""
    profile = tmp_path / "profile.yaml"
    profile.write_text("profile")
    monkeypatch.setattr(collector, "PROFILE", profile)
    payload_file = tmp_path / "summary.json"
    payload_file.write_text(json.dumps(payload))
    binary = tmp_path / "latt"
    binary.write_text(f"#!/bin/sh\ncat '{payload_file}'\n")
    binary.chmod(0o755)
    return binary


def run_preflight(binary: Path, tmp_path: Path) -> dict[str, object]:
    expected = collector.expected_invariants(E2E_SCENARIO)
    return collector.preflight(binary, tmp_path / "adapter", tmp_path, "HEAD~1", expected, 0)


def test_e2e_preflight_accepts_the_declared_graph(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = {
        "edge_counts": {"references": 16383},
        # Reversed on purpose: tally order is not a contract.
        "finding_counts": list(reversed(E2E_PREFLIGHT["finding_counts"])),
        "node_counts": {"item": 16384},
    }
    binary = summary_stub(tmp_path, monkeypatch, payload)
    assert run_preflight(binary, tmp_path) == E2E_PREFLIGHT


def test_e2e_preflight_refuses_a_revision_that_drops_nodes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = {**E2E_PREFLIGHT, "node_counts": {"item": 16383}}
    binary = summary_stub(tmp_path, monkeypatch, payload)
    with pytest.raises(
        RuntimeError,
        match=r"revision HEAD~1 violates the node_counts invariant: "
        r'expected \{"item": 16384\}, got \{"item": 16383\}',
    ):
        run_preflight(binary, tmp_path)


def test_e2e_preflight_refuses_a_wrong_finding_tally(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = {
        **E2E_PREFLIGHT,
        "finding_counts": [{"code": "UNTRACED", "severity": "info", "count": 1}],
    }
    binary = summary_stub(tmp_path, monkeypatch, payload)
    with pytest.raises(RuntimeError, match="revision HEAD~1 violates the finding_counts invariant"):
        run_preflight(binary, tmp_path)


def test_e2e_preflight_refuses_an_empty_successful_summary(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = {"node_counts": {}, "edge_counts": {}, "finding_counts": []}
    binary = summary_stub(tmp_path, monkeypatch, payload)
    with pytest.raises(RuntimeError, match="node_counts invariant"):
        run_preflight(binary, tmp_path)


def test_e2e_measured_findings_must_match_the_scenario() -> None:
    expected = collector.expected_invariants(E2E_SCENARIO)
    collector.ensure_expected_findings(
        {
            "finding_count": 2,
            "by_code": {"UNREFERENCED": 1, "UNTRACED": 1},
            "by_severity": {"info": 2},
        },
        "main",
        expected,
    )
    with pytest.raises(RuntimeError, match="revision main violates the measured findings invariant"):
        collector.ensure_expected_findings(
            {"finding_count": 0, "by_code": {}, "by_severity": {}}, "main", expected
        )


def test_e2e_scenario_expectations_agree_with_its_declared_counts() -> None:
    drifted = {**E2E_SCENARIO, "node_count": 16385}
    with pytest.raises(RuntimeError, match="expected.node_counts sums to 16384"):
        collector.expected_invariants(drifted)


@needs_e2e_records
def test_retained_e2e_records_validate_against_the_schema() -> None:
    validator = Draft202012Validator(json.loads(REPORT_SCHEMA_PATH.read_text()))
    for path in E2E_RECORDS:
        record = json.loads(path.read_text())
        if record["schema_version"] != "1.0":
            validator.validate(record)


@needs_e2e_records
def test_a_schema_1_2_record_requires_the_preflight() -> None:
    validator = Draft202012Validator(json.loads(REPORT_SCHEMA_PATH.read_text()))
    record = json.loads(E2E_RECORDS[-1].read_text())
    record = {**record, "schema_version": "1.2", "scenario": E2E_SCENARIO}
    assert not validator.is_valid(record)
    record["revisions"] = [
        {**revision, "preflight": E2E_PREFLIGHT} for revision in record["revisions"]
    ]
    validator.validate(record)

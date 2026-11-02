#!/usr/bin/env python3
"""Build and compare Lattice revisions on one Linux runner."""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tarfile
import time
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
SCENARIO = ROOT / "benchmarks" / "e2e" / "scenario.json"
PROFILE = ROOT / "benchmarks" / "e2e" / "profile.yaml"
GENERATOR = ROOT / "tools" / "generate-e2e-fixture.py"
REPORT_SCHEMA_VERSION = "1.2"
RELEASE_CANDIDATE_PATTERN = re.compile(r"v0\.8\.0-rc\.[1-9][0-9]*")


def run_text(args: list[str], *, cwd: Path = ROOT) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"{' '.join(args)} failed ({result.returncode}): {detail}")
    return result.stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def cpu_model() -> str:
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        for line in cpuinfo.read_text(errors="replace").splitlines():
            if line.startswith("model name") and ":" in line:
                return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def runner_class() -> str:
    if os.environ.get("GITHUB_ACTIONS"):
        return "github-actions"
    if os.environ.get("GITLAB_CI"):
        return "gitlab-ci"
    return "local"


def execution_identity() -> dict[str, str]:
    if os.environ.get("GITLAB_CI"):
        names = {
            "pipeline_id": "CI_PIPELINE_ID",
            "job_id": "CI_JOB_ID",
            "runner_id": "CI_RUNNER_ID",
            "runner_description": "CI_RUNNER_DESCRIPTION",
        }
        provider = "gitlab"
    elif os.environ.get("GITHUB_ACTIONS"):
        names = {
            "run_id": "GITHUB_RUN_ID",
            "run_attempt": "GITHUB_RUN_ATTEMPT",
            "job": "GITHUB_JOB",
            "runner_name": "RUNNER_NAME",
        }
        provider = "github"
    else:
        return {"provider": "local"}
    identity = {"provider": provider}
    identity.update(
        {field: value for field, name in names.items() if (value := os.environ.get(name))}
    )
    return identity


def safe_label(ref: str, index: int) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", ref).strip("-.")
    return f"{index:02d}-{cleaned or 'revision'}"


def resolve_commit(ref: str) -> str:
    candidates = [ref]
    if not ref.startswith("refs/") and not ref.startswith("origin/"):
        candidates.append(f"refs/remotes/origin/{ref}")
    errors: list[str] = []
    for candidate in candidates:
        try:
            return run_text(["git", "rev-parse", "--verify", f"{candidate}^{{commit}}"])
        except RuntimeError as error:
            errors.append(str(error))
    raise RuntimeError(f"cannot resolve benchmark revision {ref!r}: {'; '.join(errors)}")


def validate_release_qualification_refs(refs: list[str]) -> None:
    if len(refs) != 3:
        raise RuntimeError("release qualification requires exactly three refs")
    baseline, main_commit, candidate = refs
    if baseline != "v0.7.1":
        raise RuntimeError("0.8 release qualification baseline must be v0.7.1")
    if re.fullmatch(r"[0-9a-fA-F]{40}", main_commit) is None:
        raise RuntimeError("release qualification main ref must be a full 40-character commit SHA")
    if RELEASE_CANDIDATE_PATTERN.fullmatch(candidate) is None:
        raise RuntimeError("release qualification candidate must be v0.8.0-rc.N")
    tagged_commit = run_text(
        ["git", "rev-parse", "--verify", f"refs/tags/{candidate}^{{commit}}"]
    )
    if resolve_commit(candidate) != tagged_commit:
        raise RuntimeError(f"release qualification candidate {candidate} is not the matching tag")


def built(build: Path, name: str, before_rename: str) -> Path:
    """Return the release binary `name`, or its pre-0.8.0 name when only that exists."""
    current, old = build / "release" / name, build / "release" / before_rename
    return old if old.is_file() and not current.is_file() else current


def toml_adapter(build: Path, binary: Path) -> Path:
    """Return the program to run as the external toml adapter for one revision.

    Revisions before `latt adapter` build a `latt-adapter-toml` (or `adapter-toml`)
    binary. Later ones build the adapter into `latt` alone, so a two-line wrapper runs
    `latt adapter toml`: every revision is then measured on the same external-adapter
    path, at the cost of one `sh` exec per run in the later ones.
    """
    adapter = built(build, "latt-adapter-toml", "adapter-toml")
    if adapter.is_file():
        return adapter
    wrapper = build / "release" / "latt-adapter-toml-wrapper"
    wrapper.write_text(f"#!/bin/sh\nexec '{binary}' adapter toml \"$@\"\n")
    wrapper.chmod(0o755)
    return wrapper


def export_and_build(ref: str, index: int, work: Path) -> dict[str, str]:
    commit = resolve_commit(ref)
    slot = work / "revisions" / safe_label(ref, index)
    source = slot / "source"
    build = slot / "target"
    archive = slot / "source.tar"
    if slot.exists():
        shutil.rmtree(slot)
    source.mkdir(parents=True)
    run_text(["git", "archive", "--format=tar", f"--output={archive}", commit])
    with tarfile.open(archive) as bundle:
        bundle.extractall(source, filter="data")
    archive.unlink()
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "--manifest-path",
            str(source / "Cargo.toml"),
            "--target-dir",
            str(build),
        ],
        cwd=source,
        check=True,
    )
    # Revisions before the 0.8.0 rename build `lattice`; later ones build `latt`.
    binary = built(build, "latt", "lattice")
    if not binary.is_file():
        raise RuntimeError(f"release build for {ref} did not produce latt")
    adapter = toml_adapter(build, binary)
    return {
        "ref": ref,
        "commit": commit,
        "binary": str(binary),
        "adapter": str(adapter),
        "binary_sha256": sha256(binary),
        "adapter_sha256": sha256(adapter),
        "version": run_text([str(binary), "--version"]),
        "rustc": run_text(["rustc", "--version", "--verbose"], cwd=source),
    }


def finding_summary(output: bytes) -> dict[str, Any]:
    try:
        payload = json.loads(output)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"benchmark output is not valid UTF-8 JSON: {error}") from error
    if not isinstance(payload, dict) or not isinstance(payload.get("findings"), list):
        raise RuntimeError("benchmark JSON output must carry a findings array")
    findings = payload["findings"]
    codes: Counter[str] = Counter()
    severities: Counter[str] = Counter()
    for finding in findings:
        if not isinstance(finding, dict):
            raise RuntimeError("benchmark finding must be an object")
        code = finding.get("code")
        severity = finding.get("severity")
        if not isinstance(code, str) or not isinstance(severity, str):
            raise RuntimeError("benchmark finding must carry string code and severity")
        codes[code] += 1
        severities[severity] += 1
    return {
        "finding_count": len(findings),
        "by_code": dict(sorted(codes.items())),
        "by_severity": dict(sorted(severities.items())),
    }


def measure(
    binary: Path,
    adapter: Path,
    fixture: Path,
    sample_dir: Path,
    expected_exit_status: int,
) -> dict[str, Any]:
    sample_dir.mkdir(parents=True, exist_ok=True)
    stdout_path = sample_dir / "stdout.json"
    stderr_path = sample_dir / "stderr.txt"
    command = [
        str(binary),
        "validate",
        "--profile",
        str(PROFILE),
        "--adapter",
        str(adapter),
        "--target",
        str(fixture),
        "--format",
        "json",
    ]
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        started = time.perf_counter_ns()
        process = subprocess.Popen(
            command,
            stdout=stdout,
            stderr=stderr,
        )
        _, wait_status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(wait_status)
        elapsed = time.perf_counter_ns() - started
    output = stdout_path.read_bytes()
    stderr = stderr_path.read_text(errors="replace")
    if stderr:
        raise RuntimeError(f"benchmark command wrote stderr: {stderr.strip()}")
    if process.returncode != expected_exit_status:
        raise RuntimeError(
            "benchmark command returned "
            f"{process.returncode}, expected {expected_exit_status}"
        )
    semantics = finding_summary(output)
    return {
        "wall_time_nanoseconds": elapsed,
        "peak_rss_kibibytes": usage.ru_maxrss,
        "exit_status": process.returncode,
        "output_bytes": len(output),
        "output_sha256": hashlib.sha256(output).hexdigest(),
        "findings": semantics,
    }


def tally_map(tallies: Any, where: str) -> dict[tuple[str, str], int]:
    """Key finding tallies by (code, severity); list order is not a contract."""
    if not isinstance(tallies, list):
        raise RuntimeError(f"{where} finding_counts must be an array")
    keyed: dict[tuple[str, str], int] = {}
    for tally in tallies:
        if not isinstance(tally, dict):
            raise RuntimeError(f"{where} finding tally must be an object")
        code, severity, count = tally.get("code"), tally.get("severity"), tally.get("count")
        if not isinstance(code, str) or not isinstance(severity, str) or not isinstance(count, int):
            raise RuntimeError(f"{where} finding tally must carry code, severity and count")
        keyed[(code, severity)] = keyed.get((code, severity), 0) + count
    return keyed


def sorted_tallies(keyed: dict[tuple[str, str], int]) -> list[dict[str, Any]]:
    return [
        {"code": code, "severity": severity, "count": count}
        for (code, severity), count in sorted(keyed.items())
    ]


def expected_invariants(scenario: dict[str, Any]) -> dict[str, Any]:
    """Return the scenario's expected graph and findings, checked against its own counts.

    The kind names are profile vocabulary, so the manifest spells them out; their sums
    must still agree with the declared `node_count` and `edge_count` the generator builds.
    """
    expected = scenario.get("expected")
    if not isinstance(expected, dict):
        raise RuntimeError("scenario must declare expected node, edge and finding counts")
    for field, total in (("node_counts", "node_count"), ("edge_counts", "edge_count")):
        counts = expected.get(field)
        if not isinstance(counts, dict) or not all(
            isinstance(value, int) for value in counts.values()
        ):
            raise RuntimeError(f"scenario expected.{field} must map kinds to integers")
        if sum(counts.values()) != scenario.get(total):
            raise RuntimeError(
                f"scenario expected.{field} sums to {sum(counts.values())}, "
                f"but {total} is {scenario.get(total)}"
            )
    tally_map(expected.get("finding_counts"), "scenario expected")
    return expected


def preflight(
    binary: Path,
    adapter: Path,
    fixture: Path,
    ref: str,
    expected: dict[str, Any],
    expected_exit_status: int,
) -> dict[str, Any]:
    """Check, untimed, that one revision ingests the scenario's graph and finds what it should.

    `summary --format json` reports node, edge and finding counts in every supported
    revision, so the measured `validate` command carries no extra work. A revision whose
    adapter drops rows and still exits cleanly is refused here rather than measured.
    """
    command = [
        str(binary),
        "summary",
        "--profile",
        str(PROFILE),
        "--adapter",
        str(adapter),
        "--target",
        str(fixture),
        "--format",
        "json",
    ]
    result = subprocess.run(command, capture_output=True, check=False)
    if result.stderr:
        detail = result.stderr.decode(errors="replace").strip()
        raise RuntimeError(f"preflight for {ref} wrote stderr: {detail}")
    if result.returncode != expected_exit_status:
        raise RuntimeError(
            f"preflight for {ref} returned {result.returncode}, "
            f"expected {expected_exit_status}"
        )
    try:
        payload = json.loads(result.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"preflight for {ref} is not valid UTF-8 JSON: {error}") from error
    if not isinstance(payload, dict):
        raise RuntimeError(f"preflight for {ref} must be a JSON object")
    observed = {
        "node_counts": payload.get("node_counts"),
        "edge_counts": payload.get("edge_counts"),
        "finding_counts": sorted_tallies(
            tally_map(payload.get("finding_counts"), f"preflight for {ref}")
        ),
    }
    wanted = {
        "node_counts": expected["node_counts"],
        "edge_counts": expected["edge_counts"],
        "finding_counts": sorted_tallies(tally_map(expected["finding_counts"], "scenario")),
    }
    for invariant, value in wanted.items():
        if observed[invariant] != value:
            raise RuntimeError(
                f"revision {ref} violates the {invariant} invariant: "
                f"expected {json.dumps(value, sort_keys=True)}, "
                f"got {json.dumps(observed[invariant], sort_keys=True)}"
            )
    return {"command": "summary", "exit_status": result.returncode, **observed}


def ensure_expected_findings(
    findings: dict[str, Any], ref: str, expected: dict[str, Any]
) -> None:
    """Refuse a measured output whose finding tallies differ from the scenario's."""
    codes: Counter[str] = Counter()
    severities: Counter[str] = Counter()
    for (code, severity), count in tally_map(expected["finding_counts"], "scenario").items():
        codes[code] += count
        severities[severity] += count
    wanted = {
        "finding_count": sum(codes.values()),
        "by_code": dict(sorted(codes.items())),
        "by_severity": dict(sorted(severities.items())),
    }
    if findings != wanted:
        raise RuntimeError(
            f"revision {ref} violates the measured findings invariant: "
            f"expected {json.dumps(wanted, sort_keys=True)}, "
            f"got {json.dumps(findings, sort_keys=True)}"
        )


def ensure_consistent(samples: list[dict[str, Any]], ref: str) -> None:
    stable_fields = ("exit_status", "output_bytes", "output_sha256", "findings")
    expected = {field: samples[0][field] for field in stable_fields}
    for number, sample in enumerate(samples[1:], start=2):
        actual = {field: sample[field] for field in stable_fields}
        if actual != expected:
            raise RuntimeError(f"sample {number} for {ref} produced inconsistent output")


def numeric_summary(values: list[int]) -> dict[str, float | int]:
    return {
        "minimum": min(values),
        "median": statistics.median(values),
        "mean": statistics.mean(values),
        "maximum": max(values),
        "population_standard_deviation": statistics.pstdev(values),
    }


def summarize(revision: dict[str, str], samples: list[dict[str, Any]]) -> dict[str, Any]:
    ensure_consistent(samples, revision["ref"])
    first = samples[0]
    return {
        "ref": revision["ref"],
        "commit": revision["commit"],
        "version": revision["version"],
        "binary_sha256": revision["binary_sha256"],
        "adapter_sha256": revision["adapter_sha256"],
        "release_build": {"locked": True, "cargo_profile": "release"},
        "toolchain": {"rustc": revision["rustc"]},
        "preflight": revision["preflight"],
        "output": {
            "exit_status": first["exit_status"],
            "bytes": first["output_bytes"],
            "sha256": first["output_sha256"],
            "findings": first["findings"],
        },
        "measurements": {
            "wall_time_nanoseconds": {
                "samples": [sample["wall_time_nanoseconds"] for sample in samples],
                "summary": numeric_summary(
                    [sample["wall_time_nanoseconds"] for sample in samples]
                ),
            },
            "peak_rss_kibibytes": {
                "samples": [sample["peak_rss_kibibytes"] for sample in samples],
                "summary": numeric_summary(
                    [sample["peak_rss_kibibytes"] for sample in samples]
                ),
            },
        },
    }


def percent_change(candidate: float, baseline: float) -> float:
    if baseline == 0:
        raise RuntimeError("cannot compare against a zero baseline")
    return (candidate / baseline - 1.0) * 100.0


def comparisons(revisions: list[dict[str, Any]]) -> list[dict[str, Any]]:
    records = []
    for baseline, candidate in itertools.combinations(revisions, 2):
        base_wall = baseline["measurements"]["wall_time_nanoseconds"]["summary"]["median"]
        candidate_wall = candidate["measurements"]["wall_time_nanoseconds"]["summary"]["median"]
        base_rss = baseline["measurements"]["peak_rss_kibibytes"]["summary"]["median"]
        candidate_rss = candidate["measurements"]["peak_rss_kibibytes"]["summary"]["median"]
        records.append(
            {
                "baseline_ref": baseline["ref"],
                "candidate_ref": candidate["ref"],
                "wall_time_median_percent_change": percent_change(candidate_wall, base_wall),
                "peak_rss_median_percent_change": percent_change(candidate_rss, base_rss),
                "output_identical": candidate["output"] == baseline["output"],
                "threshold_enforced": False,
            }
        )
    return records


def collect_measurements(
    built: list[dict[str, str]],
    warmups: int,
    sample_count: int,
    fixture: Path,
    work: Path,
    expected_exit_status: int,
) -> list[list[dict[str, Any]]]:
    for round_number in range(warmups):
        order = range(len(built)) if round_number % 2 == 0 else reversed(range(len(built)))
        for revision_index in order:
            revision = built[revision_index]
            measure(
                Path(revision["binary"]),
                Path(revision["adapter"]),
                fixture,
                work
                / "warmups"
                / safe_label(revision["ref"], revision_index)
                / f"{round_number:02d}",
                expected_exit_status,
            )

    collected: list[list[dict[str, Any]]] = [[] for _revision in built]
    for round_number in range(sample_count):
        order = range(len(built)) if round_number % 2 == 0 else reversed(range(len(built)))
        for revision_index in order:
            revision = built[revision_index]
            sample = measure(
                Path(revision["binary"]),
                Path(revision["adapter"]),
                fixture,
                work
                / "samples"
                / safe_label(revision["ref"], revision_index)
                / f"{round_number:02d}",
                expected_exit_status,
            )
            collected[revision_index].append(sample)
    return collected


def build_report(
    scenario: dict[str, Any],
    warmups: int,
    sample_count: int,
    fixture_file: Path,
    revisions: list[dict[str, Any]],
) -> dict[str, Any]:
    dirty = bool(run_text(["git", "status", "--porcelain", "--untracked-files=normal"]))
    return {
        "schema_version": REPORT_SCHEMA_VERSION,
        "generated_unix_seconds": int(time.time()),
        "collector": {
            "commit": run_text(["git", "rev-parse", "HEAD"]),
            "working_tree_dirty": dirty,
        },
        "toolchain": {"rustc": run_text(["rustc", "--version", "--verbose"])},
        "environment": {
            "os": platform.system(),
            "architecture": platform.machine(),
            "cpu": cpu_model(),
            "runner_class": runner_class(),
            "execution": execution_identity(),
        },
        "policy": {
            "warmup_runs": warmups,
            "measurement_runs": sample_count,
            "round_order": "alternating",
            "performance_threshold_enforced": False,
        },
        "scenario": scenario,
        "inputs": {
            "scenario_manifest_sha256": sha256(SCENARIO),
            "profile_sha256": sha256(PROFILE),
            "generated_fixture_sha256": sha256(fixture_file),
            "generated_fixture_bytes": fixture_file.stat().st_size,
        },
        "revisions": revisions,
        "comparisons": comparisons(revisions),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("refs", nargs="*", default=["v0.7.1", "main"])
    parser.add_argument("--warmups", type=int)
    parser.add_argument("--samples", type=int)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--release-qualification",
        action="store_true",
        help="require v0.7.1, an exact main SHA, and an immutable v0.8.0-rc.N tag",
    )
    args = parser.parse_args()
    if len(args.refs) < 2:
        raise SystemExit("at least two Git refs are required")
    if sys.platform != "linux" or not hasattr(os, "wait4"):
        raise SystemExit("end-to-end collection requires Linux wait4 resource accounting")
    if args.release_qualification:
        validate_release_qualification_refs(args.refs)

    scenario = json.loads(SCENARIO.read_text())
    warmups = args.warmups if args.warmups is not None else scenario["warmup_runs"]
    sample_count = args.samples if args.samples is not None else scenario["measurement_runs"]
    expected_exit_status = scenario["expected_exit_status"]
    if warmups < 0 or sample_count < 1:
        raise SystemExit("warmups must be non-negative and samples must be positive")
    if not isinstance(expected_exit_status, int):
        raise SystemExit("scenario expected_exit_status must be an integer")
    expected = expected_invariants(scenario)
    work = Path(os.environ.get("LATTICE_E2E_WORK_DIR", ROOT / "target" / "e2e-benchmark"))
    output_path = args.output or ROOT / "target" / "benchmarks" / "lattice-e2e-v1.json"
    fixture = work / "fixture"
    subprocess.run([sys.executable, str(GENERATOR), str(SCENARIO), str(fixture)], check=True)
    fixture_file = fixture / "register.toml"

    built = [export_and_build(ref, index, work) for index, ref in enumerate(args.refs)]
    for revision in built:
        revision["preflight"] = preflight(
            Path(revision["binary"]),
            Path(revision["adapter"]),
            fixture,
            revision["ref"],
            expected,
            expected_exit_status,
        )
    collected = collect_measurements(
        built, warmups, sample_count, fixture, work, expected_exit_status
    )
    for revision, samples in zip(built, collected):
        for sample in samples:
            ensure_expected_findings(sample["findings"], revision["ref"], expected)

    revisions = [
        summarize(revision, collected[revision_index])
        for revision_index, revision in enumerate(built)
    ]
    report = build_report(scenario, warmups, sample_count, fixture_file, revisions)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, indent=2) + "\n")
    print(f"End-to-end comparison saved to {output_path}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2) from error

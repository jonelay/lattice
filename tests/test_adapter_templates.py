"""Cross-check of the adapter templates in examples/adapters: each prints expected.json."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

import pytest

from conftest import CORE_BIN, require_binary


ROOT = Path(__file__).resolve().parent.parent
EXAMPLES = ROOT / "examples" / "adapters"
PROFILE = EXAMPLES / "profile.yaml"
FIXTURE = EXAMPLES / "fixture"
EXPECTED = EXAMPLES / "expected.json"
TEMPLATES = ("python", "go", "rust")
DECLARATION = (
    'line is not a declaration "<kind> <id> [<edge-kind> <target-id>]... : <text>", '
    "so it was not read"
)


@pytest.fixture(scope="session")
def go_binary(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """Build the Go template once per session, or skip naming the missing toolchain."""
    go = shutil.which("go")
    if go is None:
        pytest.skip("go template: no go on PATH")
    out = tmp_path_factory.mktemp("go") / "lines-adapter"
    subprocess.run([go, "build", "-o", str(out), "."], cwd=EXAMPLES / "go", check=True)
    return out


@pytest.fixture(params=TEMPLATES)
def program(request: pytest.FixtureRequest) -> list[str]:
    """Return the command that runs one template, or skip naming what is missing."""
    name = request.param
    if name == "python":
        return [sys.executable, "-I", str(EXAMPLES / "python" / "adapter.py")]
    if name == "go":
        return [str(request.getfixturevalue("go_binary"))]
    binary = CORE_BIN.parent / "lines-adapter"
    require_binary(binary)
    return [str(binary)]


def resolved(tmp_path: Path, **adapter: Any) -> Path:
    """Write the core's resolved profile, with `adapter` keys replaced (None removes one)."""
    require_binary(CORE_BIN)
    document = json.loads(
        subprocess.run(
            [str(CORE_BIN), "show", "--profile", str(PROFILE)], capture_output=True, text=True, check=True
        ).stdout
    )
    for key, value in adapter.items():
        if value is None:
            document["adapter"].pop(key, None)
        else:
            document["adapter"][key] = value
    path = tmp_path / "resolved.json"
    path.write_text(json.dumps(document))
    return path


def run(program: list[str], *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([*program, *args], capture_output=True, text=True, check=False)


def document(program: list[str], profile: Path, target: Path) -> dict[str, Any]:
    result = run(program, "--profile", str(profile), "--target", str(target))
    assert result.returncode == 0, result.stderr
    assert result.stderr == ""
    return json.loads(result.stdout)


# Requirement: adapter-templates/Three templates print one document
def test_each_template_prints_expected_json(program: list[str], tmp_path: Path) -> None:
    printed = document(program, resolved(tmp_path), FIXTURE)
    if os.environ.get("LATTICE_REGEN_BASELINES") == "1" and program[-1].endswith("adapter.py"):
        EXPECTED.write_text(json.dumps(printed, indent=2, ensure_ascii=False) + "\n")
    assert printed == json.loads(EXPECTED.read_text())


# Requirement: adapter-templates/Three templates print one document
def test_validate_through_each_template(program: list[str], tmp_path: Path) -> None:
    # latt runs the adapter by path; a wrapper turns the Python command into one.
    adapter = tmp_path / "adapter"
    adapter.write_text("#!/bin/sh\nexec " + " ".join(f"'{part}'" for part in program) + ' "$@"\n')
    adapter.chmod(0o755)
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", str(PROFILE), "--adapter", str(adapter),
         "--target", str(FIXTURE), "--format", "json"],
        capture_output=True, text=True, check=False,
    )
    assert result.returncode == 1, result.stderr
    report = json.loads(result.stdout)
    expected = json.loads(EXPECTED.read_text())
    assert report["scope"]["nodes"] == dict(Counter(node["kind"] for node in expected["nodes"]))
    assert report["scope"]["edges"] == dict(Counter(edge["kind"] for edge in expected["edges"]))
    codes = Counter(finding["code"] for finding in report["findings"])
    assert codes["PARSE_ERROR"] == 4
    assert any(f["code"] == "VACANCY" and "REQ-9" in f["message"] for f in report["findings"])


# Requirement: adapter-templates/The templates read the line register format
def test_core_keys_in_the_adapter_block_are_ignored(program: list[str], tmp_path: Path) -> None:
    profile = resolved(tmp_path, native="md", timeout_seconds=30, command_timeout_seconds=10)
    assert document(program, profile, FIXTURE) == json.loads(EXPECTED.read_text())


# Requirement: adapter-templates/Unreadable input becomes a finding
@pytest.mark.parametrize("directory", ["nowhere", "register/notes.txt"])
def test_a_missing_directory_is_one_finding(program: list[str], tmp_path: Path, directory: str) -> None:
    printed = document(program, resolved(tmp_path, directory=directory), FIXTURE)
    assert printed["nodes"] == [] and printed["edges"] == []
    assert printed["findings"] == [{
        "severity": "error", "code": "PARSE_ERROR",
        "message": f'directory "{directory}" does not exist or is not a directory',
        "provenance": {"file": directory, "line": 0}, "node_id": None,
    }]


# Requirement: adapter-templates/Unreadable input becomes a finding
def test_nothing_selected_is_one_info_finding(program: list[str], tmp_path: Path) -> None:
    printed = document(program, resolved(tmp_path, extension=".none"), FIXTURE)
    assert printed["nodes"] == []
    assert printed["findings"] == [{
        "severity": "info", "code": "NO_MATCHING_FILES",
        "message": 'no file in "register" ends in ".none"',
        "provenance": {"file": "register", "line": 0}, "node_id": None,
    }]


# Requirement: adapter-templates/Unreadable input becomes a finding
def test_an_unreadable_file_is_a_finding_and_the_rest_is_read(program: list[str], tmp_path: Path) -> None:
    if os.geteuid() == 0:
        pytest.skip("root reads a file with no permissions")
    target = tmp_path / "target"
    shutil.copytree(FIXTURE, target)
    (target / "register" / "tests.reg").chmod(0)
    try:
        printed = document(program, resolved(tmp_path), target)
    finally:
        (target / "register" / "tests.reg").chmod(0o644)
    assert [node["id"] for node in printed["nodes"]] == ["REQ-1", "REQ-2", "REQ-3"]
    unreadable = [f for f in printed["findings"] if f["provenance"] == {"file": "register/tests.reg", "line": 0}]
    assert len(unreadable) == 1
    assert unreadable[0]["code"] == "PARSE_ERROR"
    assert unreadable[0]["message"].startswith("file could not be read: ")


# Requirement: adapter-templates/Unreadable input becomes a finding
def test_a_file_name_that_is_not_utf8_is_a_finding(program: list[str], tmp_path: Path) -> None:
    target = tmp_path / "target"
    shutil.copytree(FIXTURE, target)
    try:
        open(os.path.join(os.fsencode(target / "register"), b"caf\xe9.reg"), "wb").close()
    except OSError:
        pytest.skip("this file system refuses a non-UTF-8 name")
    printed = document(program, resolved(tmp_path), target)
    assert {
        "severity": "error", "code": "PARSE_ERROR",
        "message": "a file name in this directory is not valid UTF-8, so it was not read",
        "provenance": {"file": "register", "line": 0}, "node_id": None,
    } in printed["findings"]
    assert len(printed["nodes"]) == len(json.loads(EXPECTED.read_text())["nodes"])


# Requirement: adapter-templates/A template that cannot produce a document exits non-zero
@pytest.mark.parametrize(
    "args",
    [
        pytest.param(lambda p: ["--target", str(FIXTURE)], id="no-profile"),
        pytest.param(lambda p: ["--profile", str(p)], id="no-target"),
        pytest.param(lambda p: ["--profile", str(p), "--target", str(FIXTURE), "--strict"], id="unknown-argument"),
        pytest.param(lambda p: ["--profile", str(PROFILE), "--target", str(FIXTURE)], id="yaml-profile"),
        pytest.param(lambda p: ["--profile", str(p.with_name("absent.json")), "--target", str(FIXTURE)], id="no-file"),
    ],
)
def test_setup_failures_exit_2_with_empty_stdout(program: list[str], tmp_path: Path, args: Any) -> None:
    argv = args(resolved(tmp_path))
    result = run(program, *argv)
    assert result.returncode == 2
    assert result.stdout == ""
    assert len(result.stderr.strip().splitlines()) == 1
    assert result.stderr.startswith("lines-adapter: ")
    if "--profile" in argv and argv[argv.index("--profile") + 1].endswith((".yaml", "absent.json")):
        assert argv[argv.index("--profile") + 1] in result.stderr


# Requirement: adapter-templates/A template that cannot produce a document exits non-zero
@pytest.mark.parametrize(
    ("adapter", "named"),
    [
        ({"recursive": True}, "recursive"),
        ({"directory": None}, "directory"),
        ({"extension": 3}, "extension"),
    ],
)
def test_a_bad_adapter_block_exits_2_naming_the_key(
    program: list[str], tmp_path: Path, adapter: dict[str, Any], named: str
) -> None:
    result = run(program, "--profile", str(resolved(tmp_path, **adapter)), "--target", str(FIXTURE))
    assert result.returncode == 2
    assert result.stdout == ""
    assert result.stderr.startswith("lines-adapter: ")
    assert named in result.stderr


# Requirement: adapter-templates/The templates stand alone and ship
def test_the_python_template_runs_copied_alone(tmp_path: Path) -> None:
    copy = tmp_path / "elsewhere" / "adapter.py"
    copy.parent.mkdir()
    shutil.copy(EXAMPLES / "python" / "adapter.py", copy)
    printed = document([sys.executable, "-I", str(copy)], resolved(tmp_path), FIXTURE)
    assert printed == json.loads(EXPECTED.read_text())

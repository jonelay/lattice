"""The shell commands the documentation shows hold against the binary.

The ``sh`` fences in the README and docs/cli.md are the first thing a reader types:
every ``latt`` command there without a placeholder runs from the repo root
against the debug build and must exit 0 or 1, never 2. Commands with a placeholder
path, in those or any page under docs/, are checked for their flag names against the named
subcommand's ``--help``, so a renamed flag cannot survive in the docs.
"""
from __future__ import annotations

import json
import re
import shlex
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
CORE_BIN = REPO_ROOT / "target" / "debug" / "latt"
DOC_FILES = ["README.md", *sorted(
    str(p.relative_to(REPO_ROOT)) for p in (REPO_ROOT / "docs").glob("*.md")
)]
PLACEHOLDERS = ("/path/to", "yours", "<")
BINARY_SPELLINGS = ("latt", "target/debug/latt", "target/release/latt")

needs_core = pytest.mark.skipif(
    not CORE_BIN.exists(), reason="core not built; run: cargo build"
)


@needs_core
# Requirement: development-verification/One complete local verification command
def test_shipped_binary_has_native_adapters() -> None:
    """Fail when a later feature-restricted build left the binary without its defaults.

    Every gate after the workspace build sees ``target/debug/latt`` as whichever
    cargo invocation wrote it last, and CI ships that file as the artifact.
    """
    target = ["--profile", "profiles/md.yaml", "--target", "tests/fixtures/mini-md"]
    validate = subprocess.run(
        [str(CORE_BIN), "validate", *target], capture_output=True, text=True, cwd=REPO_ROOT
    )
    assert validate.returncode == 1, validate.stderr
    counts = subprocess.run(
        [str(CORE_BIN), "query", "counts", *target, "--format", "json"],
        capture_output=True, text=True, cwd=REPO_ROOT,
    )
    payload, _ = json.JSONDecoder().raw_decode(counts.stdout)
    assert payload == {"edges": {"traces_to": 3}, "nodes": {"requirement": 3}}, counts.stderr


def sh_commands(file: str) -> list[tuple[str, str]]:
    """Every command line in the file's ``sh`` fences, continuations joined."""
    commands: list[tuple[str, str]] = []
    in_fence = False
    pending = ""
    for number, line in enumerate((REPO_ROOT / file).read_text().splitlines(), 1):
        stripped = line.strip()
        if not in_fence:
            in_fence = stripped == "```sh"
            continue
        if stripped == "```":
            in_fence = False
            continue
        if stripped.endswith("\\"):
            pending += stripped[:-1] + " "
            continue
        if stripped:
            commands.append((f"{file}:{number}", (pending + stripped).strip()))
        pending = ""
    return commands


def lattice_commands(file: str) -> list[tuple[str, list[str]]]:
    """The ``latt`` invocations in a file, as argv after the binary."""
    found = []
    for where, command in sh_commands(file):
        argv = shlex.split(command)
        if argv and argv[0] in BINARY_SPELLINGS:
            found.append((where, argv[1:]))
    return found


def has_placeholder(argv: list[str]) -> bool:
    return any(marker in token for token in argv for marker in PLACEHOLDERS)


# The README and the command reference: every command there without a placeholder runs.
RUNNABLE_FILES = ("README.md", "docs/cli.md")
RUNNABLE = [
    (w, a) for file in RUNNABLE_FILES for w, a in lattice_commands(file)
    if not has_placeholder(a)
]
FLAG_CHECKED = [
    (w, a) for file in DOC_FILES for w, a in lattice_commands(file)
    if file not in RUNNABLE_FILES or has_placeholder(a)
]


def test_the_readme_shows_commands_this_test_can_run():
    assert len(RUNNABLE) >= 3, "the command reference runs against this repo"
    assert FLAG_CHECKED, "the docs show commands with placeholder paths"


@needs_core
@pytest.mark.parametrize("where,argv", RUNNABLE, ids=[w for w, _ in RUNNABLE])
def test_readme_command_runs_against_this_repo(where: str, argv: list[str]):
    result = subprocess.run(
        [str(CORE_BIN), *argv], cwd=REPO_ROOT, capture_output=True, text=True,
        timeout=300,
    )
    assert result.returncode in (0, 1), (
        f"{where}: `lattice {' '.join(argv)}` exited {result.returncode}\n{result.stderr}"
    )


def _has_browse() -> bool:
    """True when the debug binary was built with `browse`, which the tui feature adds."""
    top = subprocess.run([str(CORE_BIN), "--help"], capture_output=True, text=True, timeout=60)
    return re.search(r"^\s+browse\b", top.stdout, re.MULTILINE) is not None


def subcommand_path(argv: list[str]) -> list[str]:
    """``[validate]`` or ``[query, reaches]``: the words before the first flag."""
    path = []
    for token in argv:
        if token.startswith("-"):
            break
        path.append(token)
        if token != "query":
            break
    return path


@needs_core
@pytest.mark.parametrize("where,argv", FLAG_CHECKED, ids=[w for w, _ in FLAG_CHECKED])
def test_documented_flags_exist_on_the_subcommand(where: str, argv: list[str]):
    path = subcommand_path(argv)
    if path == ["browse"] and not _has_browse():
        pytest.skip("target/debug/latt was built without the tui feature")
    help_run = subprocess.run(
        [str(CORE_BIN), *path, "--help"], capture_output=True, text=True, timeout=60,
    )
    assert help_run.returncode == 0, f"{where}: no subcommand {' '.join(path)}"
    known = set(re.findall(r"--[a-z][a-z-]*", help_run.stdout))
    used = {token.split("=", 1)[0] for token in argv if token.startswith("--")}
    assert used <= known, f"{where}: flags not in `lattice {' '.join(path)} --help`: {used - known}"

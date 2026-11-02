"""The Rust adapter shims under ``adapters/`` run from a release install.

An install has ``latt`` and ``latt-adapter-entomologist`` on PATH and no ``target/``
tree; a checkout has the opposite. Each shim looks on PATH first, then under
``$CARGO_TARGET_DIR``, then in ``../target/debug``, and fails naming all three when
none holds its program. The entomologist shim runs ``latt-adapter-entomologist``; the
native adapters' shims run ``latt adapter <name>``. These tests run the shims from a
copy of ``adapters/`` with stub binaries, so they never need the built tree.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
EXTERNAL_SHIMS = ("entomologist",)
NATIVE_SHIMS = ("md", "prose-xref", "toml")
SHIMS = EXTERNAL_SHIMS + NATIVE_SHIMS
# The shim needs `sh`, `dirname` and `command`; a stub directory alone is not a PATH.
SYSTEM_PATH = "/usr/bin:/bin"


def _program(shim: str) -> str:
    return "latt" if shim in NATIVE_SHIMS else f"latt-adapter-{shim}"


def _leading_args(shim: str) -> list[str]:
    """What the shim puts before its own arguments: the subcommand, for a native one."""
    return ["adapter", shim] if shim in NATIVE_SHIMS else []


def _stub(directory: Path, name: str, label: str) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    stub = directory / name
    stub.write_text(f'#!/bin/sh\nprintf "%s\\n" "{label}" "$@"\n')
    stub.chmod(0o755)


def _run(adapters: Path, shim: str, path: str, **env_extra: str) -> subprocess.CompletedProcess:
    env = {k: v for k, v in os.environ.items() if k not in ("PATH", "CARGO_TARGET_DIR")}
    env["PATH"] = path
    env.update(env_extra)
    return subprocess.run(
        [str(adapters / shim), "--profile", "p.json", "--target", "t"],
        capture_output=True, text=True, env=env,
    )


@pytest.fixture
def adapters(tmp_path: Path) -> Path:
    """A copy of ``adapters/`` whose ``../target`` does not exist."""
    copy = tmp_path / "adapters"
    shutil.copytree(REPO_ROOT / "adapters", copy)
    assert not (tmp_path / "target").exists()
    return copy


# Requirement: builtin-adapter-dispatch/Adapter shims run from an install
@pytest.mark.parametrize("shim", SHIMS)
def test_shim_runs_the_binary_on_path(adapters: Path, tmp_path: Path, shim: str) -> None:
    _stub(tmp_path / "bin", _program(shim), "path")
    result = _run(adapters, shim, f"{tmp_path / 'bin'}{os.pathsep}{SYSTEM_PATH}")
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [
        "path", *_leading_args(shim), "--profile", "p.json", "--target", "t"
    ]


# Requirement: builtin-adapter-dispatch/Adapter shims run from an install
@pytest.mark.parametrize("shim", SHIMS)
def test_shim_falls_back_to_cargo_target_dir(adapters: Path, tmp_path: Path, shim: str) -> None:
    _stub(tmp_path / "cargo-target" / "debug", _program(shim), "cargo-target")
    result = _run(adapters, shim, SYSTEM_PATH, CARGO_TARGET_DIR=str(tmp_path / "cargo-target"))
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [
        "cargo-target", *_leading_args(shim), "--profile", "p.json", "--target", "t"
    ]


# Requirement: builtin-adapter-dispatch/Adapter shims run from an install
@pytest.mark.parametrize("shim", SHIMS)
def test_shim_falls_back_to_the_checkout_target(adapters: Path, tmp_path: Path, shim: str) -> None:
    _stub(tmp_path / "target" / "debug", _program(shim), "checkout")
    result = _run(adapters, shim, SYSTEM_PATH)
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [
        "checkout", *_leading_args(shim), "--profile", "p.json", "--target", "t"
    ]


# Requirement: builtin-adapter-dispatch/Adapter shims run from an install
@pytest.mark.parametrize("shim", SHIMS)
def test_shim_names_every_place_tried_when_none_holds_the_binary(
    adapters: Path, shim: str
) -> None:
    result = _run(adapters, shim, SYSTEM_PATH)
    assert result.returncode == 127
    assert result.stdout == ""
    for place in (_program(shim), "PATH", "CARGO_TARGET_DIR", "target/debug"):
        assert place in result.stderr, result.stderr


def test_the_native_shims_are_one_template() -> None:
    """The three native copies differ only in the adapter name they carry."""
    bodies = {
        shim: (REPO_ROOT / "adapters" / shim).read_text().replace(shim, "<name>")
        for shim in NATIVE_SHIMS
    }
    assert len(set(bodies.values())) == 1, "the native shims have drifted apart"


PYTHON_SHIMS = {"openspec": "adapters.openspec", "latt-suggest": "adapters.latt_suggest"}


def _bare_path(tmp_path: Path) -> str:
    """A PATH holding only what the shims need besides an interpreter."""
    tools = tmp_path / "tools"
    tools.mkdir(exist_ok=True)
    os.symlink(shutil.which("dirname"), tools / "dirname")
    return str(tools)


# Requirement: release-install/Python entry points run without a virtual environment
@pytest.mark.parametrize("shim", PYTHON_SHIMS)
def test_python_shim_prefers_the_venv(adapters: Path, tmp_path: Path, shim: str) -> None:
    _stub(tmp_path / ".venv" / "bin", "python", "venv")
    _stub(tmp_path / "bin", "python3", "path")
    result = _run(adapters, shim, f"{tmp_path / 'bin'}{os.pathsep}{SYSTEM_PATH}")
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines()[0] == "venv"
    assert PYTHON_SHIMS[shim] in result.stdout.splitlines()


# Requirement: release-install/Python entry points run without a virtual environment
@pytest.mark.parametrize("shim", PYTHON_SHIMS)
def test_python_shim_falls_back_to_python3_on_path(
    adapters: Path, tmp_path: Path, shim: str
) -> None:
    _stub(tmp_path / "bin", "python3", "path")
    result = _run(adapters, shim, f"{tmp_path / 'bin'}{os.pathsep}{SYSTEM_PATH}")
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines()[0] == "path"
    assert PYTHON_SHIMS[shim] in result.stdout.splitlines()


# Requirement: release-install/Python entry points run without a virtual environment
@pytest.mark.parametrize("shim", PYTHON_SHIMS)
def test_python_shim_imports_its_own_package_from_another_checkout(
    adapters: Path, tmp_path: Path, shim: str
) -> None:
    """The caller's directory is a checkout with its own ``adapters`` package."""
    init = adapters / "__init__.py"
    init.write_text(init.read_text() + '\nimport sys\nsys.stderr.write("install-package\\n")\n')
    interpreter = Path(sys.executable).parent
    env = {k: v for k, v in os.environ.items() if k not in ("PATH", "PYTHONPATH")}
    env["PATH"] = f"{interpreter}{os.pathsep}{SYSTEM_PATH}"
    result = subprocess.run(
        [str(adapters / shim), "--help"], capture_output=True, text=True, env=env, cwd=REPO_ROOT
    )
    assert result.returncode == 0, result.stderr
    assert "install-package" in result.stderr


# Requirement: release-install/Python entry points run without a virtual environment
@pytest.mark.parametrize("shim", PYTHON_SHIMS)
def test_python_shim_writes_no_bytecode_into_the_install(adapters: Path, shim: str) -> None:
    """An install is read, never written: no ``__pycache__`` appears beside the modules."""
    shutil.rmtree(adapters / "__pycache__", ignore_errors=True)
    interpreter = Path(sys.executable).parent
    env = {
        k: v for k, v in os.environ.items()
        if k not in ("PATH", "PYTHONPATH", "PYTHONDONTWRITEBYTECODE", "PYTHONPYCACHEPREFIX")
    }
    env["PATH"] = f"{interpreter}{os.pathsep}{SYSTEM_PATH}"
    result = subprocess.run(
        [str(adapters / shim), "--help"], capture_output=True, text=True, env=env
    )
    assert result.returncode == 0, result.stderr
    assert list(adapters.rglob("__pycache__")) == []


# Requirement: release-install/Python entry points run without a virtual environment
@pytest.mark.parametrize("shim", PYTHON_SHIMS)
def test_python_shim_names_both_places_when_no_interpreter(
    adapters: Path, tmp_path: Path, shim: str
) -> None:
    result = _run(adapters, shim, _bare_path(tmp_path))
    assert result.returncode == 127
    assert result.stdout == ""
    assert len(result.stderr.splitlines()) == 1, result.stderr
    for place in (".venv/bin/python", "python3", "PATH"):
        assert place in result.stderr, result.stderr


def test_the_python_shims_are_one_template() -> None:
    """The two copies differ only in their opening line and the module they run."""
    bodies = {
        shim: (REPO_ROOT / "adapters" / shim).read_text().split("\n", 2)[2].replace(module, "<m>")
        for shim, module in PYTHON_SHIMS.items()
    }
    assert len(set(bodies.values())) == 1, "the Python shims have drifted apart"

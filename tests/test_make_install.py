"""``tools/make-install.sh`` builds the release-style install a consumer runs.

The install is ``<dest>/latt/``: ``bin/`` with ``latt`` and one binary per
workspace adapter crate, beside the repository paths ``tools/install-files``
lists. A failed run leaves no install behind, and a file the manifest selects
that is missing on disk fails the run rather than vanishing from it.
"""
from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SCRIPT = REPO_ROOT / "tools" / "make-install.sh"
DEBUG = REPO_ROOT / "target" / "debug"

needs_core = pytest.mark.skipif(
    not (DEBUG / "latt").exists(), reason="core not built; run: cargo build"
)


def manifest() -> list[str]:
    lines = (REPO_ROOT / "tools" / "install-files").read_text().splitlines()
    return [line.strip() for line in lines if line.strip() and not line.startswith("#")]



def workspace_binaries() -> list[str]:
    """The ``latt`` package's binaries the release-install spec names, written out
    rather than read back from the manifest the script reads."""
    return ["latt", "latt-adapter-entomologist"]


def tracked(repo: Path) -> set[str]:
    listed = subprocess.run(
        ["git", "-C", str(repo), "ls-files", "-z", "--", *manifest()],
        capture_output=True, text=True, check=True,
    ).stdout
    return {p for p in listed.split("\0") if p}


def installed(install: Path) -> set[str]:
    return {
        str(p.relative_to(install)) for p in install.rglob("*")
        if p.is_file() and not p.is_relative_to(install / "bin")
    }


def make_install(
    bin_dir: Path, dest: Path, repo: Path = REPO_ROOT, cwd: Path | None = None
) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(repo / "tools" / "make-install.sh"), str(bin_dir), str(dest)],
        capture_output=True, text=True, cwd=cwd or repo,
    )


def fake_binaries(directory: Path, names: list[str]) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    for name in names:
        stub = directory / name
        stub.write_text("#!/bin/sh\n")
        stub.chmod(0o755)
    return directory


def git(repo: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@t",
         "-c", "commit.gpgsign=false", *args],
        check=True, capture_output=True,
    )


def mini_repo(tmp_path: Path) -> Path:
    """A git repo carrying the script, the manifest and a small copy of each path."""
    repo = tmp_path / "repo"
    (repo / "tools").mkdir(parents=True)
    shutil.copy2(SCRIPT, repo / "tools")
    shutil.copy2(REPO_ROOT / "tools" / "install-files", repo / "tools")
    (repo / "crates" / "latt").mkdir(parents=True)
    shutil.copy2(REPO_ROOT / "crates" / "latt" / "Cargo.toml", repo / "crates" / "latt")
    for path in manifest():
        if (REPO_ROOT / path).is_file():
            (repo / path).write_text(path)
        else:
            (repo / path).mkdir()
            (repo / path / "page one.md").write_text("spaced name")
    git(repo, "init", "-q")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "mini")
    return repo


# Requirement: release-install/The install is built from a binary directory
@needs_core
def test_install_from_the_debug_build(tmp_path: Path) -> None:
    result = make_install(DEBUG, tmp_path)
    assert result.returncode == 0, result.stderr
    install = tmp_path / "latt"
    assert sorted(p.name for p in (install / "bin").iterdir()) == sorted(workspace_binaries())
    for binary in (install / "bin").iterdir():
        assert os.access(binary, os.X_OK), binary
    assert installed(install) == tracked(REPO_ROOT)


# Requirement: release-install/The install is built from a binary directory
def test_a_missing_binary_is_named_and_nothing_is_written(tmp_path: Path) -> None:
    names = [n for n in workspace_binaries() if n != "latt-adapter-entomologist"]
    result = make_install(fake_binaries(tmp_path / "bin", names), tmp_path / "out")
    assert result.returncode != 0
    assert "latt-adapter-entomologist" in result.stderr
    assert not (tmp_path / "out").exists() or not any((tmp_path / "out").iterdir())


# Requirement: release-install/The install is built from a binary directory
def test_a_stale_binary_is_not_copied(tmp_path: Path) -> None:
    """``latt-adapter-md`` is what an earlier build leaves: its crate is now a library."""
    stale = ["adapter-gone", "latt-adapter-md"]
    bins = fake_binaries(tmp_path / "bin", [*workspace_binaries(), *stale])
    result = make_install(bins, tmp_path / "out")
    assert result.returncode == 0, result.stderr
    shipped = sorted(p.name for p in (tmp_path / "out/latt/bin").iterdir())
    assert shipped == sorted(workspace_binaries())


# Requirement: release-install/The install is built from a binary directory
def test_a_manifest_without_binaries_is_refused(tmp_path: Path) -> None:
    """An unreadable binary list must not read as an install of nothing."""
    repo = mini_repo(tmp_path)
    (repo / "crates" / "latt" / "Cargo.toml").write_text('[package]\nname = "latt"\n')
    bins = fake_binaries(tmp_path / "bin", workspace_binaries())
    result = make_install(bins, tmp_path / "out", repo=repo)
    assert result.returncode != 0
    assert "no [[bin]] target" in result.stderr
    assert not (tmp_path / "out" / "latt").exists()


# Requirement: release-install/The install is built from a binary directory
def test_an_existing_install_is_refused(tmp_path: Path) -> None:
    (tmp_path / "out" / "latt").mkdir(parents=True)
    (tmp_path / "out" / "latt" / "keep").write_text("earlier")
    result = make_install(fake_binaries(tmp_path / "bin", workspace_binaries()), tmp_path / "out")
    assert result.returncode != 0
    assert "exists" in result.stderr
    assert (tmp_path / "out" / "latt" / "keep").read_text() == "earlier"


# Requirement: release-install/The install is built from a binary directory
def test_a_tracked_file_missing_on_disk_fails(tmp_path: Path) -> None:
    repo = mini_repo(tmp_path)
    (repo / "docs" / "page one.md").unlink()
    bins = fake_binaries(tmp_path / "bin", workspace_binaries())
    result = make_install(bins, tmp_path / "out", repo)
    assert result.returncode != 0
    assert "docs/page one.md" in result.stderr
    assert not (tmp_path / "out" / "latt").exists()


# Requirement: release-install/The install is built from a binary directory
def test_untracked_files_stay_out_and_spaces_survive(tmp_path: Path) -> None:
    repo = mini_repo(tmp_path)
    (repo / "adapters" / "__pycache__").mkdir()
    (repo / "adapters" / "__pycache__" / "x.pyc").write_text("")
    (repo / "docs" / "draft.md").write_text("untracked")
    bins = fake_binaries(tmp_path / "bin", workspace_binaries())
    result = make_install(bins, tmp_path / "out", repo, cwd=tmp_path)
    assert result.returncode == 0, result.stderr
    assert installed(tmp_path / "out" / "latt") == tracked(repo)
    assert (tmp_path / "out/latt/docs/page one.md").read_text() == "spaced name"
    assert "make-install: git checkout" in result.stderr


# Requirement: release-install/The install is built from a binary directory
def test_install_from_an_extracted_archive(tmp_path: Path) -> None:
    repo = mini_repo(tmp_path)
    extracted = tmp_path / "extracted"
    extracted.mkdir()
    archive = subprocess.run(
        ["git", "-C", str(repo), "archive", "HEAD"], capture_output=True, check=True
    ).stdout
    subprocess.run(["tar", "-x", "-C", str(extracted)], input=archive, check=True)
    (extracted / "adapters" / "__pycache__").mkdir()
    (extracted / "adapters" / "__pycache__" / "x.pyc").write_text("")
    bins = fake_binaries(tmp_path / "bin", workspace_binaries())
    result = make_install(bins, tmp_path / "out", extracted)
    assert result.returncode == 0, result.stderr
    assert installed(tmp_path / "out" / "latt") == tracked(repo)
    assert "shipping every file" in result.stderr
    assert "(git: " in result.stderr


# Requirement: release-install/The install is built from a binary directory
def test_install_from_a_worktree(tmp_path: Path) -> None:
    repo = mini_repo(tmp_path)
    worktree = tmp_path / "worktree"
    git(repo, "worktree", "add", "-q", str(worktree))
    assert (worktree / ".git").is_file()
    bins = fake_binaries(tmp_path / "bin", workspace_binaries())
    result = make_install(bins, tmp_path / "out", worktree)
    assert result.returncode == 0, result.stderr
    assert installed(tmp_path / "out" / "latt") == tracked(repo)

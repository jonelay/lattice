from __future__ import annotations

import os
import shutil
import stat
import subprocess
from pathlib import Path

import pytest

from conftest import CORE_BIN, needs_core


# Requirement: release-install/Release binaries run on glibc 2.28 and later

ROOT = Path(__file__).parent.parent
SCRIPT = ROOT / "tools" / "check-release-binaries.sh"

# `readelf -V` lines as binutils prints them; the stub replays one per binary.
VERNEED = """\
Version needs section '.gnu.version_r' contains 2 entries:
  000000: Version: 1  File: libgcc_s.so.1  Cnt: 1
  0x0010:   Name: GCC_4.2.0  Flags: none  Version: 5
  0x0020: Version: 1  File: libc.so.6  Cnt: 4
  0x0030:   Name: GLIBC_2.2.5  Flags: none  Version: 3
  0x0040:   Name: GLIBC_2.17  Flags: none  Version: 4
{extra}"""


def stub_readelf(bin_dir: Path) -> None:
    """Write a `readelf` that prints `<file>.verneed`, and fails as for a non-ELF file without one."""
    bin_dir.mkdir()
    readelf = bin_dir / "readelf"
    readelf.write_text(
        '#!/bin/sh\n[ "$1" = -V ] || exit 64\n'
        '[ -f "$2.verneed" ] || { echo "readelf: Error: Not an ELF file" >&2; exit 1; }\n'
        'cat "$2.verneed"\n'
    )
    readelf.chmod(readelf.stat().st_mode | stat.S_IXUSR)


def binary(tmp_path: Path, name: str, extra: str = "", body: str = "ELF") -> Path:
    path = tmp_path / name
    path.write_text(body)
    path.with_name(name + ".verneed").write_text(VERNEED.format(extra=extra))
    return path


def check(tmp_path: Path, *args: str, path: str | None = None) -> subprocess.CompletedProcess[str]:
    env = {**os.environ, "HOME": "/home/builder"}
    env["PATH"] = path if path is not None else f"{tmp_path / 'stub'}:{os.environ['PATH']}"
    return subprocess.run(
        ["/bin/sh", str(SCRIPT), *args], capture_output=True, text=True, check=False, env=env
    )


@pytest.fixture
def stubbed(tmp_path: Path) -> Path:
    stub_readelf(tmp_path / "stub")
    return tmp_path


def test_binaries_at_or_below_the_floor_pass(stubbed: Path) -> None:
    a = binary(stubbed, "latt", "  0x0050:   Name: GLIBC_2.28  Flags: WEAK  Version: 6\n")
    b = binary(stubbed, "latt-adapter-x", "  0x0050:   Name: GLIBC_2.25  Flags: none  Version: 6\n")
    result = check(stubbed, "2.28", str(a), str(b))
    assert result.returncode == 0, result.stderr
    assert "glibc 2.28" in result.stdout


def test_a_version_above_the_floor_is_refused_by_name(stubbed: Path) -> None:
    a = binary(stubbed, "latt", "  0x0050:   Name: GLIBC_2.34  Flags: none  Version: 6\n")
    result = check(stubbed, "2.28", str(a))
    assert result.returncode == 1
    assert f"{a}: needs GLIBC_2.34" in result.stderr


def test_a_weak_version_above_the_floor_is_refused(stubbed: Path) -> None:
    # The loader prints "weak version `GLIBC_2.39' not found" on every run below it.
    a = binary(stubbed, "latt", "  0x0050:   Name: GLIBC_2.39  Flags: WEAK  Version: 6\n")
    result = check(stubbed, "2.34", str(a))
    assert result.returncode == 1
    assert f"{a}: needs GLIBC_2.39 (weak)" in result.stderr


def test_versions_compare_numerically(stubbed: Path) -> None:
    a = binary(stubbed, "latt", "  0x0050:   Name: GLIBC_2.3.4  Flags: none  Version: 6\n")
    assert check(stubbed, "2.28", str(a)).returncode == 0
    b = binary(stubbed, "latt-b", "  0x0050:   Name: GLIBC_2.100  Flags: none  Version: 6\n")
    assert check(stubbed, "2.28", str(b)).returncode == 1


def test_the_builders_home_is_refused(stubbed: Path) -> None:
    a = binary(stubbed, "latt", body="panicked at /home/builder/.cargo/registry/src/x.rs")
    result = check(stubbed, "2.28", str(a))
    assert result.returncode == 1
    assert f"{a}: carries the builder's home path" in result.stderr


def test_a_file_readelf_cannot_read_exits_2(stubbed: Path) -> None:
    other = stubbed / "notes.txt"
    other.write_text("text")
    result = check(stubbed, "2.28", str(other))
    assert result.returncode == 2
    assert str(other) in result.stderr


def test_missing_readelf_exits_2(tmp_path: Path) -> None:
    a = binary(tmp_path, "latt")
    # Every tool the script runs, readelf aside.
    tools = tmp_path / "tools"
    tools.mkdir()
    for name in ("awk", "grep"):
        (tools / name).symlink_to(shutil.which(name) or pytest.fail(f"{name} not found"))
    result = check(tmp_path, "2.28", str(a), path=str(tools))
    assert result.returncode == 2
    assert "readelf" in result.stderr


def test_usage_errors_exit_2(stubbed: Path) -> None:
    assert check(stubbed).returncode == 2
    assert check(stubbed, "2.28").returncode == 2
    assert check(stubbed, "two", str(binary(stubbed, "latt"))).returncode == 2


@needs_core
def test_a_host_build_is_refused_at_the_release_floor(tmp_path: Path) -> None:
    result = check(tmp_path, "2.28", str(CORE_BIN), path=os.environ["PATH"])
    if result.returncode == 0:
        pytest.skip("this host's glibc is 2.28 or older")
    assert result.returncode == 1, result.stderr
    assert "needs GLIBC_2." in result.stderr


BUILD = ROOT / "tools" / "release-binaries.sh"


def build(tmp_path: Path, *args: str, zigbuild: str = "cargo-zigbuild 0.24.0") -> subprocess.CompletedProcess[str]:
    """Run the build script with a stub cargo-zigbuild printing `zigbuild` as its version."""
    stub = tmp_path / "zigstub"
    stub.mkdir(exist_ok=True)
    tool = stub / "cargo-zigbuild"
    tool.write_text(f'#!/bin/sh\necho "{zigbuild}"\n')
    tool.chmod(tool.stat().st_mode | stat.S_IXUSR)
    env = {**os.environ, "PATH": f"{stub}:{os.environ['PATH']}"}
    return subprocess.run(
        ["/bin/sh", str(BUILD), *args], capture_output=True, text=True, check=False, env=env, cwd=ROOT
    )


def test_build_usage_exits_2(tmp_path: Path) -> None:
    result = build(tmp_path, "HEAD")
    assert result.returncode == 2
    assert "usage" in result.stderr


def test_build_refuses_an_existing_out_dir(tmp_path: Path) -> None:
    out = tmp_path / "out"
    out.mkdir()
    result = build(tmp_path, "HEAD", str(out))
    assert result.returncode == 2
    assert f"{out} exists" in result.stderr


def test_build_refuses_a_rev_that_is_not_a_commit(tmp_path: Path) -> None:
    result = build(tmp_path, "no-such-rev-s180", str(tmp_path / "out"))
    assert result.returncode == 2
    assert "no-such-rev-s180 is not a commit" in result.stderr


def test_build_names_the_install_command_for_a_wrong_zigbuild(tmp_path: Path) -> None:
    out = tmp_path / "out"
    result = build(tmp_path, "HEAD", str(out), zigbuild="cargo-zigbuild 0.1.0")
    assert result.returncode == 2
    assert "cargo install cargo-zigbuild --version 0.24.0 --locked" in result.stderr
    assert not out.exists()

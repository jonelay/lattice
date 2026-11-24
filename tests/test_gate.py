import fnmatch
import subprocess
import tomllib

import pytest
import yaml

from conftest import REPO_ROOT, gate_skip_allowed, require_binary


def workspace_binaries():
    workspace = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())["workspace"]
    names = set()
    for member in workspace["members"]:
        directory = REPO_ROOT / member
        manifest = tomllib.loads((directory / "Cargo.toml").read_text())
        names.update(binary["name"] for binary in manifest.get("bin", []))
        if (directory / "src/main.rs").exists():
            names.add(manifest["package"]["name"])
    return sorted(names)


@pytest.fixture
def ci_jobs():
    return yaml.safe_load((REPO_ROOT / ".github/workflows/ci.yml").read_text())["jobs"]


@pytest.mark.parametrize("name", workspace_binaries())
def test_ci_transfers_executable_workspace_binaries(ci_jobs, tmp_path, name):
    # Cover all workspace binaries, including those require_binary can skip on.
    upload = next(step["with"] for step in ci_jobs["rust-check"]["steps"]
                  if step.get("with", {}).get("name") == "lattice-binary")
    patterns = upload["path"].splitlines()
    path = f"target/debug/{name}"
    assert any(fnmatch.fnmatchcase(path, p) for p in patterns if not p.startswith("!")), name
    assert not any(fnmatch.fnmatchcase(path, p[1:]) for p in patterns if p.startswith("!")), name

    steps = ci_jobs["python-check"]["steps"]
    download = next(step["with"] for step in steps
                    if step.get("uses", "").startswith("actions/download-artifact@"))
    assert download == {"name": upload["name"], "path": "target/debug"}
    directory = tmp_path / download["path"]
    directory.mkdir(parents=True)
    for binary in workspace_binaries():
        (directory / binary).touch(mode=0o644)
    chmod = next(step["run"] for step in steps if step.get("run", "").startswith("chmod "))
    result = subprocess.run(["sh", "-eu", "-c", chmod], cwd=tmp_path, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    assert (directory / name).stat().st_mode & 0o111, name


def test_ci_pytest_enforces_snapshot_gate(ci_jobs):
    steps = [step for step in ci_jobs["python-check"]["steps"]
             if "-m pytest" in step.get("run", "")]
    assert steps
    for step in steps:
        assert step.get("env", {}).get("LATT_GATE") == "1"
        assert step.get("env", {}).get("LATT_GATE_SNAPSHOT") == "1"


@pytest.mark.parametrize("name", workspace_binaries())
def test_missing_binary_fails_even_in_snapshot_gate(monkeypatch, tmp_path, name):
    monkeypatch.setenv("LATT_GATE", "1")
    monkeypatch.setenv("LATT_GATE_SNAPSHOT", "1")
    with pytest.raises(pytest.fail.Exception, match="not built"):
        require_binary(tmp_path / name)


@pytest.mark.parametrize(
    "filename, reason",
    [
        ("test_benchmark_tools.py", "retained benchmark records not in the tree"),
        ("test_suggest.py", "measurement harness not in the tree"),
    ],
)
def test_snapshot_gate_skips(monkeypatch, filename, reason):
    monkeypatch.setenv("LATT_GATE", "1")
    path = REPO_ROOT / "tests" / filename

    monkeypatch.delenv("LATT_GATE_SNAPSHOT", raising=False)
    assert not gate_skip_allowed(path, reason)

    monkeypatch.setenv("LATT_GATE_SNAPSHOT", "1")
    assert gate_skip_allowed(path, reason)
    assert not gate_skip_allowed(path, reason + " (other)")
    assert not gate_skip_allowed(path.with_name("test_gate.py"), reason)

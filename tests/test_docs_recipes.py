"""The profile recipes in ``docs/profiles.md`` produce the findings the page promises.

Each test extracts the fence under a heading, writes it beside a fixture register
and runs the built ``latt`` over it. The binary is ``$CARGO_TARGET_DIR/debug/latt``
when that variable is set, else ``target/debug/latt``. An absent binary skips the
recipes in developer runs and fails the gate.
"""
from __future__ import annotations

import json
import re
import shutil
import subprocess

import pytest

from conftest import CORE_BIN, FIXTURES_DIR, REPO_ROOT, require_binary

FIXTURE = FIXTURES_DIR / "obligations-toml"
PAGE = REPO_ROOT / "docs" / "profiles.md"


def fence_under(heading: str) -> str:
    """The first yaml fence after ``heading`` in ``docs/profiles.md``."""
    text = PAGE.read_text()
    start = text.index(f"\n{heading}\n")
    match = re.search(r"```yaml\n(.*?)\n```", text[start:], re.S)
    assert match, f"no yaml fence under {heading}"
    return match.group(1) + "\n"


def run_validate(
    tmp_path_factory, profile: str, register: str | None = None, *flags: str
) -> tuple[int, list[str]]:
    """Run ``latt validate`` on the fixture; return the exit code and rendered findings.

    Each finding renders as ``severity CODE node_id: message``, sorted.
    """
    require_binary(CORE_BIN)
    target = tmp_path_factory.mktemp("obligations")
    shutil.copytree(FIXTURE / "registers", target / "registers")
    if register is not None:
        (target / "registers" / "r.toml").write_text(register)
    (target / "lattice.yaml").write_text(profile)
    result = subprocess.run(
        [str(CORE_BIN), "validate", "--profile", "lattice.yaml", "--adapter", "toml",
         "--target", ".", "--format", "json", *flags],
        cwd=target, capture_output=True, text=True,
    )
    assert result.returncode in (0, 1), result.stderr
    findings = json.loads(result.stdout)["findings"]
    rendered = sorted(
        f"{f['severity']} {f['code']} {f['node_id']}: {f['message']}" for f in findings
    )
    return result.returncode, rendered


def without_severity(profile: str) -> str:
    """The profile with its explicit ``severity: error`` lines removed."""
    return "".join(
        line for line in profile.splitlines(keepends=True) if "severity: error" not in line
    )


@pytest.fixture(scope="module")
def obligations(tmp_path_factory) -> tuple[int, list[str]]:
    return run_validate(tmp_path_factory, fence_under("### Per-item obligations"))


def test_obligation_profile_findings(obligations) -> None:
    code, rendered = obligations
    assert code == 1
    assert rendered == (FIXTURE / "expected-findings.txt").read_text().splitlines()


def test_obligation_omitted_attr_is_not_exempt(obligations) -> None:
    _, rendered = obligations
    assert any(r.startswith("error ATTR_REQUIRED D-5:") for r in rendered)
    assert not any(" COVERAGE D-5:" in r for r in rendered)


def test_obligation_unwanted_implementation_is_rejected(obligations) -> None:
    _, rendered = obligations
    assert any(r.startswith("error EDGE_WHERE I-2:") for r in rendered)


def test_obligation_severity_gates_the_exit_code(tmp_path_factory) -> None:
    """D-5 gets both flags so ATTR_REQUIRED cannot fail the run by itself."""
    profile = fence_under("### Per-item obligations")
    register = (FIXTURE / "registers" / "r.toml").read_text().replace(
        "needs_impl = false\n[[impl]]", "needs_impl = false\nneeds_test = false\n[[impl]]"
    )
    assert "needs_test = false\n[[impl]]" in register
    code, rendered = run_validate(tmp_path_factory, profile, register)
    assert code == 1 and {r.split()[0] for r in rendered} == {"error"}
    assert len(rendered) == 4

    stripped = without_severity(profile)
    assert stripped != profile
    code, rendered = run_validate(tmp_path_factory, stripped, register)
    assert code == 0 and {r.split()[0] for r in rendered} == {"warning"}
    assert len(rendered) == 4

    code, rendered = run_validate(tmp_path_factory, stripped, register, "--strict")
    assert code == 1 and {r.split()[0] for r in rendered} == {"error"}
    assert len(rendered) == 4

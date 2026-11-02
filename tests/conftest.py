import json
import shutil
import subprocess
import tempfile
from pathlib import Path

import pytest
import yaml

from adapters._profile import Profile
from adapters._profile import load_profile as _load_resolved


FIXTURES_DIR = Path(__file__).parent / "fixtures"


def resolve_profile(yaml_path: str | Path, out_dir: str | Path) -> Path:
    """Write the resolved JSON document the core would hand an adapter.

    Tests author profiles as YAML for readability; the reader under test only
    ever sees core-resolved JSON, so this stands in for the core's resolution.
    """
    data = yaml.safe_load(Path(yaml_path).read_text())
    data["resolved_schema"] = "1"
    out = Path(out_dir) / (Path(yaml_path).stem + ".resolved.json")
    out.write_text(json.dumps(data))
    return out


def uncomment_actions(yaml_path: str | Path, out_dir: str | Path) -> Path:
    """Write a copy of a shipped profile with its commented `actions` block on.

    The shipped profiles carry owner actions commented out; an adapter must
    still read the profile a user gets by uncommenting them.
    """
    lines: list[str] = []
    in_block = False
    for line in Path(yaml_path).read_text().splitlines():
        stripped = line.lstrip()
        indent = line[: len(line) - len(stripped)]
        if stripped == "# actions:":
            in_block = True
            lines.append(f"{indent}actions:")
        elif in_block and stripped.startswith("#   "):
            lines.append(f"{indent}  {stripped[4:]}")
        else:
            in_block = False
            lines.append(line)
    out = Path(out_dir) / Path(yaml_path).name
    out.write_text("\n".join(lines) + "\n")
    assert out.read_text() != Path(yaml_path).read_text(), "no actions block found"
    return out


def load_yaml_profile(yaml_path: str | Path) -> Profile:
    """Load a YAML profile as an adapter would see it: resolved first."""
    with tempfile.TemporaryDirectory() as scratch:
        return _load_resolved(resolve_profile(yaml_path, scratch))


def git(repo: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=mini-ent", "-c",
         "user.email=mini-ent@fixture.invalid", *args],
        check=True, capture_output=True,
    )


@pytest.fixture(scope="session")
def mini_ent_repo(tmp_path_factory) -> Path:
    """A throwaway repo with mini-ent committed on an orphan entomologist-data branch.

    The checked-in fixture is plain files because an orphan branch cannot be
    checked into a git repo; this builder is what turns it into the register
    the adapter actually reads. The non-UTF-8 description is written here, not
    checked in, so the fixture tree stays safe for text tooling.
    """
    repo = tmp_path_factory.mktemp("mini-ent") / "repo"
    shutil.copytree(FIXTURES_DIR / "mini-ent", repo)
    (repo / ("ff" * 16) / "description").write_bytes(b"\xff\xfe not utf-8")
    git(repo, "init", "-q", "-b", "entomologist-data")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "mini-ent register")
    return repo

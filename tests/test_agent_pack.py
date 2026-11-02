"""The agent material, ``AGENTS.md`` and ``skills/``, loads where an agent looks for it.

A skill directory holds ``SKILL.md``, whose front matter gives the ``name`` an agent
calls it by and the ``description`` it decides by; a skill without them is never
offered. The name matches its directory, as the Agent Skills format requires.
"""
from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent
SKILLS = REPO_ROOT / "skills"
FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---\n", re.S)


def front_matter(path: Path) -> dict[str, str]:
    match = FRONT_MATTER.match(path.read_text())
    assert match, f"{path}: no front matter"
    fields = {}
    for line in match.group(1).splitlines():
        key, _, value = line.partition(":")
        fields[key.strip()] = value.strip()
    return fields


# Requirement: release-install/Agent material ships in the install
def test_each_skill_names_itself_and_says_when_to_use_it():
    skills = sorted(SKILLS.glob("*/SKILL.md"))
    assert skills, "no skill under skills/"
    for skill in skills:
        fields = front_matter(skill)
        assert fields.get("name") == skill.parent.name, skill
        assert re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", fields["name"]), skill
        assert 0 < len(fields.get("description", "")) <= 1024, skill


# Requirement: release-install/Agent material ships in the install
def test_the_install_ships_the_agent_material():
    shipped = (REPO_ROOT / "tools" / "install-files").read_text().splitlines()
    assert "skills" in shipped
    assert "AGENTS.md" in shipped
    assert (REPO_ROOT / "AGENTS.md").is_file()

"""The CI recipes in ``docs/ci.md`` do what their markers say, from an install.

``tests/ci_recipes.py`` is the guard; the consumer CI job runs it directly
against an install with no toolchain. Here it runs against an install built
from ``target/debug`` by ``tools/make-install.sh``, with one item per recipe
and revision, plus tests of the guard itself over synthetic pages.
"""
from __future__ import annotations

import ast
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

import ci_recipes
from ci_recipes import Fixture, Recipe

REPO_ROOT = Path(__file__).parent.parent
DEBUG = REPO_ROOT / "target" / "debug"
DOC = REPO_ROOT / "docs" / "ci.md"
if DOC.exists():
    DOC_RECIPES, DOC_PROBLEMS = ci_recipes.parse_doc(DOC.read_text())
else:
    DOC_RECIPES, DOC_PROBLEMS = [], ["docs/ci.md is missing"]
RECIPE_NAMES = sorted(set(ci_recipes.INVENTORY) | {r.name for r in DOC_RECIPES})

needs_core = pytest.mark.skipif(
    not (DEBUG / "latt").exists(), reason="core not built; run: cargo build"
)

GITLAB = """\
lattice:
  script:
    - latt validate --profile lattice.yaml --adapter md --target .
  allow_failure:
    exit_codes: [1]
"""


def page(*recipes: tuple[str, str, str]) -> str:
    """A page of (marker, lang, body) recipes."""
    parts = ["# Recipes\n"]
    for marker, lang, body in recipes:
        parts.append(f"<!-- recipe: {marker} -->\n```{lang}\n{body}```\n")
    return "\n".join(parts)


@pytest.fixture(scope="session")
def install(tmp_path_factory) -> Path:
    if not (DEBUG / "latt").exists():
        pytest.skip("core not built; run: cargo build")
    dest = tmp_path_factory.mktemp("install")
    result = subprocess.run(
        [str(REPO_ROOT / "tools" / "make-install.sh"), str(DEBUG), str(dest)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    return dest / "latt"


@pytest.fixture(scope="session")
def env(install: Path, tmp_path_factory) -> dict[str, str]:
    return ci_recipes.recipe_env(install / "bin", tmp_path_factory.mktemp("home"))


@pytest.fixture(scope="session")
def fixture(env: dict[str, str], tmp_path_factory) -> Fixture:
    return Fixture(tmp_path_factory.mktemp("fixture"), env)


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_markers_an_unmarked_runnable_fence_fails() -> None:
    _, problems = ci_recipes.parse_doc("# Page\n\n```sh\nlattice validate\n```\n", "p.md")
    assert problems == ["p.md:3: a sh fence with no recipe marker"]


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_markers_an_orphan_marker_fails() -> None:
    text = "<!-- recipe: x posture=error kind=sh clean=0 warning=0 broken=1 unloadable=2 -->\n\nprose\n"
    _, problems = ci_recipes.parse_doc(text, "p.md")
    assert problems == ["p.md:1: marker not followed by a fence"]


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_markers_none_and_other_languages_are_not_run() -> None:
    text = page(("none", "sh", "latt --help\n")) + "\n```text\nexample\n```\n"
    recipes, problems = ci_recipes.parse_doc(text)
    assert (recipes, problems) == ([], [])


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_markers_a_value_the_kind_cannot_have_fails() -> None:
    marker = "x posture=setup kind=hook clean=0 warning=0 broken=0 unloadable=reject"
    _, problems = ci_recipes.parse_doc(page((marker, "sh", "true\n")), "p.md")
    assert len(problems) == 1 and "clean='0' is not a hook outcome" in problems[0]


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_matrix_a_marker_contradicting_its_posture_fails() -> None:
    marker = "error-gate posture=error kind=sh clean=0 warning=1 broken=1 unloadable=2"
    recipes, _ = ci_recipes.parse_doc(page((marker, "sh", "true\n")))
    problems = ci_recipes.check_markers(recipes)
    assert any("warning=1" in p and "error recipe passes there" in p for p in problems), problems


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_inventory_a_missing_recipe_is_named() -> None:
    problems = ci_recipes.check_markers([])
    assert {p for p in problems} == {f"docs/ci.md has no {n} recipe" for n in ci_recipes.INVENTORY}


# Requirement: ci-recipes/The guard runs every recipe against every revision
def test_shape_the_gitlab_reader_takes_the_documented_shape() -> None:
    assert ci_recipes.gitlab_job(GITLAB) == (
        ["latt validate --profile lattice.yaml --adapter md --target ."], {1}
    )


# Requirement: ci-recipes/The guard runs every recipe against every revision
@pytest.mark.parametrize("body", [
    GITLAB + "other:\n  script:\n    - true\n",
    GITLAB.replace("    exit_codes: [1]\n", "    exit_codes:\n      - 1\n"),
])
def test_shape_the_gitlab_reader_rejects_another_shape(body: str) -> None:
    with pytest.raises(ValueError):
        ci_recipes.gitlab_job(body)


def test_stdlib_the_guard_imports_only_the_standard_library() -> None:
    tree = ast.parse((REPO_ROOT / "tests" / "ci_recipes.py").read_text())
    modules = {
        alias.name.split(".")[0] for node in ast.walk(tree) if isinstance(node, ast.Import)
        for alias in node.names
    } | {
        node.module.split(".")[0] for node in ast.walk(tree)
        if isinstance(node, ast.ImportFrom) and node.module and node.level == 0
    }
    assert modules - {"__future__"} <= sys.stdlib_module_names, modules


# Requirement: ci-recipes/The guard runs every recipe against every revision
@needs_core
def test_drift_a_command_that_no_longer_matches_its_marker_fails(
    fixture: Fixture, env: dict[str, str]
) -> None:
    """An error gate whose command was edited to `--strict` stops at warning."""
    recipe = Recipe(
        "error-gate", "error", "sh",
        {"clean": "0", "warning": "0", "broken": "1", "unloadable": "2"},
        "latt validate --strict --profile lattice.yaml --adapter md --target .\n", 1,
    )
    problems = ci_recipes.check_recipe(recipe, fixture, "warning", env)
    assert any("error-gate at warning: 1, marker says 0" in p for p in problems), problems


# Requirement: ci-recipes/The guard runs every recipe against every revision
@needs_core
def test_zero_edges_at_clean_fail_before_any_recipe(
    env: dict[str, str], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    edgeless = tmp_path / "ci-consumer"
    shutil.copytree(ci_recipes.FIXTURE, edgeless)
    table = edgeless / "clean" / "requirements.md"
    table.write_text(table.read_text().replace("| REQ-2 |\n", "| |\n").replace("| REQ-0 |\n", "| |\n"))
    monkeypatch.setattr(ci_recipes, "FIXTURE", edgeless)
    problems = ci_recipes.check_revisions(Fixture(tmp_path / "work", env), env)
    assert problems == [
        "clean: the register yields no edges, so a zero-finding result proves nothing"
    ]


# Requirement: ci-recipes/The guard runs every recipe against every revision
@needs_core
def test_revisions_carry_exactly_their_findings(fixture: Fixture, env: dict[str, str]) -> None:
    assert ci_recipes.check_revisions(fixture, env) == []


# Requirement: release-install/Every shipped entry point runs from the install
@needs_core
def test_install_every_entry_point_runs(
    install: Path, env: dict[str, str], tmp_path: Path
) -> None:
    assert ci_recipes.check_install(install, tmp_path, env) == []


# Requirement: release-install/Every shipped entry point runs from the install
@needs_core
def test_install_the_checkout_cannot_rescue_a_broken_install(
    install: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    broken = tmp_path / "latt"
    shutil.copytree(install, broken, symlinks=True)
    (broken / "bin" / "latt-adapter-entomologist").unlink()
    monkeypatch.setenv("CARGO_TARGET_DIR", str(REPO_ROOT / "target"))
    env = ci_recipes.recipe_env(broken / "bin", tmp_path / "home")
    problems = ci_recipes.check_install(broken, tmp_path, env)
    assert [p for p in problems if p.startswith("adapters/entomologist ")], problems


def doc_recipe(name: str) -> Recipe:
    for recipe in DOC_RECIPES:
        if recipe.name == name:
            return recipe
    pytest.fail(f"docs/ci.md has no {name} recipe")


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_markers_the_page_is_well_formed() -> None:
    assert DOC_PROBLEMS == []


# Requirement: ci-recipes/Recipes cover four postures and state their outcomes
def test_inventory_the_page_holds_every_required_recipe() -> None:
    assert ci_recipes.check_markers(DOC_RECIPES) == []


# Requirement: ci-recipes/The guard runs every recipe against every revision
@needs_core
@pytest.mark.parametrize("rev", ci_recipes.REVISIONS)
@pytest.mark.parametrize("name", RECIPE_NAMES)
def test_recipe(name: str, rev: str, fixture: Fixture, env: dict[str, str]) -> None:
    assert ci_recipes.check_recipe(doc_recipe(name), fixture, rev, env) == []


# Requirement: ci-recipes/The guard runs every recipe against every revision
@needs_core
@pytest.mark.parametrize("name", RECIPE_NAMES)
def test_recipe_without_lattice(name: str, fixture: Fixture, env: dict[str, str]) -> None:
    assert ci_recipes.check_without_lattice(doc_recipe(name), fixture, env) == []

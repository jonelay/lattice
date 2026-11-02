"""Run the recipes in ``docs/ci.md`` against fixture revisions and an install.

Every ``sh`` and ``yaml`` fence in the page is preceded by a marker comment::

    <!-- recipe: <name> posture=<p> kind=<k> clean=<r> warning=<r> broken=<r> unloadable=<r> -->

or ``<!-- recipe: none -->`` for an illustration that is deliberately not run.
The guard builds a git repository from ``tests/fixtures/ci-consumer`` (one
commit per revision), runs each recipe at each revision in its own clone with
``latt`` from the install's ``bin/``, and fails on any outcome that differs
from the marker or from the posture's matrix held here, so a command and its
marker cannot drift together. A recipe it cannot run is a failure, not a skip.

Standard library only: the consumer CI job runs this file with the image's
``python3`` and nothing installed. ``tests/test_ci_recipes.py`` runs the same
functions under pytest from a checkout.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DOC = REPO_ROOT / "docs" / "ci.md"
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "ci-consumer"
MINI_ENT = REPO_ROOT / "tests" / "fixtures" / "mini-ent"
MINI_ENT_ISSUES = 6

REVISIONS = ("clean", "warning", "broken", "unloadable")
# The findings above hint each loadable revision carries, as (code, severity).
FINDINGS = {
    "clean": [],
    "warning": [("COVERAGE", "warning")],
    "broken": [("VACANCY", "error")],
}
# What a merge-request report must name at each revision, diffed against clean.
DIFF_NAMES = {"warning": "REQ-3", "broken": "REQ-9"}

# Whether each posture lets a revision through (`pass`) or stops on it.
POSTURES = {
    "advisory": ("pass", "pass", "pass", "pass"),
    "setup": ("pass", "pass", "pass", "stop"),
    "error": ("pass", "pass", "stop", "stop"),
    "strict": ("pass", "stop", "stop", "stop"),
    "report": ("pass", "pass", "pass", "stop"),
    "test": ("pass", "stop", "stop", "stop"),
}
KINDS = {
    "sh": None,  # an exit status
    "mr": None,
    "hook": {"commit": "pass", "reject": "stop"},
    "gitlab": {"pass": "pass", "warn": "pass", "fail": "stop"},
}
# The recipes docs/ci.md must hold, by name: (posture, kind).
INVENTORY = {
    "pre-commit-hook": ("setup", "hook"),
    "gitlab-job": ("setup", "gitlab"),
    "advisory": ("advisory", "sh"),
    "error-gate": ("error", "sh"),
    "strict-gate": ("strict", "sh"),
    "json-check": ("error", "sh"),
    "mr-report": ("report", "mr"),
    "profile-test": ("test", "sh"),
}
COULD_NOT_RUN = "lattice could not run"

MARKER = re.compile(r"^<!--\s*recipe:\s*(.*?)\s*-->\s*$")
FENCE = re.compile(r"^```(\w*)\s*$")


@dataclass
class Recipe:
    name: str
    posture: str
    kind: str
    expected: dict[str, str]
    body: str
    line: int


def parse_doc(text: str, where: str = "docs/ci.md") -> tuple[list[Recipe], list[str]]:
    """Return the page's marked recipes and every marker or fence problem."""
    recipes: list[Recipe] = []
    problems: list[str] = []
    lines = text.splitlines()
    pending: tuple[int, str] | None = None
    i = 0
    while i < len(lines):
        number, line = i + 1, lines[i]
        marker = MARKER.match(line)
        fence = FENCE.match(line)
        if marker:
            if pending:
                problems.append(f"{where}:{pending[0]}: marker not followed by a fence")
            pending = (number, marker.group(1))
            i += 1
            continue
        if fence:
            end = next((j for j in range(i + 1, len(lines)) if lines[j].startswith("```")), None)
            if end is None:
                problems.append(f"{where}:{number}: fence never closes")
                break
            body = "\n".join(lines[i + 1:end]) + "\n"
            runnable = fence.group(1) in ("sh", "yaml")
            if pending and pending[0] == number - 1:
                if pending[1] != "none":
                    recipe, problem = _recipe(pending[1], body, number, where)
                    if problem:
                        problems.append(problem)
                    else:
                        recipes.append(recipe)
            elif runnable:
                problems.append(f"{where}:{number}: a {fence.group(1)} fence with no recipe marker")
            if pending and pending[0] != number - 1:
                problems.append(f"{where}:{pending[0]}: marker not followed by a fence")
            pending = None
            i = end + 1
            continue
        if pending and line.strip():
            problems.append(f"{where}:{pending[0]}: marker not followed by a fence")
            pending = None
        i += 1
    if pending:
        problems.append(f"{where}:{pending[0]}: marker not followed by a fence")
    return recipes, problems


def _recipe(spec: str, body: str, line: int, where: str) -> tuple[Recipe, str | None]:
    name, _, rest = spec.partition(" ")
    fields = dict(f.split("=", 1) for f in rest.split() if "=" in f)
    recipe = Recipe(name, fields.get("posture", ""), fields.get("kind", ""),
                    {r: fields.get(r, "") for r in REVISIONS}, body, line)
    if recipe.posture not in POSTURES:
        return recipe, f"{where}:{line}: {name}: unknown posture {recipe.posture!r}"
    if recipe.kind not in KINDS:
        return recipe, f"{where}:{line}: {name}: unknown kind {recipe.kind!r}"
    for rev, value in recipe.expected.items():
        values = KINDS[recipe.kind]
        if (values is None and not value.isdigit()) or (values and value not in values):
            return recipe, f"{where}:{line}: {name}: {rev}={value!r} is not a {recipe.kind} outcome"
    return recipe, None


def outcome_class(kind: str, value: str) -> str:
    """Map a marker or observed outcome onto the posture matrix's pass/stop."""
    values = KINDS[kind]
    if values is None:
        return "pass" if value == "0" else "stop"
    return values[value]


def check_markers(recipes: list[Recipe]) -> list[str]:
    """Check every marker against its posture's matrix, and the inventory."""
    problems = []
    for recipe in recipes:
        for rev, want in zip(REVISIONS, POSTURES[recipe.posture]):
            got = outcome_class(recipe.kind, recipe.expected[rev])
            if got != want:
                problems.append(
                    f"{recipe.name}: marker says {rev}={recipe.expected[rev]}, which "
                    f"{'stops' if got == 'stop' else 'passes'}; a {recipe.posture} recipe "
                    f"{'stops' if want == 'stop' else 'passes'} there"
                )
    by_name = {r.name: r for r in recipes}
    for name, (posture, kind) in INVENTORY.items():
        recipe = by_name.get(name)
        if recipe is None:
            problems.append(f"docs/ci.md has no {name} recipe")
        elif (recipe.posture, recipe.kind) != (posture, kind):
            problems.append(f"{name}: is {recipe.posture}/{recipe.kind}, expected {posture}/{kind}")
    return problems


def gitlab_job(body: str) -> tuple[list[str], set[int]]:
    """Return a GitLab recipe's script lines and allowed exit codes.

    The shape is restricted so a stdlib reader suffices: one job, `script:` as
    a list of plain `- ` lines, and `exit_codes: [..]` in flow form.
    `docs_examples.rs` parses the same block with a real YAML parser.
    """
    lines = body.splitlines()
    jobs = [l for l in lines if l and not l[0].isspace() and not l.startswith("#")]
    if len(jobs) != 1 or not jobs[0].endswith(":"):
        raise ValueError(f"expected one job, found top-level lines {jobs}")
    script: list[str] = []
    in_script = False
    for line in lines:
        if re.match(r"^\s+script:\s*$", line):
            in_script = True
            continue
        item = re.match(r"^\s+- (.*)$", line)
        if in_script and item:
            script.append(item.group(1))
        elif in_script and line.strip():
            in_script = False
    codes = re.search(r"exit_codes:\s*\[([0-9,\s]+)\]", body)
    if not script or not codes:
        raise ValueError("expected a `script:` list and `allow_failure: exit_codes: [..]`")
    return script, {int(c) for c in codes.group(1).split(",") if c.strip()}


def recipe_env(bin_dir: Path, home: Path) -> dict[str, str]:
    """The environment every recipe and probe runs in: nothing inherited that
    could let the checkout or the caller's git configuration stand in."""
    templates = home / "git-templates"
    templates.mkdir(parents=True, exist_ok=True)
    return {
        "PATH": os.pathsep.join([str(bin_dir), str(Path(sys.executable).parent), "/usr/bin", "/bin"]),
        "HOME": str(home),
        "LANG": "C.UTF-8",
        "TMPDIR": os.environ.get("TMPDIR", "/tmp"),
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_TEMPLATE_DIR": str(templates),
        "GIT_AUTHOR_NAME": "ci-recipes",
        "GIT_AUTHOR_EMAIL": "ci-recipes@example.invalid",
        "GIT_COMMITTER_NAME": "ci-recipes",
        "GIT_COMMITTER_EMAIL": "ci-recipes@example.invalid",
    }


def _git(cwd: Path, env: dict[str, str], *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()


class Fixture:
    """The fixture revisions as one repository, each commit tagged by name."""

    def __init__(self, work: Path, env: dict[str, str]) -> None:
        self.env = env
        self.work = work
        self.origin = work / "origin"
        self.origin.mkdir(parents=True)
        _git(self.origin, env, "init", "-q")
        for rev in REVISIONS:
            for entry in self.origin.iterdir():
                if entry.name != ".git":
                    entry.unlink()
            for source in (FIXTURE / rev).iterdir():
                shutil.copy2(source, self.origin / source.name)
            _git(self.origin, env, "add", "-A")
            _git(self.origin, env, "commit", "-q", "-m", rev)
            _git(self.origin, env, "tag", rev)
        self.count = 0

    def clone(self, rev: str) -> Path:
        """Return a fresh clone checked out at `rev`."""
        self.count += 1
        clone = self.work / f"clone-{self.count}-{rev}"
        _git(self.work, self.env, "clone", "-q", str(self.origin), str(clone))
        _git(clone, self.env, "checkout", "-q", rev)
        return clone

    def sha(self, rev: str) -> str:
        return _git(self.origin, self.env, "rev-parse", rev)


def _lattice(args: list[str], cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess:
    return subprocess.run(["latt", *args], cwd=cwd, env=env, capture_output=True, text=True)


def check_revisions(fixture: Fixture, env: dict[str, str]) -> list[str]:
    """Check each revision carries exactly the findings its name states, that
    `clean` has edges, and that each loadable revision holds a suppressed error
    a recipe must not count, before any recipe result is believed."""
    target = ["--profile", "lattice.yaml", "--adapter", "md", "--target", "."]
    clean = fixture.clone("clean")
    counts = _lattice(["query", "counts", *target, "--format", "json"], clean, env)
    if counts.returncode not in (0, 1):
        return [f"clean: query counts exited {counts.returncode}: {counts.stderr.strip()}"]
    first, _ = json.JSONDecoder().raw_decode(counts.stdout)
    if not sum(first.get("edges", {}).values()):
        return ["clean: the register yields no edges, so a zero-finding result proves nothing"]
    problems = []
    for rev in REVISIONS:
        result = _lattice(["validate", *target, "--format", "json"], fixture.clone(rev), env)
        if rev == "unloadable":
            if result.returncode != 2:
                problems.append(f"unloadable: validate exited {result.returncode}, not 2")
            continue
        findings = json.loads(result.stdout)["findings"]
        found = sorted(
            (f["code"], f["severity"]) for f in findings
            if f["severity"] != "hint" and not f.get("suppressed")
        )
        if found != FINDINGS[rev]:
            problems.append(f"{rev}: findings above hint are {found}, expected {FINDINGS[rev]}")
        if not any(f["severity"] == "error" and f.get("suppressed") for f in findings):
            problems.append(f"{rev}: no suppressed error, so a recipe that counts one passes")
    return problems


def run_recipe(
    recipe: Recipe, fixture: Fixture, rev: str, env: dict[str, str], errexit: bool = False
) -> tuple[str, str]:
    """Run one recipe at one revision; return its outcome and its output.

    With `errexit`, a shell recipe runs under `bash -eo pipefail`, as a GitLab
    `script` line does."""
    clone = fixture.clone(rev)
    if recipe.kind == "hook":
        hook = clone / ".git" / "hooks" / "pre-commit"
        hook.parent.mkdir(exist_ok=True)
        hook.write_text(recipe.body)
        hook.chmod(0o755)
        (clone / "ci-sentinel").write_text(rev)
        _git(clone, env, "add", "ci-sentinel")
        before = _git(clone, env, "rev-parse", "HEAD")
        result = subprocess.run(
            ["git", "commit", "-q", "-m", "sentinel"], cwd=clone, env=env,
            capture_output=True, text=True,
        )
        after = _git(clone, env, "rev-parse", "HEAD")
        return ("commit" if after != before else "reject"), result.stdout + result.stderr
    if recipe.kind == "gitlab":
        script, allowed = gitlab_job(recipe.body)
        runner = clone.parent / f"{clone.name}.gitlab.sh"
        runner.write_text("set -eo pipefail\n" + "\n".join(script) + "\n")
        result = subprocess.run(
            ["bash", str(runner)], cwd=clone, env=env, capture_output=True, text=True
        )
        code = result.returncode
        return ("pass" if code == 0 else "warn" if code in allowed else "fail"), (
            result.stdout + result.stderr
        )
    run_env = dict(env)
    if recipe.kind == "mr":
        run_env["CI_MERGE_REQUEST_DIFF_BASE_SHA"] = fixture.sha("clean")
    script = clone.parent / f"{clone.name}.sh"
    script.write_text(recipe.body)
    shell = ["bash", "-eo", "pipefail"] if errexit else ["sh"]
    result = subprocess.run(
        [*shell, str(script)], cwd=clone, env=run_env, capture_output=True, text=True
    )
    return str(result.returncode), result.stdout + result.stderr


def shells(recipe: Recipe) -> tuple[bool, ...]:
    """Return the errexit settings a recipe is run under: both, for a shell recipe."""
    return (False, True) if recipe.kind in ("sh", "mr") else (False,)


def check_recipe(recipe: Recipe, fixture: Fixture, rev: str, env: dict[str, str]) -> list[str]:
    """Return the ways one recipe's run at one revision differs from its marker."""
    problems = []
    for errexit in shells(recipe):
        problems += _check_run(recipe, fixture, rev, env, errexit)
    return problems


def _check_run(
    recipe: Recipe, fixture: Fixture, rev: str, env: dict[str, str], errexit: bool
) -> list[str]:
    where = f"{recipe.name} at {rev}" + (" under set -e" if errexit else "")
    try:
        got, output = run_recipe(recipe, fixture, rev, env, errexit)
    except (ValueError, subprocess.CalledProcessError, OSError) as error:
        return [f"{where}: could not run: {error}"]
    problems = []
    want = recipe.expected[rev]
    if got != want:
        problems.append(f"{where}: {got}, marker says {want}\n{output}")
    if outcome_class(recipe.kind, got) != POSTURES[recipe.posture][REVISIONS.index(rev)]:
        problems.append(f"{where}: {got} contradicts the {recipe.posture} posture")
    says_why = recipe.kind == "hook" and got == "reject" or (
        recipe.posture == "advisory" and rev == "unloadable"
    )
    if says_why and COULD_NOT_RUN not in output:
        problems.append(f"{where}: no '{COULD_NOT_RUN}' line\n{output}")
    if recipe.kind == "mr" and rev in DIFF_NAMES and DIFF_NAMES[rev] not in output:
        problems.append(f"{where}: the report does not name {DIFF_NAMES[rev]}")
    return problems


def check_without_lattice(recipe: Recipe, fixture: Fixture, env: dict[str, str]) -> list[str]:
    """A setup gate stops, and an advisory recipe passes saying why, when
    `latt` is missing or not executable."""
    if recipe.posture not in ("setup", "advisory"):
        return []
    problems = []
    broken_bin = fixture.work / "non-executable-bin"
    broken_bin.mkdir(exist_ok=True)
    (broken_bin / "latt").write_text("#!/bin/sh\nexit 0\n")
    (broken_bin / "latt").chmod(0o644)
    rest = env["PATH"].split(os.pathsep)[1:]
    for label, path in (
        ("missing", rest),
        ("non-executable", [str(broken_bin), *rest]),
    ):
        stripped = dict(env, PATH=os.pathsep.join(path))
        for errexit in shells(recipe):
            where = f"{recipe.name} with lattice {label}" + (" under set -e" if errexit else "")
            try:
                got, output = run_recipe(recipe, fixture, "clean", stripped, errexit)
            except (ValueError, subprocess.CalledProcessError, OSError) as error:
                problems.append(f"{where}: could not run: {error}")
                continue
            want = "pass" if recipe.posture == "advisory" else "stop"
            if outcome_class(recipe.kind, got) != want:
                problems.append(f"{where}: {got}, expected it to {want}")
            if (recipe.posture == "advisory" or recipe.kind == "hook") and COULD_NOT_RUN not in output:
                problems.append(f"{where}: no '{COULD_NOT_RUN}' line")
    return problems


def check_install(install: Path, work: Path, env: dict[str, str]) -> list[str]:
    """Every adapter entry point reaches its program from the install, and the
    core resolves a bare external adapter name through PATH."""
    problems = []
    probe_dir = work / "probe"
    probe_dir.mkdir(exist_ok=True)
    for entry in sorted((install / "adapters").iterdir()):
        if entry.suffix or not entry.is_file() or not os.access(entry, os.X_OK):
            continue
        result = subprocess.run(
            [str(entry), "--help"], cwd=probe_dir, env=env, capture_output=True, text=True
        )
        if result.returncode != 0:
            problems.append(f"adapters/{entry.name} --help exited {result.returncode}: "
                            f"{result.stderr.strip()}")
    repo = work / "ent"
    shutil.copytree(MINI_ENT, repo)
    _git(repo, env, "init", "-q", "-b", "entomologist-data")
    _git(repo, env, "add", "-A")
    _git(repo, env, "commit", "-q", "-m", "register")
    result = _lattice(
        ["query", "counts", "--profile", str(install / "profiles" / "entomologist.yaml"),
         "--adapter", "entomologist", "--target", str(repo), "--format", "json"],
        probe_dir, env,
    )
    if result.returncode not in (0, 1):
        problems.append(f"bare --adapter entomologist exited {result.returncode}: "
                        f"{result.stderr.strip()}")
    else:
        counts, _ = json.JSONDecoder().raw_decode(result.stdout)
        if counts.get("nodes", {}).get("issue") != MINI_ENT_ISSUES:
            problems.append(f"bare --adapter entomologist read {counts}, "
                            f"expected {MINI_ENT_ISSUES} issues")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--install", type=Path, required=True,
                        help="the install directory make-install.sh wrote (<dest>/latt)")
    parser.add_argument("--doc", type=Path, default=DOC)
    args = parser.parse_args(argv)
    install = args.install.resolve()
    with tempfile.TemporaryDirectory(prefix="ci-recipes-") as tmp:
        work = Path(tmp)
        env = recipe_env(install / "bin", work / "home")
        python3 = shutil.which("python3", path=env["PATH"])
        print(f"install: {install}")
        print(f"latt: {shutil.which('latt', path=env['PATH'])}")
        print(f"python3: {python3} ({subprocess.run([python3 or 'python3', '--version'], capture_output=True, text=True).stdout.strip()})")
        recipes, problems = parse_doc(args.doc.read_text())
        problems += check_markers(recipes)
        problems += check_install(install, work, env)
        fixture = Fixture(work / "fixture", env)
        revision_problems = check_revisions(fixture, env)
        problems += revision_problems
        if not revision_problems:
            for recipe in recipes:
                for rev in REVISIONS:
                    found = check_recipe(recipe, fixture, rev, env)
                    print(f"{'FAIL' if found else 'ok  '} {recipe.name} at {rev}: "
                          f"expected {recipe.expected[rev]}")
                    problems += found
                problems += check_without_lattice(recipe, fixture, env)
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

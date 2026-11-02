from __future__ import annotations

import subprocess
from pathlib import Path

import pytest


ROOT = Path(__file__).parent.parent
SCRIPT = ROOT / "tools" / "check-breaking.sh"
LOG = ROOT / "tests" / "fixtures" / "check-breaking" / "semver.log"

LISTED = """\
- **`query::Direction` is hidden** (`enum_now_doc_hidden`), which seals
  `algo::Neighbors`: `trait_newly_sealed`.
- **`emit_document()` is removed**; `function_missing`.
"""


def check(
    tmp_path: Path, changelog: str, version: str = "0.8.0-rc.9", log: str | None = None
) -> subprocess.CompletedProcess[str]:
    path = tmp_path / "CHANGELOG.md"
    path.write_text(changelog)
    log_path = LOG
    if log is not None:
        log_path = tmp_path / "semver.log"
        log_path.write_text(log)
    return subprocess.run(
        ["sh", str(SCRIPT), str(path), version, str(log_path)],
        capture_output=True,
        text=True,
        check=False,
    )


# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_every_listed_lint_passes(tmp_path: Path) -> None:
    result = check(tmp_path, f"## [Unreleased]\n\n### Breaking\n{LISTED}\n### Fixed\n- x\n")
    assert result.returncode == 0, result.stderr
    assert "3 breaking lint(s), all listed" in result.stdout


# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_an_unlisted_lint_fails_naming_configuration_lint_and_item(tmp_path: Path) -> None:
    breaking = LISTED.replace("; `function_missing`", "")
    result = check(tmp_path, f"## [Unreleased]\n\n### Breaking\n{breaking}")
    assert result.returncode == 1
    assert result.stdout.splitlines() == [
        "unlisted: adapter-core default-features: function_missing: function adapter_core::emit_document"
    ]


@pytest.mark.parametrize(
    "changelog",
    [
        # Named only outside a Breaking subsection.
        f"## [Unreleased]\n\n### Changed\n{LISTED}",
        # Named only under a release other than Unreleased or the version given.
        f"## [Unreleased]\n\n## [0.8.0-rc.8] — 2026-10-01\n\n### Breaking\n{LISTED}",
        # Lint and item in different bullets.
        "## [Unreleased]\n\n### Breaking\n- `enum_now_doc_hidden`, `trait_newly_sealed`, `function_missing`\n"
        "- `Direction`, `Neighbors`, `emit_document`\n",
        # An item name inside a longer identifier is not the item.
        "## [Unreleased]\n\n### Breaking\n- `enum_now_doc_hidden` `Direct`; `trait_newly_sealed` `Neighbor`;"
        " `function_missing` `emit`\n",
    ],
    ids=["outside-breaking", "other-release", "split-bullets", "partial-name"],
)
# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_lint_not_listed_in_one_accepted_bullet_fails(tmp_path: Path, changelog: str) -> None:
    result = check(tmp_path, changelog)
    assert result.returncode == 1
    assert len(result.stdout.splitlines()) == 3


# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_the_release_section_of_the_given_version_counts(tmp_path: Path) -> None:
    changelog = f"## [Unreleased]\n\n## [0.8.0-rc.9] — 2026-10-06\n\n### Breaking\n{LISTED}"
    assert check(tmp_path, changelog).returncode == 0
    assert check(tmp_path, changelog, version="0.8.0-rc.10").returncode == 1


@pytest.mark.parametrize(
    ("released", "version", "refused"),
    [
        ("0.8.0", "0.8.1", True),
        ("0.8.0", "0.8.0", True),
        ("0.8.0", "0.9.0", False),
        ("0.8.0", "0.9.0-rc.1", False),
        # A pre-release is on its way to its version: 0.8.0-rc.8 → rc.9 heads for 0.8.0.
        ("0.8.0-rc.8", "0.8.0-rc.9", False),
        ("0.7.1", "0.8.0-rc.9", False),
    ],
)
# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_listed_break_is_refused_on_a_patch_line(
    tmp_path: Path, released: str, version: str, refused: bool
) -> None:
    changelog = tmp_path / "CHANGELOG.md"
    changelog.write_text(f"## [Unreleased]\n\n### Breaking\n{LISTED}")
    result = subprocess.run(
        ["sh", str(SCRIPT), "--released", released, str(changelog), version, str(LOG)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == (1 if refused else 0), result.stdout + result.stderr
    if refused:
        assert len(result.stdout.splitlines()) == 3
        assert result.stdout.startswith("refused: ")
        assert f"{version} is a patch release after {released}" in result.stderr


def test_bad_usage_exits_two(tmp_path: Path) -> None:
    missing = subprocess.run(
        ["sh", str(SCRIPT), str(tmp_path / "absent.md"), "0.8.0", str(LOG)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert missing.returncode == 2
    too_few = subprocess.run(["sh", str(SCRIPT), str(LOG)], capture_output=True, text=True, check=False)
    assert too_few.returncode == 2


def breaking(*bullets: str) -> str:
    return "## [Unreleased]\n\n### Breaking\n" + "".join(f"- {b}\n" for b in bullets)


def rust_log(lint: str, *items: str) -> str:
    lines = "".join(f"  {item}\n" for item in items)
    return f"== semver-checks: latt-core all-features\n--- failure {lint}: x ---\n\nFailed in:\n{lines}"


@pytest.mark.parametrize(
    ("item", "name", "listed"),
    [
        ("function latt_core::query::reach, previously in file /b/src/query.rs:9", "reach", True),
        ("function latt_core::query::reach, previously in file /b/src/query.rs:9", "query::reach()", True),
        # A containing module is not the item.
        ("function latt_core::query::reach, previously in file /b/src/query.rs:9", "query", False),
        ("function latt_core::query::reach, previously in file /b/src/query.rs:9", "other::reach", False),
        # The location is not part of the item, in any of the forms semver-checks prints.
        ("struct latt_core::Node in /w/src/graph.rs:4", "graph", False),
        ("struct latt_core::Node in /w/src/graph.rs:4", "Node", True),
        ("latt_core::Edge::new, previously at /b/src/graph.rs:7", "graph", False),
        ("latt_core::Edge::new, previously at /b/src/graph.rs:7", "Edge::new", True),
        ("latt_core::Edge::kind now takes &self, not self, in /w/src/graph.rs:9", "Edge::kind", True),
        # A field is named through its struct, never by the struct alone.
        ("field Profile.kinds in file /w/src/profile.rs:3", "Profile.kinds", True),
        ("field Profile.kinds in file /w/src/profile.rs:3", "Profile::kinds", True),
        ("field Profile.kinds in file /w/src/profile.rs:3", "Profile", False),
        # A rustdoc-JSON comparison (`--baseline-rustdoc`) prints relative locations.
        ("function probe::reach, previously in file src/lib.rs:1", "src", False),
        ("function probe::reach, previously in file src/lib.rs:1", "file", False),
        ("function probe::reach, previously in file src/lib.rs:1", "reach", True),
        ("Node::K, previously at src/lib.rs:3", "src", False),
        ("C in file src/lib.rs:7", "C", True),
        # 0.51.0 names a member through its container in words.
        ("field kinds of struct Node, previously in file src/lib.rs:2", "Node.kinds", True),
        ("field kinds of struct Node, previously in file src/lib.rs:2", "kinds", True),
        ("field kinds of struct Node, previously in file src/lib.rs:2", "Node", False),
        ("field kinds of struct Node, previously in file src/lib.rs:2", "struct", False),
        ("method m of trait Tr, previously in file src/lib.rs:5", "Tr::m", True),
        ("method m of trait Tr, previously in file src/lib.rs:5", "Tr", False),
        ("method m of trait Tr, previously in file src/lib.rs:5", "Other::m", False),
    ],
)
# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_rust_item_is_listed_by_its_own_name(tmp_path: Path, item: str, name: str, listed: bool) -> None:
    result = check(tmp_path, breaking(f"`function_missing` removes `{name}`"), log=rust_log("function_missing", item))
    assert result.returncode == (0 if listed else 1), result.stdout + result.stderr


# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_failure_with_no_items_is_bad_input(tmp_path: Path) -> None:
    log = (
        "== semver-checks: latt-core all-features\n"
        "     Checked [   0.005s] 196 checks: 195 pass, 1 fail, 0 warn, 57 skip\n"
        "--- failure function_missing: pub fn removed or renamed ---\n\nDescription:\nx\n"
    )
    result = check(tmp_path, breaking("`function_missing` removes `reach`"), log=log)
    assert result.returncode == 2
    assert "latt-core all-features: function_missing" in result.stderr


RUSTDOC_LOG = LOG.with_name("semver-0.51-rustdoc.log")
RUSTDOC_LISTED = breaking(
    "`enum_missing`: `Hid`",
    "`enum_variant_missing`: `Dir::Down`",
    "`function_missing`: `query::deep`, `reach`",
    "`inherent_associated_pub_const_missing`: `Node::K`",
    "`inherent_method_missing`: `Node::walk`",
    "`pub_module_level_const_missing`: `C`",
    "`struct_pub_field_missing`: `Node.kinds`",
    "`trait_added_supertrait`, `trait_newly_sealed`: `Seal`",
    "`trait_method_missing`: `Tr::m`",
)


# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_captured_rustdoc_comparison_reconciles(tmp_path: Path) -> None:
    # Captured from cargo-semver-checks 0.51.0 over two rustdoc JSON files.
    result = check(tmp_path, RUSTDOC_LISTED, log=RUSTDOC_LOG.read_text())
    assert result.returncode == 0, result.stdout + result.stderr
    assert "11 breaking lint(s), all listed" in result.stdout
    # Naming only the containers of a field and a trait method lists neither.
    containers = RUSTDOC_LISTED.replace("`Node.kinds`", "`Node`").replace("`Tr::m`", "`Tr`")
    result = check(tmp_path, containers, log=RUSTDOC_LOG.read_text())
    assert result.returncode == 1
    assert result.stdout.splitlines() == [
        "unlisted: probe default-features: struct_pub_field_missing: field kinds of struct Node",
        "unlisted: probe default-features: trait_method_missing: method m of trait Tr",
    ]


def contract_log(file: str, lint: str, item: str) -> str:
    # The shape `contract_breaks_since_the_release_are_listed` writes.
    return f"== semver-checks: contracts {file}\n--- failure {lint}: contract break ---\nFailed in:\n  {item}\n"


@pytest.mark.parametrize(
    ("file", "lint", "item", "name", "listed"),
    [
        ("cli.txt", "cli_option_removed", "latt validate --profile", "latt validate --profile", True),
        ("cli.txt", "cli_option_removed", "latt validate --profile", "--profile", False),
        ("cli.txt", "cli_option_removed", "latt validate --profile", "latt validate", False),
        ("id-patterns.tsv", "id_pattern_changed", "openspec.yaml requirement", "openspec.yaml requirement", True),
        ("id-patterns.tsv", "id_pattern_changed", "openspec.yaml requirement", "requirement", False),
        ("codes.tsv", "code_removed", "PARSE_ERROR", "PARSE_ERROR", True),
    ],
)
# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_contract_item_is_listed_verbatim(
    tmp_path: Path, file: str, lint: str, item: str, name: str, listed: bool
) -> None:
    result = check(tmp_path, breaking(f"`{lint}` changes `{name}`"), log=contract_log(file, lint, item))
    assert result.returncode == (0 if listed else 1), result.stdout + result.stderr


# The form verify-rust-tooling.sh writes for a package the baseline lacks (the 0.8.0
# crate rename): one failure naming the crate, never a skipped comparison.
MISSING_PACKAGE_LOG = (
    "== semver-checks: latt-core all-features\n"
    "--- failure package_missing_from_baseline: latt-core ---\n\n"
    "Failed in:\n  crate latt_core\n\n"
)


@pytest.mark.parametrize(
    ("name", "listed"), [("latt_core", True), ("lattice_core", False), ("latt-core", False)]
)
# Requirement: development-verification/Pre-1.0 embedding API compatibility is checked
def test_a_package_missing_from_the_baseline_must_be_listed(
    tmp_path: Path, name: str, listed: bool
) -> None:
    result = check(
        tmp_path,
        breaking(f"`package_missing_from_baseline`: `{name}`"),
        log=MISSING_PACKAGE_LOG,
    )
    assert result.returncode == (0 if listed else 1), result.stdout + result.stderr


def test_the_tooling_gate_refuses_an_unknown_flag() -> None:
    result = subprocess.run(
        ["sh", str(ROOT / "tools" / "verify-rust-tooling.sh"), "--no-api"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert "usage:" in result.stderr

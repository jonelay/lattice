"""Minimal TOML register adapter: registers, their items, and the references between them.

Domain-free on purpose. This is lattice's standing evidence that the adapter
contract and the profile schema carry a register that is not markdown, so it
reads a fixture rather than a live repository and names no vocabulary of its own.
"""
from __future__ import annotations

import tomllib
from pathlib import Path

from adapters._emit import DocumentBuilder
from adapters._profile import Profile
from adapters._read import adapter_paths
from adapters._types import AxisError, Issue, Provenance, Severity

# Top-level tables this adapter reads. Any other array of tables is reported
# rather than skipped, so a new row shape cannot shrink the graph in silence.
_REGISTER_KEY = "register"
_ITEM_KEY = "item"


def _error(graph: DocumentBuilder, code: str, message: str, path: Path,
           severity: Severity = Severity.ERROR, node_id: str | None = None) -> None:
    graph.add_issue(
        Issue(severity, code, message, Provenance(str(path), 0), node_id=node_id)
    )


def _resolve_config(profile: Profile, target_path: Path) -> dict:
    """Read the adapter section from the profile and resolve its paths.

    Raises rather than reporting: a profile that cannot say where the register
    lives is a broken setup, which is exit code 2 and not a finding.
    """
    paths = adapter_paths(profile, "adapter.paths.register_dir")
    if not isinstance(paths.get("register_dir"), str):
        raise ValueError(
            "profile 'adapter' section has no string adapter.paths.register_dir"
        )
    axis_file = paths.get("axis_file")
    return {
        "register_dir": target_path / paths["register_dir"],
        "axis_file": target_path / axis_file if isinstance(axis_file, str) else None,
    }


def _load(graph: DocumentBuilder, path: Path) -> dict | None:
    """Decode one register file, reporting a decode failure instead of raising."""
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeDecodeError, OSError) as e:
        _error(graph, "PARSE_ERROR", f"{path.name}: {e}", path)
        return None


def _attach_stage_axis(graph: DocumentBuilder, path: Path | None) -> None:
    """Attach the register's declared stage axis; report a declaration that fails.

    The register owns both values. A target declaring neither has no axis and
    that is not a finding; a target declaring one, or an unusable pair, is
    making a claim about itself that does not hold.
    """
    if path is None or not path.is_file():
        return
    data = _load(graph, path)
    if data is None:
        return
    register = data.get(_REGISTER_KEY)
    if not isinstance(register, dict):
        return

    current = register.get("current_stage")
    order = register.get("stage_order")
    if current is None and order is None:
        return

    if current is None or order is None:
        missing = "stage_order" if order is None else "current_stage"
        _error(
            graph, "AXIS_INVALID",
            f"{path.name}: stage axis declares only one of the pair, "
            f"missing '{missing}'",
            path, severity=Severity.WARNING,
        )
        return

    if not isinstance(current, str) or not isinstance(order, list) or not all(
        isinstance(x, str) for x in order
    ):
        _error(
            graph, "AXIS_INVALID",
            f"{path.name}: stage axis expects current_stage to be a string and "
            f"stage_order a list of strings",
            path, severity=Severity.WARNING,
        )
        return

    try:
        graph.set_axis("stage", order, current)
    except AxisError as e:
        _error(
            graph, "AXIS_INVALID", f"{path.name}: {e}",
            path, severity=Severity.WARNING,
        )


def _read_register(graph: DocumentBuilder, data: dict, path: Path) -> bool:
    """Emit the file's register node; False when it has no identity to emit one from."""
    register = data.get(_REGISTER_KEY)
    if not isinstance(register, dict):
        _error(
            graph, "PARSE_ERROR",
            f"{path.name}: no [register] table, so the file has no identity "
            f"its items could be qualified by",
            path,
        )
        return False

    attrs = {
        key: register[key]
        for key in ("id", "register_version", "status")
        if key in register
    }
    graph.add_node(path.stem, "register", attrs, Provenance(str(path), 0))
    return True


def _read_items(graph: DocumentBuilder, data: dict, path: Path) -> None:
    """Emit item nodes and their edges, register-qualifying every ID."""
    rows = data.get(_ITEM_KEY, [])
    if not isinstance(rows, list):
        _error(
            graph, "PARSE_ERROR",
            f"{path.name}: '{_ITEM_KEY}' is {type(rows).__name__}, not an array "
            f"of tables",
            path,
        )
        return

    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            _error(
                graph, "PARSE_ERROR",
                f"{path.name}: item row {index} is {type(row).__name__}, not a table",
                path,
            )
            continue

        raw_id = row.get("id")
        if not isinstance(raw_id, str):
            _error(
                graph, "PARSE_ERROR",
                f"{path.name}: item row {index} declares no string 'id', so it "
                f"has nothing to be named by",
                path,
            )
            continue

        node_id = f"{path.stem}/{raw_id}"
        attrs: dict = {"id": raw_id}
        stage = row.get("stage")
        if isinstance(stage, str):
            attrs["stage"] = stage
        graph.add_node(node_id, "item", attrs, Provenance(str(path), 0))

        _read_register_ref(graph, row, node_id, path)
        _read_refs(graph, row, node_id, path)


def _read_register_ref(
    graph: DocumentBuilder, row: dict, node_id: str, path: Path
) -> None:
    """Emit the item's `belongs_to` edge, reporting a key of the wrong shape."""
    target = row.get(_REGISTER_KEY)
    if target is None:
        return
    if not isinstance(target, str):
        _error(
            graph, "PARSE_ERROR",
            f"{path.name}: item '{node_id}' has '{_REGISTER_KEY}' of type "
            f"{type(target).__name__}, expected a string",
            path, node_id=node_id,
        )
        return
    graph.add_edge(node_id, target, "belongs_to", Provenance(str(path), 0))


def _read_refs(graph: DocumentBuilder, row: dict, node_id: str, path: Path) -> None:
    """Emit the item's `references` edges without checking that they resolve.

    Whether an endpoint exists is DANGLING_REF's question at validation. An
    adapter that dropped an unresolvable edge would answer it by hiding it.
    """
    refs = row.get("refs")
    if refs is None:
        return
    if not isinstance(refs, list) or not all(isinstance(r, str) for r in refs):
        shape = type(refs).__name__
        _error(
            graph, "PARSE_ERROR",
            f"{path.name}: item '{node_id}' has 'refs' of type {shape}, "
            f"expected a list of strings",
            path, node_id=node_id,
        )
        return
    for target in refs:
        graph.add_edge(node_id, target, "references", Provenance(str(path), 0))


def _report_unread(graph: DocumentBuilder, data: dict, path: Path) -> None:
    """Report a row-bearing table this adapter has no reader for."""
    for key, value in data.items():
        if key in (_REGISTER_KEY, _ITEM_KEY):
            continue
        if isinstance(value, list) and value and all(
            isinstance(item, dict) for item in value
        ):
            _error(
                graph, "PARSE_ERROR",
                f"{path.name}: no reader for row-bearing table '{key}' "
                f"({len(value)} rows not read)",
                path,
            )


def build_graph(profile: Profile, target_path: Path) -> DocumentBuilder:
    """Read a directory of TOML registers into a contract document.

    Every file is read whatever the others do: an unreadable one becomes an
    issue and the rest still reach the graph, because a register the adapter
    stayed silent about reads as a register with nothing in it.
    """
    config = _resolve_config(profile, target_path)
    graph = DocumentBuilder()

    _attach_stage_axis(graph, config["axis_file"])

    register_dir = config["register_dir"]
    if not register_dir.is_dir():
        _error(
            graph, "PARSE_ERROR",
            f"register directory '{register_dir}' does not exist",
            register_dir,
        )
        return graph

    for path in sorted(register_dir.glob("*.toml")):
        data = _load(graph, path)
        if data is None:
            continue
        if not _read_register(graph, data, path):
            continue
        _read_items(graph, data, path)
        _report_unread(graph, data, path)

    return graph


if __name__ == "__main__":
    from adapters._emit import run_main

    raise SystemExit(run_main(build_graph))

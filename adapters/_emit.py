"""The writing half of the adapter interface, for the Python openspec adapter.

The reading half is the core's `document.rs`. They are deliberately separate
implementations of one format; that independence is what the contract is for.
A change to the document shape touches both.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from adapters._profile import load_profile
from adapters._types import Issue, Provenance

INTERFACE_VERSION = "1.4"


class DocumentBuilder:
    """What an adapter builds into: a document under construction.

    Not a `LatticeGraph`. `add_node` there rejects a repeated ID, which is
    correct for the core but wrong here. The core resolves duplicates at
    ingest, and it can only do that if both occurrences reach it. A builder
    that deduplicated would drop the second in silence, which is the one thing
    an adapter must never do.
    """

    def __init__(self) -> None:
        self._nodes: list[tuple[str, dict]] = []
        self._edges: list[tuple[str, str, str, dict]] = []
        self._issues: list[Issue] = []
        self._ids: set[str] = set()

    def add_node(self, id: str, kind: str, attrs: dict, provenance: Provenance) -> int:
        """Append a node entry and return its index, for `add_edge`'s `declared_by`.

        Keep the index from this call: a lookup by ID would find the first
        occurrence and pin a repeat's edges on the entry ingest keeps.
        """
        self._nodes.append(
            (id, {"kind": kind, "attrs": attrs, "provenance": provenance})
        )
        self._ids.add(id)
        return len(self._nodes) - 1

    def add_edge(self, src: str, tgt: str, kind: str, provenance: Provenance,
                 declared_by: int | None = None) -> None:
        """Append an edge; `declared_by` is the index of the node entry whose
        row declared it, so ingest drops it with that entry if the entry repeats
        an ID."""
        data: dict = {"provenance": provenance}
        if declared_by is not None:
            data["declared_by"] = declared_by
        self._edges.append((src, tgt, kind, data))

    def add_issue(self, issue: Issue) -> None:
        self._issues.append(issue)

    def has_node(self, id: str) -> bool:
        """True when this ID was already added.

        An adapter uses this to add a node once for something it meets many
        times, not to suppress a duplicate the register really declares twice.
        """
        return id in self._ids

    def node_data(self, id: str) -> dict:
        """The first occurrence of `id`, which is the one ingest will keep as the node."""
        for nid, data in self._nodes:
            if nid == id:
                return dict(data)
        raise KeyError(id)

    @property
    def adapter_issues(self) -> list[Issue]:
        return list(self._issues)

    def iter_nodes(self):
        yield from self._nodes

    def iter_edges(self):
        for src, tgt, kind, data in self._edges:
            yield src, tgt, kind, data


def _provenance(prov: Provenance, root: Path) -> dict:
    """Render one provenance, naming its file relative to the target root.

    An absolute path records where this checkout happens to sit, so two runs of
    the same commit under different paths would disagree byte for byte. A path
    outside the target, or a placeholder like `<profile>`, is left as it is,
    because relativising it would say something untrue about where it came from.
    """
    file = prov.file
    candidate = Path(file)
    if candidate.is_absolute() and candidate.is_relative_to(root):
        file = candidate.relative_to(root).as_posix()
    return {"file": file, "line": prov.line}


def serialize_graph(graph: DocumentBuilder, root: Path) -> dict:
    """Render a built graph as an interface document, rooted at the target.

    Nodes, edges and issues keep the graph's own order: node order is what
    decides the surviving duplicate on ingest, and issue order is the adapter's
    account of the register in the sequence it read them.
    """
    root = Path(root).resolve()
    return {
        "interface_version": INTERFACE_VERSION,
        "nodes": [
            {
                "id": nid,
                "kind": data["kind"],
                "attrs": data["attrs"],
                "provenance": _provenance(data["provenance"], root),
            }
            for nid, data in graph.iter_nodes()
        ],
        "edges": [
            {
                "src": src,
                "tgt": tgt,
                "kind": kind,
                "provenance": _provenance(data["provenance"], root),
                **({"declared_by": data["declared_by"]}
                   if "declared_by" in data else {}),
            }
            for src, tgt, kind, data in graph.iter_edges()
        ],
        # No Python adapter reads a pathway. The key is optional in the
        # interface document; emitting it empty keeps every document one shape.
        "pathways": [],
        "findings": [
            {
                "severity": i.severity.value,
                "code": i.code,
                "message": i.message,
                "provenance": _provenance(i.provenance, root),
                "node_id": i.node_id,
            }
            for i in graph.adapter_issues
        ],
    }


def run_main(build_graph, argv: list[str] | None = None) -> int:
    """Run an adapter as an interface program: paths in, document on stdout.

    Exits non-zero only when the adapter could not run at all. A register it
    could not read is a document of issues and still exits 0, because the core
    reads a non-zero exit as a broken setup rather than as a finding.
    """
    parser = argparse.ArgumentParser(description="Lattice adapter program.")
    parser.add_argument("--profile", required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args(argv)

    try:
        profile = load_profile(args.profile)
    except Exception as e:
        print(f"Error: could not load profile {args.profile}: {e}", file=sys.stderr)
        return 2

    # Resolved once here, so an adapter builds every provenance from an absolute
    # path and the target-relative rendering does not depend on whether the
    # caller spelled `--target` absolutely or relatively.
    target = Path(args.target).resolve()
    # A profile that cannot say where the register lives raises ValueError
    # (`_read.py`); that is a broken setup, so one line and exit 2, not a
    # traceback. The user's own `--profile` path is unknown here: the core
    # hands an adapter a scratch copy, so the message names the key, not a file.
    try:
        graph = build_graph(profile, target)
    except ValueError as e:
        print(f"Error: adapter configuration: {e}", file=sys.stderr)
        return 2
    json.dump(serialize_graph(graph, target), sys.stdout)
    sys.stdout.write("\n")
    return 0

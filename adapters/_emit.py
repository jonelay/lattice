"""The writing half of the adapter interface, shared by both adapters.

The reading half is the core's `document.rs`. They are deliberately separate
implementations of one format — that independence is what the contract is for —
so a change to the document shape touches both.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from adapters._profile import load_profile
from adapters._types import Pathway, PathwayError, Issue, Provenance

INTERFACE_VERSION = "1.2"


class DocumentBuilder:
    """What an adapter builds into: a document under construction.

    Not a `LatticeGraph`. `add_node` there rejects a repeated ID, which is
    correct for the core but wrong here — the core resolves duplicates at
    ingest now, and it can only do that if both occurrences reach it. A builder
    that deduplicated would drop the second in silence, which is the one thing
    an adapter must never do.
    """

    def __init__(self) -> None:
        self._nodes: list[tuple[str, dict]] = []
        self._edges: list[tuple[str, str, str, dict]] = []
        self._issues: list[Issue] = []
        self._pathways: dict[str, Pathway] = {}
        self._ids: set[str] = set()

    def add_node(self, id: str, kind: str, attrs: dict, provenance: Provenance) -> None:
        self._nodes.append(
            (id, {"kind": kind, "attrs": attrs, "provenance": provenance})
        )
        self._ids.add(id)

    def add_edge(self, src: str, tgt: str, kind: str, provenance: Provenance) -> None:
        self._edges.append((src, tgt, kind, {"provenance": provenance}))

    def add_issue(self, issue: Issue) -> None:
        self._issues.append(issue)

    def has_node(self, id: str) -> bool:
        """True when this ID was already added.

        An adapter uses this to add a node once for something it meets many
        times, not to suppress a duplicate the register really declares twice.
        """
        return id in self._ids

    def node_data(self, id: str) -> dict:
        """The first occurrence of `id` — the one ingest will keep as the node."""
        for nid, data in self._nodes:
            if nid == id:
                return dict(data)
        raise KeyError(id)

    def set_pathway(self, name: str, order: list[str], current: str) -> None:
        """Attach a pathway, raising `PathwayError` if it is not internally valid.

        Validation is the adapter's, per the adapter-contract capability: it
        read the declaration and can name the file. The core refuses an invalid
        pathway outright rather than reporting it.
        """
        if len(set(order)) != len(order):
            raise PathwayError(f"pathway '{name}': positions are not unique: {order}")
        if current not in order:
            raise PathwayError(
                f"pathway '{name}': current position '{current}' is not in {order}"
            )
        self._pathways[name] = Pathway(name=name, order=list(order), current=current)

    def pathway(self, name: str) -> Pathway | None:
        return self._pathways.get(name)

    @property
    def adapter_issues(self) -> list[Issue]:
        return list(self._issues)

    def iter_nodes(self):
        yield from self._nodes

    def iter_edges(self):
        for src, tgt, kind, data in self._edges:
            yield src, tgt, kind, data

    def iter_pathways(self):
        yield from self._pathways.values()


def _provenance(prov: Provenance, root: Path) -> dict:
    """Render one provenance, naming its file relative to the target root.

    An absolute path records where this checkout happens to sit, so two runs of
    the same commit under different paths would disagree byte for byte. A path
    outside the target — or a placeholder like `<profile>` — is left as it is,
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
            }
            for src, tgt, kind, data in graph.iter_edges()
        ],
        "pathways": [
            {"name": a.name, "order": list(a.order), "current": a.current}
            for a in graph.iter_pathways()
        ],
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
    graph = build_graph(profile, target)
    json.dump(serialize_graph(graph, target), sys.stdout)
    sys.stdout.write("\n")
    return 0

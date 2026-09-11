"""Rank candidate edges the register does not yet declare.

A suggestion producer, not a decider. It reads an interface document and the
core-resolved profile, ranks candidate pairings by similarity of the text each
node carries, and writes a suggestion document to stdout. The core renders that
document as hints; a human decides whether any of it becomes a marker edit.

Nothing here writes to the register, and the document it emits is scratch: the
durable artifact of an accepted suggestion is a reviewed diff, never this file.

Stdlib only, like the adapters. The default backend runs offline, so a plain
run needs no network, no key and no GPU.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

from adapters._profile import Profile, ProfileError, load_profile

SUGGESTION_VERSION = "1.0"
PRODUCER = "lattice-suggest 0.2.0"


class SuggestError(Exception):
    """The sidecar could not run at all, distinct from finding nothing."""


def _texts(node: dict, profile: Profile) -> list[str]:
    """The text a node offers a ranker: the attrs its kind declares as text.

    The profile names them, in order, so a register carrying text in a second
    place needs a profile edit and no code change here. This program names
    no attr of its own, which is the same reason node kinds and edge kinds are
    profile data rather than literals.

    A kind declaring no `text_attrs` falls back to its `summary_attr`, so a
    profile written before the key existed ranks as it always did. A kind
    declaring an empty list does not fall back: that is a statement that there
    is nothing to rank, and the textless report is the honest answer.
    """
    kind = profile.node_kinds.get(node["kind"])
    if kind is None:
        return []
    names = kind.text_attrs
    if names is None:
        names = (kind.summary_attr,) if kind.summary_attr else ()

    attrs = node.get("attrs") or {}
    out = []
    for attr in names:
        value = attrs.get(attr)
        if isinstance(value, str) and value.strip():
            out.append(value)
    return out


def _chunks(text: str, prefix: str) -> list[str]:
    """Partition *text* before each line starting with *prefix*.

    A partition, never an extraction: `"".join` of the parts reproduces the
    input, separators included, save for any part that is entirely whitespace.
    Text a splitter drops is invisible in a ranked list. Nothing downstream can
    tell a missing region from one that scored low, so a part carrying any
    text at all is always kept.

    Each part but the last keeps its trailing newline, because the cut falls
    after the separator rather than before the marker. That is not cosmetic:
    the parts are embedded verbatim, so a newline moved between them would
    change the vectors.

    Splits on `\\n` alone rather than `str.splitlines`, which also cuts on form
    feed, vertical tab and `\\u2028`; those would divide a body at a character
    the profile never named.
    """
    raw = text.split("\n")
    lines = [line + "\n" for line in raw[:-1]] + raw[-1:]
    cuts = [i for i, line in enumerate(lines) if line.startswith(prefix)]
    if not cuts:
        return [text]
    bounds = sorted({0, *cuts, len(lines)})
    parts = ["".join(lines[a:b]) for a, b in zip(bounds, bounds[1:])]
    # A target whose every part is blank still has to be rankable as something.
    return [p for p in parts if p.strip()] or [text]


def _first_occurrences(nodes: list[dict]) -> list[dict]:
    """One node per ID, the first declared. This is the occurrence the core keeps."""
    seen: dict[str, dict] = {}
    for n in nodes:
        seen.setdefault(n["id"], n)
    return list(seen.values())


def _target_chunks(node: dict, profile: Profile) -> list[str]:
    """The texts a target offers a ranker: one, or several if its kind declares
    a split marker."""
    texts = _texts(node, profile)
    if not texts:
        return []
    whole = " \n".join(texts)
    kind = profile.node_kinds.get(node["kind"])
    prefix = kind.text_chunk_line_prefix if kind else None
    return _chunks(whole, prefix) if prefix else [whole]


def _report_chunking(
    targets: list[dict], chunks_by_id: dict[str, list[str]], profile: Profile
) -> None:
    """Report how the declared markers actually divided the targets.

    A declared marker that splits nothing is a profile typo or an adapter that
    stopped emitting the text it names, and the ranking it produces is the
    unchunked one, indistinguishable on stdout from a register with no blocks.
    The chunk total is printed for the same reason a finding count is read
    beside an edge count: it is what shows the feature is on.
    """
    counts: dict[str, list[int]] = {}
    for node in targets:
        kind = profile.node_kinds.get(node["kind"])
        if kind is None or not kind.text_chunk_line_prefix:
            continue
        counts.setdefault(node["kind"], []).append(len(chunks_by_id[node["id"]]))
    for kind_name, per_target in sorted(counts.items()):
        prefix = profile.node_kinds[kind_name].text_chunk_line_prefix
        if max(per_target) == 1:
            print(
                f"lattice-suggest: '{kind_name}' declares text_chunk_line_prefix "
                f"{prefix!r} but no target divided at it; ranking "
                f"{len(per_target)} target(s) undivided",
                file=sys.stderr,
            )
        else:
            print(
                f"lattice-suggest: split {len(per_target)} '{kind_name}' target(s) "
                f"into {sum(per_target)} chunk(s) at {prefix!r}",
                file=sys.stderr,
            )


def candidates(
    document: dict, profile: Profile, edge_kind: str
) -> tuple[list[dict], list[dict]]:
    """The nodes to rank and the nodes to rank them against, for *edge_kind*.

    Sources are nodes of a legal source kind carrying no outgoing edge of this
    kind, the population the register has not attributed. Targets are every
    node of a legal target kind. Pairings the profile does not allow are never
    proposed: the core would reject one as an EDGE_CONSTRAINT violation, so
    offering it spends a reviewer's attention on an edit that cannot land.
    """
    kind = profile.edge_kinds.get(edge_kind)
    if kind is None:
        raise SuggestError(
            f"edge kind {edge_kind!r} is not declared in the profile; "
            f"declared: {', '.join(sorted(profile.edge_kinds)) or '(none)'}"
        )
    if not kind.allowed:
        raise SuggestError(f"edge kind {edge_kind!r} declares no allowed pairings")

    src_kinds = {src for src, _ in kind.allowed}
    tgt_kinds = {tgt for _, tgt in kind.allowed}
    attributed = {
        edge["src"] for edge in document.get("edges") or [] if edge["kind"] == edge_kind
    }
    nodes = document.get("nodes") or []

    sources = [
        n for n in nodes if n["kind"] in src_kinds and n["id"] not in attributed
    ]
    targets = [n for n in nodes if n["kind"] in tgt_kinds]
    return sources, targets


def _cosine(a: list[float], b: list[float]) -> float:
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(y * y for y in b))
    if na == 0.0 or nb == 0.0:
        return 0.0
    return dot / (na * nb)


def rank(
    sources: list[dict],
    targets: list[dict],
    profile: Profile,
    edge_kind: str,
    embed,
    model: str,
    top_k: int,
) -> list[dict]:
    """Score every legal pairing and keep each source's *top_k* targets.

    Every legal pairing, with no pre-filter. At this register's size the full
    product is cheap, and a pre-filter would silently bound recall. That is
    the one failure a ranked review list must not have, since a candidate it
    never surfaces is one no reviewer can rescue.

    The one exclusion is a node offering no text at all: the baseline is
    similarity over declared text, so a textless node has no defined input and
    any score for it would be fabricated. A textless register once produced
    thousands of suggestions all at cosine 1.0, in silence. Excluded nodes are
    reported on stderr, before the embed call, so an all-textless population
    never needs a backend.
    """
    allowed = set(profile.edge_kinds[edge_kind].allowed)
    # A duplicated ID ranks once, over the union of its occurrences' text.
    # The interface document retains every occurrence and the core reports the defect;
    # here, keying by ID must not silently keep one occurrence. A matching
    # occurrence never scored is a candidate no reviewer can rescue.
    src_pieces: dict[str, list[str]] = {}
    for n in sources:
        src_pieces.setdefault(n["id"], []).extend(_texts(n, profile))
    source_texts = {i: " \n".join(p) for i, p in src_pieces.items()}
    # Targets only: a kind can be a target of one edge kind and a source of
    # another, so chunking belongs to the ranked role, not to the kind.
    target_chunks: dict[str, list[str]] = {}
    for n in targets:
        target_chunks.setdefault(n["id"], []).extend(_target_chunks(n, profile))

    merged = (len(sources) - len(src_pieces)) + (len(targets) - len(target_chunks))
    if merged:
        print(
            f"lattice-suggest: merged {merged} duplicate occurrence(s) under "
            "their first-declared ids for ranking",
            file=sys.stderr,
        )
    sources = _first_occurrences(sources)
    targets = _first_occurrences(targets)

    empty_src = {i for i, t in source_texts.items() if not t}
    empty_tgt = {i for i, c in target_chunks.items() if not c}
    if empty_src or empty_tgt:
        print(
            f"lattice-suggest: excluded {len(empty_src)} source(s) and "
            f"{len(empty_tgt)} target(s) carrying no text to rank on",
            file=sys.stderr,
        )
        sources = [n for n in sources if n["id"] not in empty_src]
        targets = [n for n in targets if n["id"] not in empty_tgt]
        source_texts = {i: t for i, t in source_texts.items() if i not in empty_src}
        target_chunks = {i: c for i, c in target_chunks.items() if i not in empty_tgt}
    if not source_texts or not target_chunks:
        print(
            "lattice-suggest: nothing to rank; emitting zero suggestions",
            file=sys.stderr,
        )
        return []

    _report_chunking(targets, target_chunks, profile)

    # Sources first, then each target's chunks contiguously, so a profile
    # declaring no marker hands the backend the batch it always did.
    flat = list(source_texts.values())
    spans: dict[str, tuple[int, int]] = {}
    for target_id, chunks in target_chunks.items():
        spans[target_id] = (len(flat), len(flat) + len(chunks))
        flat.extend(chunks)

    vectors = embed(flat)
    by_id = dict(zip(source_texts, vectors[: len(source_texts)]))

    out = []
    for src in sources:
        scored = []
        for tgt in targets:
            if (src["kind"], tgt["kind"]) not in allowed or src["id"] == tgt["id"]:
                continue
            lo, hi = spans[tgt["id"]]
            # Max, not mean: averaging the per-chunk scores puts every
            # irrelevant block back into the number, which measured below not
            # chunking at all.
            score = max(_cosine(by_id[src["id"]], v) for v in vectors[lo:hi])
            scored.append((score, tgt["id"]))
        scored.sort(key=lambda pair: (-pair[0], pair[1]))
        for score, tgt_id in scored[:top_k]:
            out.append(
                {
                    "src": src["id"],
                    "tgt": tgt_id,
                    "kind": edge_kind,
                    "score": round(score, 6),
                    "basis": f"cosine {score:.4f}; model {model}",
                }
            )

    # Descending score, ties broken on identity, so the document is
    # byte-reproducible for a given backend and input.
    out.sort(key=lambda s: (-s["score"], s["src"], s["tgt"], s["kind"]))
    return out


def offline_backend(texts: list[str]) -> list[list[float]]:
    """The default: no backend, so no vectors and no network."""
    raise SuggestError(
        "no embedding backend configured; pass --backend ollama --model <name> "
        "to rank, or expect a document with zero suggestions"
    )


def ollama_backend(host: str, model: str):
    """An embedding backend over ollama's HTTP API, via urllib. No new dependency."""

    def embed(texts: list[str]) -> list[list[float]]:
        import urllib.error
        import urllib.request

        request = urllib.request.Request(
            f"{host.rstrip('/')}/api/embed",
            data=json.dumps({"model": model, "input": texts}).encode(),
            headers={"Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=300) as response:
                payload = json.loads(response.read())
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            raise SuggestError(f"embedding backend {host} unreachable: {e}") from e
        embeddings = payload.get("embeddings")
        if not isinstance(embeddings, list) or len(embeddings) != len(texts):
            raise SuggestError(
                f"embedding backend returned {len(embeddings or [])} vectors "
                f"for {len(texts)} texts"
            )
        return embeddings

    return embed


def build_document(suggestions: list[dict]) -> dict:
    return {
        "suggestion_version": SUGGESTION_VERSION,
        "producer": PRODUCER,
        "suggestions": suggestions,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="lattice-suggest",
        description="Rank candidate edges a register does not yet declare.",
    )
    parser.add_argument("--profile", required=True, help="resolved profile document")
    parser.add_argument(
        "--interface", default="-", help="interface document, or - for stdin"
    )
    parser.add_argument("--edge-kind", required=True)
    parser.add_argument("--backend", default="none", choices=["none", "ollama"])
    parser.add_argument("--model", default="nomic-embed-text")
    parser.add_argument("--host", default="http://192.168.1.202:11434")
    parser.add_argument("--top-k", type=int, default=5)
    args = parser.parse_args(argv)

    try:
        profile = load_profile(args.profile)
        text = (
            sys.stdin.read()
            if args.interface == "-"
            else Path(args.interface).read_text(encoding="utf-8")
        )
        document = json.loads(text)
        sources, targets = candidates(document, profile, args.edge_kind)

        if args.backend == "none":
            # Reported, never silent: a document of zero suggestions because no
            # backend ran must not read like a backend that found nothing.
            print(
                f"lattice-suggest: no backend configured; "
                f"{len(sources)} unattributed {args.edge_kind} source(s) went unranked",
                file=sys.stderr,
            )
            suggestions = []
        else:
            print(
                f"lattice-suggest: ranking {len(sources)} source(s) "
                f"against {len(targets)} target(s)",
                file=sys.stderr,
            )
            embed = ollama_backend(args.host, args.model)
            suggestions = rank(
                sources, targets, profile, args.edge_kind, embed,
                args.model, args.top_k,
            )
    except (ProfileError, SuggestError) as e:
        print(f"lattice-suggest: {e}", file=sys.stderr)
        return 2
    except (OSError, json.JSONDecodeError) as e:
        print(f"lattice-suggest: could not read the interface document: {e}", file=sys.stderr)
        return 2

    json.dump(build_document(suggestions), sys.stdout)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

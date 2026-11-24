#!/usr/bin/env python3
"""Template Lattice adapter: reads line registers and prints an interface document.

Usage: adapter.py --profile <resolved-profile.json> --target <dir>

The register is the files in one directory whose names end in one extension, each line a
declaration:

    <kind> <id> [<edge-kind> <target-id>]... : <text>

Copy this file and change `read_line` for your own format. The contract it follows is
docs/adapters.md, "Writing your own adapter". Standard library only, Python 3.10 or later.
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any, NoReturn

INTERFACE_VERSION = "1.4"
# Keys of the profile's `adapter:` block that belong to the core; an adapter accepts them.
CORE_KEYS = {"native", "timeout_seconds", "command_timeout_seconds"}
DECLARATION = '"<kind> <id> [<edge-kind> <target-id>]... : <text>"'


def fail(message: str) -> NoReturn:
    """Exit 2 with no document: the core then reports the adapter failed to run."""
    print(f"lines-adapter: {message}", file=sys.stderr)
    sys.exit(2)


class Document:
    """The interface document, built in source order."""

    def __init__(self) -> None:
        self.nodes: list[dict[str, Any]] = []
        self.edges: list[dict[str, Any]] = []
        self.findings: list[dict[str, Any]] = []

    def add_node(self, node_id: str, kind: str, text: str, file: str, line: int) -> int:
        """Append a node and return its index, which its edges carry as `declared_by`."""
        self.nodes.append({
            "id": node_id, "kind": kind, "attrs": {"text": text},
            "provenance": {"file": file, "line": line},
        })
        return len(self.nodes) - 1

    def add_edge(self, src: str, tgt: str, kind: str, file: str, line: int, declared_by: int) -> None:
        self.edges.append({
            "src": src, "tgt": tgt, "kind": kind,
            "provenance": {"file": file, "line": line}, "declared_by": declared_by,
        })

    def finding(self, code: str, message: str, file: str, line: int, severity: str = "error") -> None:
        """Report input the adapter could not read; never skip it in silence."""
        self.findings.append({
            "severity": severity, "code": code, "message": message,
            "provenance": {"file": file, "line": line}, "node_id": None,
        })

    def to_json(self) -> dict[str, Any]:
        return {
            "interface_version": INTERFACE_VERSION,
            "nodes": self.nodes, "edges": self.edges, "pathways": [], "findings": self.findings,
        }


def parse_args(argv: list[str]) -> tuple[str, str]:
    values: dict[str, str] = {}
    rest = list(argv)
    while rest:
        flag = rest.pop(0)
        if flag not in ("--profile", "--target") or not rest:
            fail(f"usage: --profile <resolved-profile.json> --target <dir>; unexpected {flag!r}")
        values[flag] = rest.pop(0)
    if set(values) != {"--profile", "--target"}:
        fail("usage: --profile <resolved-profile.json> --target <dir>")
    return values["--profile"], values["--target"]


def load_config(path: str) -> tuple[str, str]:
    """Return (directory, extension) from the resolved profile's `adapter:` block."""
    try:
        with open(path, "rb") as handle:
            profile = json.loads(handle.read())
    except (OSError, ValueError) as error:
        fail(f"cannot read profile {path}: {error}")
    if not isinstance(profile, dict) or profile.get("resolved_schema") != "1":
        fail(f"{path} is not a resolved profile; pass the file `latt show` prints")
    adapter = profile.get("adapter")
    if not isinstance(adapter, dict):
        fail(f"{path}: the adapter block is missing")
    for key in adapter:
        if key not in CORE_KEYS | {"directory", "extension"}:
            fail(f"{path}: unknown adapter key {key!r}")
    for key in ("directory", "extension"):
        if not isinstance(adapter.get(key), str):
            fail(f"{path}: adapter key {key!r} must be a string")
    return adapter["directory"], adapter["extension"]


def read_line(document: Document, raw: bytes, file: str, number: int) -> None:
    """Read one line of a register file into the document."""
    try:
        line = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        document.finding(
            "PARSE_ERROR", f"line is not valid UTF-8 at byte {error.start + 1}, so it was not read",
            file, number,
        )
        return
    if not line.strip() or line.lstrip().startswith("#"):
        return
    head, colon, text = line.partition(":")
    words = head.split()
    if not colon or len(words) < 2 or len(words) % 2:
        document.finding("PARSE_ERROR", f"line is not a declaration {DECLARATION}, so it was not read", file, number)
        return
    kind, node_id = words[0], words[1]
    index = document.add_node(node_id, kind, text.strip(), file, number)
    for edge_kind, target_id in zip(words[2::2], words[3::2]):
        document.add_edge(node_id, target_id, edge_kind, file, number, index)


def read_file(document: Document, path: str, file: str) -> None:
    try:
        with open(path, "rb") as handle:
            data = handle.read()
    except OSError as error:
        document.finding("PARSE_ERROR", f"file could not be read: {error.strerror}", file, 0)
        return
    if data.startswith(b"\xef\xbb\xbf"):
        data = data[3:]
    for number, raw in enumerate(data.split(b"\n"), start=1):
        read_line(document, raw.removesuffix(b"\r"), file, number)


def build(target: str, directory: str, extension: str) -> Document:
    document = Document()
    root = os.path.join(target, directory)
    try:
        names = sorted(os.listdir(os.fsencode(root)))
    except OSError:
        document.finding("PARSE_ERROR", f'directory "{directory}" does not exist or is not a directory', directory, 0)
        return document
    selected = 0
    for raw_name in names:
        try:
            name = raw_name.decode("utf-8")
        except UnicodeDecodeError:
            document.finding(
                "PARSE_ERROR", "a file name in this directory is not valid UTF-8, so it was not read", directory, 0,
            )
            continue
        if not name.endswith(extension):
            continue
        selected += 1
        file = f"{directory}/{name}"
        path = os.path.join(root, name)
        if not os.path.isfile(path) or os.path.islink(path):
            document.finding("PARSE_ERROR", "not a regular file, so it was not read", file, 0)
            continue
        read_file(document, path, file)
    if not selected:
        document.finding("NO_MATCHING_FILES", f'no file in "{directory}" ends in "{extension}"', directory, 0, "info")
    return document


def main() -> None:
    profile, target = parse_args(sys.argv[1:])
    directory, extension = load_config(profile)
    # The document is UTF-8 whatever the locale says.
    text = json.dumps(build(target, directory, extension).to_json(), ensure_ascii=False)
    sys.stdout.buffer.write(text.encode("utf-8") + b"\n")


if __name__ == "__main__":
    main()

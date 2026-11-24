#!/usr/bin/env python3
"""Generate the deterministic TOML chain used by end-to-end benchmarks."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def text_spec(manifest: dict[str, object]) -> tuple[int, str] | None:
    """Return the manifest's text size in bytes and its replacement, or None without text.

    The replacement takes the place of leading ASCII bytes of equal UTF-8 length, so every
    node's text stays exactly `bytes` long whatever the replacement is.
    """
    text = manifest.get("text")
    if text is None:
        return None
    if not isinstance(text, dict) or set(text) - {"bytes", "replace"}:
        raise SystemExit("text must be an object with bytes and an optional replace")
    size = text.get("bytes")
    if not isinstance(size, int) or size < 1:
        raise SystemExit("text.bytes must be a positive integer")
    replace = text.get("replace", "")
    if not isinstance(replace, str) or not all(c not in '"\\' and c.isprintable() for c in replace):
        raise SystemExit("text.replace must be a string of printable characters other than quotes and backslashes")
    if len(replace.encode("utf-8")) > size:
        raise SystemExit("text.replace is longer than text.bytes")
    return size, replace


def node_text(index: int, size: int, replace: str) -> str:
    seed = f"text of node n{index:05d}; "
    ascii_text = (seed * (size // len(seed) + 1))[:size]
    return replace + ascii_text[len(replace.encode("utf-8")) :]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text())
    if manifest.get("generator") != "deterministic-forward-chain-v1":
        raise SystemExit("unsupported end-to-end fixture generator")
    count = manifest.get("node_count")
    if not isinstance(count, int) or not 2 <= count <= 99_999:
        raise SystemExit("node_count must be an integer from 2 through 99999")
    if manifest.get("edge_count") != count - 1:
        raise SystemExit("edge_count must equal node_count - 1")
    text = text_spec(manifest)

    args.output.mkdir(parents=True, exist_ok=True)
    target = args.output / "register.toml"
    with target.open("w", encoding="utf-8", newline="\n") as stream:
        for index in range(count):
            stream.write("[[item]]\n")
            stream.write(f'id = "n{index:05d}"\n')
            if text is not None:
                stream.write(f'text = "{node_text(index, *text)}"\n')
            if index + 1 < count:
                stream.write(f'refs = "n{index + 1:05d}"\n')
            stream.write("\n")


if __name__ == "__main__":
    main()

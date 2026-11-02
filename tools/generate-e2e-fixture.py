#!/usr/bin/env python3
"""Generate the deterministic TOML chain used by end-to-end benchmarks."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


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

    args.output.mkdir(parents=True, exist_ok=True)
    target = args.output / "register.toml"
    with target.open("w", encoding="utf-8", newline="\n") as stream:
        for index in range(count):
            stream.write("[[item]]\n")
            stream.write(f'id = "n{index:05d}"\n')
            if index + 1 < count:
                stream.write(f'refs = "n{index + 1:05d}"\n')
            stream.write("\n")


if __name__ == "__main__":
    main()

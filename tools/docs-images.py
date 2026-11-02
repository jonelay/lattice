#!/usr/bin/env python3
"""Regenerate the terminal images under docs/img/ from a live `latt browse`.

Each scene drives `browse` on Lattice's own register, as the README's quick start reads
it, in a detached tmux session, sends its keys, and captures the screen with its colours
(`tmux capture-pane -e`). A still becomes an SVG; an animation's frames are
rasterised with Inkscape and joined into a GIF by ImageMagick. The keys are the
script, so after a TUI change the images are rebuilt rather than re-recorded:

    cargo build --release
    python3 tools/docs-images.py --lattice target/release/latt --rev v0.8.0

The register is the release snapshot of `<rev>` (`tools/release-snapshot.sh`, the paths
a clone of the release holds), so the images show the tree the README's quoted output
came from; pass the tag being cut.

Needs tmux and Python 3.12 or later (the openspec adapter); the GIF also needs `inkscape` and ImageMagick `convert`. Nothing is
written outside docs/img/ and a temporary directory.
"""
from __future__ import annotations

import argparse
import html
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field, replace
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "docs" / "img"
BROWSE_ARGS = "--profile profiles/openspec.yaml --adapter adapters/openspec --target ."
COLS, ROWS = 120, 32
CELL_W, CELL_H, FONT_PX, PAD = 8.4, 17.0, 14, 12

# xterm's 16 colours, then the 6x6x6 cube and the grey ramp of the 256-colour table.
BASE16 = [
    "#1e1e2e", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
    "#5c6370", "#ef7f88", "#a9d68b", "#f0d08a", "#7cc0f5", "#d79be8", "#6fd0dc", "#e6e9ef",
]
DEFAULT_FG, DEFAULT_BG = "#d4d7de", "#1e1e2e"


def palette(n: int) -> str:
    """Return the hex colour of xterm 256-colour index `n`."""
    if n < 16:
        return BASE16[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (steps[n // 36], steps[(n // 6) % 6], steps[n % 6])
    level = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (level, level, level)


@dataclass(frozen=True)
class Style:
    fg: str | None = None
    bg: str | None = None
    bold: bool = False
    dim: bool = False
    italic: bool = False
    underline: bool = False
    reverse: bool = False


def apply_sgr(style: Style, params: list[int]) -> Style:
    """Fold one SGR parameter list into a style, as a terminal would."""
    if not params:
        params = [0]
    i = 0
    while i < len(params):
        p = params[i]
        if p == 0:
            style = Style()
        elif p == 1:
            style = replace(style, bold=True)
        elif p == 2:
            style = replace(style, dim=True)
        elif p == 3:
            style = replace(style, italic=True)
        elif p == 4:
            style = replace(style, underline=True)
        elif p == 7:
            style = replace(style, reverse=True)
        elif p == 22:
            style = replace(style, bold=False, dim=False)
        elif p == 23:
            style = replace(style, italic=False)
        elif p == 24:
            style = replace(style, underline=False)
        elif p == 27:
            style = replace(style, reverse=False)
        elif 30 <= p <= 37:
            style = replace(style, fg=palette(p - 30))
        elif 90 <= p <= 97:
            style = replace(style, fg=palette(p - 90 + 8))
        elif p == 39:
            style = replace(style, fg=None)
        elif 40 <= p <= 47:
            style = replace(style, bg=palette(p - 40))
        elif 100 <= p <= 107:
            style = replace(style, bg=palette(p - 100 + 8))
        elif p == 49:
            style = replace(style, bg=None)
        elif p in (38, 48) and i + 1 < len(params):
            if params[i + 1] == 5 and i + 2 < len(params):
                colour = palette(params[i + 2])
                i += 2
            elif params[i + 1] == 2 and i + 4 < len(params):
                colour = "#%02x%02x%02x" % tuple(params[i + 2:i + 5])
                i += 4
            else:
                colour = None
            style = replace(style, **{"fg" if p == 38 else "bg": colour})
        i += 1
    return style


SGR = re.compile(r"\x1b\[([0-9;]*)m")


def parse(screen: str) -> list[list[tuple[str, Style]]]:
    """Split captured rows into (character, style) cells; style carries across rows."""
    rows, style = [], Style()
    for line in screen.split("\n")[:ROWS]:
        cells: list[tuple[str, Style]] = []
        pos = 0
        for m in SGR.finditer(line):
            cells.extend((ch, style) for ch in line[pos:m.start()])
            style = apply_sgr(style, [int(x) if x else 0 for x in m.group(1).split(";")])
            pos = m.end()
        cells.extend((ch, style) for ch in line[pos:])
        rows.append(cells[:COLS])
    return rows


def colours(style: Style) -> tuple[str, str | None]:
    fg, bg = style.fg or DEFAULT_FG, style.bg
    if style.reverse:
        fg, bg = (style.bg or DEFAULT_BG), (style.fg or DEFAULT_FG)
    return fg, bg


def render_svg(screen: str, title: str) -> str:
    """Draw a captured screen as an SVG terminal panel, with no window decoration."""
    width = COLS * CELL_W + 2 * PAD
    height = ROWS * CELL_H + 2 * PAD
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" '
        f'viewBox="0 0 {width:.0f} {height:.0f}" role="img" aria-label="{html.escape(title)}">',
        f"<title>{html.escape(title)}</title>",
        f'<rect width="100%" height="100%" rx="4" fill="{DEFAULT_BG}"/>',
        f'<g font-family="DejaVu Sans Mono, Menlo, Consolas, monospace" font-size="{FONT_PX}" '
        f'xml:space="preserve" transform="translate({PAD},{PAD})">',
    ]
    for r, cells in enumerate(parse(screen)):
        y = r * CELL_H
        c = 0
        while c < len(cells):
            start, style = c, cells[c][1]
            while c < len(cells) and cells[c][1] == style:
                c += 1
            text = "".join(ch for ch, _ in cells[start:c])
            fg, bg = colours(style)
            x = start * CELL_W
            if bg:
                out.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{(c - start) * CELL_W:.1f}" '
                           f'height="{CELL_H}" fill="{bg}"/>')
            if text.strip():
                attrs = [f'x="{x:.1f}"', f'y="{y + CELL_H - 4:.1f}"', f'fill="{fg}"',
                         f'textLength="{(c - start) * CELL_W:.1f}"', 'lengthAdjust="spacingAndGlyphs"']
                if style.bold:
                    attrs.append('font-weight="bold"')
                if style.italic:
                    attrs.append('font-style="italic"')
                if style.dim:
                    attrs.append('opacity="0.6"')
                if style.underline:
                    attrs.append('text-decoration="underline"')
                out.append(f'<text {" ".join(attrs)}>{html.escape(text)}</text>')
    out.append("</g></svg>")
    return "\n".join(out) + "\n"


@dataclass
class Frame:
    keys: list[str]
    delay: int = 130  # ImageMagick delay, in centiseconds


@dataclass
class Scene:
    name: str
    title: str
    frames: list[Frame] = field(default_factory=list)
    gif: bool = False


SCENES = [
    Scene("browse-nodes", "latt browse: cli/Exit codes and the tests that verify it",
          [Frame(["/", "cli/Exit codes", "Enter", "Enter"])]),
    Scene("browse-graph", "latt browse: the graph view centred on a requirement",
          [Frame(["/", "graph-model/Dangling edge targets", "Enter", "5"])]),
    Scene("browse-walk", "latt browse: from a finding to its node, then along an edge",
          [
              Frame([], 180),
              Frame(["3"], 220),
              Frame(["j"], 70),
              Frame(["Enter"], 260),
              Frame(["q"], 50),
              Frame(["1"], 50),
              Frame(["/", "cli/Exit codes", "Enter"], 180),
              Frame(["Enter"], 230),
              Frame(["Tab"], 50),
              *[Frame(["j"], 35) for _ in range(8)],
              Frame(["Enter"], 230),
          ], gif=True),
]


class Session:
    """A detached tmux session running browse on the register in `tree`."""

    def __init__(self, lattice: Path, tree: Path, name: str) -> None:
        self.name = f"lattice-img-{name}"
        command = (
            f"env TERM=xterm-256color LANG=C.UTF-8 {lattice} browse "
            f"{BROWSE_ARGS}; sleep 30"
        )
        subprocess.run(["tmux", "kill-session", "-t", self.name], capture_output=True)
        subprocess.run(["tmux", "new-session", "-d", "-s", self.name, "-x", str(COLS),
                        "-y", str(ROWS), "-c", str(tree), command], check=True)
        time.sleep(5.0)  # the openspec adapter reads the whole repository

    def send(self, keys: list[str]) -> None:
        for key in keys:
            subprocess.run(["tmux", "send-keys", "-t", self.name, key], check=True)
            time.sleep(0.08)
        time.sleep(0.6)

    def capture(self) -> str:
        return subprocess.run(["tmux", "capture-pane", "-p", "-e", "-t", self.name],
                              check=True, capture_output=True, text=True).stdout

    def close(self) -> None:
        subprocess.run(["tmux", "send-keys", "-t", self.name, "Q"], capture_output=True)
        subprocess.run(["tmux", "kill-session", "-t", self.name], capture_output=True)


def build(scene: Scene, lattice: Path, tree: Path, scratch: Path) -> Path:
    session = Session(lattice, tree, scene.name)
    try:
        frames = []
        for frame in scene.frames:
            session.send(frame.keys)
            frames.append(render_svg(session.capture(), scene.title))
    finally:
        session.close()
    if not scene.gif:
        target = OUT / f"{scene.name}.svg"
        target.write_text(frames[-1])
        return target
    pngs = []
    for i, svg in enumerate(frames):
        src, png = scratch / f"{scene.name}-{i}.svg", scratch / f"{scene.name}-{i}.png"
        src.write_text(svg)
        subprocess.run(["inkscape", str(src), "--export-type=png", f"--export-filename={png}",
                        "--export-width=1000"], check=True, capture_output=True)
        pngs.append(str(png))
    target = OUT / f"{scene.name}.gif"
    convert = ["convert", "-loop", "0"]
    for frame, png in zip(scene.frames, pngs, strict=True):
        convert.extend(["-delay", str(frame.delay), png])
    subprocess.run([*convert, "-layers", "Optimize", str(target)], check=True)
    return target


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lattice", type=Path, default=REPO / "target/release/latt",
                        help="a lattice binary built with the default tui feature")
    parser.add_argument("--rev", default="HEAD",
                        help="the revision whose tree browse shows (default: HEAD)")
    parser.add_argument("scenes", nargs="*", help="scene names (default: all)")
    args = parser.parse_args()
    lattice = args.lattice.resolve()
    help_text = subprocess.run([str(lattice), "--help"], capture_output=True, text=True).stdout
    if not re.search(r"^\s+browse\b", help_text, re.MULTILINE):
        print(f"{lattice} has no browse; build it with the default features", file=sys.stderr)
        return 2
    for tool in ("tmux",):
        if not shutil.which(tool):
            print(f"{tool} not found on PATH", file=sys.stderr)
            return 2
    OUT.mkdir(parents=True, exist_ok=True)
    wanted = [s for s in SCENES if not args.scenes or s.name in args.scenes]
    with tempfile.TemporaryDirectory() as scratch:
        tree = Path(scratch) / "tree"
        tree.mkdir()
        # The snapshot's parent only places it in history; rev itself serves.
        snapshot = subprocess.run([str(REPO / "tools/release-snapshot.sh"), args.rev, args.rev],
                                  cwd=REPO, check=True, capture_output=True, text=True).stdout.strip()
        archive = subprocess.run(["git", "-C", str(REPO), "archive", snapshot],
                                 check=True, capture_output=True).stdout
        subprocess.run(["tar", "-x", "-C", str(tree)], input=archive, check=True)
        for scene in wanted:
            if scene.gif and not (shutil.which("inkscape") and shutil.which("convert")):
                print(f"{scene.name}: needs inkscape and convert; skipped", file=sys.stderr)
                continue
            print(build(scene, lattice, tree, Path(scratch)).relative_to(REPO))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

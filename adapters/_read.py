"""Shared loss-accounting reads and adapter-config access.

Two boundaries, one module, because every adapter needs both and the failure
routing differs: a *target* file the adapter cannot read becomes a PARSE_ERROR
issue and a None return, never an exception. Raising would turn a register
problem into exit 2, which callers read as the adapter itself being broken.
A *profile* that cannot say where the register lives is that broken setup,
so config access raises ValueError instead of reporting.
"""
from __future__ import annotations

from pathlib import Path

from adapters._emit import DocumentBuilder
from adapters._profile import Profile
from adapters._types import Issue, Provenance, Severity


def read_utf8_text(graph: DocumentBuilder, path: Path) -> str | None:
    """Read *path* as UTF-8, reporting failure as an error PARSE_ERROR.

    OSError is caught beside UnicodeDecodeError deliberately: a missing
    permission or a directory where a file was expected is register input
    failing, not the adapter failing.
    """
    try:
        # utf-8-sig: a leading byte order mark is encoding, not content.
        return path.read_text(encoding="utf-8-sig")
    except UnicodeDecodeError:
        # The bytes are reread to show each bad line, so the reread is under
        # the same OSError handler: a file gone by then fails as register input.
        try:
            data = path.read_bytes()
        except OSError as e:
            message = f"could not read {path}: {e}"
        else:
            _report_undecodable_lines(graph, path, data)
            return None
    except OSError as e:
        message = f"could not read {path}: {e}"
    graph.add_issue(Issue(
        Severity.ERROR, "PARSE_ERROR", message, Provenance(str(path), 0),
    ))
    return None


def _report_undecodable_lines(graph: DocumentBuilder, path: Path, data: bytes) -> None:
    # Source code is defined over UTF-8, so the file is read as nothing, as
    # a syntax error would be; each line that broke it is shown as written.
    for lineno, raw in enumerate(data.split(b"\n"), 1):
        try:
            raw.decode("utf-8-sig" if lineno == 1 else "utf-8")
            continue
        except UnicodeDecodeError:
            pass
        written, column = _as_written(raw.removeprefix(b"\xef\xbb\xbf"))
        graph.add_issue(Issue(
            Severity.ERROR, "PARSE_ERROR",
            f"line is not valid UTF-8 at column {column}: {written}; "
            "the file must be UTF-8, so it was not read",
            Provenance(str(path), lineno),
        ))


def read_utf8_lines(graph: DocumentBuilder, path: Path) -> list[str] | None:
    """Line-split variant of read_utf8_text; None propagates a reported failure."""
    text = read_utf8_text(graph, path)
    return None if text is None else text.splitlines()


def _as_written(raw: bytes) -> tuple[str, int]:
    """Render a line's bytes as written, each one that is not UTF-8 as `\\xNN`.

    Return the rendering and the one-based character column of the first such
    byte. Uppercase hex, as the Rust adapters write it.
    """
    out, column, pos = [], 0, 0
    while pos < len(raw):
        try:
            out.append(raw[pos:].decode("utf-8"))
            break
        except UnicodeDecodeError as e:
            out.append(raw[pos:pos + e.start].decode("utf-8"))
            if not column:
                column = len("".join(out)) + 1
            out.extend(f"\\x{b:02X}" for b in raw[pos + e.start:pos + e.end])
            pos += e.end
    return "".join(out).rstrip("\r"), column or 1


def read_utf8_lines_granular(
    graph: DocumentBuilder, path: Path,
) -> tuple[list[str], set[int]] | None:
    """Read *path* line by line, reporting each line that is not UTF-8 where it is.

    Return the lines and the one-based numbers of the unreadable ones, which are
    decoded with replacement characters so headings keep their structure; the
    caller reads none of them as content. Each is one PARSE_ERROR at its line,
    shown as written, so one stray byte withholds only what holds it rather than
    the whole file. None when the file cannot be read at all.
    """
    try:
        data = path.read_bytes()
    except OSError as e:
        graph.add_issue(Issue(
            Severity.ERROR, "PARSE_ERROR", f"could not read {path}: {e}",
            Provenance(str(path), 0),
        ))
        return None
    if data.startswith(b"\xef\xbb\xbf"):
        data = data[3:]
    try:
        return data.decode("utf-8").splitlines(), set()
    except UnicodeDecodeError:
        pass
    lines, unread = [], set()
    for lineno, raw in enumerate(data.split(b"\n"), 1):
        try:
            lines.append(raw.decode("utf-8").rstrip("\r"))
            continue
        except UnicodeDecodeError:
            pass
        lines.append(raw.decode("utf-8", errors="replace").rstrip("\r"))
        unread.add(lineno)
        written, column = _as_written(raw)
        graph.add_issue(Issue(
            Severity.ERROR, "PARSE_ERROR",
            f"line is not valid UTF-8 at column {column}, so it was not read: {written}",
            Provenance(str(path), lineno),
        ))
    if lines and lines[-1] == "" and data.endswith(b"\n"):
        lines.pop()
    return lines, unread


def adapter_paths(profile: Profile, needs: str) -> dict:
    """Return the profile's adapter.paths mapping, or raise ValueError.

    *needs* names the keys the caller requires, so the exit-2 message tells
    the profile author what to declare. Key-level validation stays with the
    caller, since the adapters genuinely differ there.
    """
    adapter = profile.extra.get("adapter")
    if not isinstance(adapter, dict):
        raise ValueError(
            f"profile has no 'adapter' section; the adapter requires {needs}"
        )
    paths = adapter.get("paths")
    if not isinstance(paths, dict):
        raise ValueError(
            f"profile 'adapter' section has no 'paths' map; "
            f"the adapter requires {needs}"
        )
    return paths

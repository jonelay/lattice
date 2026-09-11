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
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as e:
        message = f"could not decode {path} as UTF-8: {e}"
    except OSError as e:
        message = f"could not read {path}: {e}"
    graph.add_issue(Issue(
        Severity.ERROR, "PARSE_ERROR", message, Provenance(str(path), 0),
    ))
    return None


def read_utf8_lines(graph: DocumentBuilder, path: Path) -> list[str] | None:
    """Line-split variant of read_utf8_text; None propagates a reported failure."""
    text = read_utf8_text(graph, path)
    return None if text is None else text.splitlines()


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

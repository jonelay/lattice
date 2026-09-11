"""Shared structural discovery of Python test definitions.

Discovery is static and AST-based: one source definition is one test node,
however many cases a runner expands it to: a `test_`-named function at module
top level, or a `test_`-named direct method of a top-level class. Text that
merely looks like a definition (a `def test_` inside a string literal) is not
one, and a module that does not parse is reported rather than pattern-matched.
Edge binding (pytest markers, citation comments) stays per-adapter.
"""
from __future__ import annotations

import ast
from pathlib import Path
from typing import Iterator

from adapters._emit import DocumentBuilder
from adapters._read import read_utf8_text
from adapters._types import Issue, Provenance, Severity

TestFn = ast.FunctionDef | ast.AsyncFunctionDef


def parse_test_source(
    graph: DocumentBuilder, source: str, file_str: str,
) -> ast.Module | None:
    """Parse *source*, reporting a SyntaxError as a warning PARSE_ERROR.

    A warning, not an error: the file failing to parse is one file's finding,
    and the run must go on to the rest of the register.
    """
    try:
        return ast.parse(source, filename=file_str)
    except SyntaxError as e:
        graph.add_issue(Issue(
            Severity.WARNING, "PARSE_ERROR",
            f"syntax error in {file_str}: {e}",
            Provenance(file_str, e.lineno or 0),
        ))
        return None


def parse_test_module(graph: DocumentBuilder, path: Path) -> ast.Module | None:
    """Read and parse *path*; None propagates an already-reported failure."""
    source = read_utf8_text(graph, path)
    if source is None:
        return None
    return parse_test_source(graph, source, str(path))


def iter_test_holders(
    tree: ast.Module,
) -> Iterator[tuple[ast.ClassDef | None, list[TestFn]]]:
    """Yield each top-level class with its test methods, in source order.

    A top-level test function arrives as `(None, [fn])`. A class is yielded
    even when its test-method list is empty, so a caller reading class-level
    markers still sees, and can report on, a class holding no tests.
    """
    for node in ast.iter_child_nodes(tree):
        if isinstance(node, ast.ClassDef):
            yield node, [
                m for m in node.body
                if isinstance(m, TestFn) and m.name.startswith("test_")
            ]
        elif isinstance(node, TestFn) and node.name.startswith("test_"):
            yield None, [node]

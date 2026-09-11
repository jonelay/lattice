"""OpenSpec register adapter: capabilities, requirements, and the tests citing them.

Reads `<spec_dir>/<capability>/spec.md` files and the citation comments the
target's test files carry. A `verifies` edge records that a test cites a
requirement, never that it passed. The adapter reads only source text.
"""
from __future__ import annotations

import re
from pathlib import Path
from typing import Iterator

from adapters._emit import DocumentBuilder
from adapters._profile import Profile
from adapters._python_tests import iter_test_holders, parse_test_source
from adapters._read import adapter_paths, read_utf8_lines, read_utf8_text
from adapters._types import Issue, Provenance, Severity

_REQUIREMENT_HEADING = re.compile(r"^### Requirement:\s*(.*?)\s*$")
_SCENARIO_HEADING = re.compile(r"^#### Scenario:")

# A requirement's body ends at the next heading of level 1-3; a `#### Scenario:`
# belongs to the requirement above it and stays in the body.
_BODY_TERMINATOR = re.compile(r"^#{1,3} ")

# The register writes its verification pointer both ways. Matching one form
# leaves the other in the body, where it names a test command whose `cargo test`
# and `pytest` boilerplate is shared across most requirements. That text pulls
# every requirement toward every test rather than toward the right one.
_VERIFIED_BY = re.compile(r"^\*{0,2}Verified by:?\*{0,2}\s")

# The citation prefix inside a comment. The whole remainder is one title;
# splitting on any separator would guess, so a compound citation resolves to
# nothing and surfaces as VACANCY, which is the signal to split the line.
_CITATION_PREFIX = "Requirement:"

_RUST_FN = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")


def _error(graph: DocumentBuilder, code: str, message: str, path: Path,
           line: int = 0, severity: Severity = Severity.ERROR) -> None:
    graph.add_issue(Issue(severity, code, message, Provenance(str(path), line)))


def _citation(line: str, marker: str) -> str | None:
    """The cited title, or None when the line is not a whole-line citation."""
    stripped = line.strip()
    if not stripped.startswith(marker):
        return None
    rest = stripped[len(marker):].strip()
    if not rest.startswith(_CITATION_PREFIX):
        return None
    return rest[len(_CITATION_PREFIX):].strip()


def _requirement_body(lines: list[str], heading_line: int) -> str:
    """The prose, rationale and scenarios under a requirement heading."""
    body = []
    for line in lines[heading_line:]:
        if _BODY_TERMINATOR.match(line):
            break
        if _VERIFIED_BY.match(line):
            continue
        body.append(line)
    return "\n".join(body).strip()


def _rust_body(lines: list[str], fn_line: int) -> str | None:
    """A Rust test function's body, bounded by brace depth, or None if unbounded.

    Depth, not a column-0 `}`: that rule does not recognise `fn a() {}` or an
    indented brace, so it runs on and attaches the *next* test's code to this
    node. Wrong text presented as the node's own, which no reviewer reading a
    ranked list could see. Braces inside strings and comments do not count.
    """
    depth = 0
    started = False
    body: list[str] = []
    for raw in lines[fn_line - 1:]:
        opens_at = None
        closes_at = None
        for column, char in enumerate(_strip_rust_noise(raw)):
            if char == "{":
                depth += 1
                if not started:
                    started = True
                    opens_at = column + 1
            elif char == "}":
                depth -= 1
                if started and depth == 0:
                    closes_at = column
                    break
        if not started:
            continue
        # The declaration line contributes only what follows its brace.
        begin = opens_at if opens_at is not None else 0
        if closes_at is not None:
            tail = raw[begin:closes_at]
            if tail.strip():
                body.append(tail)
            return "\n".join(body).strip()
        body.append(raw[begin:])
    return None


def _strip_rust_noise(line: str) -> str:
    """The line with string literals and a line comment blanked out.

    Only braces matter downstream, so blanking is enough and no real lexer is
    warranted; the point is that a `"}"` must not close a function.
    """
    out = []
    in_string = False
    escaped = False
    index = 0
    while index < len(line):
        char = line[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            out.append(" ")
        elif char == '"':
            in_string = True
            out.append(" ")
        elif char == "/" and line[index:index + 2] == "//":
            break
        else:
            out.append(char)
        index += 1
    return "".join(out)


def _python_body(lines: list[str], def_line: int) -> str:
    """A Python test function's body, bounded by the first line dedented past it.

    A dedent is the language's own block terminator, so unlike the Rust case
    there is no shape that runs on into the next function.
    """
    indent = len(lines[def_line - 1]) - len(lines[def_line - 1].lstrip())
    body = []
    for line in lines[def_line:]:
        if line.strip() and (len(line) - len(line.lstrip())) <= indent:
            break
        body.append(line)
    return "\n".join(body).strip()


def _scan_rust(
    graph: DocumentBuilder, path: Path, lines: list[str],
) -> Iterator[tuple[str, str, int]]:
    """Yield ('citation', title, line) and ('test', name, line) events.

    A `#[test]` attribute marks the next `fn` as a test; the mark survives
    intervening attribute lines such as `#[ignore]`. *graph* and *path* are
    the dialect-scanner signature; this scanner has nothing to report.
    """
    pending_test = False
    for lineno, line in enumerate(lines, 1):
        title = _citation(line, "//")
        if title is not None:
            yield "citation", title, lineno
            continue
        if line.strip().startswith("#[test]"):
            pending_test = True
            continue
        if pending_test:
            m = _RUST_FN.match(line)
            if m:
                yield "test", m.group(1), lineno
                pending_test = False


def _scan_python(
    graph: DocumentBuilder, path: Path, lines: list[str],
) -> Iterator[tuple[str, str, int]]:
    """Yield ('citation', title, line) and ('test', name, line) events.

    Definitions come from the shared AST scanner; a file that does not parse
    is a reported PARSE_ERROR and yields no tests. Citations are comments,
    which an AST cannot see, so they stay a line scan. The two streams merge
    by line number, which is the order the old single pass saw them in. A
    method is qualified by its class (`TestX::test_y`): two classes in one
    file may reuse a method name, and an unqualified ID would conflate two
    tests the file really declares into one duplicate node.
    """
    events: list[tuple[int, str, str]] = []
    tree = parse_test_source(graph, "\n".join(lines), str(path))
    if tree is not None:
        for holder, methods in iter_test_holders(tree):
            for method in methods:
                name = (f"{holder.name}::{method.name}" if holder
                        else method.name)
                events.append((method.lineno, "test", name))
    for lineno, line in enumerate(lines, 1):
        title = _citation(line, "#")
        if title is not None:
            events.append((lineno, "citation", title))
    for lineno, event, text in sorted(events, key=lambda e: e[0]):
        yield event, text, lineno


_DIALECTS = {"rust": _scan_rust, "python": _scan_python}


def _resolve_config(profile: Profile, target_path: Path) -> dict:
    """Read the adapter section from the profile and resolve its paths.

    Raises rather than reporting: a profile that cannot say where the register
    and its tests live is a broken setup, which is exit code 2 and not a finding.
    """
    paths = adapter_paths(
        profile, "adapter.paths.spec_dir and adapter.paths.tests")
    if not isinstance(paths.get("spec_dir"), str):
        raise ValueError(
            "profile 'adapter' section has no string adapter.paths.spec_dir"
        )
    tests = paths.get("tests")
    if not isinstance(tests, list) or not tests:
        raise ValueError(
            "profile 'adapter' section has no non-empty list adapter.paths.tests"
        )
    for entry in tests:
        if (not isinstance(entry, dict)
                or not isinstance(entry.get("glob"), str)
                or entry.get("dialect") not in _DIALECTS):
            raise ValueError(
                f"adapter.paths.tests entry {entry!r} needs a string 'glob' and "
                f"a 'dialect' in {sorted(_DIALECTS)}"
            )
    return {
        "spec_dir": target_path / paths["spec_dir"],
        "tests": [(e["glob"], e["dialect"]) for e in tests],
    }


def _read_spec_file(graph: DocumentBuilder, path: Path, capability: str) -> None:
    """Emit the file's capability node and one requirement node per heading.

    The capability's identity is its directory name, so the node is emitted
    whether or not the file decodes. An unreadable register is a capability
    whose contents are an issue, not an absent capability.
    """
    graph.add_node(capability, "capability", {}, Provenance(str(path), 0))
    lines = read_utf8_lines(graph, path)
    if lines is None:
        return

    title: str | None = None
    title_line = 0
    scenarios = 0

    def emit() -> None:
        if title is None:
            return
        attrs = {"title": title, "scenarios": scenarios}
        body = _requirement_body(lines, title_line)
        if body:
            attrs["body"] = body
        graph.add_node(
            title, "requirement", attrs,
            Provenance(str(path), title_line),
        )
        graph.add_edge(capability, title, "contains", Provenance(str(path), title_line))

    for lineno, line in enumerate(lines, 1):
        m = _REQUIREMENT_HEADING.match(line)
        if m:
            emit()
            title, title_line, scenarios = m.group(1), lineno, 0
        elif _SCENARIO_HEADING.match(line):
            scenarios += 1
    emit()


def _read_test_file(graph: DocumentBuilder, path: Path, target: Path,
                    dialect: str) -> None:
    """Emit test nodes and the `verifies` edges their citations declare.

    A citation is a section header: it binds every test function after it until
    the next citation or end of file. Directly consecutive citation lines stack,
    binding the same group once per line; any other line between two citations
    makes the later one replace the earlier. A test above any citation enters
    the graph with no edges rather than being omitted.

    Lines split on newline only, not `splitlines`: that also cuts at U+2028
    and friends, which desynchronizes citation and body line numbers from the
    parsed tree's linenos when such a character sits inside a string literal.
    """
    text = read_utf8_text(graph, path)
    if text is None:
        return
    lines = text.split("\n")

    rel = path.relative_to(target).as_posix()
    cited: list[tuple[str, int]] = []
    for event, text, lineno in _DIALECTS[dialect](graph, path, lines):
        if event == "citation":
            if not (cited and lineno == cited[-1][1] + 1):
                cited = []
            cited.append((text, lineno))
            continue
        node_id = f"{rel}::{text}"
        attrs = {"file": rel, "function": text}
        if dialect == "rust":
            body = _rust_body(lines, lineno)
            if body is None:
                _error(
                    graph, "PARSE_ERROR",
                    f"{path.name}: could not find the end of test function "
                    f"'{text}' at line {lineno}; its body is not carried",
                    path, lineno, severity=Severity.WARNING,
                )
        else:
            body = _python_body(lines, lineno)
        if body:
            attrs["body"] = body
        graph.add_node(
            node_id, "test", attrs,
            Provenance(str(path), lineno),
        )
        for title, cite_line in cited:
            graph.add_edge(node_id, title, "verifies", Provenance(str(path), cite_line))


def build_graph(profile: Profile, target_path: Path) -> DocumentBuilder:
    """Read an OpenSpec register and its citing tests into an interface document.

    Every spec and test file is read whatever the others do: an unreadable one
    becomes an issue and the rest still reach the graph, because a file the
    adapter stayed silent about reads as a file with nothing to audit.
    """
    config = _resolve_config(profile, target_path)
    graph = DocumentBuilder()

    spec_dir = config["spec_dir"]
    if not spec_dir.is_dir():
        _error(
            graph, "PARSE_ERROR",
            f"spec directory '{spec_dir}' does not exist",
            spec_dir,
        )
    else:
        for path in sorted(spec_dir.glob("*/spec.md")):
            _read_spec_file(graph, path, path.parent.name)

    for pattern, dialect in config["tests"]:
        paths = sorted(target_path.glob(pattern))
        if not paths:
            _error(
                graph, "PARSE_ERROR",
                f"test glob '{pattern}' matched no file",
                target_path, severity=Severity.WARNING,
            )
            continue
        for path in paths:
            _read_test_file(graph, path, target_path, dialect)

    return graph


if __name__ == "__main__":
    from adapters._emit import run_main

    raise SystemExit(run_main(build_graph))

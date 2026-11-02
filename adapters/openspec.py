"""OpenSpec register adapter: capabilities, requirements, and the tests citing them.

Reads `<spec_dir>/<capability>/spec.md` files and the citation comments the
target's test files carry. A `verifies` edge records that a test cites a
requirement, never that it passed. The adapter reads only source text.
"""
from __future__ import annotations

import fnmatch
import io
import os
import re
import tokenize
from pathlib import Path
from typing import Iterator

from adapters._emit import DocumentBuilder
from adapters._profile import Profile
from adapters._python_tests import iter_test_holders, parse_test_source
from adapters._read import adapter_paths, read_utf8_lines_granular, read_utf8_text
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

# The citation prefix inside a comment. The whole remainder is one requirement
# ID, `<capability>/<title>`; splitting on any separator would guess, so a compound citation resolves to
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
    noise = _RustNoise()
    for raw in lines[fn_line - 1:]:
        opens_at = None
        closes_at = None
        for column, char in enumerate(noise.blank(raw)):
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


_STRING_OPEN = re.compile(r'(?:[bc]?r(#*)|[bc]?)"')


class _RustNoise:
    """Blank string literals and comments out of Rust source, a line at a time.

    Only braces matter downstream, so blanking is enough; the point is that a
    `"}"` must not close a function. The lexer state outlives the line: a
    string literal continues past a trailing `\\`, and a raw string or block
    comment past any newline, so a continuation line that opens no quote of
    its own is still text. Lexing each line afresh read such a line as code,
    so the `{{` and `\\"` of a multi-line `format!` string left a test's
    brace depth unbalanced and its end unfound.
    """

    def __init__(self) -> None:
        self._in_string = False
        self._raw_hashes: int | None = None  # closer is `"` plus this many `#`
        self._comment_depth = 0

    def in_code(self) -> bool:
        """True when the next line opens as code, not inside a string or block comment."""
        return not self._in_string and not self._comment_depth

    def blank(self, line: str) -> str:
        out: list[str] = []
        index = 0
        escaped = False  # a `\` at line end escapes the newline itself
        while index < len(line):
            char = line[index]
            if self._in_string:
                if self._raw_hashes is None:
                    if escaped:
                        escaped = False
                    elif char == "\\":
                        escaped = True
                    elif char == '"':
                        self._in_string = False
                else:
                    closer = '"' + "#" * self._raw_hashes
                    if line.startswith(closer, index):
                        out.append(" " * len(closer))
                        index += len(closer)
                        self._in_string = False
                        continue
                out.append(" ")
                index += 1
            elif self._comment_depth:
                if line.startswith("/*", index):
                    self._comment_depth += 1
                    out.append("  ")
                    index += 2
                elif line.startswith("*/", index):
                    self._comment_depth -= 1
                    out.append("  ")
                    index += 2
                else:
                    out.append(" ")
                    index += 1
            elif line.startswith("//", index):
                break
            elif line.startswith("/*", index):
                self._comment_depth = 1
                out.append("  ")
                index += 2
            elif (opener := _string_opener(line, index)) is not None:
                self._in_string = True
                self._raw_hashes = (
                    len(opener.group(1)) if opener.group(1) is not None else None
                )
                out.append(" " * (opener.end() - index))
                index = opener.end()
            elif char == "'" and (end := _char_literal_end(line, index)) is not None:
                out.append(" " * (end - index))
                index = end
            else:
                out.append(char)
                index += 1
        return "".join(out)


def _string_opener(line: str, index: int) -> re.Match[str] | None:
    """A string literal's opening at *index*, prefix included, or None.

    A prefixed form (`r#"`, `b"`, `br"`, `c"`) counts only when the prefix is
    not the tail of an identifier, so the lexer advancing one character at a
    time cannot open a string from inside a name.
    """
    m = _STRING_OPEN.match(line, index)
    if m is None:
        return None
    if line[index] != '"' and index > 0:
        before = line[index - 1]
        if before.isalnum() or before == "_":
            return None
    return m


def _char_literal_end(line: str, index: int) -> int | None:
    """The index past the char literal opened at *index*, or None for a lifetime.

    `'\\n'`, `'\\''`, `'\\x7d'` and `'\\u{7d}'` close at the first quote after
    the escape; `'}'` is a quote two characters on. Anything else opening
    with a quote is a lifetime or loop label and blanks nothing.
    """
    if line.startswith("\\", index + 1):
        close = line.find("'", index + 3)
        return None if close == -1 else close + 1
    if line.startswith("'", index + 2):
        return index + 3
    return None


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
    intervening attribute lines such as `#[ignore]`. A line that opens inside a
    string literal or block comment is text, so it neither cites nor declares,
    and attributes and `fn`s are read from the line with literals blanked.
    *graph* and *path* are the dialect-scanner signature; this scanner has
    nothing to report.
    """
    pending_test = False
    noise = _RustNoise()
    for lineno, line in enumerate(lines, 1):
        in_code = noise.in_code()
        code = noise.blank(line)
        if not in_code:
            continue
        title = _citation(line, "//")
        if title is not None:
            yield "citation", title, lineno
            continue
        if code.strip().startswith("#[test]"):
            pending_test = True
            continue
        if pending_test:
            m = _RUST_FN.match(code)
            if m:
                yield "test", m.group(1), lineno
                pending_test = False


def _scan_python(
    graph: DocumentBuilder, path: Path, lines: list[str],
) -> Iterator[tuple[str, str, int]]:
    """Yield ('citation', title, line) and ('test', name, line) events.

    Definitions come from the shared AST scanner; a file that does not parse
    is a reported PARSE_ERROR and yields no tests. Citations are comments,
    which an AST cannot see, so they are a line scan limited to lines the
    tokenizer finds a comment on: a `# Requirement:` inside a string is text.
    The two streams merge by line number, which is the order the old single
    pass saw them in. A
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
    comments = _comment_lines("\n".join(lines)) if tree is not None else None
    for lineno, line in enumerate(lines, 1):
        if comments is not None and lineno not in comments:
            continue
        title = _citation(line, "#")
        if title is not None:
            events.append((lineno, "citation", title))
    for lineno, event, text in sorted(events, key=lambda e: e[0]):
        yield event, text, lineno


def _comment_lines(text: str) -> set[int] | None:
    """Lines holding a real comment token, so a `#` inside a string is not one.

    None when the tokenizer refuses source the AST accepted, which leaves the
    caller's line scan as it was rather than dropping every citation.
    """
    try:
        return {
            token.start[0]
            for token in tokenize.generate_tokens(io.StringIO(text).readline)
            if token.type == tokenize.COMMENT
        }
    except (tokenize.TokenError, SyntaxError):
        return None


_TS_CALL = re.compile(
    r"^[ \t]*(?:await\s+)?(describe|it|test)\b((?:\s*\.[A-Za-z]+)*)")


class _TsNoise:
    """Blank string literals, template text and comments out of TypeScript.

    Like `_RustNoise`, only the structure survives: a brace inside `"}"` or
    a template literal must neither close a test nor open a `describe`. A
    template literal spans lines, and its `${...}` holes are code again, so
    the state is a stack: `"tpl"` for template text, an int for the brace
    depth inside a hole. Test files embed whole spec documents in templates,
    and a line of one that reads `it("x", ...)` is text, not a test.
    """

    def __init__(self) -> None:
        self._stack: list[str | int] = []
        self._comment = False
        self._quote: str | None = None

    def in_code(self) -> bool:
        """True when the next line opens as code."""
        return (not self._comment and self._quote is None
                and (not self._stack or self._stack[-1] != "tpl"))

    def blank(self, line: str) -> str:
        out: list[str] = []
        index = 0
        while index < len(line):
            char = line[index]
            top = self._stack[-1] if self._stack else None
            if self._comment:
                if line.startswith("*/", index):
                    self._comment = False
                    out.append("  ")
                    index += 2
                    continue
                out.append(" ")
            elif self._quote is not None:
                if char == "\\":
                    out.append("  ")
                    index += 2
                    continue
                if char == self._quote:
                    self._quote = None
                out.append(" ")
            elif top == "tpl":
                if char == "\\":
                    out.append("  ")
                    index += 2
                    continue
                if char == "`":
                    self._stack.pop()
                elif line.startswith("${", index):
                    self._stack.append(0)
                    out.append("  ")
                    index += 2
                    continue
                out.append(" ")
            elif line.startswith("//", index):
                break
            elif line.startswith("/*", index):
                self._comment = True
                out.append("  ")
                index += 2
                continue
            elif char == "/" and _starts_regex(out):
                end = _regex_end(line, index)
                out.append(" " * (end - index))
                index = end
                continue
            elif char in "\"'":
                self._quote = char
                out.append(" ")
            elif char == "`":
                self._stack.append("tpl")
                out.append(" ")
            elif char == "{":
                if isinstance(top, int):
                    self._stack[-1] = top + 1
                out.append(char)
            elif char == "}":
                if top == 0:
                    self._stack.pop()  # the hole closes; template text resumes
                    out.append(" ")
                else:
                    if isinstance(top, int):
                        self._stack[-1] = top - 1
                    out.append(char)
            else:
                out.append(char)
            index += 1
        if self._quote is not None and not line.endswith("\\"):
            self._quote = None  # a quoted string does not cross a bare newline
        return "".join(out)


def _starts_regex(out: list[str]) -> bool:
    """True when a `/` after the code blanked so far opens a regex literal.

    After an operand (a name, a number, `)` or `]`) a slash divides; after
    an operator, an opening bracket or nothing, it opens a regex. A quote
    inside `/'/g` would otherwise open a string that never closes.
    """
    for char in reversed(out):
        if char != " ":
            return char in "(,=:[!&|?{};+-*%<>~^"
    return True


def _regex_end(line: str, index: int) -> int:
    """The index past the regex literal opened at *index*, flags included."""
    cursor = index + 1
    in_class = False
    while cursor < len(line):
        char = line[cursor]
        if char == "\\":
            cursor += 2
            continue
        if char == "[":
            in_class = True
        elif char == "]":
            in_class = False
        elif char == "/" and not in_class:
            cursor += 1
            while cursor < len(line) and line[cursor].isalpha():
                cursor += 1
            return cursor
        cursor += 1
    return len(line)


def _ts_call(lines: list[str], first: int) -> tuple[str, str, int, int] | None:
    """Read the test call opening `lines[first]`: `(callee, name, line, column)`.

    *line* and *column* (0-based, into *lines*) point just past the name
    literal. Between the callee and the name sit any modifiers (`.skip`,
    `.each`) and argument groups that do not open with a quote
    (`.skipIf(cond)`, `.each([...])`), skipped by parenthesis depth over
    blanked code; the name is the first group's leading string. The call
    may span lines, so lines are read as one stream, each blanked column for
    column, as far as the call needs: an `.each` table of any length reads.
    """
    noise = _TsNoise()
    stream: list[tuple[int, int, str, str]] = []  # (line, column, raw, code)
    pending = iter(range(first, len(lines)))

    def ensure(index: int) -> bool:
        """True when the stream reaches *index*, reading lines as needed."""
        while index >= len(stream):
            line = next(pending, None)
            if line is None:
                return False
            raw = lines[line]
            code = noise.blank(raw).ljust(len(raw))
            stream.extend((line, col, raw[col], code[col]) for col in range(len(raw)))
            stream.append((line, len(raw), "\n", " "))
        return True

    ensure(0)
    m = _TS_CALL.match("".join(item[3] for item in stream))
    if m is None:
        return None
    index = m.end()
    while True:
        while ensure(index) and stream[index][3] in " \n":
            index += 1
        if not ensure(index) or stream[index][3] != "(":
            return None
        lead = index + 1
        while ensure(lead) and stream[lead][2] in " \t\n":
            lead += 1
        if ensure(lead) and stream[lead][2] in "\"'`":
            break
        depth = 0
        while ensure(index):
            depth += {"(": 1, ")": -1}.get(stream[index][3], 0)
            index += 1
            if depth == 0:
                break
        else:
            return None
        # A member after a group (`.each(t).skip(...)`) continues the chain.
        while ensure(index) and stream[index][3] in " \n":
            index += 1
        while ensure(index) and stream[index][3] == ".":
            index += 1
            while ensure(index) and stream[index][3].isalpha():
                index += 1
    quote = stream[lead][2]
    name: list[str] = []
    cursor = lead + 1
    while ensure(cursor):
        char = stream[cursor][2]
        if char == "\\" and ensure(cursor + 1):
            name.append(stream[cursor + 1][2])
            cursor += 2
            continue
        if char == quote:
            line, column = stream[cursor][0], stream[cursor][1] + 1
            return m.group(1), "".join(name), line, column
        if char == "\n" and quote != "`":
            return None
        name.append(char)
        cursor += 1
    return None


def _scan_typescript(
    graph: DocumentBuilder, path: Path, lines: list[str],
) -> Iterator[tuple[str, str, int]]:
    """Yield ('citation', title, line) and ('test', name, line) events.

    A test is an `it(...)` or `test(...)` call opening a line, with any
    modifiers (`.skip`, `.only`, `.each(table)`), and its name is the
    enclosing `describe` names and its own joined by `::`, so one name
    reused under two `describe`s is two tests. A `describe` lasts while the
    brace depth stays above where its line began. Lines that open inside a
    template literal or block comment are text. A call whose name is not a
    string literal (`it(name, ...)`) cannot be named, so it is reported, not
    dropped.
    """
    noise = _TsNoise()
    depth = 0
    scopes: list[tuple[str, int]] = []
    for lineno, line in enumerate(lines, 1):
        in_code = noise.in_code()
        code = noise.blank(line)
        while scopes and depth <= scopes[-1][1]:
            scopes.pop()
        if in_code:
            title = _citation(line, "//")
            if title is not None:
                yield "citation", title, lineno
            else:
                opener = _TS_CALL.match(code)
                call = None
                if opener and code[opener.end():].lstrip().startswith("("):
                    call = _ts_call(lines, lineno - 1)
                    if call is None:
                        _error(
                            graph, "PARSE_ERROR",
                            f"{path.name}: {opener.group(1)} call at line {lineno} "
                            "has no literal name; it is not read",
                            path, lineno, severity=Severity.WARNING,
                        )
                if call is not None:
                    callee, name, _, _ = call
                    if callee == "describe":
                        scopes.append((name, depth))
                    else:
                        yield "test", "::".join(
                            [s[0] for s in scopes] + [name]), lineno
        depth += code.count("{") - code.count("}")


def _ts_body(lines: list[str], test_line: int) -> str | None:
    """A TypeScript test's body, or None when its end cannot be found.

    The body is the callback's braces; a callback with none (`() => expr`)
    is the expression up to the call's closing parenthesis. Either way the
    walk starts past the test's name, counts only blanked code, and so
    ignores a `}` inside a string, template or comment.
    """
    noise = _TsNoise()
    paren = 1
    brace = 0
    started = False
    body: list[str] = []
    call = _ts_call(lines, test_line - 1)
    if call is None:
        return None
    _, _, name_line, name_end = call
    for index in range(test_line - 1, len(lines)):
        raw = lines[index]
        code = noise.blank(raw)
        if index < name_line:
            continue
        begin = name_end if index == name_line else 0
        piece_start = begin
        for column in range(begin, len(code)):
            char = code[column]
            if started:
                if char == "{":
                    brace += 1
                elif char == "}":
                    brace -= 1
                    if brace == 0:
                        body.append(raw[piece_start:column])
                        return "\n".join(body).strip()
                continue
            if char == "(":
                paren += 1
            elif char == ")":
                paren -= 1
                if paren == 0:
                    body.append(raw[piece_start:column])
                    text = "\n".join(body)
                    arrow = text.find("=>")
                    return text[arrow + 2:].strip() if arrow != -1 else ""
            elif char == "{" and paren == 1:
                started = True
                brace = 1
                body = []
                piece_start = column + 1
        body.append(raw[piece_start:])
    return None


_DIALECTS = {"rust": _scan_rust, "python": _scan_python,
             "typescript": _scan_typescript}


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
    # Absent or empty is a spec-only register: no test is read, none is missing.
    tests = paths.get("tests", [])
    if not isinstance(tests, list):
        raise ValueError(
            "profile 'adapter' section has adapter.paths.tests that is not a list"
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
    read = read_utf8_lines_granular(graph, path)
    if read is None:
        return
    lines, unread = read

    title: str | None = None
    title_line = 0
    scenarios = 0
    # Unreadable lines inside the current requirement: its section is the entry,
    # as a row is in a table, so one such line withholds the requirement whole.
    # Blanking the line instead would fold an unreadable heading's body and
    # scenarios into the requirement above it.
    section_unread: list[int] = []

    def emit() -> None:
        if title is None:
            return
        if section_unread:
            lines_named = ", ".join(str(n) for n in section_unread)
            _error(graph, "PARSE_ERROR",
                   f"the requirement at line {title_line} was not read: "
                   f"{'line' if len(section_unread) == 1 else 'lines'} "
                   f"{lines_named} in it {'is' if len(section_unread) == 1 else 'are'} not valid UTF-8", path, title_line)
            return
        attrs = {"title": title, "scenarios": scenarios}
        body = _requirement_body(lines, title_line)
        if body:
            attrs["body"] = body
        # Qualified by capability: a title alone is unique only by convention,
        # and two capabilities sharing one collapsed into a single node.
        node_id = f"{capability}/{title}"
        # The requirement's entry declares its `contains` edge, so a repeated
        # title's edge goes unread with it.
        index = graph.add_node(
            node_id, "requirement", attrs,
            Provenance(str(path), title_line),
        )
        graph.add_edge(capability, node_id, "contains",
                       Provenance(str(path), title_line), declared_by=index)

    # A scenario outside every requirement belongs to no node; count it against the
    # section heading it sits under so it is reported, not lost.
    section, section_line, stray = "", 0, 0

    def report_stray() -> None:
        if stray:
            noun = "scenario belongs" if stray == 1 else "scenarios belong"
            _error(graph, "PARSE_ERROR",
                   f"{stray} {noun} to no requirement under '{section}' "
                   "and went uncounted", path, section_line)

    for lineno, line in enumerate(lines, 1):
        m = _REQUIREMENT_HEADING.match(line)
        if m:
            emit()
            report_stray()
            title, title_line, scenarios = m.group(1), lineno, 0
            stray, section_unread = 0, []
        elif _BODY_TERMINATOR.match(line):
            emit()
            report_stray()
            title, section, section_line, stray = None, line.strip(), lineno, 0
            section_unread = []
        if lineno in unread and title is not None:
            section_unread.append(lineno)
        if m or _BODY_TERMINATOR.match(line):
            continue
        if _SCENARIO_HEADING.match(line):
            if title is None:
                stray += 1
            else:
                scenarios += 1
    emit()
    report_stray()


def _read_test_file(graph: DocumentBuilder, path: Path, target: Path,
                    dialect: str) -> int:
    """Emit test nodes and the `verifies` edges their citations declare.

    Return how many test nodes the file yielded, so the caller can tell a
    glob whose every file read as empty from one holding helper modules.

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
        return 0
    lines = text.split("\n")

    rel = path.relative_to(target).as_posix()
    cited: list[tuple[str, int]] = []
    found = 0
    for event, text, lineno in _DIALECTS[dialect](graph, path, lines):
        if event == "citation":
            if not (cited and lineno == cited[-1][1] + 1):
                cited = []
            cited.append((text, lineno))
            continue
        node_id = f"{rel}::{text}"
        attrs = {"file": rel, "function": text}
        if dialect in ("rust", "typescript"):
            body = (_rust_body if dialect == "rust" else _ts_body)(lines, lineno)
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
        index = graph.add_node(
            node_id, "test", attrs,
            Provenance(str(path), lineno),
        )
        for title, cite_line in cited:
            graph.add_edge(node_id, title, "verifies",
                           Provenance(str(path), cite_line), declared_by=index)
        found += 1
    return found


def _unlistable_dirs(base: Path, pattern: str) -> list[tuple[Path, OSError]]:
    """Every path `base.glob(pattern)` would need to read but cannot.

    `Path.glob` passes over a directory it may not read, so a register behind
    one would read as absent. This visits what glob visits, listing only where
    the pattern has a wildcard, and keeps the errors. A path that does not exist
    is not one: glob would find nothing there either.
    """
    found: list[tuple[Path, OSError]] = []
    parts = Path(pattern).parts

    def reach(path: Path, list_it: bool) -> list[os.DirEntry[str]] | None:
        try:
            if list_it:
                return list(os.scandir(path))
            os.stat(path)
        except (FileNotFoundError, NotADirectoryError):
            return None
        except OSError as error:
            found.append((path, error))
            return None
        return []

    def visit(directory: Path, depth: int) -> None:
        part = parts[depth]
        last = depth == len(parts) - 1
        if not any(c in part for c in "*?["):
            if last:
                reach(directory / part, list_it=False)
            else:
                visit(directory / part, depth + 1)
            return
        entries = reach(directory, list_it=True)
        if entries is None or last:
            return
        if part == "**":
            visit(directory, depth + 1)
        for entry in entries:
            if not entry.is_dir(follow_symlinks=part != "**"):
                continue
            if part == "**":
                visit(Path(entry.path), depth)
            elif fnmatch.fnmatchcase(entry.name, part):
                visit(Path(entry.path), depth + 1)

    visit(base, 0)
    return found


def _report_unlistable(graph: DocumentBuilder, base: Path, pattern: str) -> bool:
    """Report each directory the pattern could not list; True when there was one."""
    unlistable = _unlistable_dirs(base, pattern)
    for directory, error in unlistable:
        _error(
            graph, "PARSE_ERROR",
            f"could not read directory '{directory}': {error.strerror or error}",
            directory,
        )
    return bool(unlistable)


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
        unlistable = _report_unlistable(graph, spec_dir, "*/spec.md")
        spec_paths = sorted(spec_dir.glob("*/spec.md"))
        if not spec_paths and not unlistable:
            # A wrong spec_dir that exists must not read as a clean register
            # with nothing in it; an unlistable one is already reported.
            _error(
                graph, "PARSE_ERROR",
                f"spec directory '{spec_dir}' holds no '<capability>/spec.md' file",
                spec_dir, severity=Severity.WARNING,
            )
        for path in spec_paths:
            _read_spec_file(graph, path, path.parent.name)

    for pattern, dialect in config["tests"]:
        _report_unlistable(graph, target_path, pattern)
        paths = sorted(target_path.glob(pattern))
        if not paths:
            _error(
                graph, "PARSE_ERROR",
                f"test glob '{pattern}' matched no file",
                target_path, severity=Severity.WARNING,
            )
            continue
        found = sum(_read_test_file(graph, path, target_path, dialect)
                    for path in paths)
        if found == 0:
            # Per-file silence is normal (a helper module beside the tests);
            # a glob that matched files and read a test from none of them is
            # the wrong dialect, or the wrong glob, and must not read as a
            # register with nothing in it.
            _error(
                graph, "PARSE_ERROR",
                f"{len(paths)} file{'s' if len(paths) != 1 else ''} matched "
                f"test glob '{pattern}' and none held a {dialect} test item",
                target_path, severity=Severity.WARNING,
            )

    return graph


if __name__ == "__main__":
    from adapters._emit import run_main

    raise SystemExit(run_main(build_graph))

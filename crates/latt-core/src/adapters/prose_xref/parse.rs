use std::ops::Range;
use std::sync::LazyLock;

use latt_adapter::fence::FenceState;
use regex::Regex;

/// Compile a pattern that is a literal in this file; failure is a programming error.
fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("literal pattern compiles")
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(#{1,6})[ \t]+(.+?)\s*$"));
static TRAILING_HASHES: LazyLock<Regex> = LazyLock::new(|| pattern(r"[ \t]+#+[ \t]*$"));
/// Backtick-delimited path: `path/to/file.md`
static BACKTICK_PATH: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"`([^`\n]+\.md)`[ \t\x{A0}\x{202F}]*$"));
/// Markdown inline link to a file: [text](path/to/file.md), optionally with a
/// `#fragment` or a quoted title; the destination is the path.
static LINK_PATH: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"\[[^\]\n]*\]\(([^)\s#]+\.md)(?:#[^)\s]*)?(?:[ \t]+"[^"\n]*")?\)[ \t\x{A0}\x{202F}]*$"#,
    )
});
/// Bare path: path/to/file.md
///
/// Each path pattern allows a non-breaking space before the `§`, as typeset prose
/// writes one; matching only a plain space resolved the reference in the citing file.
static BARE_PATH: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"((?:(?:\.\.?|[\w.\-]+)/)*[\w.\-]+\.md)[ \t\x{A0}\x{202F}]*$"));
/// Between two headings of a `§§` list: a comma, `and` or `or`, or both.
static LIST_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^\s*(?:,\s*(?:(?:and|or)\s+)?|(?:and|or)\s+)"));
/// `,` `/` `and` `or` `see` `for`, brackets and dashes, and code spans between references.
static CONNECTOR: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"^[,/\s]*(?:\band\b|\bor\b|\bsee\b|\bfor\b)?[,/\s)(\-—]*(?:`[^`]+`[,/\s)(\-—]*)*$")
});

/// One-based line number for a zero-based line index, saturating past `u32::MAX`.
fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone)]
pub(crate) struct Heading {
    pub(crate) text: String,
    pub(crate) level: u32,
    pub(crate) line: u32,
    /// Path-qualified ID segment: `H1::H2::Leaf` built from the heading ancestry.
    pub(crate) path: String,
}

#[derive(Debug)]
pub(crate) struct XRef {
    pub(crate) kind: XRefKind,
    pub(crate) line: u32,
}

#[derive(Debug)]
pub(crate) enum XRefKind {
    Resolved {
        file: Option<String>,
        heading: String,
        /// When true, a bare § should inherit the preceding file target on the
        /// same line rather than defaulting to self-ref.
        carry_forward: bool,
        /// The path ending the previous line, when this § opens its line with no
        /// file of its own: a reflow split it from the file it cites. It never
        /// binds the reference; it explains a heading the citing file lacks.
        wrapped_file: Option<String>,
    },
    BareUndelimited {
        text: String,
    },
    /// A `§` with nothing after it but the end of the line or another `§`.
    Empty,
    /// A fence still open at the end of the file, at the line that opened it:
    /// everything after that line was read as code.
    UnclosedFence,
}

/// Collect all headings from a Markdown document, skipping fenced code.
pub(crate) fn collect_headings(text: &str) -> Vec<Heading> {
    let mut fences = FenceState::new();
    let mut headings = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let line_num = line_number(line_no);

        if fences.in_code(line, line_num) {
            continue;
        }

        if line.starts_with("    ") || line.starts_with('\t') {
            continue;
        }

        if let Some(caps) = HEADING.captures(line) {
            // `#{1,6}`: the length fits any integer type.
            let level = u32::try_from(caps[1].len()).unwrap_or(6);
            let text = TRAILING_HASHES.replace(&caps[2], "").trim().to_owned();
            headings.push(Heading {
                text,
                level,
                line: line_num,
                path: String::new(),
            });
        }
    }
    assign_heading_paths(&mut headings);
    headings
}

/// Ancestry-qualified path joined with `::` (never `/` — appears in corpus headings).
fn assign_heading_paths(headings: &mut [Heading]) {
    let mut stack: Vec<(u32, String)> = Vec::new();
    for heading in headings.iter_mut() {
        while let Some(top) = stack.last() {
            if top.0 >= heading.level {
                stack.pop();
            } else {
                break;
            }
        }
        let path = if stack.is_empty() {
            heading.text.clone()
        } else {
            let mut parts: Vec<&str> = stack.iter().map(|(_, t)| t.as_str()).collect();
            parts.push(&heading.text);
            parts.join("::")
        };
        heading.path = path;
        stack.push((heading.level, heading.text.clone()));
    }
}

/// Collect all § cross-references from a Markdown document.
pub(crate) fn collect_xrefs(text: &str) -> Vec<XRef> {
    let mut fences = FenceState::new();

    let mut xrefs = Vec::new();
    let mut previous: Option<&str> = None;

    for (line_no, line) in text.lines().enumerate() {
        let line_num = line_number(line_no);
        let above = previous.take();

        if fences.in_code(line, line_num) {
            continue;
        }
        previous = Some(line);

        // A § inside an inline code span documents the convention; it is not a reference.
        let spans = code_spans(line);
        let section_positions: Vec<usize> = line
            .char_indices()
            .filter(|&(i, c)| c == '§' && !spans.iter().any(|span| span.contains(&i)))
            .map(|(i, _)| i)
            .collect();

        // Track end of previous § `Heading` on this line for carry-forward.
        let mut prev_ref_end: Option<usize> = None;
        let mut prev_had_file = false;

        let mut positions = section_positions.iter().copied().peekable();
        while let Some(pos) = positions.next() {
            let mut after = pos + '§'.len_utf8();
            // `§§` is one plural marker, not an empty `§` before a second one.
            if positions.peek() == Some(&after) {
                positions.next();
                after += '§'.len_utf8();
            }
            let rest = &line[after..];

            // Check for backtick-delimited heading: § `Heading`
            if let Some((heading, heading_end_offset)) = parse_backtick_heading_with_end(rest) {
                let prefix = &line[..pos];
                let file = extract_preceding_path(prefix);
                let carry_forward = file.is_none()
                    && prev_had_file
                    && prev_ref_end.is_some_and(|end| is_connector_text(&line[end..pos]));
                // A local reference ends the chain: a connector after it must
                // not revive the file an earlier reference named.
                prev_had_file = file.is_some() || carry_forward;
                let wrapped_file = if file.is_none() && !carry_forward && prefix.trim().is_empty() {
                    above.and_then(extract_preceding_path)
                } else {
                    None
                };
                let mut end = after + heading_end_offset;
                let mut headings = vec![heading];
                if after - pos > '§'.len_utf8() {
                    while let Some((next, next_end)) = parse_listed_heading(&line[end..]) {
                        headings.push(next);
                        end += next_end;
                    }
                }
                prev_ref_end = Some(end);
                for heading in headings {
                    xrefs.push(XRef {
                        kind: XRefKind::Resolved {
                            file: file.clone(),
                            heading,
                            carry_forward,
                            wrapped_file: wrapped_file.clone(),
                        },
                        line: line_num,
                    });
                }
                continue;
            }

            // Bare § without backticks — grab text until next § or sentence boundary.
            let bare = extract_bare_text(rest);
            let kind = if bare.is_empty() {
                XRefKind::Empty
            } else {
                XRefKind::BareUndelimited { text: bare }
            };
            xrefs.push(XRef {
                kind,
                line: line_num,
            });
        }
    }
    if let Some(line) = fences.unclosed() {
        xrefs.push(XRef {
            kind: XRefKind::UnclosedFence,
            line,
        });
    }
    xrefs
}

/// Byte ranges of the line's inline code spans: a backtick run closes only at the
/// next run of the same length (CommonMark); a run with no partner is literal.
fn code_spans(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            runs.push(start..i);
        } else {
            i += 1;
        }
    }
    let mut spans = Vec::new();
    let mut open = 0;
    while open < runs.len() {
        let len = runs[open].len();
        match runs[open + 1..].iter().position(|run| run.len() == len) {
            Some(offset) => {
                let close = open + 1 + offset;
                spans.push(runs[open].start..runs[close].end);
                open = close + 1;
            }
            None => open += 1,
        }
    }
    spans
}

fn parse_backtick_heading_with_end(text: &str) -> Option<(String, usize)> {
    let leading = text.len() - text.trim_start().len();
    let trimmed = &text[leading..];
    if !trimmed.starts_with('`') {
        return None;
    }
    let inner = &trimmed[1..];
    let end = inner.find('`')?;
    let heading = inner[..end].to_owned();
    if heading.is_empty() {
        return None;
    }
    // Return offset past the closing backtick, relative to input text.
    Some((heading, leading + 1 + end + 1))
}

fn is_connector_text(text: &str) -> bool {
    let stripped = text.trim();
    if stripped.is_empty() {
        return true;
    }
    CONNECTOR.is_match(stripped)
}

/// Read the next heading of a `§§` list: `` , `B` ``, `` and `B` ``, `` , or `B` ``.
///
/// Every listed heading yields a reference, the second and later ones included;
/// read as prose, they would be lost with no finding.
fn parse_listed_heading(text: &str) -> Option<(String, usize)> {
    let separator = LIST_SEPARATOR.find(text)?;
    let (heading, end) = parse_backtick_heading_with_end(&text[separator.end()..])?;
    Some((heading, separator.end() + end))
}

fn extract_preceding_path(prefix: &str) -> Option<String> {
    BACKTICK_PATH
        .captures(prefix)
        .or_else(|| LINK_PATH.captures(prefix))
        .or_else(|| BARE_PATH.captures(prefix))
        .map(|caps| caps[1].to_owned())
}

fn extract_bare_text(text: &str) -> String {
    let trimmed = text.trim_start();
    let mut end = 0;
    for (i, c) in trimmed.char_indices() {
        if c == '§' || c == '\n' {
            break;
        }
        end = i + c.len_utf8();
    }
    let candidate = trimmed[..end].trim();
    // Strip trailing punctuation.
    candidate
        .trim_end_matches(|c: char| "),.;".contains(c))
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_skip_fenced_code() {
        let text = "# Real\n```\n## Fake\n```\n## Also Real\n";
        let headings = collect_headings(text);
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].text, "Real");
        assert_eq!(headings[1].text, "Also Real");
    }

    #[test]
    fn indented_lines_carry_references_but_not_headings() {
        let text = "- item\n\n    continues at § `Spaced`\n\tand § `Tabbed`\n    # Not a heading\n";
        let headings: Vec<_> = collect_xrefs(text)
            .into_iter()
            .map(|xref| match xref.kind {
                XRefKind::Resolved { heading, .. } => heading,
                other => panic!("expected Resolved, got {other:?}"),
            })
            .collect();
        assert_eq!(headings, ["Spaced", "Tabbed"]);
        assert!(collect_headings(text).is_empty());
    }

    #[test]
    fn backtick_delimited_xref() {
        let text = "See `foo.md` § `Bar Heading` for details.\n";
        let xrefs = collect_xrefs(text);
        assert_eq!(xrefs.len(), 1);
        match &xrefs[0].kind {
            XRefKind::Resolved { file, heading, .. } => {
                assert_eq!(file.as_deref(), Some("foo.md"));
                assert_eq!(heading, "Bar Heading");
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn wrapped_file_is_the_path_ending_the_previous_line() {
        for (text, expected) in [
            ("See `a.md`\n§ `X`.", Some("a.md")),
            ("See `a.md`\n  § `X`.", Some("a.md")),
            ("See [a](docs/a.md)\n§ `X`.", Some("docs/a.md")),
            ("See `a.md`, then\n§ `X`.", None),
            ("See `a.md`\nand § `X`.", None),
            ("See `a.md`\n\n§ `X`.", None),
        ] {
            let xrefs = collect_xrefs(text);
            let [
                XRef {
                    kind:
                        XRefKind::Resolved {
                            file, wrapped_file, ..
                        },
                    ..
                },
            ] = xrefs.as_slice()
            else {
                panic!("{text:?}: expected one resolved reference, got {xrefs:?}");
            };
            assert_eq!(file.as_deref(), None, "{text:?}");
            assert_eq!(wrapped_file.as_deref(), expected, "{text:?}");
        }
    }

    #[test]
    fn markdown_link_prefix_names_the_file() {
        for (line, expected) in [
            ("See [a](docs/a.md) § `X`.", Some("docs/a.md")),
            ("See [a](a.md#x) § `X`.", Some("a.md")),
            ("See [a](a.md \"A file\") § `X`.", Some("a.md")),
            ("See [a](https://x/a.html) § `X`.", None),
            ("See [a](a.md) then § `X`.", None),
        ] {
            let xrefs = collect_xrefs(line);
            let [
                XRef {
                    kind: XRefKind::Resolved { file, .. },
                    ..
                },
            ] = xrefs.as_slice()
            else {
                panic!("{line}: expected one resolved reference, got {xrefs:?}");
            };
            assert_eq!(file.as_deref(), expected, "{line}");
        }
    }

    #[test]
    fn a_non_breaking_space_before_the_sign_keeps_the_file() {
        for space in ['\u{a0}', '\u{202f}'] {
            for (line, expected) in [
                (format!("See `a.md`{space}§ `X`."), "a.md"),
                (format!("See [a](a.md){space}§ `X`."), "a.md"),
                (format!("See docs/a.md{space}§ `X`."), "docs/a.md"),
            ] {
                let xrefs = collect_xrefs(&line);
                let [
                    XRef {
                        kind: XRefKind::Resolved { file, .. },
                        ..
                    },
                ] = xrefs.as_slice()
                else {
                    panic!("{line}: expected one resolved reference, got {xrefs:?}");
                };
                assert_eq!(file.as_deref(), Some(expected), "{line}");
            }
        }
    }

    #[test]
    fn self_ref_xref() {
        let text = "See § `Local Heading` below.\n";
        let xrefs = collect_xrefs(text);
        assert_eq!(xrefs.len(), 1);
        match &xrefs[0].kind {
            XRefKind::Resolved { file, heading, .. } => {
                assert!(file.is_none());
                assert_eq!(heading, "Local Heading");
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn bare_undelimited_xref() {
        let text = "See § Bare Reference here.\n";
        let xrefs = collect_xrefs(text);
        assert_eq!(xrefs.len(), 1);
        match &xrefs[0].kind {
            XRefKind::BareUndelimited { text } => {
                assert_eq!(text, "Bare Reference here");
            }
            other => panic!("expected BareUndelimited, got {other:?}"),
        }
    }

    #[test]
    fn path_qualified_ids() {
        let text = "# Top\n## Parent A\n### Child\n## Parent B\n### Child\n";
        let headings = collect_headings(text);
        assert_eq!(headings.len(), 5);
        assert_eq!(headings[0].path, "Top");
        assert_eq!(headings[1].path, "Top::Parent A");
        assert_eq!(headings[2].path, "Top::Parent A::Child");
        assert_eq!(headings[3].path, "Top::Parent B");
        assert_eq!(headings[4].path, "Top::Parent B::Child");
    }

    #[test]
    fn path_qualified_skipped_levels() {
        let text = "## Outer\n#### Deep\n";
        let headings = collect_headings(text);
        assert_eq!(headings[0].path, "Outer");
        assert_eq!(headings[1].path, "Outer::Deep");
    }

    #[test]
    fn heading_strips_trailing_hashes() {
        let text = "## Heading With Trailing ## \n";
        let headings = collect_headings(text);
        assert_eq!(headings[0].text, "Heading With Trailing");
    }

    #[test]
    fn carry_forward_on_same_line() {
        let text = "See `foo.md` § `Act I` / § `Act II` here.\n";
        let xrefs = collect_xrefs(text);
        assert_eq!(xrefs.len(), 2);
        match &xrefs[0].kind {
            XRefKind::Resolved {
                file,
                carry_forward,
                ..
            } => {
                assert_eq!(file.as_deref(), Some("foo.md"));
                assert!(!carry_forward);
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
        match &xrefs[1].kind {
            XRefKind::Resolved {
                file,
                heading,
                carry_forward,
                ..
            } => {
                assert!(file.is_none());
                assert_eq!(heading, "Act II");
                assert!(*carry_forward);
            }
            other => panic!("expected Resolved with carry_forward, got {other:?}"),
        }
    }

    #[test]
    fn no_carry_forward_across_prose() {
        let text = "See `eco.md` § `Chemistry`. Garden nodes (§ `Local`) are important.\n";
        let xrefs = collect_xrefs(text);
        assert_eq!(xrefs.len(), 2);
        match &xrefs[1].kind {
            XRefKind::Resolved {
                file,
                carry_forward,
                ..
            } => {
                assert!(file.is_none());
                assert!(!carry_forward);
            }
            other => panic!("expected Resolved without carry_forward, got {other:?}"),
        }
    }

    #[test]
    fn no_carry_forward_revived_after_a_local_reference() {
        // The prose after `X` broke the chain; `Y` is local, so the connector
        // before `Z` carries the local context, not a.md's.
        let text = "See `a.md` § `X`. Locally see § `Y` and § `Z`.\n";
        let xrefs = collect_xrefs(text);
        assert_eq!(xrefs.len(), 3);
        let carried: Vec<bool> = xrefs
            .iter()
            .map(|x| match &x.kind {
                XRefKind::Resolved { carry_forward, .. } => *carry_forward,
                other => panic!("expected Resolved, got {other:?}"),
            })
            .collect();
        assert_eq!(carried, [false, false, false]);
    }

    fn headings_of(xrefs: &[XRef]) -> Vec<String> {
        xrefs
            .iter()
            .map(|x| match &x.kind {
                XRefKind::Resolved { heading, .. } => heading.clone(),
                XRefKind::BareUndelimited { text } => format!("bare:{text}"),
                XRefKind::Empty => "empty".into(),
                XRefKind::UnclosedFence => format!("unclosed@{}", x.line),
            })
            .collect()
    }

    #[test]
    fn section_sign_in_inline_code_is_ignored() {
        let text = "The marker is `§`; see § `Real` here.\n";
        assert_eq!(headings_of(&collect_xrefs(text)), ["Real"]);
    }

    #[test]
    fn whole_reference_in_inline_code_is_ignored() {
        let text = "Write `` see § `Heading` `` in prose.\n";
        assert!(collect_xrefs(text).is_empty());
    }

    #[test]
    fn double_backtick_span_then_reference() {
        let text = "``code with § inside`` and § `Real`\n";
        assert_eq!(headings_of(&collect_xrefs(text)), ["Real"]);
    }

    #[test]
    fn unmatched_backtick_is_literal() {
        let text = "See § `Real` and a stray ` here.\n";
        assert_eq!(headings_of(&collect_xrefs(text)), ["Real"]);
    }

    #[test]
    fn reference_without_space_resolves() {
        let text = "See §`NoSpace` here.\n";
        assert_eq!(headings_of(&collect_xrefs(text)), ["NoSpace"]);
    }

    #[test]
    fn empty_section_sign_is_reported() {
        assert_eq!(headings_of(&collect_xrefs("See §\n")), ["empty"]);
        assert_eq!(headings_of(&collect_xrefs("See §§\n")), ["empty"]);
        assert_eq!(headings_of(&collect_xrefs("See § §\n")), ["empty", "empty"]);
    }

    #[test]
    fn double_section_sign_reads_every_listed_heading() {
        for (line, expected) in [
            ("See §§ `A`\n", vec!["A"]),
            ("See §§ `A`, `B` and `C`.\n", vec!["A", "B", "C"]),
            ("See §§ `A`, `B`, or `C`\n", vec!["A", "B", "C"]),
            ("See §§`A`,`B`\n", vec!["A", "B"]),
            ("See §§ `A` then `B`\n", vec!["A"]),
            ("See §§ Bare\n", vec!["bare:Bare"]),
        ] {
            assert_eq!(headings_of(&collect_xrefs(line)), expected, "{line}");
        }
        let xrefs = collect_xrefs("See `x.md` §§ `A`, `B`\n");
        let files: Vec<_> = xrefs
            .iter()
            .map(|x| match &x.kind {
                XRefKind::Resolved { file, .. } => file.as_deref(),
                other => panic!("expected resolved, got {other:?}"),
            })
            .collect();
        assert_eq!(files, [Some("x.md"), Some("x.md")]);
    }

    /// Headings and references read outside fences; `In` marks fenced content.
    fn outside_fences(text: &str) -> (Vec<String>, Vec<String>) {
        let headings = collect_headings(text).into_iter().map(|h| h.text).collect();
        (headings, headings_of(&collect_xrefs(text)))
    }

    fn assert_fenced(text: &str) {
        let (headings, xrefs) = outside_fences(text);
        assert_eq!(headings, ["After"], "{text:?}");
        assert_eq!(xrefs, ["After"], "{text:?}");
    }

    #[test]
    fn fence_with_trailing_content_does_not_close() {
        for c in ["```", "~~~"] {
            assert_fenced(&format!(
                "{c}\n{c} not a close\n# In\nSee § `In`\n{c} \t\n# After\nSee § `After`\n"
            ));
        }
    }

    #[test]
    fn shorter_or_opposite_fence_does_not_close() {
        for (c, other) in [("`", "~"), ("~", "`")] {
            let open = c.repeat(4);
            let short = c.repeat(3);
            let opposite = other.repeat(4);
            assert_fenced(&format!(
                "{open}\n{short}\n{opposite}\n# In\nSee § `In`\n{open}\n# After\nSee § `After`\n"
            ));
        }
    }

    #[test]
    fn fence_indent_up_to_three_opens() {
        for c in ["```", "~~~"] {
            assert_fenced(&format!(
                "   {c}\n# In\nSee § `In`\n   {c}\n# After\nSee § `After`\n"
            ));
        }
    }

    #[test]
    fn fence_opens_and_closes_at_any_indentation() {
        for c in ["```", "~~~"] {
            for indent in ["    ", "      ", "\t"] {
                assert_fenced(&format!(
                    "- item\n\n{indent}{c}\n{indent}See § `In`\n{indent}{c}\n# After\nSee § `After`\n"
                ));
            }
        }
    }

    #[test]
    fn unclosed_fence_is_reported_where_it_opens() {
        let text = "# Top\nSee § `Top`\n\n    ```\n# In\nSee § `In`\n";
        assert_eq!(headings_of(&collect_xrefs(text)), ["Top", "unclosed@4"]);
        let closed = "```\nSee § `In`\n```\n";
        assert!(collect_xrefs(closed).is_empty());
    }
}

#[cfg(test)]
mod properties {
    use std::ops::Range;

    use proptest::prelude::*;

    use super::{XRefKind, code_spans, collect_xrefs};

    /// One inline code span: its delimiter run length and backtick-free content.
    #[derive(Debug, Clone)]
    struct Span {
        run: usize,
        content: String,
    }

    fn span() -> impl Strategy<Value = Span> {
        (1usize..=3, "[a-z0-9 §]{1,8}").prop_map(|(run, content)| Span { run, content })
    }

    /// Prose between spans: never empty, no backtick, so neighbouring delimiter runs stay apart.
    fn gap() -> impl Strategy<Value = String> {
        "[a-z0-9 ,.§]{1,8}"
    }

    /// Lay out spans with prose between them; return the line and where each span lands.
    fn compose(lead: &str, spans: &[Span], gaps: &[String]) -> (String, Vec<Range<usize>>) {
        let mut line = lead.to_owned();
        let mut expected = Vec::new();
        for (i, span) in spans.iter().enumerate() {
            if i > 0 {
                line.push_str(&gaps[i - 1]);
            }
            let start = line.len();
            let fence = "`".repeat(span.run);
            line.push_str(&fence);
            line.push_str(&span.content);
            line.push_str(&fence);
            expected.push(start..line.len());
        }
        (line, expected)
    }

    proptest! {
        /// Recover exactly the spans a line was composed from: each run closes at the next
        /// run of its own length and nothing between spans is read as code.
        #[test]
        fn composed_spans_are_recovered(
            lead in "[a-z0-9 ,.§]{0,8}",
            spans in prop::collection::vec(span(), 0..5),
            gaps in prop::collection::vec(gap(), 4),
            tail in "[a-z0-9 ,.§]{0,8}",
        ) {
            let (mut line, expected) = compose(&lead, &spans, &gaps);
            line.push_str(&tail);
            prop_assert_eq!(code_spans(&line), expected);
        }

        /// Return sorted, disjoint, in-bounds ranges that open and close on a backtick,
        /// for any mix of backticks and prose, including unmatched runs.
        #[test]
        fn spans_are_well_formed(line in "[`a §]{0,24}") {
            let spans = code_spans(&line);
            let bytes = line.as_bytes();
            let mut previous_end = 0;
            for span in &spans {
                prop_assert!(span.start >= previous_end, "{:?}: {:?}", line, spans);
                prop_assert!(span.end <= bytes.len(), "{:?}: {:?}", line, spans);
                prop_assert!(span.len() >= 2, "{:?}: {:?}", line, spans);
                prop_assert_eq!(bytes[span.start], b'`', "{:?}: {:?}", line, spans);
                prop_assert_eq!(bytes[span.end - 1], b'`', "{:?}: {:?}", line, spans);
                previous_end = span.end;
            }
        }

        /// Read every reference written between code spans and none written inside one:
        /// a `§` in code documents the convention, a `§` in prose cites a heading.
        #[test]
        fn references_outside_spans_are_the_ones_read(
            items in prop::collection::vec((span(), "[A-Za-z0-9]{1,8}"), 1..5),
        ) {
            let mut line = String::new();
            for (span, heading) in &items {
                let fence = "`".repeat(span.run);
                line.push_str(&format!("{fence}{}{fence} see § `{heading}` then ", span.content));
            }
            line.push('\n');
            let read: Vec<String> = collect_xrefs(&line)
                .into_iter()
                .map(|xref| match xref.kind {
                    XRefKind::Resolved { heading, .. } => heading,
                    other => panic!("{line:?}: expected Resolved, got {other:?}"),
                })
                .collect();
            let written: Vec<String> = items.iter().map(|(_, h)| h.clone()).collect();
            prop_assert_eq!(read, written);
        }
    }
}

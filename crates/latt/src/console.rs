//! Color decisions and the stdout/stderr writers.

use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use std::borrow::Cow;

use latt_core::output::{Format, sanitize_text, strip_ansi};

/// Set when a write to stdout failed, so `main` can override the exit code.
///
/// A report that did not reach its reader is not a result: exit 0 over a
/// truncated file would read as clean, and `println!` would panic with 101.
pub(super) static STDOUT_FAILED: AtomicBool = AtomicBool::new(false);
/// Set when a required diagnostic or scope line could not reach stderr.
pub(super) static STDERR_FAILED: AtomicBool = AtomicBool::new(false);

/// The environment that decides whether output may carry color.
pub(super) struct ColorEnv<'a> {
    no_color: Option<&'a OsStr>,
    force_color: Option<&'a OsStr>,
    term: Option<&'a OsStr>,
}

/// Set and non-empty, the rule <https://no-color.org> gives for `NO_COLOR`
/// and <https://force-color.org> for `FORCE_COLOR`.
fn is_set(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// True when output may carry color. `NO_COLOR` wins over `FORCE_COLOR` —
/// force-color.org's own order is the reverse, but no-color.org leaves
/// overriding a user's `NO_COLOR` to an argument or a configuration file.
/// `FORCE_COLOR` then wins over `TERM=dumb` and a stream
/// that is not a terminal.
///
/// None of these removes layout — they never change which format
/// [`auto_format`](crate::auto_format) picks.
pub(super) fn color_allowed(is_terminal: bool, env: &ColorEnv<'_>) -> bool {
    if is_set(env.no_color) {
        return false;
    }
    if is_set(env.force_color) {
        return true;
    }
    is_terminal && !dumb_terminal(env.term)
}

/// True when `TERM` names a terminal without cursor addressing.
pub(super) fn dumb_terminal(term: Option<&OsStr>) -> bool {
    term == Some(OsStr::new("dumb"))
}

/// Run `decide` over the process's color environment.
pub(super) fn with_color_env<T>(decide: impl FnOnce(&ColorEnv<'_>) -> T) -> T {
    let (no_color, force_color, term) = (
        std::env::var_os("NO_COLOR"),
        std::env::var_os("FORCE_COLOR"),
        std::env::var_os("TERM"),
    );
    decide(&ColorEnv {
        no_color: no_color.as_deref(),
        force_color: force_color.as_deref(),
        term: term.as_deref(),
    })
}

/// Write a rendered payload: strip ANSI from `rich` output when the stream is
/// not a terminal or `NO_COLOR` is set, then append the newline.
///
/// The stripping is not cosmetic: `rich` colors unconditionally, so writing the
/// rendered text straight out would differ from every captured baseline on
/// every colored line. `plain` and `json` carry no escape sequences of their
/// own and are written as rendered.
///
/// `plain` and `rich` pass through [`escape_controls`] first, a backstop for a
/// field a formatter left raw; `json` is written as rendered.
pub(super) fn echo(text: &str, format: Format, stream: Stream) {
    if format == Format::Json {
        return write_line(text, stream);
    }
    let is_terminal = match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    };
    let keep = if format == Format::Rich {
        RICH_SGR
    } else {
        &[]
    };
    let safe = escape_controls(text, keep);
    let stripped;
    let text = if format == Format::Rich && !with_color_env(|env| color_allowed(is_terminal, env)) {
        stripped = strip_ansi(&safe);
        stripped.as_str()
    } else {
        &safe
    };
    write_line(text, stream);
}

fn write_line(text: &str, stream: Stream) {
    match stream {
        Stream::Stdout => write_stdout(text),
        Stream::Stderr => write_stderr_raw(text),
    }
}

/// The SGR sequences `rich` emits: the severity colours and the reset
/// (latt-core `output/mod.rs`).
pub(super) const RICH_SGR: &[&str] = &[
    "\u{1b}[0m",
    "\u{1b}[31m",
    "\u{1b}[33m",
    "\u{1b}[35m",
    "\u{1b}[36m",
];

/// The SGR sequences clap's default styles emit: reset, bold, underline, red,
/// green and yellow.
pub(super) const CLAP_SGR: &[&str] = &[
    "\u{1b}[0m",
    "\u{1b}[1m",
    "\u{1b}[4m",
    "\u{1b}[31m",
    "\u{1b}[32m",
    "\u{1b}[33m",
];

/// Escape every control character in `text` but newline and tab, as
/// [`sanitize_text`] writes it; a sequence `keep` names stays as it is, so a
/// format's own colours pass and any other escape (conceal, `ESC [ 2 J`) shows.
///
/// One pass over the bytes; clean text is borrowed and nothing is allocated
/// until the first character that needs an escape.
pub(super) fn escape_controls<'a>(text: &'a str, keep: &[&str]) -> Cow<'a, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let width = match byte {
            b'\n' | b'\t' | 0x20..=0x7e => {
                at += 1;
                continue;
            }
            0x1b if let Some(kept) = keep
                .iter()
                .find(|seq| bytes[at..].starts_with(seq.as_bytes())) =>
            {
                at += kept.len();
                continue;
            }
            // ASCII controls and the lead bytes of every escaped non-ASCII
            // character (U+0080-009F, U+061C, U+200E-206F); whether it is
            // escaped is `sanitize_text`'s decision.
            0x00..=0x7f => 1,
            0xc2 | 0xd8 => 2,
            0xe2 => 3,
            _ => {
                at += 1;
                continue;
            }
        };
        let end = (at + width).min(bytes.len());
        if let Cow::Owned(escaped) = sanitize_text(&text[at..end]) {
            if out.capacity() == 0 {
                out.reserve(text.len() + 16);
            }
            out.push_str(&text[copied..at]);
            out.push_str(&escaped);
            copied = end;
        }
        at = end;
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// True for a character `serde_json` writes raw that a terminal acts on: DEL,
/// C1, the bidi controls and the line and paragraph separators.
fn is_raw_json_control(c: char) -> bool {
    matches!(
        c,
        '\u{7f}'..='\u{9f}'
            | '\u{61c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Write each character [`is_raw_json_control`] names as a JSON `\uXXXX`
/// escape. JSON allows a non-ASCII character only inside a string, so the
/// text parses to the same value; clean text is borrowed.
pub(super) fn escape_json_controls(text: &str) -> Cow<'_, str> {
    use std::fmt::Write as _;
    let Some(first) = text.find(is_raw_json_control) else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str(&text[..first]);
    for c in text[first..].chars() {
        if is_raw_json_control(c) {
            let _ = write!(out, "\\u{:04x}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// A writer that passes JSON text through [`escape_json_controls`], holding
/// back a character split across two writes until its last byte arrives.
///
/// Escaped output the inner writer has not yet taken waits in `staged`, and
/// only what the inner writer reports written leaves it, so a failed write
/// never sends a byte twice. A `write` that returns an error has consumed
/// nothing of its buffer; one that returns `Ok` has taken all of it, and a
/// failure delivering it surfaces on the next `write` or `flush`. A clean
/// chunk (nothing held back, nothing to escape) goes to the inner writer
/// without a copy; only what it does not take is staged.
pub(super) struct JsonControlEscape<W> {
    pub(super) inner: W,
    /// The start of a character whose last byte has not arrived.
    pending: Vec<u8>,
    /// Escaped bytes the inner writer has not taken.
    staged: Vec<u8>,
}

impl<W> JsonControlEscape<W> {
    pub(super) fn new(inner: W) -> Self {
        Self {
            inner,
            pending: Vec::new(),
            staged: Vec::new(),
        }
    }
}

impl<W: Write> JsonControlEscape<W> {
    /// Hand `staged` to the inner writer, dropping only what it took.
    fn drain(&mut self) -> std::io::Result<()> {
        let mut sent = 0;
        let result = loop {
            if sent == self.staged.len() {
                break Ok(());
            }
            match self.inner.write(&self.staged[sent..]) {
                Ok(0) => break Err(std::io::ErrorKind::WriteZero.into()),
                Ok(n) => sent += n,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => break Err(error),
            }
        };
        self.staged.drain(..sent);
        result
    }
}

impl<W: Write> Write for JsonControlEscape<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Clean text goes straight through: every character this escapes
        // starts with one of these bytes, and a chunk ending in ASCII splits
        // no character. The chunk is still taken whole: what the inner writer
        // leaves of it is staged, so no caller resends a split character.
        if self.pending.is_empty()
            && self.staged.is_empty()
            && buf.last().is_none_or(u8::is_ascii)
            && !buf.iter().any(|&b| matches!(b, 0x7f | 0xc2 | 0xd8 | 0xe2))
        {
            let sent = self.inner.write(buf)?;
            self.staged.extend_from_slice(&buf[sent..]);
            // As below: the rest goes now; a failure is reported by the next call.
            let _ = self.drain();
            return Ok(buf.len());
        }
        // An earlier write's output goes first; failing here takes nothing of `buf`.
        self.drain()?;
        self.pending.extend_from_slice(buf);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                self.staged
                    .extend_from_slice(escape_json_controls(text).as_bytes());
                self.pending.clear();
            }
            Err(error) if error.error_len().is_none() => {
                let ready = error.valid_up_to();
                // Valid up to `ready`, so this cannot fail.
                let text = std::str::from_utf8(&self.pending[..ready]).unwrap_or_default();
                self.staged
                    .extend_from_slice(escape_json_controls(text).as_bytes());
                self.pending.drain(..ready);
            }
            // Not UTF-8, so not JSON text: passed on as it came.
            Err(_) => self.staged.append(&mut self.pending),
        }
        // `buf` is taken; a failure here is reported by the next call.
        let _ = self.drain();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.drain()?;
        self.inner.flush()
    }
}

/// Print a clap parse error with the text it quotes from the command line
/// escaped as [`sanitize_text`] writes it, newline included, so an argument
/// cannot forge a line or reach the terminal raw.
///
/// clap still renders and prints it, so its layout, its color decision and
/// `--help` on stdout stay as they were. A value parser's message (the
/// error's source) should arrive escaped (`parse_filter_arg` in `main.rs`);
/// one that does not is escaped in the rendering, newline included. Then any
/// control in the rendering but clap's own colours ([`CLAP_SGR`]) sends the
/// escaped, uncolored rendering through [`write_stderr`] instead.
pub(super) fn print_clap_error(mut error: clap::Error) -> std::io::Result<()> {
    use clap::error::ContextValue;
    if !error.use_stderr() {
        return error.print();
    }
    let escaped: Vec<_> = error
        .context()
        .filter_map(|(kind, value)| match value {
            ContextValue::String(text) => match sanitize_text(text) {
                Cow::Owned(text) => Some((kind, ContextValue::String(text))),
                Cow::Borrowed(_) => None,
            },
            ContextValue::Strings(texts)
                if texts
                    .iter()
                    .any(|text| matches!(sanitize_text(text), Cow::Owned(_))) =>
            {
                let texts = texts
                    .iter()
                    .map(|text| sanitize_text(text).into_owned())
                    .collect();
                Some((kind, ContextValue::Strings(texts)))
            }
            _ => None,
        })
        .collect();
    for (kind, value) in escaped {
        error.insert(kind, value);
    }
    let mut rendered = error.render().ansi().to_string();
    let mut changed = false;
    // clap writes the source with `Display`, unstyled, so it appears verbatim.
    if let Some(source) = std::error::Error::source(&error).map(ToString::to_string)
        && !source.is_empty()
        && let Cow::Owned(escaped) = sanitize_text(&source)
    {
        rendered = rendered.replace(&source, &escaped);
        changed = true;
    }
    let safe = escape_controls(&rendered, CLAP_SGR);
    if changed || matches!(safe, Cow::Owned(_)) {
        write_stderr(strip_ansi(&safe).trim_end_matches('\n'));
        return Ok(());
    }
    error.print()
}

/// Write one line to stdout, recording a failure instead of panicking.
///
/// A closed pipe (`| head`) is the reader's choice and stays quiet; any other
/// failure, such as a full disk, is said on stderr.
pub(super) fn write_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(error) = writeln!(out, "{text}").and_then(|()| out.flush()) {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            write_stderr(&format!("Error: could not write output: {error}"));
        }
        STDOUT_FAILED.store(true, Ordering::Relaxed);
    }
}

/// Write one required line to stderr and record delivery failure.
///
/// Every control character but newline and tab is escaped: a notice can quote
/// a path, a revision or an adapter's own stderr. A newline stays, because an
/// adapter's multi-line diagnosis is quoted as it came. Under
/// [`RAW_STDERR`] the text is written as it is.
pub(super) fn write_stderr(text: &str) {
    if RAW_STDERR.load(Ordering::Relaxed) {
        return write_stderr_raw(text);
    }
    write_stderr_raw(&escape_controls(text, &[]));
}

/// Set for a `trace --format json` run by `fuse`, which quotes this process's
/// stderr in a json finding and escapes it in its own human output.
pub(super) static RAW_STDERR: AtomicBool = AtomicBool::new(false);

fn write_stderr_raw(text: &str) {
    let mut out = std::io::stderr().lock();
    if writeln!(out, "{text}").and_then(|()| out.flush()).is_err() {
        STDERR_FAILED.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
pub(super) enum Stream {
    Stdout,
    Stderr,
}

#[cfg(test)]
mod escape_tests {
    use std::borrow::Cow;
    use std::io::Write;

    use super::{CLAP_SGR, JsonControlEscape, RICH_SGR, escape_controls, escape_json_controls};

    #[test]
    fn clean_text_is_borrowed_and_our_sgr_kept() {
        for text in [
            "",
            "plain\ttext\nüñï €\u{2010}",
            "\u{1b}[31mERROR\u{1b}[0m x \u{1b}[33mW\u{1b}[36mI\u{1b}[35mH",
        ] {
            assert!(matches!(escape_controls(text, RICH_SGR), Cow::Borrowed(t) if t == text));
        }
        assert!(matches!(
            escape_controls("a\nb", &[]),
            Cow::Borrowed("a\nb")
        ));
    }

    #[test]
    fn every_other_control_is_escaped_newline_and_tab_stay() {
        let text = "\u{1b}[31mA\u{1b}]8;;x\u{7}\r\u{85}\u{61c}\u{202e}\u{2028}\u{2069}\nB\t\u{1b}[2J\u{7f}";
        assert_eq!(
            escape_controls(text, RICH_SGR),
            "\u{1b}[31mA\\u{1b}]8;;x\\u{7}\\r\\u{85}\\u{61c}\\u{202e}\\u{2028}\\u{2069}\nB\t\\u{1b}[2J\\u{7f}"
        );
        assert_eq!(escape_controls("\u{1b}[31mA", &[]), "\\u{1b}[31mA");
    }

    // The fast path hands `sanitize_text` only ASCII and the lead bytes
    // 0xc2, 0xd8 and 0xe2; a control added to the escaped set outside them
    // would pass it raw, and this walk would see the two disagree.
    #[test]
    fn every_char_is_escaped_as_sanitize_text_escapes_it() {
        for c in (0..=0x10_ffff).filter_map(char::from_u32) {
            if matches!(c, '\n' | '\t') {
                continue;
            }
            let text = c.to_string();
            assert_eq!(
                escape_controls(&text, &[]),
                latt_core::output::sanitize_text(&text),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }

    #[test]
    fn json_controls_become_json_escapes_and_parse_to_the_same_value() {
        let value = "a\u{7f}\u{85}\u{9f}\u{61c}\u{200e}\u{202e}\u{2028}\u{2029}\u{2069}é\u{a0}z";
        let json = serde_json::to_string(value).unwrap();
        let escaped = escape_json_controls(&json);
        assert_eq!(
            escaped,
            "\"a\\u007f\\u0085\\u009f\\u061c\\u200e\\u202e\\u2028\\u2029\\u2069é\u{a0}z\""
        );
        assert_eq!(serde_json::from_str::<String>(&escaped).unwrap(), value);
        assert!(matches!(
            escape_json_controls("\"clean é\""),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn rich_keeps_only_its_own_colours() {
        assert_eq!(
            escape_controls("\u{1b}[31mA\u{1b}[8mB\u{1b}[1mC\u{1b}[0m", RICH_SGR),
            "\u{1b}[31mA\\u{1b}[8mB\\u{1b}[1mC\u{1b}[0m"
        );
        assert_eq!(
            escape_controls("\u{1b}[1m\u{1b}[31merror:\u{1b}[0m \u{1b}[8m", CLAP_SGR),
            "\u{1b}[1m\u{1b}[31merror:\u{1b}[0m \\u{1b}[8m"
        );
    }

    /// Accepts `budget` bytes (in writes of at most `chunk`), fails once, then
    /// accepts everything.
    struct FailOnce {
        out: Vec<u8>,
        budget: Option<usize>,
        chunk: usize,
    }

    impl Write for FailOnce {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let take = match self.budget {
                Some(0) => {
                    self.budget = None;
                    return Err(std::io::Error::other("full"));
                }
                Some(left) => buf.len().min(left).min(self.chunk),
                None => buf.len().min(self.chunk),
            };
            if let Some(left) = &mut self.budget {
                *left -= take;
            }
            self.out.extend_from_slice(&buf[..take]);
            Ok(take)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // A failed inner write re-emits nothing the sink already took: the sink
    // ends with the escaped input exactly once, in order.
    #[test]
    fn the_json_writer_survives_a_failing_sink_without_repeating_bytes() {
        let (a, b) = ("\"a\u{202e}bcdef\"".as_bytes(), "\"g\u{85}h\"".as_bytes());
        for budget in 0..12 {
            for chunk in [1, 3, 64] {
                let mut out = JsonControlEscape::new(FailOnce {
                    out: Vec::new(),
                    budget: Some(budget),
                    chunk,
                });
                let mut written = 0;
                while written < a.len() {
                    if let Ok(n) = out.write(&a[written..]) {
                        written += n;
                    }
                }
                out.write_all(b).unwrap();
                out.flush().unwrap();
                assert_eq!(
                    String::from_utf8(out.inner.out).unwrap(),
                    "\"a\\u202ebcdef\"\"g\\u0085h\"",
                    "budget {budget} chunk {chunk}"
                );
            }
        }
    }

    #[test]
    fn the_json_writer_fed_one_byte_at_a_time_matches_one_write() {
        let text = "{\"id\":\"a\u{9b}\u{2028}é\u{202e}\u{1f600}\u{7f}\"}".as_bytes();
        let mut whole = JsonControlEscape::new(Vec::new());
        whole.write_all(text).unwrap();
        let mut bytes = JsonControlEscape::new(Vec::new());
        for byte in text {
            assert_eq!(bytes.write(std::slice::from_ref(byte)).unwrap(), 1);
        }
        bytes.flush().unwrap();
        whole.flush().unwrap();
        assert_eq!(bytes.inner, whole.inner);
        assert_eq!(
            String::from_utf8(whole.inner).unwrap(),
            "{\"id\":\"a\\u009b\\u2028é\\u202e\u{1f600}\\u007f\"}"
        );
    }

    #[test]
    fn the_json_writer_escapes_a_character_split_across_writes() {
        let text = "\"x\u{202e}y\u{85}\"".as_bytes();
        for split in 0..=text.len() {
            let mut out = JsonControlEscape::new(Vec::new());
            out.write_all(&text[..split]).unwrap();
            out.write_all(&text[split..]).unwrap();
            assert_eq!(
                String::from_utf8(out.inner).unwrap(),
                "\"x\\u202ey\\u0085\"",
                "split at {split}"
            );
        }
    }

    // The clean-chunk shortcut skips the escape for text holding none of its
    // lead bytes; every character must come out as `escape_json_controls`
    // writes it, so that list cannot fall behind the escape set.
    #[test]
    fn every_character_through_the_json_writer_matches_the_escape() {
        for c in (0..=0x10_ffff).filter_map(char::from_u32) {
            let text = format!("\"{c}\"");
            let mut out = JsonControlEscape::new(Vec::new());
            out.write_all(text.as_bytes()).unwrap();
            out.flush().unwrap();
            assert_eq!(
                String::from_utf8(out.inner).unwrap(),
                escape_json_controls(&text),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }

    // A sink that cuts a clean chunk inside `é` must not leave the next
    // chunk to start mid-character, where it would read as not UTF-8 and
    // pass its U+202E raw.
    #[test]
    fn a_short_write_inside_a_character_does_not_unescape_the_next_chunk() {
        struct TwoAtATime(Vec<u8>);
        impl Write for TwoAtATime {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let n = buf.len().min(2);
                self.0.extend_from_slice(&buf[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut out = JsonControlEscape::new(TwoAtATime(Vec::new()));
        let clean = "\"\u{e9}x".as_bytes();
        assert_eq!(out.write(clean).unwrap(), clean.len());
        // Delivered without a flush, as the escaping path delivers.
        assert_eq!(out.inner.0, clean);
        out.write_all("y\u{202e}\"".as_bytes()).unwrap();
        out.flush().unwrap();
        assert_eq!(
            String::from_utf8(out.inner.0).unwrap(),
            "\"\u{e9}xy\\u202e\""
        );
    }

    #[test]
    fn a_clean_chunk_on_a_short_writing_sink_arrives_whole() {
        struct ThreeAtATime(Vec<u8>);
        impl Write for ThreeAtATime {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let n = buf.len().min(3);
                self.0.extend_from_slice(&buf[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut out = JsonControlEscape::new(ThreeAtATime(Vec::new()));
        // Clean (the shortcut, cut mid-`é` by the sink), then one to escape.
        out.write_all("{\"id\":\"plain ascii é text\"}\n".as_bytes())
            .unwrap();
        out.write_all("\"a\u{9b}b\"".as_bytes()).unwrap();
        out.flush().unwrap();
        assert_eq!(
            String::from_utf8(out.inner.0).unwrap(),
            "{\"id\":\"plain ascii é text\"}\n\"a\\u009bb\""
        );
    }
}

#[cfg(test)]
mod color_tests {
    use std::ffi::OsStr;

    use super::{ColorEnv, color_allowed, dumb_terminal};

    fn env<'a>(
        no_color: Option<&'a str>,
        force_color: Option<&'a str>,
        term: Option<&'a str>,
    ) -> ColorEnv<'a> {
        ColorEnv {
            no_color: no_color.map(OsStr::new),
            force_color: force_color.map(OsStr::new),
            term: term.map(OsStr::new),
        }
    }

    #[test]
    fn a_terminal_gets_color_unless_no_color_is_set_and_non_empty() {
        assert!(color_allowed(true, &env(None, None, None)));
        assert!(color_allowed(true, &env(Some(""), None, None)));
        assert!(!color_allowed(true, &env(Some("1"), None, None)));
        assert!(!color_allowed(false, &env(None, None, None)));
        assert!(!color_allowed(false, &env(Some("1"), None, None)));
    }

    #[test]
    fn the_rungs_apply_in_order_no_color_force_color_dumb_terminal() {
        // FORCE_COLOR colors a pipe, and any non-empty value forces, 0 included.
        assert!(color_allowed(false, &env(None, Some("1"), None)));
        assert!(color_allowed(false, &env(None, Some("0"), None)));
        assert!(!color_allowed(false, &env(None, Some(""), None)));
        // NO_COLOR beats FORCE_COLOR: a deliberate divergence from force-color.org.
        assert!(!color_allowed(false, &env(Some("1"), Some("1"), None)));
        assert!(color_allowed(false, &env(Some(""), Some("1"), None)));
        // TERM=dumb removes color from a terminal; FORCE_COLOR overrides it.
        assert!(!color_allowed(true, &env(None, None, Some("dumb"))));
        assert!(color_allowed(true, &env(None, Some("1"), Some("dumb"))));
        assert!(color_allowed(
            true,
            &env(None, None, Some("xterm-256color"))
        ));
    }

    #[test]
    fn only_term_dumb_is_a_dumb_terminal() {
        assert!(dumb_terminal(Some(OsStr::new("dumb"))));
        assert!(!dumb_terminal(Some(OsStr::new("xterm"))));
        assert!(!dumb_terminal(Some(OsStr::new(""))));
        assert!(!dumb_terminal(None));
    }
}

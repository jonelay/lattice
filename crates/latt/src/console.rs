//! Color decisions and the stdout/stderr writers.

use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use latt_core::output::{Format, strip_ansi};

/// Set when a write to stdout failed, so `main` can override the exit code.
///
/// A report that did not reach its reader is not a result: exit 0 over a
/// truncated file would read as clean, and `println!` would panic with 101.
pub(super) static STDOUT_FAILED: AtomicBool = AtomicBool::new(false);

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
pub(super) fn echo(text: &str, format: Format, stream: Stream) {
    let is_terminal = match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    };
    let stripped;
    let text = if format == Format::Rich && !with_color_env(|env| color_allowed(is_terminal, env)) {
        stripped = strip_ansi(text);
        stripped.as_str()
    } else {
        text
    };
    match stream {
        Stream::Stdout => write_stdout(text),
        Stream::Stderr => {
            let _ = writeln!(std::io::stderr(), "{text}");
        }
    }
}

/// Write one line to stdout, recording a failure instead of panicking.
///
/// A closed pipe (`| head`) is the reader's choice and stays quiet; any other
/// failure, such as a full disk, is said on stderr.
pub(super) fn write_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(error) = writeln!(out, "{text}").and_then(|()| out.flush()) {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            let _ = writeln!(std::io::stderr(), "Error: could not write output: {error}");
        }
        STDOUT_FAILED.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
pub(super) enum Stream {
    Stdout,
    Stderr,
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

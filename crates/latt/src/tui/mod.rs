mod app;
#[cfg(all(test, feature = "native-adapters"))]
mod dispatch_tests;
mod git;
mod keymap;
mod owner_action;
#[cfg(all(test, feature = "native-adapters"))]
mod snapshot_tests;
mod ui;

pub use app::{App, Reload};

use std::ffi::OsStr;
use std::io::{self, stdout};
use std::process::{Command as ProcessCommand, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use latt_core::profile::ActionMode;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};

use app::{Action, OutputPane, OwnerRun};
use owner_action::{collect, describe_exit, owner_command, read_capped, run_detached};

/// The signals that end browse, caught as flags so the event loop leaves
/// through the terminal session's restore. `SIGINT` and `SIGQUIT` are kept
/// apart: the editor shares browse's terminal, so its Ctrl-C reaches browse
/// too, and like `system()` browse leaves that one to the editor.
struct Signals {
    terminate: Arc<AtomicBool>,
    interrupt: Arc<AtomicBool>,
}

impl Signals {
    fn register() -> io::Result<Self> {
        let signals = Self {
            terminate: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(AtomicBool::new(false)),
        };
        #[cfg(unix)]
        {
            use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
            for sig in [SIGTERM, SIGHUP] {
                signal_hook::flag::register(sig, Arc::clone(&signals.terminate))?;
            }
            for sig in [SIGINT, SIGQUIT] {
                signal_hook::flag::register(sig, Arc::clone(&signals.interrupt))?;
            }
        }
        Ok(signals)
    }

    fn received(&self) -> bool {
        self.terminate.load(Ordering::Relaxed) || self.interrupt.load(Ordering::Relaxed)
    }
}

/// Open `$EDITOR` at a target through [`suspend_and_run`], and return what the
/// status line says afterwards: nothing after a clean exit.
fn suspend_and_edit(target: &app::EditorTarget, signals: &Signals) -> Option<String> {
    let editor = std::env::var("EDITOR").unwrap_or_default();
    if editor.is_empty() {
        return None;
    }
    editor_outcome(
        &editor,
        suspend_and_run(editor_command(&editor, target), signals),
    )
}

/// The command that opens `editor` at `target`'s file and line.
fn editor_command(editor: &str, target: &app::EditorTarget) -> ProcessCommand {
    let mut cmd = ProcessCommand::new(editor);
    if let Some(line) = target.line {
        cmd.arg(format!("+{line}"));
    }
    cmd.arg(&target.file);
    cmd
}

/// Say how an editor run went when it did not exit 0: it could not start, it
/// exited non-zero, or a signal ended it.
fn editor_outcome(editor: &str, result: io::Result<ExitStatus>) -> Option<String> {
    match result {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!("{editor} {}", describe_exit(status))),
        Err(e) => Some(format!("{editor} could not start: {e}")),
    }
}

/// Leave the terminal to a program and take it back afterwards. Re-entry uses
/// the two crossterm calls directly rather than `ratatui::init()`, which would
/// chain one more panic hook on every round trip. A Ctrl-C meant for the
/// program reaches browse too; like `system()`, browse leaves it to the program.
fn suspend_and_run(mut cmd: ProcessCommand, signals: &Signals) -> io::Result<ExitStatus> {
    ratatui::restore();
    let status = cmd.status();
    signals.interrupt.store(false, Ordering::Relaxed);
    let _ = enable_raw_mode();
    let _ = execute!(stdout(), EnterAlternateScreen);
    status
}

/// What a capture run leaves for the event loop.
enum Captured {
    /// The program could not start; the status line says why.
    NotStarted,
    /// The user quit while it ran; it has been stopped.
    Quit,
    Pane(OutputPane),
}

enum CaptureEnd {
    Exited(ExitStatus),
    Stopped,
    Quit,
}

/// Run a capture action: stdin closed, output kept, `running…` drawn while
/// it waits. `Esc` stops the program; `Ctrl-C`, `Q` and a terminating
/// signal stop it and quit, so no program outlives browse.
fn run_capture(
    run: &OwnerRun,
    app: &mut App<'_>,
    terminal: &mut DefaultTerminal,
    signals: &Signals,
    draw: &impl Fn(&mut ratatui::Frame, &App<'_>),
) -> io::Result<Captured> {
    let mut cmd = owner_command(run);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            app.status_message = Some(format!("{} could not start: {e}", run.verb));
            return Ok(Captured::NotStarted);
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let out = read_capped(
        child.stdout.take().expect("stdout is piped"),
        Arc::clone(&cancel),
    );
    let err = read_capped(
        child.stderr.take().expect("stderr is piped"),
        Arc::clone(&cancel),
    );
    app.status_message = Some(format!("running {}… (Esc stops it)", run.verb));
    let waited = (|| -> io::Result<CaptureEnd> {
        loop {
            terminal.draw(|frame| draw(frame, app))?;
            if let Some(status) = child.try_wait()? {
                return Ok(CaptureEnd::Exited(status));
            }
            if signals.received() {
                return Ok(CaptureEnd::Quit);
            }
            if !event::poll(Duration::from_millis(50))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match (key.code, key.modifiers - KeyModifiers::SHIFT) {
                (KeyCode::Esc, KeyModifiers::NONE) => return Ok(CaptureEnd::Stopped),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('Q'), _) => {
                    return Ok(CaptureEnd::Quit);
                }
                _ => {}
            }
        }
    })();
    // Every way out but the program's own exit stops it, an I/O error
    // included: no program outlives browse.
    if !matches!(waited, Ok(CaptureEnd::Exited(_))) {
        let _ = child.kill();
        let _ = child.wait();
    }
    // The readers stop once the pipes hold nothing more: a descendant that
    // outlives the program and keeps them open must not hold the event loop.
    cancel.store(true, Ordering::Release);
    let end = waited?;
    let stdout = collect(out);
    let stderr = collect(err);
    app.status_message = None;
    let mut outcome = match end {
        CaptureEnd::Quit => return Ok(Captured::Quit),
        CaptureEnd::Stopped => "stopped".to_owned(),
        CaptureEnd::Exited(status) => describe_exit(status),
    };
    if stdout.failed.is_some() || stderr.failed.is_some() {
        outcome.push_str("; output incomplete");
    }
    Ok(Captured::Pane(OutputPane::new(
        format!("{} — {outcome}", run.display()),
        &stdout,
        &stderr,
    )))
}

/// How the event loop ended.
pub enum Outcome {
    Quit,
    Reload(Reload),
}

/// The terminal browse holds across reloads: entered once, restored on drop
/// whichever way browse ends.
pub struct Session {
    terminal: DefaultTerminal,
    signals: Signals,
    color: bool,
    ascii: bool,
}

impl Session {
    pub fn start(ascii: bool) -> io::Result<Self> {
        let signals = Signals::register()?;
        let terminal = ratatui::try_init()?;
        Ok(Self {
            terminal,
            signals,
            color: crate::console::with_color_env(|env| crate::console::color_allowed(true, env)),
            ascii,
        })
    }

    pub fn run(&mut self, app: &mut App<'_>) -> io::Result<Outcome> {
        event_loop(
            app,
            &mut self.terminal,
            &self.signals,
            self.color,
            self.ascii,
        )
    }
}

/// True when the character-type locale is UTF-8. The first of `LC_ALL`,
/// `LC_CTYPE` and `LANG` that is set and non-empty decides, as in POSIX; none
/// set is the `C` locale.
pub fn utf8_locale(lc_all: Option<&OsStr>, lc_ctype: Option<&OsStr>, lang: Option<&OsStr>) -> bool {
    [lc_all, lc_ctype, lang]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .is_some_and(|value| {
            let value = value.to_string_lossy().to_ascii_lowercase();
            value.contains("utf-8") || value.contains("utf8")
        })
}

impl Drop for Session {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

fn event_loop(
    app: &mut App<'_>,
    terminal: &mut DefaultTerminal,
    signals: &Signals,
    color: bool,
    ascii: bool,
) -> io::Result<Outcome> {
    let draw = |frame: &mut ratatui::Frame, app: &App<'_>| {
        ui::draw(frame, app);
        if !color {
            ui::strip_color(frame.buffer_mut());
        }
        if ascii {
            ui::ascii_glyphs(frame.buffer_mut());
        }
    };
    loop {
        app.poll_git_commits();
        terminal.draw(|frame| draw(frame, app))?;
        if signals.received() {
            return Ok(Outcome::Quit);
        }
        let tick = if app.git_fetch_pending() { 50 } else { 500 };
        if !event::poll(Duration::from_millis(tick))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match app.handle_key_event(key) {
            Action::Quit => return Ok(Outcome::Quit),
            Action::None => {}
            Action::OpenEditor(target) => {
                if let Some(message) = suspend_and_edit(&target, signals) {
                    app.status_message = Some(message);
                }
                terminal.clear()?;
            }
            Action::EditNotes(target) => {
                app.after_reload = suspend_and_edit(&target, signals);
                terminal.clear()?;
                app.status_message = Some("reloading…".to_owned());
                terminal.draw(|frame| draw(frame, app))?;
                return Ok(Outcome::Reload(Reload::All));
            }
            Action::RunOwner(run) => match run.mode {
                ActionMode::Interactive => {
                    let result = suspend_and_run(owner_command(&run), signals);
                    terminal.clear()?;
                    app.after_reload = Some(match result {
                        Ok(status) => format!("{} {}", run.verb, describe_exit(status)),
                        Err(e) => format!("{} could not start: {e}", run.verb),
                    });
                    app.status_message = Some("reloading…".to_owned());
                    terminal.draw(|frame| draw(frame, app))?;
                    return Ok(Outcome::Reload(Reload::Register));
                }
                ActionMode::Capture => match run_capture(&run, app, terminal, signals, &draw)? {
                    Captured::Quit => return Ok(Outcome::Quit),
                    Captured::Pane(pane) => app.output = Some(pane),
                    Captured::NotStarted => {}
                },
                ActionMode::Detached => app.status_message = Some(run_detached(&run)),
                _ => {
                    app.status_message = Some(format!(
                        "{} uses a run mode this browse does not support",
                        run.verb
                    ));
                }
            },
            Action::Reload(kind) => {
                // The load blocks; say so before it starts.
                app.status_message = Some("reloading…".to_owned());
                terminal.draw(|frame| draw(frame, app))?;
                return Ok(Outcome::Reload(kind));
            }
        }
    }
}

// Requirement: browse/Jump to source
#[cfg(all(test, unix))]
mod editor_tests {
    use super::app::EditorTarget;
    use super::{editor_command, editor_outcome};

    fn target() -> EditorTarget {
        EditorTarget {
            file: "reqs.md".to_owned(),
            line: std::num::NonZeroU32::new(3),
        }
    }

    #[test]
    fn an_editor_that_cannot_start_says_why() {
        let editor = "/nonexistent/editor";
        let message = editor_outcome(editor, editor_command(editor, &target()).status());
        let message = message.expect("a failed start is reported");
        assert!(
            message.starts_with("/nonexistent/editor could not start: "),
            "{message}"
        );
    }

    #[test]
    fn an_editor_exit_code_is_reported_unless_it_is_zero() {
        let dir = std::env::temp_dir().join(format!("lattice-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let failing = dir.join("failing-editor");
        crate::stub::write_stub(&failing, "#!/bin/sh\nexit 3\n");
        let clean = dir.join("clean-editor");
        crate::stub::write_stub(
            &clean,
            "#!/bin/sh\n[ \"$1\" = +3 ] && [ \"$2\" = reqs.md ]\n",
        );
        let run = |editor: &std::path::Path| {
            let editor = editor.to_str().unwrap();
            editor_outcome(editor, editor_command(editor, &target()).status())
        };
        assert_eq!(
            run(&failing),
            Some(format!("{} exited 3", failing.display()))
        );
        assert_eq!(run(&clean), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod locale_tests {
    use std::ffi::OsStr;

    use super::utf8_locale;

    #[test]
    fn the_first_set_locale_variable_decides() {
        let v = |s| Some(OsStr::new(s));
        assert!(utf8_locale(None, None, v("en_US.UTF-8")));
        assert!(utf8_locale(None, v("C.utf8"), v("C")));
        assert!(!utf8_locale(v("C"), v("en_US.UTF-8"), v("en_US.UTF-8")));
        assert!(!utf8_locale(None, None, v("POSIX")));
        assert!(!utf8_locale(None, None, None));
        // An empty variable counts as unset.
        assert!(utf8_locale(v(""), None, v("de_DE.UTF-8")));
    }
}

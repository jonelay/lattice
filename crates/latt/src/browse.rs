//! The `browse` command: the interactive TUI and its reload.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use latt_core::graph::LatticeGraph;
use latt_core::profile::Profile;

use super::console::dumb_terminal;
use super::loading::{load, load_graph, load_profile_from};
use super::{BrowseArgs, Common, Outcome, exit_for, resolve_glyphs, tui};

pub(super) fn run_browse(args: BrowseArgs) -> ExitCode {
    // Before anything is read or the terminal is touched: browse draws with
    // cursor addressing, which a dumb terminal lacks.
    if dumb_terminal(std::env::var_os("TERM").as_deref()) {
        let _ = writeln!(
            std::io::stderr(),
            "Error: browse needs a terminal with cursor addressing (TERM=dumb); \
             the other commands read line by line with --format plain"
        );
        return Outcome::CouldNotRun.into();
    }
    // Without a terminal on stdout the session cannot open, and the OS error it
    // would report names a device, not the cause.
    if !std::io::stdout().is_terminal() {
        let _ = writeln!(
            std::io::stderr(),
            "Error: browse needs a terminal, and stdout is not one; \
             the other commands read line by line with --format plain"
        );
        return Outcome::CouldNotRun.into();
    }
    let common = Common {
        profile: args.profile,
        profile_version: args.profile_version,
        adapter: args.adapter,
        target: args.target,
        format: None,
    };
    let (mut profile, mut graph) = match load(&common) {
        Ok(loaded) => loaded,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "Error: {message}");
            return Outcome::CouldNotRun.into();
        }
    };
    let locale_is_utf8 = tui::utf8_locale(
        std::env::var_os("LC_ALL").as_deref(),
        std::env::var_os("LC_CTYPE").as_deref(),
        std::env::var_os("LANG").as_deref(),
    );
    let ascii = resolve_glyphs(args.glyphs, locale_is_utf8);
    let mut session = match tui::Session::start(ascii) {
        Ok(session) => session,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "Error: TUI failed: {e}");
            return Outcome::CouldNotRun.into();
        }
    };
    let mut carried = None;
    let mut notice: Option<String> = None;
    let mut change: Option<(usize, usize)> = None;
    loop {
        let issues = latt_core::validate::validate(&graph, &profile, args.strict);
        if let Some((added, removed)) = change.take() {
            notice = Some(format!(
                "reloaded: {} nodes (+{added} -{removed}), {} findings",
                graph.iter_nodes().count(),
                issues.len()
            ));
        }
        let mut app = tui::App::new(&profile, &graph, issues, args.filters.clone());
        app.set_target_dir(&common.target);
        if let Some(state) = carried.take() {
            app.restore(state);
        }
        app.status_message = match (app.after_reload.take(), notice.take()) {
            (Some(action), Some(reload)) => Some(format!("{action}; {reload}")),
            (action, reload) => action.or(reload),
        };
        let outcome = match session.run(&mut app) {
            Ok(outcome) => outcome,
            Err(e) => {
                drop(session);
                let _ = writeln!(std::io::stderr(), "Error: TUI failed: {e}");
                return Outcome::CouldNotRun.into();
            }
        };
        let kind = match outcome {
            tui::Outcome::Quit => {
                let has_errors = app.has_errors;
                drop(session);
                return exit_for(has_errors);
            }
            tui::Outcome::Reload(kind) => kind,
        };
        carried = Some(app.into_carried());
        match reload(kind, &common, &profile) {
            Ok((new_profile, new_graph)) => {
                change = Some(node_change(&graph, &new_graph));
                if let Some(new_profile) = new_profile {
                    profile = new_profile;
                }
                graph = new_graph;
            }
            // The old register stays on screen: a failed reload is reported,
            // never a reason to show less.
            Err(message) => {
                notice = Some(format!(
                    "reload failed, showing the previous load: {}",
                    message.lines().collect::<Vec<_>>().join(" | ")
                ));
            }
        }
    }
}

/// Load what `kind` asks for; a new profile only for a full reload.
pub(super) fn reload(
    kind: tui::Reload,
    common: &Common,
    profile: &Profile,
) -> Result<(Option<Profile>, LatticeGraph), String> {
    match kind {
        tui::Reload::Register => Ok((None, load_graph(common, profile)?)),
        tui::Reload::All => {
            let profile = load_profile_from(common)?;
            let graph = load_graph(common, &profile)?;
            Ok((Some(profile), graph))
        }
    }
}

/// Node IDs added and removed between two loads.
fn node_change(old: &LatticeGraph, new: &LatticeGraph) -> (usize, usize) {
    let added = new.iter_nodes().filter(|n| !old.has_node(&n.id)).count();
    let removed = old.iter_nodes().filter(|n| !new.has_node(&n.id)).count();
    (added, removed)
}

#[cfg(test)]
mod reload_tests {
    use std::path::{Path, PathBuf};

    use super::{Common, load, node_change, reload, tui};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A scratch copy of `mini-toml` and its profile, so a test can edit both.
    fn scratch(name: &str) -> (PathBuf, Common) {
        let dir =
            std::env::temp_dir().join(format!("lattice-reload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let registers = dir.join("target/registers");
        std::fs::create_dir_all(&registers).unwrap();
        for entry in std::fs::read_dir(repo().join("tests/fixtures/mini-toml/registers")).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, registers.join(path.file_name().unwrap())).unwrap();
        }
        std::fs::copy(repo().join("profiles/toml.yaml"), dir.join("profile.yaml")).unwrap();
        let common = Common {
            profile: dir.join("profile.yaml"),
            profile_version: None,
            adapter: None,
            target: dir.join("target"),
            format: None,
        };
        (dir, common)
    }

    fn append(path: &Path, text: &str) {
        let mut body = std::fs::read_to_string(path).unwrap();
        body.push_str(text);
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn register_reload_sees_new_rows_and_keeps_the_profile() {
        let (dir, common) = scratch("register");
        let (profile, graph) = load(&common).unwrap();
        append(
            &dir.join("target/registers/alpha.toml"),
            "\n[[item]]\nid = \"three\"\n",
        );
        let profile_text = std::fs::read_to_string(&common.profile).unwrap();
        std::fs::write(&common.profile, profile_text.replace("1.1.0", "1.9.9")).unwrap();

        let (new_profile, new_graph) = reload(tui::Reload::Register, &common, &profile).unwrap();
        assert!(new_profile.is_none(), "r leaves the profile alone");
        assert_eq!(node_change(&graph, &new_graph), (1, 0));
    }

    #[test]
    fn full_reload_rereads_the_profile() {
        let (_dir, common) = scratch("all");
        let (profile, _) = load(&common).unwrap();
        let profile_text = std::fs::read_to_string(&common.profile).unwrap();
        std::fs::write(&common.profile, profile_text.replace("1.1.0", "1.9.9")).unwrap();

        let (new_profile, _) = reload(tui::Reload::All, &common, &profile).unwrap();
        assert_eq!(new_profile.unwrap().profile_version(), "1.9.9");
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_adapter_is_an_error_naming_its_stderr() {
        let (dir, mut common) = scratch("fail");
        let (profile, _) = load(&common).unwrap();
        let adapter = dir.join("broken-adapter");
        crate::stub::write_stub(&adapter, "#!/bin/sh\necho register is locked >&2\nexit 3\n");
        common.adapter = Some(adapter);

        let message = reload(tui::Reload::Register, &common, &profile).unwrap_err();
        assert!(message.contains("register is locked"), "{message}");
    }
}

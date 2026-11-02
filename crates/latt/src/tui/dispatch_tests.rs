//! Characterization of browse's key dispatch as it stands: every key from a
//! set of reachable states, recorded as the action returned and the state
//! fields it changed. The action-table refactor must reproduce
//! `browse.dispatch.txt` byte for byte; a deliberate key change shows up as
//! a diff of that file. The help tables and footer hints are pinned the same
//! way in `browse.help.txt`, since the table will generate both.
//!
//! Set `LATTICE_REGEN_BASELINES=1` to re-capture. `e` is left out: its result
//! depends on `$EDITOR`, and the `jump_to_source` tests in `app.rs` cover it.
//! Keys are written `C-x` (Control), `A-x` (Alt) and `S-X` (Shift).

use std::path::{Path, PathBuf};

#[path = "../../tests/common/regen.rs"]
mod regen;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{Action, App, Reload, SortMode};
use super::snapshot_tests::{make_app, mini_toml};

/// Each state is reached from a fresh app by pressing its keys in order.
const STATES: &[(&str, &[&str])] = &[
    ("nodes", &[]),
    ("nodes/detail", &["Enter"]),
    ("nodes/detail-focus", &["Enter", "Tab"]),
    (
        "nodes/followed-edge",
        &["Enter", "Tab", "j", "j", "j", "j", "j", "j", "Enter"],
    ),
    ("nodes/schema", &["S"]),
    ("findings", &["3"]),
    ("findings/jumped", &["3", "Enter"]),
    ("tree", &["2"]),
    ("tree/expanded", &["2", "Space"]),
    ("tree/stacked-detail-focus", &["2", "Space", "j", "Enter"]),
    (
        "tree/stacked-list-focus",
        &["2", "Space", "j", "Enter", "Tab"],
    ),
    ("provenance", &["4"]),
    ("provenance/stacked", &["4", "Enter"]),
    ("graph", &["5"]),
    ("graph/recentered", &["5", "g", "Enter"]),
    ("graph/tab-detail", &["5", "Tab"]),
    ("graph/searched", &["5", "/", "b", "e", "t", "a", "Enter"]),
    ("mode/search", &["/", "x"]),
    ("mode/search-tree", &["2", "/", "x"]),
    ("mode/search-prov", &["4", "/", "x"]),
    ("mode/search-graph", &["5", "/", "x"]),
    (
        "mode/search-clear",
        &["5", "/", "b", "e", "t", "a", "Enter", "/"],
    ),
    ("mode/kind-filter", &["f"]),
    ("mode/kind-filter-findings", &["3", "f"]),
    ("mode/kind-filter-tree", &["2", "f"]),
    ("provenance/f-refused", &["4", "f"]),
    ("mode/kind-filter-graph", &["5", "f"]),
    ("mode/edge-filter", &["2", "F"]),
    ("mode/edge-filter-prov", &["4", "F"]),
    ("mode/edge-filter-graph", &["5", "F"]),
    ("mode/help", &["?"]),
    ("overlay/preview", &["p"]),
];

const KEYS: &[&str] = &[
    "1", "2", "3", "4", "5", "q", "Q", "/", "n", "N", "p", "f", "F", "d", "c", "C", "g", "G", "s",
    "t", "h", "l", "H", "L", "j", "k", "Space", "-", "+", "=", "?", "r", "S", "o", ".", "Esc",
    "Enter", "Tab", "BS", "Left", "Right", "Up", "Down", "PgUp", "PgDn", "Home", "End", "z", "R",
    "C-c", "C-f", "C-d", "C-u", "A-d", "S-Q",
];

fn key(name: &str) -> KeyEvent {
    let (mods, name) = match name.split_once('-') {
        Some(("C", k)) => (KeyModifiers::CONTROL, k),
        Some(("A", k)) => (KeyModifiers::ALT, k),
        Some(("S", k)) => (KeyModifiers::SHIFT, k),
        _ => (KeyModifiers::NONE, name),
    };
    KeyEvent::new(code(name), mods)
}

fn code(name: &str) -> KeyCode {
    match name {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BS" => KeyCode::Backspace,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "PgUp" => KeyCode::PageUp,
        "PgDn" => KeyCode::PageDown,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "Space" => KeyCode::Char(' '),
        c => KeyCode::Char(c.chars().next().unwrap()),
    }
}

fn bits(v: &[bool]) -> String {
    v.iter().map(|&b| if b { '1' } else { '0' }).collect()
}

fn sort(s: SortMode) -> &'static str {
    match s {
        SortMode::KindOrder => "kind",
        SortMode::Severity => "severity",
    }
}

/// Every piece of state a key can change, as `(field, value)` pairs.
fn fingerprint(app: &App<'_>) -> Vec<(&'static str, String)> {
    let search = |s: &Option<regex::Regex>| s.as_ref().map(|r| r.as_str().to_string());
    let mut expanded: Vec<_> = app.tree_state.expanded.iter().copied().collect();
    expanded.sort_unstable();
    let (nl, fi, tr, pr, gr) = (
        &app.node_list_state,
        &app.findings_state,
        &app.tree_state,
        &app.provenance_state,
        &app.graph_state,
    );
    vec![
        ("view", format!("{:?}", app.top_view)),
        ("mode", format!("{:?}", app.input_mode)),
        ("buffer", format!("{:?}", app.input_buffer)),
        ("detail", app.show_detail.to_string()),
        ("focus", app.detail_focused.to_string()),
        ("detail.cursor", app.detail_cursor.to_string()),
        (
            "stack",
            format!(
                "{:?}",
                app.view_stack
                    .iter()
                    .map(|f| format!("{}<{:?}@{}", f.node_id, f.source_view, f.source_cursor))
                    .collect::<Vec<_>>()
            ),
        ),
        ("schema", app.show_schema.to_string()),
        ("schema.cursor", app.schema_cursor.to_string()),
        ("suppressed", app.show_suppressed.to_string()),
        ("status", format!("{:?}", app.status_message)),
        (
            "preview",
            format!(
                "{:?}",
                app.preview
                    .as_ref()
                    .map(|p| format!("{}+{}", p.file, p.scroll))
            ),
        ),
        ("nodes.cursor", nl.cursor.to_string()),
        ("nodes.sort", sort(nl.sort).into()),
        ("nodes.search", format!("{:?}", search(&nl.search))),
        ("nodes.kinds", bits(&nl.kind_filter)),
        ("nodes.filter_cursor", nl.filter_cursor.to_string()),
        ("findings.cursor", fi.cursor.to_string()),
        ("findings.sort", sort(fi.sort).into()),
        ("findings.search", format!("{:?}", search(&fi.search))),
        ("findings.kinds", bits(&fi.kind_filter)),
        ("findings.filter_cursor", fi.filter_cursor.to_string()),
        ("tree.root", format!("{:?}", tr.root)),
        ("tree.direction", format!("{:?}", tr.direction)),
        ("tree.expanded", format!("{expanded:?}")),
        ("tree.cursor", tr.cursor.to_string()),
        ("tree.search", format!("{:?}", search(&tr.search))),
        ("tree.kinds", bits(&tr.kind_filter)),
        ("tree.edges", bits(&tr.edge_kind_filter)),
        ("tree.filter_cursor", tr.filter_cursor.to_string()),
        ("tree.scc_collapse", tr.scc_collapse.to_string()),
        ("prov.root", format!("{:?}", pr.root)),
        ("prov.direction", format!("{:?}", pr.direction)),
        ("prov.cursor", pr.cursor.to_string()),
        ("prov.h_scroll", pr.h_scroll.to_string()),
        ("prov.search", format!("{:?}", search(&pr.search))),
        ("prov.kinds", bits(&pr.kind_filter)),
        ("prov.edges", bits(&pr.edge_kind_filter)),
        ("prov.filter_cursor", pr.filter_cursor.to_string()),
        ("graph.center", format!("{:?}", gr.center)),
        ("graph.radius", gr.depth_radius().to_string()),
        ("graph.direction", format!("{:?}", gr.direction)),
        ("graph.cursor", gr.cursor.to_string()),
        ("graph.history", format!("{:?}", gr.center_history)),
        ("graph.search", format!("{:?}", search(&gr.search))),
        ("graph.search_filter", gr.search_filter.to_string()),
        ("graph.kinds", bits(&gr.kind_filter)),
        ("graph.edges", bits(&gr.edge_kind_filter)),
        ("graph.filter_cursor", gr.filter_cursor.to_string()),
        ("graph.columns", format!("{:?}", gr.column_mode)),
    ]
}

fn action(a: &Action) -> String {
    match a {
        Action::None => "-".into(),
        Action::Quit => "QUIT".into(),
        Action::OpenEditor(p) => format!("EDITOR {p}"),
        Action::EditNotes(p) => format!("NOTES {p}"),
        Action::Reload(Reload::Register) => "RELOAD".into(),
        Action::Reload(Reload::All) => "RELOAD+P".into(),
        Action::RunOwner(run) => format!("OWNER {:?} {}", run.mode, run.display()),
    }
}

fn baseline(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn check_baseline(name: &str, got: &str) {
    let path = baseline(name);
    if regen::regen_baselines() {
        std::fs::write(&path, got).expect("baseline is writable");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("baseline is committed");
    assert!(
        got == want,
        "{name} differs from the committed baseline; a deliberate key change \
         re-captures with LATTICE_REGEN_BASELINES=1 and the diff is the review"
    );
}

#[test]
fn key_dispatch_matches_the_baseline() {
    let (profile, graph) = mini_toml();
    let mut out = String::new();
    for &(state, setup) in STATES {
        out.push_str(&format!("## {state}  [{}]\n", setup.join(" ")));
        for &k in KEYS {
            let mut app = make_app(&profile, &graph);
            for &s in setup {
                let before = fingerprint(&app);
                app.handle_key_event(key(s));
                // A setup key that changes nothing has lost its binding, and
                // the state below would silently be a different one.
                assert!(
                    fingerprint(&app) != before,
                    "{state}: setup key {s} is dead"
                );
            }
            let before = fingerprint(&app);
            let act = app.handle_key_event(key(k));
            let changes: Vec<String> = before
                .iter()
                .zip(fingerprint(&app))
                .filter(|((_, a), (_, b))| a != b)
                .map(|((field, a), (_, b))| format!("{field} {a} -> {b}"))
                .collect();
            let changes = if changes.is_empty() {
                "unchanged".to_string()
            } else {
                changes.join("; ")
            };
            out.push_str(&format!("{k:<6} {:<6} {changes}\n", action(&act)));
        }
        out.push('\n');
    }
    check_baseline("browse.dispatch.txt", &out);
}

#[test]
fn help_and_footer_match_the_baseline() {
    let (profile, graph) = mini_toml();
    let mut out = String::new();
    for &(state, setup) in &[
        ("nodes", &[][..]),
        ("nodes/schema", &["S"][..]),
        ("tree", &["2"][..]),
        ("findings", &["3"][..]),
        ("provenance", &["4"][..]),
        ("graph", &["5"][..]),
    ] {
        let mut app = make_app(&profile, &graph);
        for &s in setup {
            app.handle_key_event(key(s));
        }
        out.push_str(&format!("## {state}\nfooter: {}\n", app.view_hints()));
        for (section, bindings) in app.view_bindings() {
            out.push_str(&format!("[{section}]\n"));
            for b in bindings {
                out.push_str(&format!("  {:<24} {}\n", b.key, b.desc));
            }
        }
        out.push('\n');
    }
    check_baseline("browse.help.txt", &out);
}

/// Row order must stay free for help layout, so no key may fire two rows of
/// one layer in any characterized state.
#[test]
fn no_two_rows_fire_for_one_key() {
    use super::keymap::{self, Layer, Pat};

    let (profile, graph) = mini_toml();
    let mut probes: Vec<(KeyCode, KeyModifiers)> = KEYS
        .iter()
        .map(|&k| {
            let e = key(k);
            (e.code, e.modifiers)
        })
        .collect();
    for entry in keymap::table() {
        for pat in entry.keys {
            if let Pat::Key(code, mods) = *pat {
                probes.push((code, mods));
            }
        }
    }
    for &(state, setup) in STATES {
        let mut app = make_app(&profile, &graph);
        for &s in setup {
            app.handle_key_event(key(s));
        }
        for layer in [Layer::Always, keymap::layer(&app)] {
            for &(code, mods) in &probes {
                let fired: Vec<_> = keymap::matches(&app, layer, code, mods)
                    .map(|e| e.cmd)
                    .collect();
                assert!(
                    fired.len() <= 1,
                    "{state}: {code:?}+{mods:?} fires {fired:?}"
                );
            }
        }
    }
}

#[test]
fn key_table_is_well_formed() {
    use super::app::VIEWS;
    use super::keymap::{self, Cmd, Help, Pat, Section, Views};

    let rows = keymap::table();
    for (i, entry) in rows.iter().enumerate() {
        assert!(!entry.keys.is_empty(), "row {i} has no key");
        match entry.help {
            Help::Join => assert!(
                i > 0 && !matches!(rows[i - 1].help, Help::None),
                "row {i} joins nothing"
            ),
            Help::Row(Section::View(view), _) => assert!(
                match entry.views {
                    Views::All => true,
                    Views::Only(views) => views.contains(&view),
                },
                "row {i} is documented under a view it does not run in"
            ),
            _ => {}
        }
    }
    for &(view, _, _) in &VIEWS {
        for &(cmd, word) in keymap::footer(view) {
            assert!(
                !keymap::label_for(cmd, view).is_empty(),
                "{view:?} footer `{word}` names {cmd:?}, which has no documented row there"
            );
        }
    }
    for &(view, c, _) in &VIEWS {
        assert!(
            rows.iter().any(|e| e.cmd == Cmd::ShowView(view)
                && matches!(e.keys[0], Pat::Key(KeyCode::Char(k), KeyModifiers::NONE) if k == c)),
            "VIEWS binds {c} to {view:?} but the key table does not"
        );
    }
}

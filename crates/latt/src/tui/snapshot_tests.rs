//! Rendering tests: the node list, tree and graph views drawn over the
//! `mini-toml` fixture on a `TestBackend`, asserted row by row. The net under
//! the hand-rolled windowing and connector arithmetic.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use latt_core::document::ingest_document;
use latt_core::graph::LatticeGraph;
use latt_core::profile::{Profile, load_profile, resolved_document};
use latt_core::validate::validate;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::{Color, Modifier};

use super::app::App;
use super::ui;

const SCROLLBAR_GLYPHS: &[&str] = &["█", "║", "↑", "↓", "▲", "▼"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture() -> PathBuf {
    repo_root().join("tests/fixtures/mini-toml")
}

pub(super) fn mini_toml() -> (Profile, LatticeGraph) {
    let profile = load_profile(&repo_root().join("profiles/toml.yaml")).unwrap();
    let dir = std::env::temp_dir().join("lattice_tui_snapshot");
    std::fs::create_dir_all(&dir).unwrap();
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let resolved = dir.join(format!(
        "resolved-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&resolved, resolved_document(&profile).unwrap()).unwrap();
    let document = latt_core::adapters::toml::build_document_value(&resolved, &fixture()).unwrap();
    let graph = ingest_document(document).unwrap();
    (profile, graph)
}

pub(super) fn make_app<'a>(profile: &'a Profile, graph: &'a LatticeGraph) -> App<'a> {
    let issues = validate(graph, profile, false);
    let mut app = App::new(profile, graph, issues, Vec::new());
    app.set_target_dir(&fixture());
    app
}

struct Screen {
    rows: Vec<String>,
    reversed: Vec<Vec<bool>>,
}

impl Screen {
    fn dump(&self) -> String {
        self.rows.join("\n")
    }

    fn row_containing(&self, needle: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.contains(needle))
    }

    fn last_column_has_scrollbar(&self) -> bool {
        self.rows.iter().any(|r| {
            r.chars()
                .last()
                .is_some_and(|c| SCROLLBAR_GLYPHS.contains(&c.to_string().as_str()))
        })
    }
}

fn render(app: &mut App<'_>, width: u16, height: u16) -> Screen {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut rows = Vec::new();
    let mut reversed = Vec::new();
    for y in 0..height {
        let mut row = String::new();
        let mut flags = Vec::new();
        for x in 0..width {
            let cell = buffer.cell((x, y)).unwrap();
            row.push_str(cell.symbol());
            flags.push(cell.modifier.contains(Modifier::REVERSED));
        }
        rows.push(row);
        reversed.push(flags);
    }
    Screen { rows, reversed }
}

fn move_cursor_to(app: &mut App<'_>, id: &str) {
    let index = app
        .visible_nodes()
        .iter()
        .position(|(node_id, _, _)| *node_id == id)
        .unwrap_or_else(|| panic!("{id} not in the node list"));
    for _ in 0..index {
        app.handle_key(KeyCode::Char('j'));
    }
}

#[test]
fn node_list_renders_first_fixture_node_under_cursor() {
    // Requirement: browse/Node list view
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    let screen = render(&mut app, 80, 24);
    // Row 0 is the tab bar, row 1 the top border, row 2 the first node.
    // `profiles/toml.yaml` declares item before memo and register.
    assert!(
        screen.rows[2].starts_with("│[item] alpha/one"),
        "first content row:\n{}",
        screen.dump()
    );
    assert!(
        screen.reversed[2][1],
        "cursor row is not highlighted:\n{}",
        screen.dump()
    );
    assert!(!screen.reversed[3][1], "second row must not be highlighted");
}

#[test]
fn node_list_scrollbar_only_when_overflowing() {
    // Requirement: browse/Node list view
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    let total = app.visible_nodes().len();
    assert!(
        total > 12,
        "fixture has {total} nodes; the overflow case needs more"
    );

    // Tab bar, two borders and the status line take four rows.
    let fits = render(&mut app, 80, (total + 4) as u16);
    assert!(
        !fits.last_column_has_scrollbar(),
        "scrollbar drawn when everything fits:\n{}",
        fits.dump()
    );
    let overflow = render(&mut app, 80, 12);
    assert!(
        overflow.last_column_has_scrollbar(),
        "no scrollbar when the list overflows:\n{}",
        overflow.dump()
    );
    assert!(
        overflow.row_containing("alpha/one").is_none() || overflow.rows.len() == 12,
        "window must be clipped to the terminal"
    );
}

#[test]
fn scrolled_node_list_fills_its_window_with_the_rows_around_the_cursor() {
    // Requirement: browse/Node list view
    let (profile, graph) = mini_toml();
    let ids: Vec<String> = make_app(&profile, &graph)
        .visible_nodes()
        .iter()
        .map(|(id, _, _)| (*id).to_owned())
        .collect();
    assert_eq!(
        ids.len(),
        21,
        "the offsets below are for mini-toml's 21 nodes"
    );
    // At height 12 the window is rows 2..10: the tab bar, two borders and the
    // status line take four. The cursor sits mid-window, and the window stops at
    // the list's end.
    let window = 8;
    for (cursor, offset) in [(9, 5), (20, 13)] {
        let mut app = make_app(&profile, &graph);
        move_cursor_to(&mut app, &ids[cursor]);
        let screen = render(&mut app, 80, 12);
        let shown: Vec<&str> = screen.rows[2..2 + window]
            .iter()
            .map(|row| {
                let start = row.find("] ").map_or(row.len(), |at| at + 2);
                row[start..].split([' ', '│']).next().unwrap_or("")
            })
            .collect();
        assert_eq!(
            shown,
            ids[offset..offset + window]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "cursor {cursor}: every window row, in order, from {offset}:\n{}",
            screen.dump()
        );
        let highlighted: Vec<usize> = (0..window)
            .filter(|row| screen.reversed[2 + row][1])
            .collect();
        assert_eq!(
            highlighted,
            [cursor - offset],
            "cursor {cursor}:\n{}",
            screen.dump()
        );
    }
}

#[test]
fn tree_renders_root_and_connector_row() {
    // Requirement: browse-tree/Tree view activation and root selection
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    move_cursor_to(&mut app, "alpha/two");
    app.handle_key(KeyCode::Char('2'));
    app.handle_key(KeyCode::Char(' '));
    let screen = render(&mut app, 80, 24);
    assert!(
        screen.rows[2].starts_with("│▼ alpha/two"),
        "root row:\n{}",
        screen.dump()
    );
    assert!(
        screen.rows[3].starts_with("│  ▶ [references] ")
            && (screen.rows[3].contains("alpha/one") || screen.rows[3].contains("beta/two")),
        "connector row:\n{}",
        screen.dump()
    );
}

#[test]
fn graph_renders_center_and_forward_neighbor() {
    // Requirement: browse-map/Graph view activation
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    move_cursor_to(&mut app, "alpha/two");
    app.handle_key(KeyCode::Char('5'));
    let screen = render(&mut app, 80, 24);
    let center = screen
        .row_containing("●alpha/two")
        .unwrap_or_else(|| panic!("center node missing:\n{}", screen.dump()));
    let neighbor = screen
        .row_containing("beta/two")
        .unwrap_or_else(|| panic!("forward neighbor missing:\n{}", screen.dump()));
    assert!(
        neighbor > center,
        "forward neighbor should be on a later row (center {center}, neighbor {neighbor}):\n{}",
        screen.dump()
    );
}

#[test]
fn help_overlay_scrolls_within_its_content() {
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    app.handle_key(KeyCode::Char('?'));
    let top = render(&mut app, 80, 16);
    assert!(top.row_containing("Global").is_some(), "{}", top.dump());

    app.handle_key(KeyCode::Char('G'));
    let bottom = render(&mut app, 80, 16);
    assert!(
        bottom.row_containing("Global").is_none(),
        "{}",
        bottom.dump()
    );
    // Requirement: browse/Accessibility baseline — the overlay ends at the screen-reader path.
    for format in ["--format plain", "--format json"] {
        assert!(
            bottom.row_containing(format).is_some(),
            "{format}\n{}",
            bottom.dump()
        );
    }
    app.handle_key(KeyCode::Char('j'));
    assert_eq!(
        render(&mut app, 80, 16).dump(),
        bottom.dump(),
        "scroll is clamped"
    );

    app.handle_key(KeyCode::Char('g'));
    assert_eq!(render(&mut app, 80, 16).dump(), top.dump());
    app.handle_key(KeyCode::Char('x'));
    assert!(render(&mut app, 80, 16).row_containing("Global").is_none());
}

#[test]
fn no_color_frame_keeps_every_meaning_in_modifiers() {
    // Requirement: browse/Accessibility baseline
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    app.show_detail = true;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| {
            ui::draw(frame, &app);
            ui::strip_color(frame.buffer_mut());
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let cell = |x: u16, y: u16| buffer.cell((x, y)).unwrap();
    for y in 0..24 {
        for x in 0..80 {
            assert_eq!((cell(x, y).fg, cell(x, y).bg), (Color::Reset, Color::Reset));
        }
    }
    // The tab bar is reversed; the active tab (first) is not, and is bold.
    assert!(!cell(1, 0).modifier.contains(Modifier::REVERSED));
    assert!(cell(1, 0).modifier.contains(Modifier::BOLD));
    assert!(cell(79, 0).modifier.contains(Modifier::REVERSED));
    // The cursor row is reversed.
    assert!(cell(1, 2).modifier.contains(Modifier::REVERSED));
    // Focus is a weight: the focused node list's border is bold, the detail pane's dim.
    assert!(cell(0, 1).modifier.contains(Modifier::BOLD));
    let detail_top = (2..24)
        .find(|&y| cell(0, y).symbol() == "┌")
        .expect("a detail pane below the node list");
    assert!(cell(0, detail_top).modifier.contains(Modifier::DIM));
    // The status bar is reversed.
    assert!(cell(0, 23).modifier.contains(Modifier::REVERSED));

    app.handle_key(KeyCode::Char('/'));
    for ch in "two".chars() {
        app.handle_key(KeyCode::Char(ch));
    }
    app.handle_key(KeyCode::Enter);
    let screen = render(&mut app, 80, 24);
    // The cursor moved to alpha/two; beta/two is the match off the cursor.
    let row = screen
        .row_containing("beta/two")
        .unwrap_or_else(|| panic!("{}", screen.dump())) as u16;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let matched = terminal.backend().buffer().cell((1, row)).unwrap();
    // A search match off the cursor is underlined, not only colored.
    assert!(
        matched.modifier.contains(Modifier::UNDERLINED),
        "{}",
        screen.dump()
    );
}

#[test]
fn ascii_frame_keeps_layout_with_ascii_glyphs() {
    // Requirement: browse/Accessibility baseline
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    app.show_detail = true;
    let unicode = render(&mut app, 80, 24);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| {
            ui::draw(frame, &app);
            ui::ascii_glyphs(frame.buffer_mut());
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    for y in 0..24 {
        let row: String = (0..80)
            .map(|x| buffer.cell((x, y)).unwrap().symbol())
            .collect();
        assert!(row.is_ascii(), "row {y}: {row}");
        // One cell per glyph: the ASCII frame lines up with the Unicode one.
        assert_eq!(
            row.len(),
            unicode.rows[y as usize].chars().count(),
            "row {y}"
        );
    }
    let top: String = (0..80)
        .map(|x| buffer.cell((x, 1)).unwrap().symbol())
        .collect();
    assert!(top.starts_with("+ Nodes ---"), "{top}");
}

/// The terminal cursor after drawing `app`, with the buffer it was drawn over;
/// `None` when the frame hid it.
fn draw_cursor(
    app: &App<'_>,
    width: u16,
    height: u16,
) -> (Option<(u16, u16)>, ratatui::buffer::Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let backend = terminal.backend();
    let position = backend.cursor_position();
    let cursor = backend.cursor_visible().then_some((position.x, position.y));
    (cursor, backend.buffer().clone())
}

/// Assert the cursor is shown on a reversed (selected) cell; return it.
fn assert_cursor_on_selection(app: &App<'_>, what: &str) -> (u16, u16) {
    let (cursor, buffer) = draw_cursor(app, 80, 24);
    let (x, y) = cursor.unwrap_or_else(|| panic!("{what}: the cursor is hidden"));
    assert!(
        buffer
            .cell((x, y))
            .unwrap()
            .modifier
            .contains(Modifier::REVERSED),
        "{what}: the cursor at ({x}, {y}) is not on the highlighted selection"
    );
    (x, y)
}

#[test]
fn cursor_follows_the_selection() {
    // Requirement: browse/Accessibility baseline — terminal cursor.
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);

    let (x0, y0) = assert_cursor_on_selection(&app, "node list");
    assert_eq!((x0, y0), (1, 2), "first inner cell of the first row");
    app.handle_key(KeyCode::Char('j'));
    app.handle_key(KeyCode::Char('j'));
    assert_eq!(
        assert_cursor_on_selection(&app, "node list after j j"),
        (1, 4)
    );

    app.show_detail = true;
    app.handle_key(KeyCode::Tab);
    let (_, y) = assert_cursor_on_selection(&app, "focused detail pane");
    assert!(
        y > 12,
        "the cursor left the node list for the detail pane: y = {y}"
    );
    app.handle_key(KeyCode::Tab);
    app.show_detail = false;

    app.handle_key(KeyCode::Char('S'));
    assert_cursor_on_selection(&app, "schema");
    app.handle_key(KeyCode::Char('S'));

    for (key, view) in [
        ('2', "tree"),
        ('3', "findings"),
        ('4', "provenance"),
        ('5', "graph"),
    ] {
        app.handle_key(KeyCode::Char(key));
        assert_cursor_on_selection(&app, view);
    }
    for mode in ["graph columns 1", "graph columns 2", "graph columns 3"] {
        app.handle_key(KeyCode::Char('C'));
        assert_cursor_on_selection(&app, mode);
    }

    app.handle_key(KeyCode::Char('F'));
    assert_cursor_on_selection(&app, "edge-kind picker");
    app.handle_key(KeyCode::Esc);
    app.handle_key(KeyCode::Char('1'));
    app.handle_key(KeyCode::Char('f'));
    assert_cursor_on_selection(&app, "kind picker");
    app.handle_key(KeyCode::Esc);

    // The search prompt: after the typed text, on the status line.
    app.handle_key(KeyCode::Char('/'));
    app.handle_key(KeyCode::Char('a'));
    let (cursor, buffer) = draw_cursor(&app, 80, 24);
    let (x, y) = cursor.expect("the cursor is shown in the search prompt");
    assert_eq!(y, 23, "on the status line");
    let line: String = (0..x)
        .map(|c| buffer.cell((c, y)).unwrap().symbol())
        .collect();
    assert!(
        line.trim_end().ends_with('a'),
        "after the typed text: {line:?}"
    );
    app.handle_key(KeyCode::Esc);

    // Help and preview: the first content cell, inside the popup's corner.
    for (open, close, what) in [('?', KeyCode::Esc, "help"), ('p', KeyCode::Esc, "preview")] {
        app.handle_key(KeyCode::Char(open));
        let (cursor, buffer) = draw_cursor(&app, 80, 24);
        let (x, y) = cursor.unwrap_or_else(|| panic!("{what}: the cursor is hidden"));
        assert_eq!(buffer.cell((x - 1, y - 1)).unwrap().symbol(), "┌", "{what}");
        app.handle_key(close);
    }
}

#[test]
fn cursor_stays_inside_a_tiny_frame() {
    // Requirement: browse/Accessibility baseline — terminal cursor.
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    app.handle_key(KeyCode::Char('5'));
    app.handle_key(KeyCode::Char('/'));
    for ch in "no-such-node".chars() {
        app.handle_key(KeyCode::Char(ch));
    }
    app.handle_key(KeyCode::Enter);
    app.handle_key(KeyCode::Char('&'));
    let (cursor, buffer) = draw_cursor(&app, 80, 24);
    let (x, y) = cursor.expect("the cursor is shown in an empty pane");
    assert_eq!(
        buffer.cell((x - 1, y - 1)).unwrap().symbol(),
        "┌",
        "first inner cell"
    );

    let mut app = make_app(&profile, &graph);
    for key in ['j', '5', '2', '?'] {
        app.handle_key(KeyCode::Char(key));
        let (cursor, _) = draw_cursor(&app, 20, 5);
        let (x, y) = cursor.expect("the cursor is shown in a tiny frame");
        assert!(
            x < 20 && y < 5,
            "({x}, {y}) is outside a 20x5 frame after {key}"
        );
    }
}

/// A register where REQ-002 is reachable from REQ-001 only through the
/// undeclared GHOST, whose `refs` edges are cross-source, and where the allowed
/// `satisfies` triple has no edges.
fn tainted_register() -> (Profile, LatticeGraph) {
    let dir = std::env::temp_dir().join("lattice_tui_snapshot");
    std::fs::create_dir_all(&dir).unwrap();
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = dir.join(format!(
        "tainted-{}-{}.yaml",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(
        &path,
        r#"
name: tainted
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: '^REQ-\d+$'
    summary_attr: title
    attrs:
      title: { type: string }
  spec:
    id_pattern: '^SPEC-\d+$'
edge_kinds:
  refs:
    allowed: [[req, req]]
    cross_source: true
  satisfies:
    allowed: [[spec, req]]
"#,
    )
    .unwrap();
    let profile = load_profile(&path).unwrap();
    let graph = ingest_document(serde_json::json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "REQ-001", "kind": "req", "attrs": {"title": "Start"},
             "provenance": {"file": "reqs.md", "line": 1}},
            {"id": "REQ-002", "kind": "req", "attrs": {"title": "Behind a ghost"},
             "provenance": {"file": "reqs.md", "line": 5}}
        ],
        "edges": [
            {"src": "REQ-001", "tgt": "GHOST", "kind": "refs",
             "provenance": {"file": "reqs.md", "line": 2}},
            {"src": "GHOST", "tgt": "REQ-002", "kind": "refs",
             "provenance": {"file": "reqs.md", "line": 3}}
        ]
    }))
    .unwrap();
    (profile, graph)
}

/// Glyphs that may be dimmed: connector lanes, borders and scrollbars. A dimmed
/// cell holding anything else is dimmed text.
const STRUCTURAL: &[&str] = &[
    " ", "│", "─", "┼", "├", "┤", "┌", "┐", "└", "┘", "╱", "╲", "▲", "▼", "█", "║",
];

/// Render `app` and return the screen text and every dimmed non-structural cell.
fn dimmed_text(app: &App<'_>, width: u16, height: u16) -> (String, Vec<String>, usize) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut screen = String::new();
    let mut violations = Vec::new();
    let mut dimmed = 0;
    for y in 0..height {
        for x in 0..width {
            let cell = buffer.cell((x, y)).unwrap();
            screen.push_str(cell.symbol());
            if cell.modifier.contains(Modifier::DIM) {
                dimmed += 1;
                if !STRUCTURAL.contains(&cell.symbol()) {
                    violations.push(format!("({x}, {y}) {:?}", cell.symbol()));
                }
            }
        }
        screen.push('\n');
    }
    (screen, violations, dimmed)
}

#[test]
fn no_text_is_dim() {
    // Requirement: browse/Accessibility baseline — dim; browse-map/Vacancy rendering.
    let (profile, graph) = mini_toml();
    let mut app = make_app(&profile, &graph);
    app.show_detail = true;
    let mut seen = String::new();
    let mut dimmed_total = 0;
    let mut check = |app: &App<'_>, what: &str| {
        let (screen, violations, dimmed) = dimmed_text(app, 120, 40);
        assert!(
            violations.is_empty(),
            "{what}: dimmed text {violations:?}\n{screen}"
        );
        seen.push_str(&screen);
        dimmed_total += dimmed;
    };

    check(&app, "node list with detail");
    move_cursor_to(&mut app, "dangling/points_nowhere");
    check(&app, "detail of a node with a dangling edge");
    app.handle_key(KeyCode::Tab);
    check(&app, "detail focused, node list unfocused");
    app.handle_key(KeyCode::Tab);
    app.handle_key(KeyCode::Char('S'));
    check(&app, "schema");
    app.handle_key(KeyCode::Char('S'));
    for (key, view) in [
        ('2', "tree"),
        ('3', "findings"),
        ('4', "provenance"),
        ('5', "graph"),
    ] {
        app.handle_key(KeyCode::Char(key));
        check(&app, view);
    }
    app.handle_key(KeyCode::Char('1'));
    move_cursor_to(&mut app, "index/root");
    for (key, view) in [('2', "tree from the root"), ('5', "graph from the root")] {
        app.handle_key(KeyCode::Char(key));
        check(&app, view);
    }

    // What mini-toml lacks: a node reachable only through a vacancy (taint), a
    // hint-severity finding (a cross-source VACANCY), and an allowed triple with
    // no edges (a zero-count schema row).
    let (profile, graph) = tainted_register();
    let mut app = App::new(
        &profile,
        &graph,
        validate(&graph, &profile, false),
        Vec::new(),
    );
    check(&app, "synthetic node list");
    app.handle_key(KeyCode::Char('S'));
    check(&app, "synthetic schema");
    app.handle_key(KeyCode::Char('S'));
    // Tree: expand REQ-001, then GHOST, to reach the tainted REQ-002.
    for key in ['2', ' ', 'j', ' '] {
        app.handle_key(KeyCode::Char(key));
    }
    check(&app, "synthetic tree");
    for (key, view) in [('3', "synthetic findings"), ('5', "synthetic graph")] {
        app.handle_key(KeyCode::Char(key));
        check(&app, view);
    }

    // The risky content was on screen, so the check ran against it.
    let missing: Vec<_> = ["◌", "⊘", "~tainted", "HINT", "(0)", "▶"]
        .into_iter()
        .filter(|needle| !seen.contains(needle))
        .collect();
    assert!(missing.is_empty(), "never rendered: {missing:?}");
    assert!(
        dimmed_total > 0,
        "no dimmed cell at all: the detector never fired"
    );
}

// Requirement: browse/Findings list view (notes and `A`)
mod notes {
    use super::*;
    use crate::tui::app::Action;

    /// `mini-toml` under a profile that extends the shipped one and names a
    /// notes file holding `notes`, both in a fresh directory.
    fn noted(notes: Option<&str>) -> (Profile, LatticeGraph, PathBuf) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lattice_tui_notes_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // A reused process ID would otherwise find an earlier run's notes file
        // where this case wants none.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shipped = std::path::absolute(repo_root().join("profiles/toml.yaml")).unwrap();
        std::fs::write(
            dir.join("lattice.yaml"),
            format!(
                "name: toml-notes\nprofile_version: \"1.0.0\"\nextends: {}\nannotations: notes.yaml\n",
                shipped.display()
            ),
        )
        .unwrap();
        if let Some(notes) = notes {
            std::fs::write(dir.join("notes.yaml"), notes).unwrap();
        }
        let profile = load_profile(&dir.join("lattice.yaml")).unwrap();
        let (_, graph) = mini_toml();
        (profile, graph, dir.join("notes.yaml"))
    }

    const NOTES: &str = "# triage\n\n- finding: {code: VACANCY, node: dangling/overdue}\n  note: Target lands later.\n";

    fn to_finding(app: &mut App<'_>, code: &str, node: Option<&str>) {
        app.handle_key(KeyCode::Char('3'));
        let index = app
            .visible_findings()
            .iter()
            .position(|i| i.code == code && i.node_id.as_deref() == node)
            .unwrap_or_else(|| panic!("{code} on {node:?} not in the findings list"));
        for _ in 0..index {
            app.handle_key(KeyCode::Char('j'));
        }
    }

    fn status(app: &App<'_>) -> String {
        app.status_message.clone().unwrap_or_default()
    }

    #[test]
    fn notes_show_under_the_finding_in_the_list() {
        let (profile, graph, _) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        app.handle_key(KeyCode::Char('3'));
        let screen = render(&mut app, 120, 60);
        let row = screen
            .rows
            .iter()
            .position(|r| r.contains("VACANCY") && r.contains("dangling/overdue"))
            .expect("the noted finding is listed");
        assert!(
            screen.rows[row + 1].contains("note: Target lands later."),
            "{}",
            screen.dump()
        );
    }

    #[test]
    fn notes_show_in_node_detail() {
        let (profile, graph, _) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        move_cursor_to(&mut app, "dangling/overdue");
        app.handle_key(KeyCode::Enter);
        let screen = render(&mut app, 140, 50);
        assert!(
            screen.row_containing("note: Target lands later.").is_some(),
            "{}",
            screen.dump()
        );
    }

    #[test]
    fn notes_expired_note_is_marked() {
        let (profile, graph, _) = noted(Some(&format!("{NOTES}  until: 2000-01-01\n")));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        app.handle_key(KeyCode::Char('3'));
        let screen = render(&mut app, 120, 60);
        assert!(
            screen
                .row_containing("note: (expired) Target lands later.")
                .is_some(),
            "{}",
            screen.dump()
        );
    }

    #[test]
    fn notes_a_opens_the_entry_line() {
        let (profile, graph, path) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "VACANCY", Some("dangling/overdue"));
        match app.handle_key(KeyCode::Char('A')) {
            Action::EditNotes(at) => {
                assert_eq!(Path::new(&at.file), path.as_path());
                assert_eq!(at.line.map(u32::from), Some(3));
            }
            _ => panic!("A on a noted finding opens the notes file"),
        }
    }

    #[test]
    fn notes_a_on_a_flow_style_entry_opens_the_file_without_a_line() {
        let flow = "[{finding: {code: VACANCY, node: dangling/overdue}, note: Later.}]\n";
        let (profile, graph, path) = noted(Some(flow));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "VACANCY", Some("dangling/overdue"));
        match app.handle_key(KeyCode::Char('A')) {
            Action::EditNotes(at) => {
                assert_eq!(Path::new(&at.file), path.as_path());
                assert_eq!(at.line, None, "a flow-style entry has no line to name");
            }
            _ => panic!("A on a noted finding opens the notes file"),
        }
    }

    #[test]
    fn notes_a_shows_the_stub_then_opens_at_the_end() {
        let (profile, graph, path) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "VACANCY", Some("dangling/points_nowhere"));
        assert!(matches!(app.handle_key(KeyCode::Char('A')), Action::None));
        let shown = status(&app);
        assert!(
            shown.contains("- finding: {code: VACANCY, node: dangling/points_nowhere}"),
            "{shown}"
        );
        assert!(shown.contains("A again"), "{shown}");
        match app.handle_key(KeyCode::Char('A')) {
            Action::EditNotes(at) => {
                assert_eq!(Path::new(&at.file), path.as_path());
                assert_eq!(at.line.map(u32::from), Some(4), "the file's last line");
            }
            _ => panic!("a second A opens the notes file"),
        }
    }

    #[test]
    fn notes_a_key_between_resets_the_stub() {
        let (profile, graph, _) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "VACANCY", Some("dangling/points_nowhere"));
        app.handle_key(KeyCode::Char('A'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('k'));
        assert!(matches!(app.handle_key(KeyCode::Char('A')), Action::None));
        assert!(status(&app).contains("A again"));
    }

    #[test]
    fn notes_a_without_annotations_says_so() {
        let (profile, graph) = mini_toml();
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "VACANCY", Some("dangling/overdue"));
        assert!(matches!(app.handle_key(KeyCode::Char('A')), Action::None));
        assert!(
            status(&app).contains("declares no notes file"),
            "{}",
            status(&app)
        );
    }

    #[test]
    fn notes_a_with_a_missing_file_names_it() {
        let (profile, graph, path) = noted(None);
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "VACANCY", Some("dangling/overdue"));
        assert!(matches!(app.handle_key(KeyCode::Char('A')), Action::None));
        let shown = status(&app);
        assert!(
            shown.contains(&path.display().to_string()) && shown.contains("does not exist"),
            "{shown}"
        );
    }

    #[test]
    fn notes_a_without_an_editor_says_so() {
        let (profile, graph, _) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = false;
        to_finding(&mut app, "VACANCY", Some("dangling/overdue"));
        assert!(matches!(app.handle_key(KeyCode::Char('A')), Action::None));
        assert!(status(&app).contains("set $EDITOR"), "{}", status(&app));
    }

    #[test]
    fn notes_a_on_a_finding_without_a_node_says_so() {
        let (profile, graph, _) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        to_finding(&mut app, "PARSE_ERROR", None);
        assert!(matches!(app.handle_key(KeyCode::Char('A')), Action::None));
        assert!(status(&app).contains("no node"), "{}", status(&app));
    }

    #[test]
    fn notes_a_from_a_detail_finding_row() {
        let (profile, graph, _) = noted(Some(NOTES));
        let mut app = make_app(&profile, &graph);
        app.editor_set = true;
        move_cursor_to(&mut app, "dangling/overdue");
        app.handle_key(KeyCode::Enter);
        let row = app
            .detail_rows()
            .iter()
            .position(|r| {
                matches!(
                    r,
                    crate::tui::app::DetailRow::Finding {
                        code: "VACANCY",
                        ..
                    }
                )
            })
            .expect("the detail lists the finding");
        app.handle_key(KeyCode::Tab);
        while app.detail_cursor < row {
            app.handle_key(KeyCode::Char('j'));
        }
        assert!(matches!(
            app.handle_key(KeyCode::Char('A')),
            Action::EditNotes(_)
        ));
    }
}

/// What a real register brings that `mini-toml` lacks: IDs as long as a test path,
/// a summary the ID already ends with, an attr written over several lines, many
/// neighbours of one node, and an unattached finding that sorts before the rest.
mod real_shapes {
    use super::*;

    const FAN: usize = 12;

    fn register() -> (Profile, LatticeGraph) {
        let dir = std::env::temp_dir().join("lattice_tui_snapshot");
        std::fs::create_dir_all(&dir).unwrap();
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = dir.join(format!(
            "real-shapes-{}-{}.yaml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &path,
            r#"
name: real-shapes
profile_version: "1.0.0"
node_kinds:
  capability:
    id_pattern: '^[a-z-]+$'
  requirement:
    id_pattern: '^[a-z-]+/.+$'
    summary_attr: title
    attrs:
      title: { type: string }
      body: { type: string }
  test:
    id_pattern: '^[^:]+::.+$'
    orphan_ok: true
edge_kinds:
  contains:
    allowed: [[capability, requirement]]
  verifies:
    allowed: [[test, requirement]]
validations:
  - COVERAGE:
      target_kind: requirement
      edge_kind: verifies
      severity: hint
"#,
        )
        .unwrap();
        let profile = load_profile(&path).unwrap();
        let at = |line: u32| serde_json::json!({"file": "spec.md", "line": line});
        let mut nodes = vec![
            serde_json::json!({"id": "graph-model", "kind": "capability", "attrs": {},
                               "provenance": at(1)}),
            serde_json::json!({"id": "graph-model/Dangling edge targets", "kind": "requirement",
                               "attrs": {"title": "Dangling edge targets",
                                         "body": "An edge SHALL be kept when\nthe target is absent."},
                               "provenance": at(2)}),
            serde_json::json!({"id": "graph-model/Untested", "kind": "requirement",
                               "attrs": {"title": "Untested"}, "provenance": at(9)}),
        ];
        let mut edges = vec![serde_json::json!({
            "src": "graph-model", "tgt": "graph-model/Dangling edge targets",
            "kind": "contains", "provenance": at(2)})];
        for i in 0..FAN {
            let id = format!("crates/latt-core/tests/graph_model.rs::edge_endpoint_case_{i:02}");
            nodes.push(serde_json::json!({"id": id, "kind": "test", "attrs": {},
                                          "provenance": {"file": "graph_model.rs", "line": i + 1}}));
            edges.push(
                serde_json::json!({"src": id, "tgt": "graph-model/Dangling edge targets",
                                          "kind": "verifies",
                                          "provenance": {"file": "graph_model.rs", "line": i + 1}}),
            );
        }
        let graph = ingest_document(serde_json::json!({
            "interface_version": "1.4",
            "nodes": nodes,
            "edges": edges,
            "findings": [{"severity": "warning", "code": "PARSE_ERROR",
                          "message": "a.md: unreadable", "node_id": null,
                          "provenance": {"file": "a.md", "line": 0}}]
        }))
        .unwrap();
        (profile, graph)
    }

    #[test]
    fn a_multi_line_attr_keeps_a_space_at_each_line_break() {
        // Requirement: browse/Node detail view
        let (profile, graph) = register();
        let mut app = make_app(&profile, &graph);
        move_cursor_to(&mut app, "graph-model/Dangling edge targets");
        app.handle_key(KeyCode::Enter);
        let screen = render(&mut app, 120, 40);
        assert!(
            screen.row_containing("kept when the target").is_some(),
            "{}",
            screen.dump()
        );
    }

    #[test]
    fn the_node_list_omits_a_summary_the_id_ends_with() {
        // Requirement: browse/Node list view
        let (profile, graph) = register();
        let mut app = make_app(&profile, &graph);
        let screen = render(&mut app, 120, 40);
        let row = screen
            .row_containing("graph-model/Dangling edge targets")
            .unwrap_or_else(|| panic!("{}", screen.dump()));
        assert!(
            !screen.rows[row].contains("— Dangling"),
            "{}",
            screen.rows[row]
        );
    }

    #[test]
    fn the_status_line_counts_hints() {
        // Requirement: browse/Status line
        let (profile, graph) = register();
        let mut app = make_app(&profile, &graph);
        let screen = render(&mut app, 120, 40);
        let status = screen.rows.last().unwrap();
        assert!(
            status.trim_end().ends_with("2 warnings, 1 hint"),
            "{status}"
        );
    }

    #[test]
    fn every_finding_under_the_unattached_label_is_unattached() {
        // Requirement: browse/Findings list view
        let (profile, graph) = register();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3'));
        let screen = render(&mut app, 120, 40);
        let label = screen
            .row_containing("Unattached")
            .unwrap_or_else(|| panic!("{}", screen.dump()));
        let attached = screen
            .row_containing("[graph-model/Untested]")
            .unwrap_or_else(|| panic!("{}", screen.dump()));
        assert!(attached < label, "{}", screen.dump());
    }

    #[test]
    fn the_graph_keeps_its_center_in_view_among_wide_neighbours() {
        // Requirement: browse-map/Graph labels fit the view
        let (profile, graph) = register();
        let mut app = make_app(&profile, &graph);
        move_cursor_to(&mut app, "graph-model/Dangling edge targets");
        app.handle_key(KeyCode::Char('5'));
        let screen = render(&mut app, 120, 32);
        assert!(
            screen
                .row_containing("●graph-model/Dangling edge targets")
                .is_some(),
            "center out of view:\n{}",
            screen.dump()
        );
        assert!(
            screen
                .row_containing("…model.rs::edge_endpoint_case_")
                .is_some(),
            "a long ID keeps its tail:\n{}",
            screen.dump()
        );
        assert!(
            !screen
                .dump()
                .contains("crates/latt-core/tests/graph_model.rs::"),
            "a long ID is shown whole:\n{}",
            screen.dump()
        );
    }
}

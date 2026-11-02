//! Browse's key table: the single source for dispatch, the help overlay and
//! the footer's key labels.
//!
//! A key resolves in two layers. `Always` rows fire in every mode, before the
//! per-key bookkeeping. Otherwise the state picks exactly one layer (an
//! overlay: preview, owner-action menu, confirmation, output pane; then the input mode,
//! then `Normal`) and the first row of that
//! layer whose views, guard and key all match wins; a `Pat::Any` row fires
//! only when no other row of its layer does. Rows are meant to be
//! mutually exclusive within a layer, so their order is free to follow the
//! help overlay: within a section, help lists rows in table order, and a
//! `Join` row extends the previous row's label key by key, so `j`+`↓` and
//! `k`+`↑` read `j/k ↓/↑`. `aliases` dispatch but are never shown. The footer keeps its own per-view order
//! and word, but takes each label from the row that runs the command.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::app::{App, InputMode, TopView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    QuitNow,
    ToggleSearchFilter,
    ClosePreview,
    PreviewDown,
    PreviewUp,
    PreviewTop,
    PreviewBottom,
    PreviewPageDown,
    PreviewPageUp,
    SearchSubmit,
    SearchCancel,
    SearchType,
    SearchBackspace,
    KindFilterDown,
    KindFilterUp,
    KindFilterToggle,
    KindFilterClose,
    EdgeFilterDown,
    EdgeFilterUp,
    EdgeFilterToggle,
    EdgeFilterClose,
    CloseHelp,
    HelpDown,
    HelpUp,
    HelpPageDown,
    HelpPageUp,
    HelpTop,
    HelpBottom,
    ShowView(TopView),
    Quit,
    BackOrQuit,
    Escape,
    Search,
    NextMatch,
    PrevMatch,
    Editor,
    Note,
    Preview,
    KindFilter,
    KindFilterRefused,
    Deeper,
    Left,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Help,
    Down,
    Up,
    Enter,
    FocusDetail,
    GraphTab,
    CycleSort,
    ToggleSchema,
    ToggleSuppressed,
    TreeExpand,
    TreeCollapse,
    SccCollapse,
    ToggleDirection,
    EdgeFilter,
    ScrollLeft,
    ScrollRight,
    PopCenter,
    RadiusUp,
    RadiusDown,
    CycleColumns,
    Reload,
    ReloadAll,
    /// Run the selected node kind's action of this verb.
    Owner(&'static str),
    Launcher,
    LauncherDown,
    LauncherUp,
    LauncherRun,
    LauncherClose,
    ConfirmYes,
    ConfirmCancel,
    OutputDown,
    OutputUp,
    OutputPageDown,
    OutputPageUp,
    OutputTop,
    OutputBottom,
    CloseOutput,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Always,
    Preview,
    Launcher,
    Confirm,
    Output,
    Search,
    KindFilter,
    EdgeKindFilter,
    Help,
    Normal,
}

#[derive(Clone, Copy)]
pub enum Pat {
    Key(KeyCode, KeyModifiers),
    /// Any character key with no Control or Alt.
    AnyChar,
    /// Every key no other row of the layer claims.
    Any,
}

#[derive(Clone, Copy)]
pub enum Views {
    All,
    Only(&'static [TopView]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Global,
    View(TopView),
}

#[derive(Clone, Copy)]
pub enum Help {
    None,
    Row(Section, &'static str),
    Join,
}

type Guard = fn(&App<'_>) -> bool;

pub struct Entry {
    pub layer: Layer,
    pub views: Views,
    pub keys: &'static [Pat],
    pub aliases: &'static [Pat],
    pub when: Option<Guard>,
    pub cmd: Cmd,
    pub help: Help,
}

const fn ch(c: char) -> Pat {
    Pat::Key(KeyCode::Char(c), KeyModifiers::NONE)
}

const fn key(code: KeyCode) -> Pat {
    Pat::Key(code, KeyModifiers::NONE)
}

const fn ctrl(c: char) -> Pat {
    Pat::Key(KeyCode::Char(c), KeyModifiers::CONTROL)
}

const fn row(layer: Layer, views: Views, keys: &'static [Pat], cmd: Cmd) -> Entry {
    Entry {
        layer,
        views,
        keys,
        aliases: &[],
        when: None,
        cmd,
        help: Help::None,
    }
}

impl Entry {
    const fn when(mut self, guard: Guard) -> Self {
        self.when = Some(guard);
        self
    }

    const fn help(mut self, section: Section, desc: &'static str) -> Self {
        self.help = Help::Row(section, desc);
        self
    }

    const fn alias(mut self, aliases: &'static [Pat]) -> Self {
        self.aliases = aliases;
        self
    }

    const fn join(mut self) -> Self {
        self.help = Help::Join;
        self
    }

    fn applies_to(&self, view: TopView) -> bool {
        match self.views {
            Views::All => true,
            Views::Only(views) => views.contains(&view),
        }
    }
}

const fn normal(keys: &'static [Pat], cmd: Cmd) -> Entry {
    row(Layer::Normal, Views::All, keys, cmd)
}

const fn mode(layer: Layer, keys: &'static [Pat], cmd: Cmd) -> Entry {
    row(layer, Views::All, keys, cmd)
}

const fn in_view(views: &'static [TopView], keys: &'static [Pat], cmd: Cmd) -> Entry {
    row(Layer::Normal, Views::Only(views), keys, cmd)
}

use Cmd as C;
use Layer as L;
use Section::Global;
use TopView::{Findings, Graph, NodeList, Provenance, Tree};

const G: Section = Global;
const NODES: &[TopView] = &[NodeList];
const FINDINGS: &[TopView] = &[Findings];
const TREE: &[TopView] = &[Tree];
const PROV: &[TopView] = &[Provenance];
const GRAPH: &[TopView] = &[Graph];

fn not_focused(app: &App<'_>) -> bool {
    !app.detail_focused
}

fn detail_open(app: &App<'_>) -> bool {
    app.show_detail
}

fn tree_detail_open(app: &App<'_>) -> bool {
    app.show_detail || !app.view_stack.is_empty()
}

fn kind_filter_allowed(app: &App<'_>) -> bool {
    !(app.detail_focused
        || app.top_view == Provenance
        || app.top_view == NodeList && app.show_schema)
}

fn kind_filter_refused(app: &App<'_>) -> bool {
    !kind_filter_allowed(app)
}

fn graph_at_top(app: &App<'_>) -> bool {
    app.view_stack.is_empty() && !app.show_detail
}

fn graph_esc_pops_center(app: &App<'_>) -> bool {
    app.top_view == Graph && graph_at_top(app) && !app.graph_state.center_history.is_empty()
}

fn esc_steps_back(app: &App<'_>) -> bool {
    !graph_esc_pops_center(app)
}

pub fn table() -> &'static [Entry] {
    TABLE
}

const TABLE: &[Entry] = &[
    // Preview overlay
    mode(
        L::Preview,
        &[ch('p'), ch('q'), key(KeyCode::Esc)],
        C::ClosePreview,
    ),
    mode(L::Preview, &[ch('Q')], C::Quit),
    mode(L::Preview, &[ch('j'), key(KeyCode::Down)], C::PreviewDown),
    mode(L::Preview, &[ch('k'), key(KeyCode::Up)], C::PreviewUp),
    mode(L::Preview, &[ch('g'), key(KeyCode::Home)], C::PreviewTop),
    mode(L::Preview, &[ch('G'), key(KeyCode::End)], C::PreviewBottom),
    mode(
        L::Preview,
        &[key(KeyCode::PageDown), ctrl('d')],
        C::PreviewPageDown,
    ),
    mode(
        L::Preview,
        &[key(KeyCode::PageUp), ctrl('u')],
        C::PreviewPageUp,
    ),
    // Owner-action menu
    mode(L::Launcher, &[ch('Q')], C::Quit),
    mode(L::Launcher, &[ch('j'), key(KeyCode::Down)], C::LauncherDown),
    mode(L::Launcher, &[ch('k'), key(KeyCode::Up)], C::LauncherUp),
    mode(L::Launcher, &[key(KeyCode::Enter)], C::LauncherRun),
    mode(
        L::Launcher,
        &[key(KeyCode::Esc), ch('q'), ch('!')],
        C::LauncherClose,
    ),
    // Owner-action confirmation: `y` runs, `Q` quits, any other key cancels
    mode(L::Confirm, &[ch('y')], C::ConfirmYes),
    mode(L::Confirm, &[ch('Q')], C::Quit),
    mode(L::Confirm, &[Pat::Any], C::ConfirmCancel),
    // Owner-action output pane
    mode(L::Output, &[ch('Q')], C::Quit),
    mode(L::Output, &[ch('j'), key(KeyCode::Down)], C::OutputDown),
    mode(L::Output, &[ch('k'), key(KeyCode::Up)], C::OutputUp),
    mode(L::Output, &[ch('g'), key(KeyCode::Home)], C::OutputTop),
    mode(L::Output, &[ch('G'), key(KeyCode::End)], C::OutputBottom),
    mode(
        L::Output,
        &[key(KeyCode::PageDown), ctrl('d')],
        C::OutputPageDown,
    ),
    mode(
        L::Output,
        &[key(KeyCode::PageUp), ctrl('u')],
        C::OutputPageUp,
    ),
    mode(L::Output, &[key(KeyCode::Esc), ch('q')], C::CloseOutput),
    // Search prompt: `Q` is typed, not quit
    mode(L::Search, &[key(KeyCode::Enter)], C::SearchSubmit),
    mode(L::Search, &[key(KeyCode::Esc)], C::SearchCancel),
    mode(L::Search, &[Pat::AnyChar], C::SearchType),
    mode(L::Search, &[key(KeyCode::Backspace)], C::SearchBackspace),
    // Kind filter picker
    mode(L::KindFilter, &[ch('Q')], C::Quit),
    mode(
        L::KindFilter,
        &[ch('j'), key(KeyCode::Down)],
        C::KindFilterDown,
    ),
    mode(L::KindFilter, &[ch('k'), key(KeyCode::Up)], C::KindFilterUp),
    mode(L::KindFilter, &[ch(' ')], C::KindFilterToggle),
    mode(
        L::KindFilter,
        &[key(KeyCode::Enter), key(KeyCode::Esc), ch('f')],
        C::KindFilterClose,
    ),
    // Edge-kind filter picker
    mode(L::EdgeKindFilter, &[ch('Q')], C::Quit),
    mode(
        L::EdgeKindFilter,
        &[ch('j'), key(KeyCode::Down)],
        C::EdgeFilterDown,
    ),
    mode(
        L::EdgeKindFilter,
        &[ch('k'), key(KeyCode::Up)],
        C::EdgeFilterUp,
    ),
    mode(L::EdgeKindFilter, &[ch(' ')], C::EdgeFilterToggle),
    mode(
        L::EdgeKindFilter,
        &[key(KeyCode::Enter), key(KeyCode::Esc), ch('F')],
        C::EdgeFilterClose,
    ),
    // Help overlay: navigation keys scroll it, `Q` quits, any other key closes it
    mode(L::Help, &[ch('Q')], C::Quit),
    mode(L::Help, &[ch('j'), key(KeyCode::Down)], C::HelpDown),
    mode(L::Help, &[ch('k'), key(KeyCode::Up)], C::HelpUp),
    mode(
        L::Help,
        &[key(KeyCode::PageDown), ctrl('d')],
        C::HelpPageDown,
    ),
    mode(L::Help, &[key(KeyCode::PageUp), ctrl('u')], C::HelpPageUp),
    mode(L::Help, &[ch('g'), key(KeyCode::Home)], C::HelpTop),
    mode(L::Help, &[ch('G'), key(KeyCode::End)], C::HelpBottom),
    mode(L::Help, &[Pat::Any], C::CloseHelp),
    // Global
    normal(&[ch('1')], C::ShowView(NodeList)).help(G, "Nodes"),
    normal(&[ch('2')], C::ShowView(Tree)).help(G, "Tree"),
    normal(&[ch('3')], C::ShowView(Findings)).help(G, "Findings"),
    normal(&[ch('4')], C::ShowView(Provenance)).help(G, "Provenance"),
    normal(&[ch('5')], C::ShowView(Graph)).help(G, "Graph"),
    normal(&[ch('Q')], C::Quit).help(G, "Quit"),
    row(L::Always, Views::All, &[ctrl('c')], C::QuitNow).join(),
    normal(&[ch('q')], C::BackOrQuit).help(G, "Back / quit"),
    normal(&[ch('/')], C::Search).help(G, "Search"),
    normal(&[ch('n')], C::NextMatch).help(G, "Next/prev match"),
    normal(&[ch('N')], C::PrevMatch).join(),
    normal(&[ch('e')], C::Editor).help(G, "Edit (owner tool, else $EDITOR)"),
    normal(&[ch('A')], C::Note).help(G, "Note on finding (notes file)"),
    normal(&[ch('c')], C::Owner("comment")).help(G, "Comment (owner tool)"),
    normal(&[ch('s')], C::Owner("state")).help(G, "Change state (owner tool)"),
    normal(&[ch('a')], C::Owner("assign")).help(G, "Assign (owner tool)"),
    normal(&[ch('t')], C::Owner("tag")).help(G, "Tag (owner tool)"),
    normal(&[ch('!')], C::Launcher).help(G, "Owner actions"),
    normal(&[ch('p')], C::Preview).help(G, "Preview file"),
    normal(&[ch('f')], C::KindFilter)
        .when(kind_filter_allowed)
        .help(G, "Kind filter"),
    normal(&[ch('f')], C::KindFilterRefused).when(kind_filter_refused),
    normal(&[ch('j'), key(KeyCode::Down)], C::Down).help(G, "Down / up"),
    normal(&[ch('k'), key(KeyCode::Up)], C::Up).join(),
    normal(&[ch('l'), key(KeyCode::Right)], C::Deeper).help(G, "Deeper / back"),
    normal(&[ch('h'), key(KeyCode::Left)], C::Left).join(),
    normal(&[ch('g'), key(KeyCode::Home)], C::Top).help(G, "Top / bottom"),
    normal(&[ch('G'), key(KeyCode::End)], C::Bottom).join(),
    normal(&[key(KeyCode::PageDown), ctrl('d')], C::PageDown).help(G, "Page down / up"),
    normal(&[key(KeyCode::PageUp), ctrl('u')], C::PageUp).join(),
    normal(&[ch('.')], C::ToggleSuppressed).help(G, "Toggle suppressed"),
    normal(&[ch('r')], C::Reload).help(G, "Reload register"),
    normal(&[ch('R')], C::ReloadAll).help(G, "Reload profile and register"),
    normal(&[ch('?')], C::Help).help(G, "This help"),
    normal(&[key(KeyCode::Esc)], C::Escape).when(esc_steps_back),
    // Node list
    in_view(NODES, &[key(KeyCode::Enter)], C::Enter).help(Section::View(NodeList), "Open detail"),
    in_view(NODES, &[key(KeyCode::Tab)], C::FocusDetail)
        .when(detail_open)
        .help(Section::View(NodeList), "Focus detail"),
    in_view(NODES, &[ch('o')], C::CycleSort).help(Section::View(NodeList), "Cycle sort"),
    in_view(NODES, &[ch('S')], C::ToggleSchema).help(Section::View(NodeList), "Schema view"),
    // Findings
    in_view(FINDINGS, &[key(KeyCode::Enter)], C::Enter)
        .help(Section::View(Findings), "Jump to node"),
    in_view(FINDINGS, &[ch('o')], C::CycleSort).help(Section::View(Findings), "Cycle sort"),
    in_view(FINDINGS, &[key(KeyCode::Tab)], C::FocusDetail).when(detail_open),
    // Tree
    in_view(TREE, &[key(KeyCode::Enter)], C::Enter).help(Section::View(Tree), "Open detail"),
    in_view(TREE, &[ch(' ')], C::TreeExpand)
        .when(not_focused)
        .help(Section::View(Tree), "Expand node"),
    in_view(TREE, &[ch('-')], C::TreeCollapse)
        .when(not_focused)
        .help(Section::View(Tree), "Collapse node"),
    in_view(TREE, &[ch('z')], C::SccCollapse).help(Section::View(Tree), "Collapse cycles"),
    in_view(TREE, &[ch('d')], C::ToggleDirection).help(Section::View(Tree), "Toggle direction"),
    in_view(TREE, &[ch('F')], C::EdgeFilter).help(Section::View(Tree), "Edge-kind filter"),
    in_view(TREE, &[key(KeyCode::Tab)], C::FocusDetail)
        .when(tree_detail_open)
        .help(Section::View(Tree), "Focus detail"),
    // Provenance
    in_view(PROV, &[key(KeyCode::Enter)], C::Enter).help(Section::View(Provenance), "Open detail"),
    in_view(PROV, &[ch('d')], C::ToggleDirection)
        .help(Section::View(Provenance), "Toggle direction"),
    in_view(PROV, &[ch('F')], C::EdgeFilter).help(Section::View(Provenance), "Edge-kind filter"),
    in_view(PROV, &[ch('H')], C::ScrollLeft)
        .when(not_focused)
        .help(Section::View(Provenance), "Scroll left/right"),
    in_view(PROV, &[ch('L')], C::ScrollRight)
        .when(not_focused)
        .join(),
    in_view(PROV, &[key(KeyCode::Tab)], C::FocusDetail)
        .when(detail_open)
        .help(Section::View(Provenance), "Focus detail"),
    // Graph
    in_view(GRAPH, &[key(KeyCode::Enter)], C::Enter)
        .help(Section::View(Graph), "Re-center on node"),
    in_view(GRAPH, &[key(KeyCode::Backspace)], C::PopCenter)
        .when(graph_at_top)
        .help(Section::View(Graph), "Back (pop center)"),
    in_view(GRAPH, &[key(KeyCode::Tab)], C::GraphTab).help(Section::View(Graph), "Toggle detail"),
    in_view(GRAPH, &[ch('+')], C::RadiusUp)
        .alias(&[ch('=')])
        .when(not_focused)
        .help(Section::View(Graph), "Depth radius"),
    in_view(GRAPH, &[ch('-')], C::RadiusDown)
        .when(not_focused)
        .join(),
    in_view(GRAPH, &[ch('F')], C::EdgeFilter).help(Section::View(Graph), "Edge-kind filter"),
    in_view(GRAPH, &[ch('d')], C::ToggleDirection).help(Section::View(Graph), "Toggle direction"),
    in_view(GRAPH, &[ch('C')], C::CycleColumns)
        .when(not_focused)
        .help(Section::View(Graph), "Cycle columns"),
    in_view(GRAPH, &[ch('&')], C::ToggleSearchFilter)
        .help(Section::View(Graph), "Toggle search filter"),
    row(L::Always, Views::All, &[ctrl('f')], C::ToggleSearchFilter).join(),
    in_view(GRAPH, &[key(KeyCode::Esc)], C::PopCenter).when(graph_esc_pops_center),
];

/// The layer a key resolves in, after `Always`.
pub fn layer(app: &App<'_>) -> Layer {
    if app.preview.is_some() {
        return Layer::Preview;
    }
    if app.launcher.is_some() {
        return Layer::Launcher;
    }
    if app.confirm.is_some() {
        return Layer::Confirm;
    }
    if app.output.is_some() {
        return Layer::Output;
    }
    match app.input_mode {
        InputMode::Search => Layer::Search,
        InputMode::KindFilter => Layer::KindFilter,
        InputMode::EdgeKindFilter => Layer::EdgeKindFilter,
        InputMode::Help => Layer::Help,
        InputMode::Normal => Layer::Normal,
    }
}

fn pat_matches(pat: &Pat, code: KeyCode, mods: KeyModifiers) -> bool {
    match *pat {
        Pat::Key(c, m) => c == code && m == mods,
        Pat::AnyChar => {
            matches!(code, KeyCode::Char(_))
                && !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        }
        Pat::Any => false,
    }
}

/// Every row of `layer` that `code` + `mods` would fire in `app`'s state,
/// leaving out the `Pat::Any` fallback.
pub fn matches<'t>(
    app: &'t App<'_>,
    layer: Layer,
    code: KeyCode,
    mods: KeyModifiers,
) -> impl Iterator<Item = &'static Entry> + 't {
    let mods = mods - KeyModifiers::SHIFT;
    table().iter().filter(move |e| {
        e.layer == layer
            && e.applies_to(app.top_view)
            && e.keys
                .iter()
                .chain(e.aliases)
                .any(|p| pat_matches(p, code, mods))
            && e.when.is_none_or(|g| g(app))
    })
}

/// The command `code` + `mods` runs in `layer`. A Control or Alt chord
/// matches only a row declaring it; Shift is ignored.
pub fn lookup(app: &App<'_>, layer: Layer, code: KeyCode, mods: KeyModifiers) -> Option<Cmd> {
    matches(app, layer, code, mods)
        .next()
        .or_else(|| {
            table().iter().find(|e| {
                e.layer == layer
                    && e.applies_to(app.top_view)
                    && e.keys.iter().any(|p| matches!(p, Pat::Any))
                    && e.when.is_none_or(|g| g(app))
            })
        })
        .map(|e| e.cmd)
}

pub fn key_label(pat: &Pat) -> String {
    let Pat::Key(code, mods) = *pat else {
        return String::new();
    };
    match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) if mods.contains(KeyModifiers::CONTROL) => {
            format!("Ctrl-{}", c.to_ascii_uppercase())
        }
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Right => "→".into(),
        KeyCode::Left => "←".into(),
        KeyCode::Up => "↑".into(),
        KeyCode::Down => "↓".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        other => format!("{other:?}"),
    }
}

/// The label of the row at `index` and the `Join` rows after it, key by
/// key: the first keys joined by `/`, then the second keys, and so on.
fn row_label(rows: &[Entry], index: usize) -> String {
    let group: Vec<&Entry> = std::iter::once(&rows[index])
        .chain(
            rows[index + 1..]
                .iter()
                .take_while(|e| matches!(e.help, Help::Join)),
        )
        .collect();
    let width = group.iter().map(|e| e.keys.len()).max().unwrap_or(0);
    (0..width)
        .map(|i| {
            group
                .iter()
                .filter_map(|e| e.keys.get(i).map(key_label))
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Help rows of `section`, in table order, as `(label, description)`.
pub fn help_rows(section: Section) -> Vec<(String, &'static str)> {
    let rows = table();
    rows.iter()
        .enumerate()
        .filter_map(|(i, e)| match e.help {
            Help::Row(s, desc) if s == section => Some((row_label(rows, i), desc)),
            _ => None,
        })
        .collect()
}

/// The label of the documented row running `cmd` in `view`.
pub fn label_for(cmd: Cmd, view: TopView) -> String {
    let rows = table();
    rows.iter()
        .position(|e| e.cmd == cmd && e.applies_to(view) && matches!(e.help, Help::Row(..)))
        .map(|i| row_label(rows, i))
        .unwrap_or_default()
}

/// Footer hints per view: presentation order and word; the key comes from the table.
pub fn footer(view: TopView) -> &'static [(Cmd, &'static str)] {
    match view {
        NodeList => &[
            (C::KindFilter, "filter"),
            (C::CycleSort, "sort"),
            (C::ToggleSchema, "schema"),
            (C::Help, "help"),
        ],
        Findings => &[
            (C::KindFilter, "filter"),
            (C::CycleSort, "sort"),
            (C::ToggleSuppressed, "suppressed"),
            (C::Help, "help"),
        ],
        Tree => &[
            (C::KindFilter, "filter"),
            (C::ToggleDirection, "direction"),
            (C::EdgeFilter, "edge filter"),
            (C::Help, "help"),
        ],
        Provenance => &[
            (C::ToggleDirection, "direction"),
            (C::EdgeFilter, "edge filter"),
            (C::ScrollLeft, "scroll"),
            (C::Help, "help"),
        ],
        Graph => &[
            (C::KindFilter, "filter"),
            (C::ToggleDirection, "direction"),
            (C::RadiusUp, "radius"),
            (C::EdgeFilter, "edge filter"),
            (C::CycleColumns, "columns"),
            (C::Help, "help"),
        ],
    }
}

/// Requirement: browse/Accessibility baseline — every action has a key that is not a
/// Control chord.
#[cfg(test)]
mod no_ctrl_only_tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::{Cmd, Pat, table};

    #[test]
    fn every_ctrl_chord_has_a_plain_key_for_the_same_command() {
        let plain = |p: &Pat| matches!(p, Pat::Key(_, m) if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT));
        for entry in table() {
            // Ctrl-C is the quit that also works mid-prompt; `Q` quits everywhere else.
            if entry.cmd == Cmd::QuitNow {
                continue;
            }
            let chorded = entry
                .keys
                .iter()
                .chain(entry.aliases)
                .any(|p| !plain(p) && matches!(p, Pat::Key(..)));
            if chorded {
                assert!(
                    table().iter().filter(|e| e.cmd == entry.cmd).any(|e| e
                        .keys
                        .iter()
                        .chain(e.aliases)
                        .any(plain)),
                    "{:?} is reachable only by a Control or Alt chord",
                    entry.cmd
                );
            }
        }
    }
}

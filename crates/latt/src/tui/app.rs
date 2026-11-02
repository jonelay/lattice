use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use latt_core::graph::{LatticeGraph, Node};
use latt_core::profile::{ActionMode, Condition, Profile};
use latt_core::query::{self, AdjacencyIndex, Direction};
use latt_core::types::{
    AllowedTriple, Endpoint, GraphLayout, Issue, LaneLayout, LaneRow, LayoutNode, Note, Origin,
    Provenance, SchemaGraph, Severity, TraversalReport, Visit,
};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use regex::Regex;

use super::git;
use super::keymap::{self, Cmd, Layer, Section};
use super::owner_action::{self, Refusal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopView {
    NodeList,
    Findings,
    Tree,
    Provenance,
    Graph,
}

pub const VIEWS: [(TopView, char, &str); 5] = [
    (TopView::NodeList, '1', "Nodes"),
    (TopView::Tree, '2', "Tree"),
    (TopView::Findings, '3', "Findings"),
    (TopView::Provenance, '4', "Provenance"),
    (TopView::Graph, '5', "Graph"),
];

impl TopView {
    pub fn tab_index(self) -> usize {
        VIEWS.iter().position(|(v, _, _)| *v == self).unwrap_or(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    KindOrder,
    Severity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    KindFilter,
    EdgeKindFilter,
    Help,
}

pub enum Action {
    None,
    Quit,
    OpenEditor(EditorTarget),
    /// Open the notes file in `$EDITOR`, then reload as `R` does.
    EditNotes(EditorTarget),
    Reload(Reload),
    RunOwner(OwnerRun),
}

/// Where `$EDITOR` opens: a path browse resolved and, when one is known, the line
/// to put the cursor on. Browse's own type, so a jump never rebuilds a provenance
/// to change its path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorTarget {
    pub file: String,
    pub line: Option<NonZeroU32>,
}

impl EditorTarget {
    /// Open `file` at a provenance's line.
    fn at(file: String, provenance: &Provenance) -> Self {
        Self {
            file,
            line: provenance.line(),
        }
    }
}

impl std::fmt::Display for EditorTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}", self.file),
            None => f.write_str(&self.file),
        }
    }
}

/// An owner action resolved, its program found, and cleared to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerRun {
    pub verb: String,
    pub argv: Vec<String>,
    pub program: PathBuf,
    pub mode: ActionMode,
    pub cwd: Option<String>,
}

impl OwnerRun {
    /// The argv as one line for the menu and the confirmation prompt.
    pub fn display(&self) -> String {
        display_argv(&self.argv)
    }
}

/// Join an argv for display, quoting the arguments a reader would otherwise
/// split or miss.
pub fn display_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty()
                || a.chars()
                    .any(|c| c.is_whitespace() || c == '\'' || c == '"')
            {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An action, for first-use confirmation: node kind, verb and the `run`
/// template as written, so an edited template is a different action.
type ActionKey = (String, String, Vec<String>);

/// The `!` menu: the node it lists actions for and its cursor.
pub struct Launcher {
    pub node_id: String,
    pub cursor: usize,
}

/// A capture action's output, shown over the views until closed.
pub struct OutputPane {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: usize,
}

impl OutputPane {
    /// Stdout, then stderr under its own heading; a cut stream says so.
    pub fn new(
        title: String,
        stdout: &str,
        stdout_cut: bool,
        stderr: &str,
        stderr_cut: bool,
    ) -> Self {
        let cut = "[output cut at 1 MiB]".to_owned();
        let mut lines: Vec<String> = stdout.lines().map(str::to_owned).collect();
        if stdout_cut {
            lines.push(cut.clone());
        }
        if !stderr.is_empty() || stderr_cut {
            lines.push("── stderr ──".to_owned());
            lines.extend(stderr.lines().map(str::to_owned));
            if stderr_cut {
                lines.push(cut);
            }
        }
        if lines.is_empty() {
            lines.push("(no output)".to_owned());
        }
        Self {
            title,
            lines,
            scroll: 0,
        }
    }
}

/// A run waiting on the first-use `y`.
pub struct Confirm {
    key: ActionKey,
    pub run: OwnerRun,
}

/// Which reload a key asked for: `Register` re-runs the adapter against the
/// profile already loaded; `All` re-reads the profile first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reload {
    Register,
    All,
}

/// One list view's position and filters, carried across a reload.
struct CarriedList {
    cursor: usize,
    cursor_id: Option<String>,
    sort: SortMode,
    search: Option<Regex>,
    search_input: String,
    hidden_kinds: Vec<String>,
}

/// Browse state that survives a reload. Positions are node IDs, with the old
/// row as the fallback, and filters are kind names: row indices and filter
/// slots belong to the old register and profile. Findings have no ID and keep
/// their row. Tree expansion is lost, being keyed by traversal step.
pub struct Carried {
    top_view: TopView,
    nodes: CarriedList,
    findings: CarriedList,
    show_schema: bool,
    show_suppressed: bool,
    show_detail: bool,
    detail_focused: bool,
    detail_cursor: usize,
    stack: Vec<StackFrame>,
    tree: TreeState,
    tree_cursor_id: Option<String>,
    provenance: ProvenanceState,
    provenance_cursor_id: Option<String>,
    graph: GraphState,
    graph_cursor_id: Option<String>,
    hidden_node_kinds: [Vec<String>; 3],
    hidden_edge_kinds: [Vec<String>; 3],
    confirmed: HashSet<ActionKey>,
    after_reload: Option<String>,
}

pub struct Binding {
    pub key: String,
    pub desc: &'static str,
}

pub struct ViewState {
    pub cursor: usize,
    pub sort: SortMode,
    pub search: Option<Regex>,
    pub search_input: String,
    pub kind_filter: Vec<bool>,
    pub filter_cursor: usize,
}

impl ViewState {
    fn new(kind_count: usize) -> Self {
        Self {
            cursor: 0,
            sort: SortMode::KindOrder,
            search: None,
            search_input: String::new(),
            kind_filter: vec![true; kind_count],
            filter_cursor: 0,
        }
    }
}

pub struct TreeState {
    pub root: Option<String>,
    pub direction: Direction,
    pub expanded: HashSet<usize>,
    pub cursor: usize,
    pub search: Option<Regex>,
    pub search_input: String,
    pub kind_filter: Vec<bool>,
    pub edge_kind_filter: Vec<bool>,
    pub filter_cursor: usize,
    pub scc_collapse: bool,
}

impl TreeState {
    fn new(kind_count: usize, edge_kind_count: usize) -> Self {
        Self {
            root: None,
            direction: Direction::Forward,
            expanded: HashSet::new(),
            cursor: 0,
            search: None,
            search_input: String::new(),
            kind_filter: vec![true; kind_count],
            edge_kind_filter: vec![true; edge_kind_count],
            filter_cursor: 0,
            scc_collapse: false,
        }
    }
}

pub struct ProvenanceState {
    pub root: Option<String>,
    pub direction: Direction,
    pub cursor: usize,
    pub h_scroll: usize,
    pub search: Option<Regex>,
    pub search_input: String,
    pub kind_filter: Vec<bool>,
    pub edge_kind_filter: Vec<bool>,
    pub filter_cursor: usize,
}

impl ProvenanceState {
    fn new(kind_count: usize, edge_kind_count: usize) -> Self {
        Self {
            root: None,
            direction: Direction::Forward,
            cursor: 0,
            h_scroll: 0,
            search: None,
            search_input: String::new(),
            kind_filter: vec![true; kind_count],
            edge_kind_filter: vec![true; edge_kind_count],
            filter_cursor: 0,
        }
    }
}

pub struct GraphState {
    pub center: Option<String>,
    depth_radius: usize,
    pub direction: Direction,
    pub cursor: usize,
    pub center_history: Vec<String>,
    pub search: Option<Regex>,
    pub search_input: String,
    pub kind_filter: Vec<bool>,
    pub edge_kind_filter: Vec<bool>,
    pub filter_cursor: usize,
    pub column_mode: ColumnMode,
    pub search_filter: bool,
}

impl GraphState {
    fn new(kind_count: usize, edge_kind_count: usize) -> Self {
        Self {
            center: None,
            depth_radius: 2,
            direction: Direction::Forward,
            cursor: 0,
            center_history: Vec::new(),
            search: None,
            search_input: String::new(),
            kind_filter: vec![true; kind_count],
            edge_kind_filter: vec![true; edge_kind_count],
            filter_cursor: 0,
            column_mode: ColumnMode::Default,
            search_filter: false,
        }
    }

    pub fn depth_radius(&self) -> usize {
        self.depth_radius
    }

    pub fn increase_depth_radius(&mut self) {
        self.depth_radius += 1;
    }

    /// True when the radius moved; it never drops below 1.
    pub fn decrease_depth_radius(&mut self) -> bool {
        if self.depth_radius <= 1 {
            return false;
        }
        self.depth_radius -= 1;
        true
    }
}

pub struct TreeRow<'a> {
    pub step_index: usize,
    pub depth: usize,
    pub node_id: &'a str,
    pub edge_kind: Option<&'a str>,
    pub endpoint: Endpoint,
    pub visit: Visit,
    pub tainted: bool,
    pub severity: Option<(Severity, usize)>,
    pub summary: Option<String>,
    pub cluster: Option<Vec<String>>,
}

pub struct Preview {
    pub file: String,
    pub lines: Vec<String>,
    /// Markdown parsed once at open; `None` for any other file.
    pub rendered: Option<Text<'static>>,
    pub center_line: usize,
    pub scroll: usize,
}

impl Preview {
    pub fn line_count(&self) -> usize {
        self.rendered
            .as_ref()
            .map_or(self.lines.len(), |text| text.lines.len())
    }
}

/// Render Markdown for the file preview, without the dim modifier anywhere.
///
/// tui-markdown dims front matter, heading metadata, footnotes, HTML and math
/// through its style sheet, and images through a constant no style sheet
/// reaches; dimmed text fails contrast on common themes (WCAG2ICT, G148), so
/// the modifier is removed from every span, line and the text itself.
fn render_markdown(content: &str) -> Text<'static> {
    let undim = |style: Style| style.remove_modifier(Modifier::DIM);
    let parsed = tui_markdown::from_str(content);
    let lines = parsed
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), undim(span.style)))
                .collect();
            let mut owned = Line::from(spans).style(undim(line.style));
            owned.alignment = line.alignment;
            owned
        })
        .collect::<Vec<_>>();
    Text::from(lines).style(undim(parsed.style))
}

/// A view's rows, kept beside the state that shaped them.
///
/// A read compares the current state with the key the rows were built from
/// and rebuilds only on a difference, so a cursor move clamps against rows
/// that already exist instead of walking the report again (in Tree, that
/// walk includes the cluster condensation). Rebuilding the report or layout
/// behind a view clears its cache outright.
struct RowCache<K, R> {
    held: RefCell<Option<(K, R)>>,
}

impl<K, R> RowCache<K, R> {
    fn new() -> Self {
        Self {
            held: RefCell::new(None),
        }
    }

    fn clear(&self) {
        *self.held.borrow_mut() = None;
    }

    /// The held rows when `current` accepts their key, else the rows `build`
    /// returns, held from now on under the key it pairs them with.
    fn rows(&self, current: impl FnOnce(&K) -> bool, build: impl FnOnce() -> (K, R)) -> Ref<'_, R> {
        let stale = self
            .held
            .borrow()
            .as_ref()
            .is_none_or(|(key, _)| !current(key));
        if stale {
            let built = build();
            *self.held.borrow_mut() = Some(built);
        }
        Ref::map(self.held.borrow(), |held| {
            &held.as_ref().expect("filled when stale").1
        })
    }
}

/// The Tree state that shapes rows without rebuilding the traversal.
struct TreeRowsKey {
    scc_collapse: bool,
    kind_filter: Vec<bool>,
    expanded: HashSet<usize>,
}

/// The Graph state that shapes rows without rebuilding the layout.
struct GraphRowsKey {
    kind_filter: Vec<bool>,
    search_filter: bool,
    search: Option<String>,
}

/// One visible Graph row by position in the layout, plus what every row of
/// the layout shares; `MapRowRef` is built from it on demand.
struct GraphRows {
    seeds: Vec<GraphRowSeed>,
    center_layer: usize,
    /// Component index -> index of its first incoming layout edge.
    first_in_edge: HashMap<usize, usize>,
}

#[derive(Clone, Copy)]
struct GraphRowSeed {
    layer: usize,
    idx: usize,
    component: usize,
}

pub struct App<'a> {
    pub profile: &'a Profile,
    pub graph: &'a LatticeGraph,
    pub issues: Vec<Issue>,
    pub has_errors: bool,
    pub index: AdjacencyIndex<'a>,
    pub schema: SchemaGraph,
    /// Node kind -> its slot in every `kind_filter`, which follow the
    /// profile's `node_kinds()` key order (sorted, not declaration order).
    kind_slot: HashMap<&'a str, usize>,
    tree_rows: RowCache<TreeRowsKey, Vec<TreeRow<'a>>>,
    provenance_rows: RowCache<Vec<bool>, Vec<usize>>,
    graph_rows_cache: RowCache<GraphRowsKey, GraphRows>,
    schema_rows: RowCache<(), Vec<SchemaRow>>,
    filters: Vec<Condition>,
    pub top_view: TopView,
    pub node_list_state: ViewState,
    pub findings_state: ViewState,
    pub show_schema: bool,
    pub schema_cursor: usize,
    pub show_detail: bool,
    pub detail_focused: bool,
    pub detail_cursor: usize,
    pub view_stack: Vec<StackFrame>,
    pub show_suppressed: bool,
    pub input_mode: InputMode,
    pub help_scroll: usize,
    /// The furthest the help overlay can scroll, set by the overlay's last
    /// draw: only the renderer knows how many lines fit.
    pub help_scroll_max: Cell<usize>,
    pub input_buffer: String,
    pub status_message: Option<String>,
    pub target_dir: Option<String>,
    node_severity: HashMap<String, (Severity, usize)>,
    pub tree_state: TreeState,
    tree_report: Option<TraversalReport<'a>>,
    pub provenance_state: ProvenanceState,
    pub provenance_layout: Option<LaneLayout>,
    pub graph_state: GraphState,
    pub graph_layout: Option<GraphLayout>,
    pub centrality_cache: HashMap<&'a str, f64>,
    pub preview: Option<Preview>,
    pub launcher: Option<Launcher>,
    pub confirm: Option<Confirm>,
    pub output: Option<OutputPane>,
    /// A message for the status line after the next reload, beside the
    /// reload's own: an interactive action's exit code.
    pub after_reload: Option<String>,
    /// The finding (code, node) whose entry stub `A` last showed: a second `A`
    /// with no key between opens the notes file at its end.
    pending_note: Option<(String, String)>,
    /// Whether `$EDITOR` is set, read once; a field so a test can pin it.
    pub editor_set: bool,
    /// Actions confirmed this session. Carried across reloads, never stored.
    confirmed: HashSet<ActionKey>,
    /// The `PATH` owner programs are looked up on; a field so a test can
    /// pin it rather than depend on the ambient environment.
    path_env: Option<OsString>,
    pub last_key_instant: Instant,
    pub git_commits: Vec<git::GitCommit>,
    /// Lane assignment of `git_commits`, made once when they land.
    pub git_lanes: Vec<git::CommitRow>,
    /// True when the last fetch for `git_commit_node` expired; nothing is
    /// cached, so returning to the node fetches again.
    pub git_fetch_timed_out: bool,
    git_commit_node: Option<String>,
    /// The in-flight `git log` for `git_commit_node`. Replacing it drops the
    /// previous receiver, so a slower older fetch can never land on a newer node.
    git_fetch: Option<Receiver<(String, git::LogFetch)>>,
    git_program: PathBuf,
    git_fetch_bound: Duration,
}

pub struct StackFrame {
    pub node_id: String,
    pub detail_cursor: usize,
    pub source_view: TopView,
    pub source_cursor: usize,
}

impl<'a> App<'a> {
    pub fn new(
        profile: &'a Profile,
        graph: &'a LatticeGraph,
        issues: Vec<Issue>,
        filters: Vec<Condition>,
    ) -> Self {
        let has_errors = issues.iter().any(Issue::gates);
        let index = AdjacencyIndex::build(graph);
        let schema = query::schema_graph(profile);
        let kind_count = profile.node_kinds().len();
        let kind_slot = profile
            .node_kinds()
            .keys()
            .enumerate()
            .map(|(slot, kind)| (kind.as_str(), slot))
            .collect();

        let mut node_severity: HashMap<String, (Severity, usize)> = HashMap::new();
        for issue in &issues {
            if issue.suppressed {
                continue;
            }
            if let Some(ref id) = issue.node_id {
                let entry = node_severity
                    .entry(id.clone())
                    .or_insert((Severity::Hint, 0));
                if issue.severity < entry.0 {
                    entry.0 = issue.severity;
                }
                entry.1 += 1;
            }
        }

        Self {
            profile,
            graph,
            issues,
            has_errors,
            index,
            schema,
            kind_slot,
            tree_rows: RowCache::new(),
            provenance_rows: RowCache::new(),
            graph_rows_cache: RowCache::new(),
            schema_rows: RowCache::new(),
            filters,
            top_view: TopView::NodeList,
            node_list_state: ViewState::new(kind_count),
            findings_state: ViewState::new(kind_count),
            show_schema: false,
            schema_cursor: 0,
            show_detail: false,
            detail_focused: false,
            detail_cursor: 0,
            view_stack: Vec::new(),
            show_suppressed: false,
            input_mode: InputMode::Normal,
            help_scroll: 0,
            help_scroll_max: Cell::new(0),
            input_buffer: String::new(),
            status_message: None,
            target_dir: None,
            node_severity,
            tree_state: TreeState::new(kind_count, profile.edge_kinds().len()),
            tree_report: None,
            provenance_state: ProvenanceState::new(kind_count, profile.edge_kinds().len()),
            provenance_layout: None,
            graph_state: GraphState::new(kind_count, profile.edge_kinds().len()),
            graph_layout: None,
            centrality_cache: HashMap::new(),
            preview: None,
            launcher: None,
            confirm: None,
            output: None,
            after_reload: None,
            pending_note: None,
            editor_set: std::env::var("EDITOR").is_ok_and(|editor| !editor.is_empty()),
            confirmed: HashSet::new(),
            path_env: std::env::var_os("PATH"),
            last_key_instant: Instant::now(),
            git_commits: Vec::new(),
            git_lanes: Vec::new(),
            git_fetch_timed_out: false,
            git_commit_node: None,
            git_fetch: None,
            git_program: PathBuf::from("git"),
            git_fetch_bound: git::FETCH_BOUND,
        }
    }

    pub fn set_target_dir(&mut self, target: &Path) {
        self.target_dir = Some(target.display().to_string());
    }

    /// Point the provenance fetch at another `git` with its own bound.
    #[cfg(test)]
    fn use_git(&mut self, program: &Path, bound: Duration) {
        self.git_program = program.to_path_buf();
        self.git_fetch_bound = bound;
    }

    fn step_back(&mut self) -> bool {
        if let Some(frame) = self.view_stack.pop() {
            self.detail_cursor = frame.detail_cursor;
            self.top_view = frame.source_view;
            match frame.source_view {
                TopView::NodeList => {
                    self.node_list_state.cursor = frame.source_cursor;
                }
                TopView::Findings => {
                    self.findings_state.cursor = frame.source_cursor;
                    self.show_detail = false;
                    self.detail_focused = false;
                }
                TopView::Tree => {
                    self.tree_state.cursor = frame.source_cursor;
                    self.show_detail = false;
                    self.detail_focused = false;
                }
                TopView::Provenance => {
                    self.provenance_state.cursor = frame.source_cursor;
                    self.show_detail = false;
                    self.detail_focused = false;
                }
                TopView::Graph => {
                    self.graph_state.cursor = frame.source_cursor;
                    self.show_detail = false;
                    self.detail_focused = false;
                }
            }
            return true;
        }
        if self.detail_focused {
            self.detail_focused = false;
            return true;
        }
        if self.show_detail {
            self.show_detail = false;
            return true;
        }
        false
    }

    pub fn toggle_search_filter(&mut self) {
        if self.top_view == TopView::Graph && self.graph_state.search.is_some() {
            self.graph_state.search_filter = !self.graph_state.search_filter;
            self.graph_state.cursor = 0;
        }
    }

    /// Put the graph cursor on the center node, which the view scrolls to keep in
    /// sight; a centring command means that node is the subject.
    fn select_graph_center(&mut self) {
        if let Some(i) = self.visible_graph_rows().iter().position(|r| r.is_center) {
            self.graph_state.cursor = i;
        }
    }

    /// Give up this `App`, keeping what a reload should restore.
    pub fn into_carried(self) -> Carried {
        let node_id = self
            .visible_nodes()
            .get(self.node_list_state.cursor)
            .map(|(id, _, _)| id.to_string());
        let tree_cursor_id = self
            .visible_tree_rows()
            .get(self.tree_state.cursor)
            .map(|r| r.node_id.to_string());
        let provenance_cursor_id = self.provenance_cursor_row().map(|r| r.id.clone());
        let graph_cursor_id = self.graph_cursor_node_id();
        let hidden = |filter: &[bool], names: Vec<&String>| -> Vec<String> {
            names
                .into_iter()
                .zip(filter)
                .filter(|(_, shown)| !**shown)
                .map(|(name, _)| name.clone())
                .collect()
        };
        let node_kinds = || self.profile.node_kinds().keys().collect::<Vec<_>>();
        let edge_kinds = || self.profile.edge_kinds().keys().collect::<Vec<_>>();
        let hidden_node_kinds = [
            hidden(&self.tree_state.kind_filter, node_kinds()),
            hidden(&self.provenance_state.kind_filter, node_kinds()),
            hidden(&self.graph_state.kind_filter, node_kinds()),
        ];
        let hidden_edge_kinds = [
            hidden(&self.tree_state.edge_kind_filter, edge_kinds()),
            hidden(&self.provenance_state.edge_kind_filter, edge_kinds()),
            hidden(&self.graph_state.edge_kind_filter, edge_kinds()),
        ];
        let nodes_hidden = hidden(&self.node_list_state.kind_filter, node_kinds());
        let findings_hidden = hidden(&self.findings_state.kind_filter, node_kinds());
        Carried {
            top_view: self.top_view,
            nodes: CarriedList {
                cursor: self.node_list_state.cursor,
                cursor_id: node_id,
                sort: self.node_list_state.sort,
                search: self.node_list_state.search,
                search_input: self.node_list_state.search_input,
                hidden_kinds: nodes_hidden,
            },
            findings: CarriedList {
                cursor: self.findings_state.cursor,
                cursor_id: None,
                sort: self.findings_state.sort,
                search: self.findings_state.search,
                search_input: self.findings_state.search_input,
                hidden_kinds: findings_hidden,
            },
            show_schema: self.show_schema,
            show_suppressed: self.show_suppressed,
            show_detail: self.show_detail,
            detail_focused: self.detail_focused,
            detail_cursor: self.detail_cursor,
            stack: self.view_stack,
            tree: self.tree_state,
            tree_cursor_id,
            provenance: self.provenance_state,
            provenance_cursor_id,
            graph: self.graph_state,
            graph_cursor_id,
            hidden_node_kinds,
            hidden_edge_kinds,
            confirmed: self.confirmed,
            after_reload: self.after_reload,
        }
    }

    /// Apply what a reload carried over, dropping each position whose node
    /// the new register no longer declares.
    pub fn restore(&mut self, mut c: Carried) {
        let node_cursor_id = c.nodes.cursor_id.take();
        let shown = |names: Vec<&String>, hidden: &[String]| -> Vec<bool> {
            names.iter().map(|n| !hidden.contains(n)).collect()
        };
        let node_kinds = || self.profile.node_kinds().keys().collect::<Vec<_>>();
        let edge_kinds = || self.profile.edge_kinds().keys().collect::<Vec<_>>();
        let [tree_kinds, prov_kinds, graph_kinds] = c.hidden_node_kinds;
        let [tree_edges, prov_edges, graph_edges] = c.hidden_edge_kinds;
        self.confirmed = c.confirmed;
        self.after_reload = c.after_reload;
        let node_filters = [
            shown(node_kinds(), &c.nodes.hidden_kinds),
            shown(node_kinds(), &c.findings.hidden_kinds),
            shown(node_kinds(), &tree_kinds),
            shown(node_kinds(), &prov_kinds),
            shown(node_kinds(), &graph_kinds),
        ];
        let edge_filters = [
            shown(edge_kinds(), &tree_edges),
            shown(edge_kinds(), &prov_edges),
            shown(edge_kinds(), &graph_edges),
        ];
        let [nodes_f, findings_f, tree_f, prov_f, graph_f] = node_filters;
        let [tree_e, prov_e, graph_e] = edge_filters;

        for (state, list, filter) in [
            (&mut self.node_list_state, c.nodes, nodes_f),
            (&mut self.findings_state, c.findings, findings_f),
        ] {
            state.sort = list.sort;
            state.search = list.search;
            state.search_input = list.search_input;
            state.kind_filter = filter;
            state.cursor = list.cursor;
        }
        let nodes = self.visible_nodes();
        let last = nodes.len().saturating_sub(1);
        let node_cursor = match node_cursor_id {
            Some(id) => nodes.iter().position(|(n, _, _)| *n == id),
            None => None,
        };
        let node_found = node_cursor.is_some();
        let node_cursor = node_cursor.unwrap_or(self.node_list_state.cursor.min(last));
        drop(nodes);
        self.node_list_state.cursor = node_cursor;
        let findings_last = self.visible_findings().len().saturating_sub(1);
        self.findings_state.cursor = self.findings_state.cursor.min(findings_last);
        self.show_schema = c.show_schema;
        self.show_suppressed = c.show_suppressed;

        let survives =
            |id: &Option<String>| id.as_ref().filter(|id| self.graph.has_node(id)).cloned();
        let tree_root = survives(&c.tree.root);
        self.tree_state = TreeState {
            root: tree_root,
            kind_filter: tree_f,
            edge_kind_filter: tree_e,
            expanded: HashSet::new(),
            ..c.tree
        };
        self.rebuild_tree();
        if let Some(i) = c.tree_cursor_id.and_then(|id| {
            self.visible_tree_rows()
                .iter()
                .position(|r| r.node_id == id)
        }) {
            self.tree_state.cursor = i;
        }

        let provenance_root = survives(&c.provenance.root);
        let h_scroll = c.provenance.h_scroll;
        self.provenance_state = ProvenanceState {
            root: provenance_root,
            kind_filter: prov_f,
            edge_kind_filter: prov_e,
            ..c.provenance
        };
        self.rebuild_provenance();
        self.provenance_state.h_scroll = h_scroll;
        if let (Some(id), Some(layout)) = (c.provenance_cursor_id, &self.provenance_layout) {
            let vis = self.visible_provenance_indices();
            if let Some(i) = vis
                .iter()
                .position(|&ri| layout.rows.get(ri).is_some_and(|r| r.id == id))
            {
                drop(vis);
                self.provenance_state.cursor = i;
            }
        }

        let center = survives(&c.graph.center);
        let history = c
            .graph
            .center_history
            .iter()
            .filter(|id| self.graph.has_node(id))
            .cloned()
            .collect();
        self.graph_state = GraphState {
            center,
            center_history: history,
            kind_filter: graph_f,
            edge_kind_filter: graph_e,
            cursor: 0,
            ..c.graph
        };
        if self.graph_state.center.is_some() {
            self.rebuild_graph_and_centrality();
            if let Some(i) = c
                .graph_cursor_id
                .and_then(|id| self.visible_graph_rows().iter().position(|r| r.id == id))
            {
                self.graph_state.cursor = i;
            }
        }

        let anchored = match c.top_view {
            TopView::Tree => self.tree_state.root.is_some(),
            TopView::Provenance => self.provenance_state.root.is_some(),
            TopView::Graph => self.graph_state.center.is_some(),
            TopView::NodeList | TopView::Findings => true,
        };
        self.top_view = if anchored {
            c.top_view
        } else {
            TopView::NodeList
        };
        // A stack left over a view that fell back would pop into a view with
        // no root.
        self.view_stack = if anchored {
            c.stack
                .into_iter()
                .take_while(|frame| self.graph.has_node(&frame.node_id))
                .collect()
        } else {
            Vec::new()
        };
        let detail_survives = anchored
            && (!self.view_stack.is_empty() || self.top_view != TopView::NodeList || node_found);
        if c.show_detail && detail_survives {
            self.show_detail = true;
            self.detail_focused = c.detail_focused;
            self.detail_cursor = c
                .detail_cursor
                .min(self.detail_rows().len().saturating_sub(1));
        }
    }

    #[cfg(test)]
    pub fn handle_key(&mut self, key: KeyCode) -> Action {
        self.handle_key_event(KeyEvent::from(key))
    }

    /// Resolve `key` through the key table and run its command. `Always`
    /// rows skip the bookkeeping below, as the event loop's own chords did.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Action {
        if let Some(cmd) = keymap::lookup(self, Layer::Always, key.code, key.modifiers) {
            self.pending_note = None;
            return self.run(cmd, key.code);
        }
        self.last_key_instant = Instant::now();
        self.status_message = None;
        let cmd = keymap::lookup(self, keymap::layer(self), key.code, key.modifiers);
        if !matches!(cmd, Some(Cmd::Note)) {
            self.pending_note = None;
        }
        let action = match cmd {
            Some(cmd) => self.run(cmd, key.code),
            None => Action::None,
        };
        if self.top_view == TopView::Provenance {
            self.update_provenance_commits();
        }
        action
    }

    fn run(&mut self, cmd: Cmd, key: KeyCode) -> Action {
        match cmd {
            Cmd::QuitNow | Cmd::Quit => return Action::Quit,
            Cmd::ToggleSearchFilter => self.toggle_search_filter(),
            Cmd::ClosePreview => self.preview = None,
            Cmd::PreviewDown => {
                if let Some(ref mut pv) = self.preview
                    && pv.scroll + 1 < pv.line_count()
                {
                    pv.scroll += 1;
                }
            }
            Cmd::PreviewUp => {
                if let Some(ref mut pv) = self.preview {
                    pv.scroll = pv.scroll.saturating_sub(1);
                }
            }
            Cmd::PreviewTop => {
                if let Some(ref mut pv) = self.preview {
                    pv.scroll = 0;
                }
            }
            Cmd::PreviewPageDown => {
                if let Some(ref mut pv) = self.preview {
                    pv.scroll = (pv.scroll + PAGE).min(pv.line_count().saturating_sub(1));
                }
            }
            Cmd::PreviewPageUp => {
                if let Some(ref mut pv) = self.preview {
                    pv.scroll = pv.scroll.saturating_sub(PAGE);
                }
            }
            Cmd::PreviewBottom => {
                if let Some(ref mut pv) = self.preview {
                    pv.scroll = pv.line_count().saturating_sub(1);
                }
            }
            Cmd::SearchSubmit => self.submit_search(),
            Cmd::SearchCancel => self.input_mode = InputMode::Normal,
            Cmd::SearchType => {
                if let KeyCode::Char(c) = key {
                    self.input_buffer.push(c);
                }
            }
            Cmd::SearchBackspace => {
                self.input_buffer.pop();
            }
            Cmd::KindFilterDown => {
                let kinds_len = self.profile.node_kinds().len();
                let (filter_cursor, _) = self.kind_filter_parts_mut();
                if *filter_cursor + 1 < kinds_len {
                    *filter_cursor += 1;
                }
            }
            Cmd::KindFilterUp => {
                let (filter_cursor, _) = self.kind_filter_parts_mut();
                *filter_cursor = filter_cursor.saturating_sub(1);
            }
            Cmd::KindFilterToggle => {
                let (filter_cursor, kind_filter) = self.kind_filter_parts_mut();
                let idx = *filter_cursor;
                if idx < kind_filter.len() {
                    kind_filter[idx] = !kind_filter[idx];
                }
            }
            Cmd::KindFilterClose => {
                let (filter_cursor, _) = self.kind_filter_parts_mut();
                *filter_cursor = 0;
                self.input_mode = InputMode::Normal;
            }
            Cmd::EdgeFilterDown => {
                let kinds_len = self.profile.edge_kinds().len();
                let (filter_cursor, _) = self.edge_filter_parts_mut();
                if *filter_cursor + 1 < kinds_len {
                    *filter_cursor += 1;
                }
            }
            Cmd::EdgeFilterUp => {
                let (filter_cursor, _) = self.edge_filter_parts_mut();
                *filter_cursor = filter_cursor.saturating_sub(1);
            }
            Cmd::EdgeFilterToggle => {
                let (filter_cursor, edge_kind_filter) = self.edge_filter_parts_mut();
                let idx = *filter_cursor;
                if idx < edge_kind_filter.len() {
                    edge_kind_filter[idx] = !edge_kind_filter[idx];
                }
            }
            Cmd::EdgeFilterClose => {
                self.input_mode = InputMode::Normal;
                match self.top_view {
                    TopView::Provenance => self.rebuild_provenance(),
                    TopView::Graph => self.rebuild_graph_and_centrality(),
                    _ => self.rebuild_tree(),
                }
            }
            Cmd::CloseHelp => self.input_mode = InputMode::Normal,
            Cmd::HelpDown => self.scroll_help(1, true),
            Cmd::HelpUp => self.scroll_help(1, false),
            Cmd::HelpPageDown => self.scroll_help(PAGE, true),
            Cmd::HelpPageUp => self.scroll_help(PAGE, false),
            Cmd::HelpTop => self.scroll_help(TO_END, false),
            Cmd::HelpBottom => self.scroll_help(TO_END, true),
            Cmd::ShowView(TopView::NodeList) => {
                self.top_view = TopView::NodeList;
                self.view_stack.clear();
                self.show_detail = false;
                self.detail_focused = false;
            }
            Cmd::ShowView(TopView::Tree) => self.activate_tree_view(),
            Cmd::ShowView(TopView::Findings) => {
                self.top_view = TopView::Findings;
                self.view_stack.clear();
                self.show_detail = false;
                self.detail_focused = false;
            }
            Cmd::ShowView(TopView::Provenance) => self.activate_provenance_view(),
            Cmd::ShowView(TopView::Graph) => self.activate_graph_view(),
            Cmd::BackOrQuit => {
                if !self.step_back() {
                    return Action::Quit;
                }
            }
            Cmd::Escape => {
                self.step_back();
            }
            Cmd::PopCenter => {
                if let Some(prev) = self.graph_state.center_history.pop() {
                    self.graph_state.center = Some(prev);
                    self.rebuild_graph();
                    self.select_graph_center();
                }
            }
            Cmd::RadiusUp => {
                self.graph_state.increase_depth_radius();
                self.rebuild_graph();
            }
            Cmd::RadiusDown => {
                if self.graph_state.decrease_depth_radius() {
                    self.rebuild_graph();
                }
            }
            Cmd::ToggleDirection => match self.top_view {
                TopView::Tree => {
                    self.tree_state.direction = match self.tree_state.direction {
                        Direction::Forward => Direction::Reverse,
                        Direction::Reverse => Direction::Forward,
                    };
                    self.rebuild_tree();
                }
                TopView::Provenance => {
                    self.provenance_state.direction = match self.provenance_state.direction {
                        Direction::Forward => Direction::Reverse,
                        Direction::Reverse => Direction::Forward,
                    };
                    self.rebuild_provenance();
                }
                TopView::Graph => {
                    self.graph_state.direction = match self.graph_state.direction {
                        Direction::Forward => Direction::Reverse,
                        Direction::Reverse => Direction::Forward,
                    };
                    self.rebuild_graph();
                }
                TopView::NodeList | TopView::Findings => {}
            },
            Cmd::ScrollLeft => {
                self.provenance_state.h_scroll = self.provenance_state.h_scroll.saturating_sub(1);
            }
            Cmd::ScrollRight => {
                self.provenance_state.h_scroll += 1;
            }
            Cmd::EdgeFilter => {
                self.input_mode = InputMode::EdgeKindFilter;
                let (filter_cursor, _) = self.edge_filter_parts_mut();
                *filter_cursor = 0;
            }
            Cmd::CycleColumns => {
                self.graph_state.column_mode = self.graph_state.column_mode.cycle();
            }
            Cmd::SccCollapse => {
                self.tree_state.scc_collapse = !self.tree_state.scc_collapse;
            }
            Cmd::TreeExpand => self.tree_expand_toggle(),
            Cmd::TreeCollapse => self.tree_collapse(),
            Cmd::ToggleSchema => {
                self.show_schema = !self.show_schema;
                self.schema_cursor = 0;
            }
            Cmd::CycleSort => {
                let state = self.current_state_mut();
                state.sort = match state.sort {
                    SortMode::KindOrder => SortMode::Severity,
                    SortMode::Severity => SortMode::KindOrder,
                };
            }
            Cmd::Help => {
                self.input_mode = InputMode::Help;
                self.help_scroll = 0;
            }
            Cmd::KindFilter => {
                self.input_mode = InputMode::KindFilter;
                let (fc, _) = self.kind_filter_parts_mut();
                *fc = 0;
            }
            Cmd::KindFilterRefused => {
                let why = if self.top_view == TopView::Provenance {
                    "not in Provenance; F filters edge kinds"
                } else if self.schema_mode() {
                    "not in the schema view"
                } else {
                    "it filters the list; focus the list first"
                };
                self.status_message = Some(format!("kind filter: {why}"));
            }
            Cmd::ToggleSuppressed => {
                self.show_suppressed = !self.show_suppressed;
            }
            Cmd::Search => {
                self.input_mode = InputMode::Search;
                self.input_buffer.clear();
            }
            Cmd::NextMatch => self.jump_to_next_match(true),
            Cmd::PrevMatch => self.jump_to_next_match(false),
            Cmd::Editor => return self.edit(),
            Cmd::Note => return self.note(),
            Cmd::Owner(verb) => return self.owner_verb(verb),
            Cmd::Launcher => self.open_launcher(),
            Cmd::LauncherDown => {
                let len = self.launcher_rows().len();
                if let Some(ref mut l) = self.launcher {
                    l.cursor = (l.cursor + 1).min(len.saturating_sub(1));
                }
            }
            Cmd::LauncherUp => {
                if let Some(ref mut l) = self.launcher {
                    l.cursor = l.cursor.saturating_sub(1);
                }
            }
            Cmd::LauncherClose => self.launcher = None,
            Cmd::LauncherRun => return self.run_launcher_row(),
            Cmd::OutputDown => {
                if let Some(ref mut o) = self.output {
                    o.scroll = (o.scroll + 1).min(o.lines.len().saturating_sub(1));
                }
            }
            Cmd::OutputUp => {
                if let Some(ref mut o) = self.output {
                    o.scroll = o.scroll.saturating_sub(1);
                }
            }
            Cmd::OutputPageDown => {
                if let Some(ref mut o) = self.output {
                    o.scroll = (o.scroll + PAGE).min(o.lines.len().saturating_sub(1));
                }
            }
            Cmd::OutputPageUp => {
                if let Some(ref mut o) = self.output {
                    o.scroll = o.scroll.saturating_sub(PAGE);
                }
            }
            Cmd::OutputTop => {
                if let Some(ref mut o) = self.output {
                    o.scroll = 0;
                }
            }
            Cmd::OutputBottom => {
                if let Some(ref mut o) = self.output {
                    o.scroll = o.lines.len().saturating_sub(1);
                }
            }
            Cmd::CloseOutput => self.output = None,
            Cmd::ConfirmYes => {
                if let Some(c) = self.confirm.take() {
                    self.confirmed.insert(c.key);
                    return Action::RunOwner(c.run);
                }
            }
            Cmd::ConfirmCancel => {
                if let Some(c) = self.confirm.take() {
                    self.status_message = Some(format!("{} not run", c.run.verb));
                }
            }
            Cmd::Preview => self.open_preview(),
            Cmd::Down => {
                if self.detail_focused {
                    let max = self.detail_rows().len().saturating_sub(1);
                    if self.detail_cursor < max {
                        self.detail_cursor += 1;
                    }
                } else if self.schema_mode() {
                    self.move_schema_cursor(1);
                } else if self.top_view == TopView::Tree {
                    let max = self.visible_tree_rows().len().saturating_sub(1);
                    if self.tree_state.cursor < max {
                        self.tree_state.cursor += 1;
                    }
                } else if self.top_view == TopView::Provenance {
                    let max = self.provenance_row_count().saturating_sub(1);
                    if self.provenance_state.cursor < max {
                        self.provenance_state.cursor += 1;
                    }
                } else if self.top_view == TopView::Graph {
                    let max = self.graph_row_count().saturating_sub(1);
                    if self.graph_state.cursor < max {
                        self.graph_state.cursor += 1;
                    }
                } else {
                    let max = self.list_len().saturating_sub(1);
                    let state = self.current_state_mut();
                    if state.cursor < max {
                        state.cursor += 1;
                    }
                }
            }
            Cmd::Up => {
                if self.detail_focused {
                    self.detail_cursor = self.detail_cursor.saturating_sub(1);
                } else if self.schema_mode() {
                    self.move_schema_cursor(-1);
                } else if self.top_view == TopView::Tree {
                    self.tree_state.cursor = self.tree_state.cursor.saturating_sub(1);
                } else if self.top_view == TopView::Provenance {
                    self.provenance_state.cursor = self.provenance_state.cursor.saturating_sub(1);
                } else if self.top_view == TopView::Graph {
                    self.graph_state.cursor = self.graph_state.cursor.saturating_sub(1);
                } else {
                    let state = self.current_state_mut();
                    state.cursor = state.cursor.saturating_sub(1);
                }
            }
            Cmd::PageDown => self.move_by(PAGE, true),
            Cmd::PageUp => self.move_by(PAGE, false),
            Cmd::Top => self.move_by(TO_END, false),
            Cmd::Bottom => self.move_by(TO_END, true),
            Cmd::Deeper => {
                if self.show_detail && !self.detail_focused {
                    self.detail_focused = true;
                } else if !self.detail_focused {
                    return self.handle_enter();
                }
            }
            Cmd::Left => {
                if !self.step_back() {
                    match self.top_view {
                        TopView::Graph => {
                            if let Some(prev) = self.graph_state.center_history.pop() {
                                self.graph_state.center = Some(prev);
                                self.rebuild_graph();
                                self.select_graph_center();
                            }
                        }
                        TopView::Tree => self.tree_collapse(),
                        _ => {}
                    }
                }
            }
            Cmd::GraphTab => {
                if let Some(frame) = self.view_stack.pop() {
                    self.graph_state.cursor = frame.source_cursor;
                    self.show_detail = false;
                    self.detail_focused = false;
                } else if let Some(node_id) = self.graph_cursor_node_id() {
                    // Push detail
                    self.view_stack.push(StackFrame {
                        node_id,
                        detail_cursor: self.detail_cursor,
                        source_view: TopView::Graph,
                        source_cursor: self.graph_state.cursor,
                    });
                    self.show_detail = true;
                    self.detail_focused = true;
                    self.detail_cursor = 0;
                }
            }
            Cmd::FocusDetail => {
                self.detail_focused = !self.detail_focused;
            }
            Cmd::Enter => return self.handle_enter(),
            Cmd::Reload => return Action::Reload(Reload::Register),
            Cmd::ReloadAll => return Action::Reload(Reload::All),
        }
        Action::None
    }

    fn scroll_help(&mut self, rows: usize, down: bool) {
        self.help_scroll = step(self.help_scroll, rows, down, self.help_scroll_max.get());
    }

    /// Move the focused cursor `rows` down or up, clamped to its list.
    fn move_by(&mut self, rows: usize, down: bool) {
        if self.detail_focused {
            let max = self.detail_rows().len().saturating_sub(1);
            self.detail_cursor = step(self.detail_cursor, rows, down, max);
        } else if self.schema_mode() {
            let delta = rows.min(isize::MAX as usize / 2) as isize;
            self.move_schema_cursor(if down { delta } else { -delta });
        } else if self.top_view == TopView::Tree {
            let max = self.visible_tree_rows().len().saturating_sub(1);
            self.tree_state.cursor = step(self.tree_state.cursor, rows, down, max);
        } else if self.top_view == TopView::Provenance {
            let max = self.provenance_row_count().saturating_sub(1);
            self.provenance_state.cursor = step(self.provenance_state.cursor, rows, down, max);
        } else if self.top_view == TopView::Graph {
            let max = self.graph_row_count().saturating_sub(1);
            self.graph_state.cursor = step(self.graph_state.cursor, rows, down, max);
        } else {
            let max = self.list_len().saturating_sub(1);
            let state = self.current_state_mut();
            state.cursor = step(state.cursor, rows, down, max);
        }
    }

    fn submit_search(&mut self) {
        if self.input_buffer.is_empty() {
            match self.top_view {
                TopView::Tree => self.tree_state.search = None,
                TopView::Provenance => self.provenance_state.search = None,
                TopView::Graph => {
                    self.graph_state.search = None;
                    self.graph_state.search_filter = false;
                }
                _ => self.current_state_mut().search = None,
            }
        } else {
            let buf = self.input_buffer.clone();
            match Regex::new(&buf) {
                Ok(re) => {
                    match self.top_view {
                        TopView::Tree => {
                            self.tree_state.search_input = buf;
                            self.tree_state.search = Some(re);
                        }
                        TopView::Provenance => {
                            self.provenance_state.search_input = buf;
                            self.provenance_state.search = Some(re);
                        }
                        TopView::Graph => {
                            self.graph_state.search_input = buf;
                            self.graph_state.search = Some(re);
                        }
                        _ => {
                            let state = self.current_state_mut();
                            state.search_input = buf;
                            state.search = Some(re);
                        }
                    }
                    self.jump_to_next_match(true);
                }
                Err(e) => {
                    self.status_message = Some(format!("invalid regex: {e}"));
                }
            }
        }
        self.input_mode = InputMode::Normal;
    }

    fn edge_filter_parts_mut(&mut self) -> (&mut usize, &mut Vec<bool>) {
        match self.top_view {
            TopView::Provenance => (
                &mut self.provenance_state.filter_cursor,
                &mut self.provenance_state.edge_kind_filter,
            ),
            TopView::Graph => (
                &mut self.graph_state.filter_cursor,
                &mut self.graph_state.edge_kind_filter,
            ),
            _ => (
                &mut self.tree_state.filter_cursor,
                &mut self.tree_state.edge_kind_filter,
            ),
        }
    }

    fn kind_filter_parts_mut(&mut self) -> (&mut usize, &mut Vec<bool>) {
        match self.top_view {
            TopView::Graph => (
                &mut self.graph_state.filter_cursor,
                &mut self.graph_state.kind_filter,
            ),
            TopView::Tree => (
                &mut self.tree_state.filter_cursor,
                &mut self.tree_state.kind_filter,
            ),
            TopView::Provenance => (
                &mut self.provenance_state.filter_cursor,
                &mut self.provenance_state.kind_filter,
            ),
            TopView::Findings => (
                &mut self.findings_state.filter_cursor,
                &mut self.findings_state.kind_filter,
            ),
            TopView::NodeList => (
                &mut self.node_list_state.filter_cursor,
                &mut self.node_list_state.kind_filter,
            ),
        }
    }

    fn kind_filter_label(&self) -> String {
        let (_, filter) = self.kind_filter_parts();
        let active: Vec<&str> = self
            .profile
            .node_kinds()
            .keys()
            .zip(filter.iter())
            .filter(|(_, enabled)| !**enabled)
            .map(|(name, _)| name.as_str())
            .collect();
        if active.is_empty() {
            String::new()
        } else {
            format!(" [-{}]", active.join(",-"))
        }
    }

    pub fn kind_filter_parts(&self) -> (usize, &[bool]) {
        match self.top_view {
            TopView::Graph => (
                self.graph_state.filter_cursor,
                &self.graph_state.kind_filter,
            ),
            TopView::Tree => (self.tree_state.filter_cursor, &self.tree_state.kind_filter),
            TopView::Provenance => (
                self.provenance_state.filter_cursor,
                &self.provenance_state.kind_filter,
            ),
            TopView::Findings => (
                self.findings_state.filter_cursor,
                &self.findings_state.kind_filter,
            ),
            TopView::NodeList => (
                self.node_list_state.filter_cursor,
                &self.node_list_state.kind_filter,
            ),
        }
    }

    fn handle_enter(&mut self) -> Action {
        if self.top_view == TopView::Graph && !self.detail_focused {
            let rows = self.visible_graph_rows();
            let cursor = self.graph_state.cursor.min(rows.len().saturating_sub(1));
            if let Some(row) = rows.get(cursor) {
                if row.is_center || row.overflow.is_some() {
                    return Action::None;
                }
                let node_id = row.id.to_string();
                if let Some(previous) = self.graph_state.center.replace(node_id) {
                    self.graph_state.center_history.push(previous);
                }
                self.rebuild_graph();
                self.select_graph_center();
            }
            return Action::None;
        }
        match self.top_view {
            TopView::NodeList if self.show_schema => {}
            TopView::NodeList if self.detail_focused => {
                self.follow_detail_edge();
            }
            TopView::NodeList => {
                self.show_detail = true;
                self.detail_cursor = 0;
            }
            TopView::Tree if self.detail_focused => {
                self.follow_detail_edge();
            }
            TopView::Tree => {
                let node_id = {
                    let rows = self.visible_tree_rows();
                    let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
                    rows.get(cursor).map(|row| row.node_id.to_string())
                };
                if let Some(node_id) = node_id
                    && self.graph.has_node(&node_id)
                {
                    let source_cursor = self.tree_state.cursor;
                    self.view_stack.push(StackFrame {
                        node_id,
                        detail_cursor: self.detail_cursor,
                        source_view: TopView::Tree,
                        source_cursor,
                    });
                    self.show_detail = true;
                    self.detail_focused = true;
                    self.detail_cursor = 0;
                }
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                let cursor = self
                    .findings_state
                    .cursor
                    .min(findings.len().saturating_sub(1));
                if let Some(issue) = findings.get(cursor)
                    && let Some(ref id) = issue.node_id
                    && self.graph.has_node(id)
                {
                    let id = id.clone();
                    let source_cursor = self.findings_state.cursor;
                    self.view_stack.push(StackFrame {
                        node_id: id.clone(),
                        detail_cursor: self.detail_cursor,
                        source_view: TopView::Findings,
                        source_cursor,
                    });
                    // The list highlights its cursor row beside the detail, so
                    // the row must be the finding's node; one hidden by a search
                    // or filter has no row and the cursor stays.
                    let row = self
                        .visible_nodes()
                        .iter()
                        .position(|(node, _, _)| *node == id.as_str());
                    if let Some(row) = row {
                        self.node_list_state.cursor = row;
                    }
                    self.top_view = TopView::NodeList;
                    self.show_detail = true;
                    self.detail_focused = true;
                    self.detail_cursor = 0;
                }
            }
            TopView::Provenance if self.detail_focused => {
                self.follow_detail_edge();
            }
            TopView::Provenance => {
                if let Some(row) = self.provenance_cursor_row() {
                    let node_id = row.id.clone();
                    if self.graph.has_node(&node_id) {
                        let source_cursor = self.provenance_state.cursor;
                        self.view_stack.push(StackFrame {
                            node_id,
                            detail_cursor: self.detail_cursor,
                            source_view: TopView::Provenance,
                            source_cursor,
                        });
                        self.show_detail = true;
                        self.detail_focused = true;
                        self.detail_cursor = 0;
                    }
                }
            }
            TopView::Graph => {
                if self.detail_focused {
                    self.follow_detail_edge();
                }
            }
        }
        Action::None
    }

    fn follow_detail_edge(&mut self) {
        let detail = self.detail_rows();
        let source_view = self.top_view;
        let source_cursor = match source_view {
            TopView::Tree => self.tree_state.cursor,
            TopView::Provenance => self.provenance_state.cursor,
            TopView::Graph => self.graph_state.cursor,
            _ => self.node_list_state.cursor,
        };
        if let Some(row) = detail.get(self.detail_cursor) {
            let node_id = match row {
                DetailRow::OutEdge { tgt, .. } => Some(tgt.to_string()),
                DetailRow::InEdge { src, .. } => Some(src.to_string()),
                _ => None,
            };
            if let Some(node_id) = node_id {
                self.view_stack.push(StackFrame {
                    node_id,
                    detail_cursor: self.detail_cursor,
                    source_view,
                    source_cursor,
                });
                self.detail_cursor = 0;
            }
        }
    }

    /// Move the cursor to the next row the view's search matches, wrapping
    /// around; the rows are built once per jump, not once per candidate.
    fn jump_to_next_match(&mut self, forward: bool) {
        let re = match self.top_view {
            TopView::Tree => self.tree_state.search.clone(),
            TopView::Provenance => self.provenance_state.search.clone(),
            TopView::Graph => self.graph_state.search.clone(),
            _ => self.current_state().search.clone(),
        };
        let Some(re) = re else { return };

        let cursor = match self.top_view {
            TopView::Tree => self.tree_state.cursor,
            TopView::Provenance => self.provenance_state.cursor,
            TopView::Graph => self.graph_state.cursor,
            _ => self.current_state().cursor,
        };

        let found = match self.top_view {
            TopView::NodeList => {
                let nodes = self.visible_nodes();
                search_order(cursor, nodes.len(), forward).find(|&i| {
                    let (id, _, summary) = &nodes[i];
                    re.is_match(id) || summary.as_ref().is_some_and(|s| re.is_match(s))
                })
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                search_order(cursor, findings.len(), forward).find(|&i| {
                    let issue = findings[i];
                    re.is_match(&issue.code)
                        || re.is_match(&issue.message)
                        || issue.node_id.as_ref().is_some_and(|id| re.is_match(id))
                })
            }
            TopView::Provenance => {
                let layout = self.provenance_layout.as_ref();
                let vis = self.visible_provenance_indices();
                search_order(cursor, vis.len(), forward).find(|&i| {
                    layout
                        .and_then(|layout| layout.rows.get(vis[i]))
                        .is_some_and(|row| re.is_match(&row.id))
                })
            }
            TopView::Tree => {
                let rows = self.visible_tree_rows();
                search_order(cursor, rows.len(), forward).find(|&i| {
                    let row = &rows[i];
                    re.is_match(row.node_id) || row.summary.as_ref().is_some_and(|s| re.is_match(s))
                })
            }
            TopView::Graph => {
                let rows = self.visible_graph_rows();
                search_order(cursor, rows.len(), forward).find(|&i| re.is_match(rows[i].id))
            }
        };

        if let Some(i) = found {
            match self.top_view {
                TopView::Tree => self.tree_state.cursor = i,
                TopView::Provenance => self.provenance_state.cursor = i,
                TopView::Graph => self.graph_state.cursor = i,
                _ => self.current_state_mut().cursor = i,
            }
        }
    }

    /// The node an owner action runs against: a pushed detail's node, else
    /// the cursor's node (which an unpushed detail pane shows), else a
    /// finding's node.
    fn selected_node(&self) -> Option<&'a Node> {
        let graph: &'a LatticeGraph = self.graph;
        if let Some(frame) = self.view_stack.last() {
            return graph.node(&frame.node_id);
        }
        match self.top_view {
            TopView::NodeList if self.show_schema => None,
            TopView::NodeList => {
                let nodes = self.visible_nodes();
                let cursor = self
                    .node_list_state
                    .cursor
                    .min(nodes.len().saturating_sub(1));
                nodes.get(cursor).and_then(|(id, _, _)| graph.node(id))
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                let cursor = self
                    .findings_state
                    .cursor
                    .min(findings.len().saturating_sub(1));
                findings
                    .get(cursor)
                    .and_then(|i| i.node_id.as_deref())
                    .and_then(|id| graph.node(id))
            }
            TopView::Tree => {
                let rows = self.visible_tree_rows();
                let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor)
                    .filter(|r| r.endpoint == Endpoint::Declared)
                    .and_then(|r| graph.node(r.node_id))
            }
            TopView::Provenance => self.provenance_cursor_row().and_then(|r| graph.node(&r.id)),
            TopView::Graph => {
                let rows = self.visible_graph_rows();
                let cursor = self.graph_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor).and_then(|r| graph.node(r.id))
            }
        }
    }

    /// `e`: the kind's `edit` action when it declares one, else `$EDITOR`.
    fn edit(&mut self) -> Action {
        if let Some(node) = self.selected_node()
            && self.declares(node, "edit")
        {
            return self.launch(node, "edit", false);
        }
        self.jump_to_source()
    }

    /// `A`: open the notes file at the selected finding's entry, or show the
    /// entry to add and open the file's end on a second press. Lattice never
    /// writes the file; the editor does.
    fn note(&mut self) -> Action {
        let Some(finding) = self.selected_finding() else {
            self.status_message = Some("select a finding to note".to_owned());
            return Action::None;
        };
        let code = finding.code.clone();
        let Some(node) = finding.node_id.clone() else {
            self.status_message =
                Some("this finding has no node; notes key on code and node".to_owned());
            return Action::None;
        };
        let Some(path) = self.profile.annotations() else {
            self.status_message =
                Some("the profile declares no notes file (annotations)".to_owned());
            return Action::None;
        };
        if !path.is_file() {
            self.status_message = Some(format!("notes file {} does not exist", path.display()));
            return Action::None;
        }
        if !self.editor_set {
            self.status_message = Some("set $EDITOR to open the notes file".to_owned());
            return Action::None;
        }
        let file = path.display().to_string();
        if let Some(entry) = self.profile.note_location(&code, &node) {
            return Action::EditNotes(EditorTarget::at(file, &entry));
        }
        let key = (code, node);
        if self.pending_note.as_ref() == Some(&key) {
            self.pending_note = None;
            let last = std::fs::read_to_string(path).map_or(1, |text| text.lines().count());
            let last = u32::try_from(last).unwrap_or(u32::MAX).max(1);
            return Action::EditNotes(EditorTarget {
                file,
                line: NonZeroU32::new(last),
            });
        }
        self.status_message = Some(format!(
            "add: - finding: {{code: {}, node: {}}}  note: …   (A again opens the notes file)",
            key.0, key.1
        ));
        self.pending_note = Some(key);
        Action::None
    }

    /// The finding under the cursor: the findings list's, or a finding row in
    /// a focused node detail.
    fn selected_finding(&self) -> Option<&Issue> {
        if self.detail_focused {
            return match self.detail_rows().get(self.detail_cursor) {
                Some(DetailRow::Finding { issue, .. }) => Some(*issue),
                _ => None,
            };
        }
        if self.top_view != TopView::Findings {
            return None;
        }
        let findings = self.visible_findings();
        let cursor = self
            .findings_state
            .cursor
            .min(findings.len().saturating_sub(1));
        findings.get(cursor).copied()
    }

    fn declares(&self, node: &Node, verb: &str) -> bool {
        self.profile
            .node_kinds()
            .get(&node.kind)
            .is_some_and(|k| k.actions.iter().any(|(v, _)| v == verb))
    }

    fn owner_verb(&mut self, verb: &str) -> Action {
        let Some(node) = self.selected_node() else {
            self.status_message = Some("no node selected".to_owned());
            return Action::None;
        };
        if !self.declares(node, verb) {
            self.status_message = Some(format!("no {verb} action for kind {}", node.kind));
            return Action::None;
        }
        self.launch(node, verb, false)
    }

    /// Resolve `verb` on `node` and find its program. A refusal goes to the
    /// status line before any prompt, so `y` is never asked for a launch that
    /// cannot happen. From a letter key, the first use of an action waits
    /// for `y`; the `!` menu has shown the argv already, so a run from it
    /// confirms the action.
    fn resolve_run(&self, node: &Node, verb: &str) -> Result<(ActionKey, OwnerRun), Refusal> {
        let kind = &self.profile.node_kinds()[&node.kind];
        let (_, decl) = kind
            .actions
            .iter()
            .find(|(v, _)| v == verb)
            .expect("callers check the kind declares the verb");
        let argv = owner_action::resolve(verb, decl, node)?;
        let program = owner_action::find_program(
            &argv[0],
            self.path_env.as_deref(),
            self.target_dir.as_deref().map(Path::new),
        )
        .ok_or_else(|| Refusal::ProgramNotFound {
            program: argv[0].clone(),
        })?;
        let key = (
            node.kind.clone(),
            verb.to_owned(),
            decl.run.iter().map(|t| t.source.clone()).collect(),
        );
        let run = OwnerRun {
            verb: verb.to_owned(),
            argv,
            program,
            mode: decl.mode,
            cwd: self.target_dir.clone(),
        };
        Ok((key, run))
    }

    fn launch(&mut self, node: &Node, verb: &str, from_menu: bool) -> Action {
        match self.resolve_run(node, verb) {
            Err(refusal) => {
                self.status_message = Some(refusal.to_string());
                Action::None
            }
            Ok((key, run)) if from_menu => {
                self.confirmed.insert(key);
                Action::RunOwner(run)
            }
            Ok((key, run)) if self.confirmed.contains(&key) => Action::RunOwner(run),
            Ok((key, run)) => {
                self.confirm = Some(Confirm { key, run });
                Action::None
            }
        }
    }

    fn open_launcher(&mut self) {
        let Some(node) = self.selected_node() else {
            self.status_message = Some("no node selected".to_owned());
            return;
        };
        let has_actions = self
            .profile
            .node_kinds()
            .get(&node.kind)
            .is_some_and(|k| !k.actions.is_empty());
        if !has_actions {
            self.status_message = Some(format!("no actions for kind {}", node.kind));
            return;
        }
        self.launcher = Some(Launcher {
            node_id: node.id.clone(),
            cursor: 0,
        });
    }

    /// The `!` menu's rows: each verb with its argv, or why it cannot run.
    pub fn launcher_rows(&self) -> Vec<(String, Result<String, String>)> {
        let Some(node) = self
            .launcher
            .as_ref()
            .and_then(|l| self.graph.node(&l.node_id))
        else {
            return Vec::new();
        };
        let Some(kind) = self.profile.node_kinds().get(&node.kind) else {
            return Vec::new();
        };
        kind.actions
            .iter()
            .map(|(verb, _)| {
                let row = self
                    .resolve_run(node, verb)
                    .map(|(_, run)| run.display())
                    .map_err(|r| r.to_string());
                (verb.clone(), row)
            })
            .collect()
    }

    fn run_launcher_row(&mut self) -> Action {
        let Some(launcher) = self.launcher.take() else {
            return Action::None;
        };
        let graph: &'a LatticeGraph = self.graph;
        let Some(node) = graph.node(&launcher.node_id) else {
            return Action::None;
        };
        let Some((verb, _)) = self.profile.node_kinds()[&node.kind]
            .actions
            .get(launcher.cursor)
            .cloned()
        else {
            return Action::None;
        };
        self.launch(node, &verb, true)
    }

    fn jump_to_source(&mut self) -> Action {
        self.jump_to_source_with(std::env::var("EDITOR").is_ok())
    }

    /// Resolve the jump given whether an editor is configured, so a test can
    /// pin both answers without depending on the ambient environment. A jump
    /// that cannot happen says why on the status line rather than doing nothing.
    fn jump_to_source_with(&mut self, has_editor: bool) -> Action {
        if !has_editor {
            self.status_message = Some("set $EDITOR to open files".to_owned());
            return Action::None;
        }

        if let Some(p) = self.current_provenance() {
            let mut file = p.file().to_owned();
            if let Some(ref dir) = self.target_dir
                && !Path::new(&file).is_absolute()
            {
                file = format!("{dir}/{file}");
            }
            if !Path::new(&file).is_file() {
                self.status_message = Some(format!(
                    "cannot edit {}: not a file in the working tree",
                    p.file()
                ));
                return Action::None;
            }
            return Action::OpenEditor(EditorTarget::at(file, p));
        }
        self.status_message = Some(
            self.unlocated_finding()
                .unwrap_or_else(|| "nothing selected to open".to_owned()),
        );
        Action::None
    }

    /// Say why the selected finding has no file to open, when it is one Lattice
    /// reports on an input as a whole rather than at a place in a file.
    fn unlocated_finding(&self) -> Option<String> {
        let finding = self.selected_finding()?;
        if finding.origin.location().is_some() {
            return None;
        }
        let input = match finding.origin {
            Origin::Profile => "the profile",
            Origin::FuseProfile => "the fuse profile",
            Origin::FuseSource => "a fuse source that could not run",
            Origin::Suggestions => "the suggestion document",
            _ => "an input Lattice reads itself",
        };
        Some(format!(
            "{} is about {input}, not a line in a file",
            finding.code
        ))
    }

    fn list_len(&self) -> usize {
        match self.top_view {
            TopView::NodeList => self.visible_nodes().len(),
            TopView::Findings => self.visible_findings().len(),
            TopView::Tree => self.visible_tree_rows().len(),
            TopView::Provenance => self.provenance_row_count(),
            TopView::Graph => self.graph_row_count(),
        }
    }

    pub fn current_state_mut(&mut self) -> &mut ViewState {
        match self.top_view {
            TopView::NodeList | TopView::Tree | TopView::Provenance | TopView::Graph => {
                &mut self.node_list_state
            }
            TopView::Findings => &mut self.findings_state,
        }
    }

    pub fn current_state(&self) -> &ViewState {
        match self.top_view {
            TopView::NodeList | TopView::Tree | TopView::Provenance | TopView::Graph => {
                &self.node_list_state
            }
            TopView::Findings => &self.findings_state,
        }
    }

    pub fn current_detail_node(&self) -> Option<&Node> {
        if let Some(frame) = self.view_stack.last() {
            return self.graph.node(&frame.node_id);
        }
        if !self.show_detail {
            return None;
        }
        let nodes = self.visible_nodes();
        let cursor = self
            .node_list_state
            .cursor
            .min(nodes.len().saturating_sub(1));
        nodes.get(cursor).and_then(|(id, _, _)| self.graph.node(id))
    }

    pub fn current_detail_id(&self) -> Option<&str> {
        if let Some(frame) = self.view_stack.last() {
            return Some(&frame.node_id);
        }
        if !self.show_detail {
            return None;
        }
        let nodes = self.visible_nodes();
        let cursor = self
            .node_list_state
            .cursor
            .min(nodes.len().saturating_sub(1));
        nodes.get(cursor).map(|(id, _, _)| *id)
    }

    pub fn visible_nodes(&self) -> Vec<(&str, &str, Option<String>)> {
        let mut nodes: Vec<_> = self
            .graph
            .iter_nodes()
            .filter(|node| {
                if let Some(&idx) = self.kind_slot.get(node.kind.as_str())
                    && idx < self.node_list_state.kind_filter.len()
                    && !self.node_list_state.kind_filter[idx]
                {
                    return false;
                }
                if !self.filters.is_empty()
                    && !latt_core::validate::eval_conditions(&self.filters, &node.attrs, true)
                {
                    return false;
                }
                true
            })
            .map(|node| {
                let summary = self
                    .profile
                    .node_kinds()
                    .get(&node.kind)
                    .and_then(|nk| nk.summary_attr.as_deref())
                    .and_then(|attr| node.attrs.get(attr))
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned));
                (node.id.as_str(), node.kind.as_str(), summary)
            })
            .collect();

        match self.node_list_state.sort {
            SortMode::KindOrder => {
                nodes.sort_by(|a, b| {
                    let ka = self.kind_slot.get(a.1).copied().unwrap_or(usize::MAX);
                    let kb = self.kind_slot.get(b.1).copied().unwrap_or(usize::MAX);
                    ka.cmp(&kb).then(a.0.cmp(b.0))
                });
            }
            SortMode::Severity => {
                nodes.sort_by(|a, b| {
                    let sa = self
                        .node_severity
                        .get(a.0)
                        .copied()
                        .unwrap_or((Severity::Hint, 0));
                    let sb = self
                        .node_severity
                        .get(b.0)
                        .copied()
                        .unwrap_or((Severity::Hint, 0));
                    let has_a = sa.1 > 0;
                    let has_b = sb.1 > 0;
                    has_b
                        .cmp(&has_a)
                        .then(sa.0.cmp(&sb.0))
                        .then(sb.1.cmp(&sa.1))
                        .then(a.0.cmp(b.0))
                });
            }
        }
        nodes
    }

    pub fn visible_findings(&self) -> Vec<&Issue> {
        let mut findings: Vec<&Issue> = self
            .issues
            .iter()
            .filter(|i| self.show_suppressed || !i.suppressed)
            .filter(|i| {
                let Some(ref node_id) = i.node_id else {
                    return true;
                };
                let Some(node) = self.graph.node(node_id) else {
                    return true;
                };
                let Some(&idx) = self.kind_slot.get(node.kind.as_str()) else {
                    return true;
                };
                idx >= self.findings_state.kind_filter.len() || self.findings_state.kind_filter[idx]
            })
            .collect();

        match self.findings_state.sort {
            SortMode::KindOrder => {
                findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
            }
            SortMode::Severity => {
                findings.sort_by(|a, b| {
                    a.severity
                        .cmp(&b.severity)
                        .then(a.sort_key().cmp(&b.sort_key()))
                });
            }
        }
        // The list labels the unattached findings once, before the first of them,
        // so they must follow every attached one.
        findings.sort_by_key(|i| !i.node_id.as_ref().is_some_and(|id| self.graph.has_node(id)));
        findings
    }

    #[cfg(test)]
    fn findings_attached(&self) -> (Vec<&Issue>, Vec<&Issue>) {
        let all = self.visible_findings();
        let mut attached = Vec::new();
        let mut unattached = Vec::new();
        for issue in all {
            if issue
                .node_id
                .as_ref()
                .is_some_and(|id| self.graph.has_node(id))
            {
                attached.push(issue);
            } else {
                unattached.push(issue);
            }
        }
        (attached, unattached)
    }

    pub fn detail_rows(&self) -> Vec<DetailRow<'_>> {
        let Some(id) = self.current_detail_id() else {
            return Vec::new();
        };

        let mut rows = Vec::new();

        let Some(node) = self.graph.node(id) else {
            rows.push(DetailRow::Header("VACANCY — node not declared"));
            rows.push(DetailRow::Attr("id", Cow::Borrowed(id)));
            return rows;
        };

        rows.push(DetailRow::Header("Attributes"));
        rows.push(DetailRow::Attr("kind", Cow::Borrowed(&node.kind)));
        for (k, v) in &node.attrs {
            // A row is one line, and the terminal drops a line break inside it, so
            // each break becomes the space that separated the words it held apart.
            let val = match v.as_str() {
                Some(s) if s.contains('\n') => Cow::Owned(s.lines().collect::<Vec<_>>().join(" ")),
                Some(s) => Cow::Borrowed(s),
                None => Cow::Owned(v.to_string()),
            };
            rows.push(DetailRow::Attr(k, val));
        }
        rows.push(DetailRow::Attr(
            "provenance",
            Cow::Owned(node.provenance.to_string()),
        ));

        let out_edges = self.index.edges_from(id);
        if !out_edges.is_empty() {
            rows.push(DetailRow::Header("Outgoing edges"));
            for edge in out_edges {
                rows.push(DetailRow::OutEdge {
                    kind: &edge.kind,
                    tgt: &edge.tgt,
                    provenance: &edge.provenance,
                });
            }
        }

        let in_edges = self.index.edges_to(id);
        if !in_edges.is_empty() {
            rows.push(DetailRow::Header("Incoming edges"));
            for edge in in_edges {
                rows.push(DetailRow::InEdge {
                    kind: &edge.kind,
                    src: &edge.src,
                    provenance: &edge.provenance,
                });
            }
        }

        let missing = self.expected_missing_edges(id, &node.kind);
        if !missing.is_empty() {
            rows.push(DetailRow::Header("Expected but missing"));
            for (triple, incoming) in missing {
                rows.push(DetailRow::MissingEdge { triple, incoming });
            }
        }

        for pw in self.graph.iter_pathways() {
            if let Some(pos_attr) = self.pathway_position_attr(&node.kind)
                && let Some(val) = node.attrs.get(pos_attr)
                && let Some(pos) = val.as_str()
                && pw.is_member(pos)
            {
                rows.push(DetailRow::Pathway {
                    name: &pw.name,
                    order: &pw.order,
                    current: &pw.current,
                    node_position: pos,
                });
            }
        }

        let node_findings: Vec<&Issue> = self
            .issues
            .iter()
            .filter(|i| i.node_id.as_deref() == Some(id) && (self.show_suppressed || !i.suppressed))
            .collect();
        if !node_findings.is_empty() {
            rows.push(DetailRow::Header("Findings"));
            for issue in node_findings {
                rows.push(DetailRow::Finding {
                    severity: issue.severity,
                    code: &issue.code,
                    message: &issue.message,
                    issue,
                });
                if let Some(note) = &issue.note {
                    rows.push(DetailRow::Note(note));
                }
            }
        }

        rows
    }

    /// Each allowed triple this node could take part in but does not, with
    /// `true` when the missing side is the incoming one. A kind the profile
    /// marks `orphan_ok` expects no edges, so it has none missing.
    fn expected_missing_edges(
        &self,
        node_id: &str,
        node_kind: &str,
    ) -> Vec<(&AllowedTriple, bool)> {
        let mut missing = Vec::new();
        if self
            .profile
            .node_kinds()
            .get(node_kind)
            .is_some_and(|kind| kind.orphan_ok)
        {
            return missing;
        }
        for triple in &self.schema.triples {
            if triple.src_kind == node_kind {
                let has = self.index.edges_from(node_id).iter().any(|e| {
                    e.kind == triple.edge_kind
                        && self
                            .graph
                            .node(&e.tgt)
                            .is_some_and(|n| n.kind == triple.tgt_kind)
                });
                if !has {
                    missing.push((triple, false));
                }
            }
            if triple.tgt_kind == node_kind {
                let has = self.index.edges_to(node_id).iter().any(|e| {
                    e.kind == triple.edge_kind
                        && self
                            .graph
                            .node(&e.src)
                            .is_some_and(|n| n.kind == triple.src_kind)
                });
                if !has {
                    missing.push((triple, true));
                }
            }
        }
        missing
    }

    fn pathway_position_attr(&self, node_kind: &str) -> Option<&str> {
        let nk = self.profile.node_kinds().get(node_kind)?;
        for binding in self.profile.pathway_bindings().values() {
            if nk.attrs.contains_key(&binding.position_attr) {
                return Some(&binding.position_attr);
            }
        }
        None
    }

    fn activate_tree_view(&mut self) {
        if self.top_view == TopView::Tree {
            let new_root = {
                let rows = self.visible_tree_rows();
                let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor).map(|row| row.node_id.to_string())
            };
            if let Some(new_root) = new_root
                && self.graph.has_node(&new_root)
            {
                self.tree_state.root = Some(new_root);
                self.rebuild_tree();
            }
            return;
        }

        let root_id = match self.top_view {
            TopView::NodeList => {
                if let Some(frame) = self.view_stack.last() {
                    Some(frame.node_id.clone())
                } else {
                    let nodes = self.visible_nodes();
                    let cursor = self
                        .node_list_state
                        .cursor
                        .min(nodes.len().saturating_sub(1));
                    nodes.get(cursor).map(|(id, _, _)| id.to_string())
                }
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                let cursor = self
                    .findings_state
                    .cursor
                    .min(findings.len().saturating_sub(1));
                findings
                    .get(cursor)
                    .and_then(|f| f.node_id.clone())
                    .filter(|id| self.graph.has_node(id))
            }
            TopView::Provenance => self
                .provenance_cursor_row()
                .map(|r| r.id.clone())
                .filter(|id| self.graph.has_node(id)),
            TopView::Graph => self
                .graph_cursor_node_id()
                .filter(|id| self.graph.has_node(id)),
            TopView::Tree => unreachable!(),
        };

        if let Some(root) = root_id {
            self.tree_state.root = Some(root);
            self.view_stack.clear();
            self.show_detail = false;
            self.detail_focused = false;
            self.top_view = TopView::Tree;
            self.rebuild_tree();
        } else {
            self.status_message = Some(no_anchor_message("Tree"));
        }
    }

    fn rebuild_tree(&mut self) {
        let Some(ref root) = self.tree_state.root else {
            self.tree_report = None;
            return;
        };
        if !self.graph.has_node(root) {
            self.tree_report = None;
            return;
        }
        let index = self.filtered_tree_index();
        match query::traverse(self.graph, &index, root, self.tree_state.direction) {
            Ok(report) => {
                self.tree_report = Some(report);
            }
            Err(_) => {
                self.tree_report = None;
            }
        }
        self.tree_state.expanded.clear();
        self.tree_state.cursor = 0;
        self.tree_rows.clear();
    }

    fn filtered_tree_index(&self) -> AdjacencyIndex<'a> {
        let edge_kinds: Vec<&str> = self
            .profile
            .edge_kinds()
            .keys()
            .zip(self.tree_state.edge_kind_filter.iter())
            .filter(|(_, enabled)| **enabled)
            .map(|(name, _)| name.as_str())
            .collect();
        AdjacencyIndex::build_filtered(self.graph, &edge_kinds)
    }

    fn activate_provenance_view(&mut self) {
        if self.top_view == TopView::Provenance {
            // Re-root: use detail node if in detail, else cursor node.
            let new_root = if let Some(frame) = self.view_stack.last() {
                Some(frame.node_id.clone())
            } else if let Some(ref layout) = self.provenance_layout {
                let vis = self.visible_provenance_indices();
                let cursor = self
                    .provenance_state
                    .cursor
                    .min(vis.len().saturating_sub(1));
                vis.get(cursor)
                    .and_then(|&ri| layout.rows.get(ri))
                    .map(|r| r.id.clone())
            } else {
                None
            };
            if let Some(root) = new_root
                && self.graph.has_node(&root)
            {
                self.provenance_state.root = Some(root);
                self.view_stack.clear();
                self.show_detail = false;
                self.detail_focused = false;
                self.rebuild_provenance();
            }
            return;
        }

        let root_id = match self.top_view {
            TopView::NodeList => {
                if let Some(frame) = self.view_stack.last() {
                    Some(frame.node_id.clone())
                } else {
                    let nodes = self.visible_nodes();
                    let cursor = self
                        .node_list_state
                        .cursor
                        .min(nodes.len().saturating_sub(1));
                    nodes.get(cursor).map(|(id, _, _)| id.to_string())
                }
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                let cursor = self
                    .findings_state
                    .cursor
                    .min(findings.len().saturating_sub(1));
                findings
                    .get(cursor)
                    .and_then(|f| f.node_id.clone())
                    .filter(|id| self.graph.has_node(id))
            }
            TopView::Tree => {
                let rows = self.visible_tree_rows();
                let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor)
                    .map(|r| r.node_id.to_string())
                    .filter(|id| self.graph.has_node(id))
            }
            TopView::Graph => self
                .graph_cursor_node_id()
                .filter(|id| self.graph.has_node(id)),
            TopView::Provenance => unreachable!(),
        };

        if let Some(root) = root_id {
            self.provenance_state.root = Some(root);
            self.view_stack.clear();
            self.show_detail = false;
            self.detail_focused = false;
            self.top_view = TopView::Provenance;
            self.rebuild_provenance();
        } else {
            self.status_message = Some(no_anchor_message("Provenance"));
        }
    }

    fn rebuild_provenance(&mut self) {
        self.provenance_rows.clear();
        let Some(ref root) = self.provenance_state.root else {
            self.provenance_layout = None;
            return;
        };
        if !self.graph.has_node(root) {
            self.provenance_layout = None;
            return;
        }
        let index = self.filtered_provenance_index();
        let layout = query::lane_layout(&index, root, self.provenance_state.direction);
        self.provenance_layout = Some(layout);
        self.provenance_state.cursor = 0;
        self.provenance_state.h_scroll = 0;
        self.git_commit_node = None;
        self.update_provenance_commits();
    }

    fn update_provenance_commits(&mut self) {
        let row = self.provenance_cursor_row();
        let node_id = row.map(|r| r.id.clone());
        if node_id == self.git_commit_node {
            return;
        }
        self.git_commit_node = node_id;
        self.git_commits.clear();
        self.git_lanes.clear();
        self.git_fetch_timed_out = false;
        if !self.git_fetch_pending() {
            self.spawn_git_fetch();
        }
    }

    /// Start `git log` for `git_commit_node`. At most one fetch runs at a
    /// time: a result for a node the cursor has left starts the fetch for the
    /// current one, so holding a key never fans out into one git process per row.
    fn spawn_git_fetch(&mut self) {
        let Some(ref dir) = self.target_dir else {
            return;
        };
        let Some(ref id) = self.git_commit_node else {
            return;
        };
        let Some(node) = self.graph.node(id) else {
            return;
        };
        let Some(line) = node.provenance.line() else {
            return;
        };
        let line = line.get() as usize;
        let (dir, file, id) = (dir.clone(), node.provenance.file().to_owned(), id.clone());
        let (program, bound) = (self.git_program.clone(), self.git_fetch_bound);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let fetch = git::parse_git_log(&program, &dir, &file, line, line, bound);
            let _ = tx.send((id, fetch));
        });
        self.git_fetch = Some(rx);
    }

    pub fn git_fetch_pending(&self) -> bool {
        self.git_fetch.is_some()
    }

    /// Install a finished fetch without blocking. True when the view changed.
    pub fn poll_git_commits(&mut self) -> bool {
        let Some(rx) = &self.git_fetch else {
            return false;
        };
        match rx.try_recv() {
            Ok((id, fetch)) => {
                self.git_fetch = None;
                if self.git_commit_node.as_deref() == Some(id.as_str()) {
                    match fetch {
                        git::LogFetch::Commits(commits) => {
                            self.git_lanes = git::commit_lanes(&commits);
                            self.git_commits = commits;
                        }
                        git::LogFetch::TimedOut => self.git_fetch_timed_out = true,
                    }
                } else {
                    self.spawn_git_fetch();
                }
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.git_fetch = None;
                false
            }
        }
    }

    #[cfg(test)]
    fn await_git_commits(&mut self) {
        while self.git_fetch_pending() {
            if !self.poll_git_commits() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn filtered_provenance_index(&self) -> AdjacencyIndex<'a> {
        let edge_kinds: Vec<&str> = self
            .profile
            .edge_kinds()
            .keys()
            .zip(self.provenance_state.edge_kind_filter.iter())
            .filter(|(_, enabled)| **enabled)
            .map(|(name, _)| name.as_str())
            .collect();
        AdjacencyIndex::build_filtered(self.graph, &edge_kinds)
    }

    pub fn provenance_cursor_row(&self) -> Option<&LaneRow> {
        let layout = self.provenance_layout.as_ref()?;
        let vis = self.visible_provenance_indices();
        let cursor = self
            .provenance_state
            .cursor
            .min(vis.len().saturating_sub(1));
        vis.get(cursor).and_then(|&ri| layout.rows.get(ri))
    }

    pub fn visible_provenance_indices(&self) -> Ref<'_, Vec<usize>> {
        let kind_filter = &self.provenance_state.kind_filter;
        self.provenance_rows.rows(
            |key| key == kind_filter,
            || {
                let indices = self
                    .provenance_layout
                    .as_ref()
                    .map_or_else(Vec::new, |layout| {
                        layout
                            .rows
                            .iter()
                            .enumerate()
                            .filter(|(_, row)| !self.is_kind_filtered_out(&row.id, kind_filter))
                            .map(|(i, _)| i)
                            .collect()
                    });
                (kind_filter.clone(), indices)
            },
        )
    }

    fn provenance_row_count(&self) -> usize {
        self.visible_provenance_indices().len()
    }

    fn provenance_status_text(&self) -> String {
        let root = self.provenance_state.root.as_deref().unwrap_or("(none)");
        let dir = match self.provenance_state.direction {
            Direction::Forward => "forward",
            Direction::Reverse => "reverse",
        };
        let (rows, peak) = self
            .provenance_layout
            .as_ref()
            .map_or((0, 0), |l| (l.rows.len(), l.peak_lanes));
        let kind_filter_label = self.kind_filter_label();
        format!(" Provenance: {root} {dir}{kind_filter_label} | {rows} nodes, {peak} peak lanes")
    }

    fn is_kind_filtered_out(&self, node_id: &str, kind_filter: &[bool]) -> bool {
        self.graph
            .node(node_id)
            .and_then(|node| self.kind_slot.get(node.kind.as_str()))
            .is_some_and(|&idx| idx < kind_filter.len() && !kind_filter[idx])
    }

    pub fn visible_tree_rows(&self) -> Ref<'_, Vec<TreeRow<'a>>> {
        let state = &self.tree_state;
        self.tree_rows.rows(
            |key| {
                key.scc_collapse == state.scc_collapse
                    && key.kind_filter == state.kind_filter
                    && key.expanded == state.expanded
            },
            || {
                let key = TreeRowsKey {
                    scc_collapse: state.scc_collapse,
                    kind_filter: state.kind_filter.clone(),
                    expanded: state.expanded.clone(),
                };
                (key, self.build_tree_rows())
            },
        )
    }

    fn build_tree_rows(&self) -> Vec<TreeRow<'a>> {
        let Some(ref report) = self.tree_report else {
            return Vec::new();
        };

        let cluster_map = if self.tree_state.scc_collapse {
            self.build_cluster_map(report)
        } else {
            HashMap::new()
        };

        let mut rows = Vec::new();
        let mut skip_cluster: Option<&[String]> = None;
        let mut skip_below_depth: Option<usize> = None;
        let mut i = 0;
        while i < report.steps.len() {
            let step = &report.steps[i];

            if let Some(d) = skip_below_depth {
                if step.depth > d {
                    i += 1;
                    continue;
                }
                skip_below_depth = None;
            }

            if let Some(members) = skip_cluster {
                if members.contains(&step.node_id.to_string())
                    && matches!(step.visit, Visit::Cycle | Visit::Reconvergence)
                {
                    i += 1;
                    continue;
                }
                skip_cluster = None;
            }

            if self.is_kind_filtered_out(step.node_id, &self.tree_state.kind_filter) {
                skip_below_depth = Some(step.depth);
                i += 1;
                continue;
            }

            if step.depth == 0 || self.is_tree_ancestor_expanded(report, i) {
                if let Some(members) = cluster_map.get(&i) {
                    let summary = format!("[{} nodes: {}]", members.len(), members.join(", "));
                    rows.push(TreeRow {
                        step_index: i,
                        depth: step.depth,
                        node_id: step.node_id,
                        edge_kind: step.edge.map(|e| e.kind.as_str()),
                        endpoint: step.endpoint,
                        visit: step.visit,
                        tainted: step.tainted,
                        severity: self.node_severity.get(step.node_id).copied(),
                        summary: Some(summary),
                        cluster: Some(members.clone()),
                    });
                    skip_cluster = Some(members);
                    i += 1;
                    continue;
                }

                let summary = if step.endpoint == Endpoint::Declared {
                    self.graph.node(step.node_id).and_then(|n| {
                        self.profile
                            .node_kinds()
                            .get(&n.kind)
                            .and_then(|nk| nk.summary_attr.as_deref())
                            .and_then(|attr| n.attrs.get(attr))
                            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                    })
                } else {
                    None
                };
                rows.push(TreeRow {
                    step_index: i,
                    depth: step.depth,
                    node_id: step.node_id,
                    edge_kind: step.edge.map(|e| e.kind.as_str()),
                    endpoint: step.endpoint,
                    visit: step.visit,
                    tainted: step.tainted,
                    severity: self.node_severity.get(step.node_id).copied(),
                    summary,
                    cluster: None,
                });
            }
            i += 1;
        }
        rows
    }

    fn build_cluster_map(&self, report: &TraversalReport<'_>) -> HashMap<usize, Vec<String>> {
        let declared: HashSet<&str> = report
            .steps
            .iter()
            .filter(|s| s.endpoint == Endpoint::Declared)
            .map(|s| s.node_id)
            .collect();
        let index = self.filtered_tree_index();
        let condensation = query::components(&index, &declared);

        let mut node_to_component: HashMap<&str, usize> = HashMap::new();
        for (ci, comp) in condensation.components.iter().enumerate() {
            if comp.cyclic && comp.members.len() > 1 {
                for member in &comp.members {
                    node_to_component.insert(member, ci);
                }
            }
        }

        let mut first_step: HashMap<usize, usize> = HashMap::new();
        for (si, step) in report.steps.iter().enumerate() {
            if let Some(&ci) = node_to_component.get(step.node_id) {
                first_step.entry(ci).or_insert(si);
            }
        }

        let mut cluster_map: HashMap<usize, Vec<String>> = HashMap::new();
        for (&ci, &si) in &first_step {
            cluster_map.insert(si, condensation.components[ci].members.clone());
        }
        cluster_map
    }

    fn is_tree_ancestor_expanded(&self, report: &TraversalReport<'_>, step_idx: usize) -> bool {
        let target_depth = report.steps[step_idx].depth;
        if target_depth == 0 {
            return true;
        }
        let mut depth_to_check = target_depth - 1;
        let mut j = step_idx;
        while j > 0 && depth_to_check < target_depth {
            j -= 1;
            let s = &report.steps[j];
            if s.depth == depth_to_check {
                if !self.tree_state.expanded.contains(&j) {
                    return false;
                }
                if depth_to_check == 0 {
                    break;
                }
                depth_to_check -= 1;
            }
        }
        true
    }

    /// The traversal step under the tree cursor.
    fn tree_cursor_step(&self) -> Option<usize> {
        let rows = self.visible_tree_rows();
        let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
        rows.get(cursor).map(|row| row.step_index)
    }

    fn tree_expand_toggle(&mut self) {
        if let Some(idx) = self.tree_cursor_step() {
            if self.tree_state.expanded.contains(&idx) {
                self.tree_state.expanded.remove(&idx);
            } else {
                self.tree_state.expanded.insert(idx);
            }
        }
    }

    fn tree_collapse(&mut self) {
        if let Some(idx) = self.tree_cursor_step() {
            self.tree_state.expanded.remove(&idx);
            if let Some(ref report) = self.tree_report {
                let depth = report.steps[idx].depth;
                let mut j = idx + 1;
                while j < report.steps.len() && report.steps[j].depth > depth {
                    self.tree_state.expanded.remove(&j);
                    j += 1;
                }
            }
        }
    }

    fn open_preview(&mut self) {
        let Some((display_file, line)) = self
            .current_provenance()
            .map(|p| (p.file().to_owned(), p.line()))
        else {
            if let Some(reason) = self.unlocated_finding() {
                self.status_message = Some(reason);
            }
            return;
        };
        let file: Cow<str> = match self.target_dir {
            Some(ref dir) if !Path::new(&display_file).is_absolute() => {
                Cow::Owned(format!("{dir}/{display_file}"))
            }
            _ => Cow::Borrowed(&display_file),
        };
        let Ok(content) = fs::read_to_string(&*file) else {
            self.status_message = Some(format!("cannot read {file}"));
            return;
        };
        let lines: Vec<String> = content.lines().map(str::to_owned).collect();
        let rendered = display_file
            .ends_with(".md")
            .then(|| render_markdown(&content));
        let center = line.map_or(0, |line| (line.get() as usize).saturating_sub(1));
        let scroll = center.saturating_sub(10);
        self.preview = Some(Preview {
            file: display_file,
            lines,
            rendered,
            center_line: center,
            scroll,
        });
    }

    fn current_provenance(&self) -> Option<&Provenance> {
        if self.detail_focused {
            let rows = self.detail_rows();
            // A row without a source of its own (a heading, an attribute)
            // stands for the node the detail shows.
            return match rows.get(self.detail_cursor) {
                Some(
                    DetailRow::OutEdge { provenance, .. } | DetailRow::InEdge { provenance, .. },
                ) => Some(*provenance),
                Some(DetailRow::Finding { issue, .. }) => issue.origin.location(),
                _ => self.current_detail_node().map(|n| &n.provenance),
            };
        }
        match self.top_view {
            TopView::NodeList => {
                let nodes = self.visible_nodes();
                let cursor = self
                    .current_state()
                    .cursor
                    .min(nodes.len().saturating_sub(1));
                nodes
                    .get(cursor)
                    .and_then(|(id, _, _)| self.graph.node(id).map(|n| &n.provenance))
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                let cursor = self
                    .findings_state
                    .cursor
                    .min(findings.len().saturating_sub(1));
                findings.get(cursor).and_then(|i| i.origin.location())
            }
            TopView::Tree => {
                let rows = self.visible_tree_rows();
                let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor)
                    .filter(|r| r.endpoint == Endpoint::Declared)
                    .and_then(|r| self.graph.node(r.node_id).map(|n| &n.provenance))
            }
            TopView::Provenance => self
                .provenance_cursor_row()
                .and_then(|r| self.graph.node(&r.id).map(|n| &n.provenance)),
            TopView::Graph => {
                let rows = self.visible_graph_rows();
                let cursor = self.graph_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor)
                    .and_then(|r| self.graph.node(r.id).map(|n| &n.provenance))
            }
        }
    }

    pub fn edge_kind_names(&self) -> Vec<&str> {
        self.profile
            .edge_kinds()
            .keys()
            .map(String::as_str)
            .collect()
    }

    fn schema_mode(&self) -> bool {
        self.top_view == TopView::NodeList && self.show_schema
    }

    #[cfg(test)]
    pub fn schema_row_count(&self) -> usize {
        self.schema_view_rows().len()
    }

    /// Step the schema cursor by `delta` rows, clamped, and never rest on a
    /// separator: a landing on one slides one more row in the direction of travel.
    fn move_schema_cursor(&mut self, delta: isize) {
        let rows = self.schema_view_rows();
        let max = rows.len().saturating_sub(1);
        let target = (self.schema_cursor as isize + delta).clamp(0, max as isize) as usize;
        let landing = match rows.get(target) {
            Some(SchemaRow::Separator) if delta > 0 => (target + 1).min(max),
            Some(SchemaRow::Separator) => target.saturating_sub(1),
            _ => target,
        };
        drop(rows);
        self.schema_cursor = landing;
    }

    /// The schema rows are a function of the loaded register alone, so they
    /// are built once per `App`; a reload builds a new `App`.
    pub fn schema_view_rows(&self) -> Ref<'_, Vec<SchemaRow>> {
        self.schema_rows
            .rows(|()| true, || ((), self.build_schema_rows()))
    }

    fn build_schema_rows(&self) -> Vec<SchemaRow> {
        let join = self.schema.join_counts(&self.index, self.graph);
        let mut rows = Vec::new();
        for (kind, count) in &join.kind_counts {
            rows.push(SchemaRow::Kind {
                name: kind.clone(),
                count: *count,
            });
        }
        if !join.triple_counts.is_empty() {
            rows.push(SchemaRow::Separator);
            for (triple, count) in &join.triple_counts {
                rows.push(SchemaRow::Triple {
                    triple: triple.clone(),
                    count: *count,
                });
            }
        }
        rows
    }

    /// Count the unsuppressed findings by severity: errors and warnings always,
    /// info and hints when there are any, so a register whose gaps are all hints
    /// does not read as clean.
    fn finding_summary(&self) -> String {
        let count = |severity: Severity| {
            self.issues
                .iter()
                .filter(|i| !i.suppressed && i.severity == severity)
                .count()
        };
        let plural = |n: usize, word: &str| {
            if n == 1 {
                format!("1 {word}")
            } else {
                format!("{n} {word}s")
            }
        };
        let mut parts = vec![
            plural(count(Severity::Error), "error"),
            plural(count(Severity::Warning), "warning"),
        ];
        let info = count(Severity::Info);
        if info > 0 {
            parts.push(format!("{info} info"));
        }
        let hints = count(Severity::Hint);
        if hints > 0 {
            parts.push(plural(hints, "hint"));
        }
        parts.join(", ")
    }

    pub fn status_text(&self) -> String {
        if self.input_mode == InputMode::Search {
            return format!(" /{}█", self.input_buffer);
        }
        if self.input_mode == InputMode::KindFilter {
            return " Kind filter: ↑↓ navigate, Space toggle, Enter/Esc close".to_string();
        }
        if let Some(ref msg) = self.status_message {
            return format!(" {msg}");
        }
        if self.top_view == TopView::Tree {
            return self.tree_status_text();
        }
        if self.top_view == TopView::Provenance {
            return self.provenance_status_text();
        }
        if self.top_view == TopView::Graph {
            return self.graph_status_text();
        }

        let view_name = match self.top_view {
            TopView::NodeList if self.show_schema => "Schema",
            TopView::NodeList => "Nodes",
            TopView::Findings => "Findings",
            TopView::Tree | TopView::Provenance | TopView::Graph => unreachable!(),
        };
        let node_count = self.graph.iter_nodes().count();
        let edge_count = self.graph.iter_edges().count();
        let findings = self.finding_summary();

        let sort_label = match self.current_state().sort {
            SortMode::KindOrder => "",
            SortMode::Severity => " [severity]",
        };
        let search_label = self
            .current_state()
            .search
            .as_ref()
            .map(|_| format!(" /{}/", self.current_state().search_input))
            .unwrap_or_default();

        let filter_label = self.kind_filter_label();

        format!(
            " {view_name}{sort_label}{filter_label}{search_label} | {node_count} nodes, {edge_count} edges | {findings}"
        )
    }

    fn tree_status_text(&self) -> String {
        if self.input_mode == InputMode::Search {
            return format!(" /{}█", self.input_buffer);
        }
        if self.input_mode == InputMode::EdgeKindFilter {
            return " Edge-kind filter: ↑↓ navigate, Space toggle, Enter/Esc close".to_string();
        }
        if let Some(ref msg) = self.status_message {
            return format!(" {msg}");
        }
        let root = self.tree_state.root.as_deref().unwrap_or("(none)");
        let dir = match self.tree_state.direction {
            Direction::Forward => "forward",
            Direction::Reverse => "reverse",
        };
        let reached = self.tree_report.as_ref().map_or(0, |r| r.steps.len());
        let search_label = self
            .tree_state
            .search
            .as_ref()
            .map(|_| format!(" /{}/", self.tree_state.search_input))
            .unwrap_or_default();

        let excluded: Vec<&str> = self
            .profile
            .edge_kinds()
            .keys()
            .zip(self.tree_state.edge_kind_filter.iter())
            .filter(|(_, enabled)| !**enabled)
            .map(|(name, _)| name.as_str())
            .collect();
        let edge_filter_label = if excluded.is_empty() {
            String::new()
        } else {
            format!(" [-{}]", excluded.join(",-"))
        };
        let kind_filter_label = self.kind_filter_label();

        format!(
            " Tree {root} {dir} {reached} reached{kind_filter_label}{edge_filter_label}{search_label}"
        )
    }

    fn activate_graph_view(&mut self) {
        let center_id = match self.top_view {
            TopView::Graph => {
                // Already in map: no-op (re-center via Enter)
                return;
            }
            TopView::NodeList => {
                if let Some(frame) = self.view_stack.last() {
                    Some(frame.node_id.clone())
                } else {
                    let nodes = self.visible_nodes();
                    let cursor = self
                        .node_list_state
                        .cursor
                        .min(nodes.len().saturating_sub(1));
                    nodes.get(cursor).map(|(id, _, _)| id.to_string())
                }
            }
            TopView::Findings => {
                let findings = self.visible_findings();
                let cursor = self
                    .findings_state
                    .cursor
                    .min(findings.len().saturating_sub(1));
                findings
                    .get(cursor)
                    .and_then(|f| f.node_id.clone())
                    .filter(|id| self.graph.has_node(id))
            }
            TopView::Tree => {
                let rows = self.visible_tree_rows();
                let cursor = self.tree_state.cursor.min(rows.len().saturating_sub(1));
                rows.get(cursor)
                    .map(|r| r.node_id.to_string())
                    .filter(|id| self.graph.has_node(id))
            }
            TopView::Provenance => self
                .provenance_cursor_row()
                .map(|r| r.id.clone())
                .filter(|id| self.graph.has_node(id)),
        };

        if let Some(center) = center_id {
            self.graph_state.center = Some(center);
            self.graph_state.center_history.clear();
            self.graph_state.cursor = 0;
            self.view_stack.clear();
            self.show_detail = false;
            self.detail_focused = false;
            self.top_view = TopView::Graph;
            self.rebuild_graph_and_centrality();
            self.select_graph_center();
        } else {
            self.status_message = Some(no_anchor_message("Graph"));
        }
    }

    fn rebuild_graph(&mut self) {
        let index = self.filtered_graph_index();
        self.rebuild_graph_with(&index);
    }

    /// Rebuild the layout and the centrality from one filtered index: both
    /// follow the same edge-kind filter, so a Graph activation builds it once.
    fn rebuild_graph_and_centrality(&mut self) {
        let index = self.filtered_graph_index();
        self.rebuild_graph_with(&index);
        self.recompute_centrality_with(&index);
    }

    fn rebuild_graph_with(&mut self, index: &AdjacencyIndex<'a>) {
        self.graph_rows_cache.clear();
        let Some(ref center) = self.graph_state.center else {
            self.graph_layout = None;
            return;
        };
        let layout = query::graph_layout(
            index,
            center,
            self.graph_state.depth_radius(),
            self.graph_state.direction,
        );
        self.graph_layout = Some(layout);
    }

    fn filtered_graph_index(&self) -> AdjacencyIndex<'a> {
        let edge_kinds: Vec<&str> = self
            .profile
            .edge_kinds()
            .keys()
            .zip(self.graph_state.edge_kind_filter.iter())
            .filter(|(_, enabled)| **enabled)
            .map(|(name, _)| name.as_str())
            .collect();
        AdjacencyIndex::build_filtered(self.graph, &edge_kinds)
    }

    fn recompute_centrality_with(&mut self, index: &AdjacencyIndex<'a>) {
        self.centrality_cache = query::betweenness_centrality(index);
    }

    pub fn visible_graph_rows(&self) -> Vec<MapRowRef<'_>> {
        let Some(layout) = self.graph_layout.as_ref() else {
            return Vec::new();
        };
        let rows = self.graph_rows();
        rows.seeds
            .iter()
            .map(|seed| {
                let comp = &layout.components[seed.component];
                let id = comp.members[0].as_str();
                MapRowRef {
                    id,
                    kind: self.graph.node(id).map_or("", |n| n.kind.as_str()),
                    is_incoming: seed.layer < rows.center_layer,
                    endpoint: if self.graph.has_node(id) {
                        Endpoint::Declared
                    } else {
                        Endpoint::Vacancy
                    },
                    cluster: comp.members.len() > 1,
                    is_center: seed.component == layout.center_component,
                    overflow: None,
                    edge_kind: rows
                        .first_in_edge
                        .get(&seed.component)
                        .map(|&ei| layout.edges[ei].kind.as_str()),
                    layer: seed.layer,
                    col: layout.x_positions[seed.layer][seed.idx],
                }
            })
            .collect()
    }

    fn graph_rows(&self) -> Ref<'_, GraphRows> {
        let state = &self.graph_state;
        let search = state.search.as_ref().map(regex::Regex::as_str);
        self.graph_rows_cache.rows(
            |key| {
                key.kind_filter == state.kind_filter
                    && key.search_filter == state.search_filter
                    && key.search.as_deref() == search
            },
            || {
                let key = GraphRowsKey {
                    kind_filter: state.kind_filter.clone(),
                    search_filter: state.search_filter,
                    search: search.map(str::to_owned),
                };
                (key, self.build_graph_rows())
            },
        )
    }

    fn build_graph_rows(&self) -> GraphRows {
        let Some(layout) = self.graph_layout.as_ref() else {
            return GraphRows {
                seeds: Vec::new(),
                center_layer: 0,
                first_in_edge: HashMap::new(),
            };
        };
        let search_filter = self.graph_state.search_filter;
        let search = self.graph_state.search.as_ref();
        let seeds = layout
            .layers
            .iter()
            .enumerate()
            .flat_map(|(li, layer)| {
                layer
                    .iter()
                    .enumerate()
                    .map(move |(idx, node)| (li, idx, node))
            })
            .filter_map(|(layer, idx, node)| {
                let LayoutNode::Real(component) = *node else {
                    return None;
                };
                let id = layout.components[component].members[0].as_str();
                if self.is_kind_filtered_out(id, &self.graph_state.kind_filter) {
                    return None;
                }
                if search_filter && search.is_some_and(|re| !re.is_match(id)) {
                    return None;
                }
                Some(GraphRowSeed {
                    layer,
                    idx,
                    component,
                })
            })
            .collect();
        GraphRows {
            seeds,
            center_layer: center_layer(layout),
            first_in_edge: first_incoming_edges(layout),
        }
    }

    fn graph_row_count(&self) -> usize {
        self.graph_rows().seeds.len()
    }

    fn graph_cursor_node_id(&self) -> Option<String> {
        let rows = self.visible_graph_rows();
        let cursor = self.graph_state.cursor.min(rows.len().saturating_sub(1));
        rows.get(cursor)
            .filter(|r| r.overflow.is_none())
            .map(|r| r.id.to_string())
    }

    fn graph_status_text(&self) -> String {
        if self.input_mode == InputMode::Search {
            return format!(" /{}█", self.input_buffer);
        }
        if self.input_mode == InputMode::EdgeKindFilter {
            return " Edge-kind filter: ↑↓ navigate, Space toggle, Enter/Esc close".to_string();
        }
        if let Some(ref msg) = self.status_message {
            return format!(" {msg}");
        }
        let center = self.graph_state.center.as_deref().unwrap_or("(none)");
        let radius = self.graph_state.depth_radius();
        let rows = self.visible_graph_rows();
        let incoming = rows.iter().filter(|r| r.is_incoming).count();
        let outgoing = rows
            .iter()
            .filter(|r| !r.is_incoming && !r.is_center)
            .count();
        let col_label = match self.graph_state.column_mode {
            ColumnMode::Default => "",
            ColumnMode::Full => " [full]",
            ColumnMode::Compact => " [compact]",
        };
        let filter_label = if self.graph_state.search_filter {
            " FILTER"
        } else {
            ""
        };
        let dir = match self.graph_state.direction {
            Direction::Forward => "fwd",
            Direction::Reverse => "rev",
        };
        format!(
            " Graph: {center} r={radius} {dir}{col_label}{filter_label} | {incoming} incoming, {outgoing} outgoing | Enter: re-center | Tab: detail"
        )
    }

    pub fn view_bindings(&self) -> Vec<(&'static str, Vec<Binding>)> {
        let rows = |section| {
            keymap::help_rows(section)
                .into_iter()
                .map(|(key, desc)| Binding { key, desc })
                .collect()
        };
        let title = match self.top_view {
            TopView::NodeList => "Node list",
            TopView::Findings => "Findings",
            TopView::Tree => "Tree",
            TopView::Provenance => "Provenance",
            TopView::Graph => "Graph",
        };
        vec![
            ("Global", rows(Section::Global)),
            (title, rows(Section::View(self.top_view))),
        ]
    }

    pub fn view_hints(&self) -> String {
        keymap::footer(self.top_view)
            .iter()
            .map(|&(cmd, word)| format!("{} {word}", keymap::label_for(cmd, self.top_view)))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

const PAGE: usize = 20;
/// Further than any list is long: `Top` and `Bottom` are pages of this size.
const TO_END: usize = usize::MAX / 4;

fn step(cursor: usize, rows: usize, down: bool, max: usize) -> usize {
    if down {
        cursor.saturating_add(rows).min(max)
    } else {
        cursor.saturating_sub(rows)
    }
}

/// Row indices in the order a search visits them: the rows after `cursor`
/// then the rows up to and including it, or the reverse of that backwards;
/// empty when there are no rows.
fn search_order(cursor: usize, count: usize, forward: bool) -> Box<dyn Iterator<Item = usize>> {
    if count == 0 {
        Box::new(std::iter::empty())
    } else if forward {
        Box::new((cursor + 1..count).chain(0..=cursor))
    } else {
        Box::new((0..cursor).rev().chain((cursor..count).rev()))
    }
}

fn center_layer(layout: &GraphLayout) -> usize {
    layout
        .layers
        .iter()
        .position(|layer| {
            layer
                .iter()
                .any(|n| matches!(n, LayoutNode::Real(ci) if *ci == layout.center_component))
        })
        .unwrap_or(0)
}

/// Component index → index of its first incoming layout edge.
fn first_incoming_edges(layout: &GraphLayout) -> HashMap<usize, usize> {
    let mut edges = HashMap::new();
    for (ei, edge) in layout.edges.iter().enumerate() {
        edges.entry(edge.tgt_component).or_insert(ei);
    }
    edges
}

pub struct MapRowRef<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub is_incoming: bool,
    pub endpoint: Endpoint,
    pub cluster: bool,
    pub is_center: bool,
    pub overflow: Option<usize>,
    pub edge_kind: Option<&'a str>,
    pub layer: usize,
    pub col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnMode {
    Default,
    Full,
    Compact,
}

impl ColumnMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Default => Self::Full,
            Self::Full => Self::Compact,
            Self::Compact => Self::Default,
        }
    }
}

#[derive(Clone)]
pub enum DetailRow<'a> {
    Header(&'a str),
    Attr(&'a str, Cow<'a, str>),
    OutEdge {
        kind: &'a str,
        tgt: &'a str,
        provenance: &'a Provenance,
    },
    InEdge {
        kind: &'a str,
        src: &'a str,
        provenance: &'a Provenance,
    },
    MissingEdge {
        triple: &'a AllowedTriple,
        /// True when the node lacks the incoming side of the triple.
        incoming: bool,
    },
    Pathway {
        name: &'a str,
        order: &'a [String],
        current: &'a str,
        node_position: &'a str,
    },
    Finding {
        severity: Severity,
        code: &'a str,
        message: &'a str,
        issue: &'a Issue,
    },
    /// The note on the finding row above it.
    Note(&'a Note),
}

pub enum SchemaRow {
    Kind { name: String, count: usize },
    Triple { triple: AllowedTriple, count: usize },
    Separator,
}

/// The status line for a rooted view's key pressed where no declared node can
/// root it, so the key never looks dead.
fn no_anchor_message(view: &str) -> String {
    format!("{view} needs a selected node: pick one in Nodes (1) or a finding about a node")
}

#[cfg(test)]
mod tests {
    use super::*;
    use latt_core::document::ingest_document;
    use latt_core::profile::load_profile;
    use latt_core::validate::validate;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn profile_from(yaml: &str) -> Profile {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join("lattice_tui_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "p{}-{}.yaml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, yaml).unwrap();
        load_profile(&path).unwrap()
    }

    fn test_profile() -> Profile {
        profile_from(
            r#"
name: test
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: title
    attrs:
      title: { type: string }
  spec:
    id_pattern: "^SPEC-\\d+$"
    summary_attr: title
    attrs:
      title: { type: string }
  test:
    id_pattern: "^TEST-\\d+$"
    attrs:
      title: { type: string }
edge_kinds:
  satisfies:
    allowed: [[spec, req]]
  verifies:
    allowed: [[test, spec]]
"#,
        )
    }

    fn test_document() -> serde_json::Value {
        json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "REQ-001", "kind": "req", "attrs": {"title": "First requirement"},
                 "provenance": {"file": "reqs.md", "line": 1}},
                {"id": "REQ-002", "kind": "req", "attrs": {"title": "Second requirement"},
                 "provenance": {"file": "reqs.md", "line": 5}},
                {"id": "SPEC-001", "kind": "spec", "attrs": {"title": "Design doc"},
                 "provenance": {"file": "specs.md", "line": 1}},
                {"id": "TEST-001", "kind": "test", "attrs": {"title": "Unit test"},
                 "provenance": {"file": "tests.md", "line": 1}}
            ],
            "edges": [
                {"src": "SPEC-001", "tgt": "REQ-001", "kind": "satisfies",
                 "provenance": {"file": "specs.md", "line": 2}},
                {"src": "TEST-001", "tgt": "SPEC-001", "kind": "verifies",
                 "provenance": {"file": "tests.md", "line": 2}}
            ]
        })
    }

    fn make_app<'a>(profile: &'a Profile, graph: &'a LatticeGraph) -> App<'a> {
        let issues = validate(graph, profile, false);
        App::new(profile, graph, issues, Vec::new())
    }

    /// Carry `app` across a reload onto `graph`, as `run_browse` does.
    fn reloaded<'a>(app: App<'_>, profile: &'a Profile, graph: &'a LatticeGraph) -> App<'a> {
        let carried = app.into_carried();
        let mut next = make_app(profile, graph);
        next.restore(carried);
        next
    }

    fn document_with(extra: serde_json::Value) -> serde_json::Value {
        let mut doc = test_document();
        doc["nodes"].as_array_mut().unwrap().push(extra);
        doc
    }

    fn document_without_spec() -> serde_json::Value {
        let mut doc = test_document();
        doc["nodes"]
            .as_array_mut()
            .unwrap()
            .retain(|n| n["id"] != "SPEC-001");
        doc["edges"]
            .as_array_mut()
            .unwrap()
            .retain(|e| e["src"] != "SPEC-001" && e["tgt"] != "SPEC-001");
        doc
    }

    fn cursor_node<'b>(app: &'b App<'_>) -> &'b str {
        app.visible_nodes()[app.node_list_state.cursor].0
    }

    #[test]
    fn reload_keeps_the_cursor_on_its_node() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(cursor_node(&app), "SPEC-001");
        app.handle_key(KeyCode::Enter);

        let grown = ingest_document(document_with(json!(
            {"id": "REQ-003", "kind": "req", "attrs": {"title": "Third"},
             "provenance": {"file": "reqs.md", "line": 9}}
        )))
        .unwrap();
        let app = reloaded(app, &profile, &grown);
        assert_eq!(cursor_node(&app), "SPEC-001", "the node moved down a row");
        assert!(app.show_detail);
    }

    #[test]
    fn reload_without_the_cursor_node_clamps_and_closes_detail() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Enter);

        let shrunk = ingest_document(document_without_spec()).unwrap();
        let app = reloaded(app, &profile, &shrunk);
        assert_eq!(app.node_list_state.cursor, 2);
        assert_eq!(cursor_node(&app), "TEST-001");
        assert!(!app.show_detail);
    }

    #[test]
    fn reload_keeps_filters_sort_and_toggles() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        for key in ['f', 'j', 'j', ' '] {
            app.handle_key(KeyCode::Char(key));
        }
        app.handle_key(KeyCode::Enter);
        app.handle_key(KeyCode::Char('o'));
        app.handle_key(KeyCode::Char('.'));

        let app = reloaded(app, &profile, &graph);
        assert_eq!(app.node_list_state.kind_filter, [true, true, false]);
        assert!(app.node_list_state.sort == SortMode::Severity);
        assert!(app.show_suppressed);
    }

    #[test]
    fn reload_cuts_the_view_stack_at_its_first_missing_node() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Enter);
        app.view_stack = ["REQ-002", "SPEC-001", "TEST-001"]
            .into_iter()
            .map(|id| StackFrame {
                node_id: id.into(),
                detail_cursor: 0,
                source_view: TopView::NodeList,
                source_cursor: 0,
            })
            .collect();

        let shrunk = ingest_document(document_without_spec()).unwrap();
        let app = reloaded(app, &profile, &shrunk);
        let ids: Vec<&str> = app.view_stack.iter().map(|f| f.node_id.as_str()).collect();
        assert_eq!(ids, ["REQ-002"]);
        assert!(app.show_detail);
    }

    #[test]
    fn reload_leaves_a_view_whose_anchor_is_gone() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 2, '2');
        assert_eq!(app.tree_state.root.as_deref(), Some("SPEC-001"));

        let shrunk = ingest_document(document_without_spec()).unwrap();
        let app = reloaded(app, &profile, &shrunk);
        assert_eq!(app.top_view, TopView::NodeList);
        assert_eq!(app.tree_state.root, None);
        assert!(app.view_stack.is_empty());
    }

    #[test]
    fn reload_drops_vanished_graph_history() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 2, '5');
        app.graph_state.center_history = vec!["SPEC-001".into(), "REQ-002".into()];
        app.graph_state.center = Some("REQ-001".into());

        let shrunk = ingest_document(document_without_spec()).unwrap();
        let app = reloaded(app, &profile, &shrunk);
        assert_eq!(app.top_view, TopView::Graph);
        assert_eq!(app.graph_state.center.as_deref(), Some("REQ-001"));
        assert_eq!(app.graph_state.center_history, ["REQ-002"]);
    }

    // --- Group 4 retroactive: App + ViewState ---

    #[test]
    fn app_initial_state() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        assert_eq!(app.top_view, TopView::NodeList);
        assert!(!app.show_detail);
        assert!(!app.show_schema);
        assert!(app.view_stack.is_empty());
    }

    #[test]
    fn view_state_preserved_across_switches() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.node_list_state.cursor, 2);
        app.handle_key(KeyCode::Char('3'));
        assert_eq!(app.top_view, TopView::Findings);
        app.handle_key(KeyCode::Char('1'));
        assert_eq!(app.node_list_state.cursor, 2);
    }

    // --- Group 5: Node list ---

    #[test]
    fn node_list_grouped_by_kind_order() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        let nodes = app.visible_nodes();
        assert!(!nodes.is_empty());
        let kinds: Vec<&str> = nodes.iter().map(|(_, k, _)| *k).collect();
        // req comes before spec comes before test in the profile
        let req_end = kinds.iter().rposition(|k| *k == "req").unwrap();
        let spec_start = kinds.iter().position(|k| *k == "spec").unwrap();
        assert!(req_end < spec_start);
    }

    #[test]
    fn node_list_shows_summary_attr() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        let nodes = app.visible_nodes();
        let req = nodes.iter().find(|(id, _, _)| *id == "REQ-001").unwrap();
        assert_eq!(req.2.as_deref(), Some("First requirement"));
    }

    #[test]
    fn node_list_nonstring_as_json() {
        let profile = profile_from(
            r#"
name: test
profile_version: "1.0.0"
node_kinds:
  item:
    id_pattern: "^I-\\d+$"
    summary_attr: count
    attrs:
      count: { type: int }
edge_kinds: {}
"#,
        );
        let graph = ingest_document(json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "I-1", "kind": "item", "attrs": {"count": 42},
                 "provenance": {"file": "f.md", "line": 1}}
            ],
            "edges": []
        }))
        .unwrap();
        let app = make_app(&profile, &graph);
        let nodes = app.visible_nodes();
        assert_eq!(nodes[0].2.as_deref(), Some("42"));
    }

    #[test]
    fn node_filter_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        let before = app.visible_nodes().len();
        // "req" is index 0 in the kind list
        app.node_list_state.kind_filter[0] = false;
        let after = app.visible_nodes().len();
        assert!(after < before);
        assert!(app.visible_nodes().iter().all(|(_, k, _)| *k != "req"));
    }

    #[test]
    fn node_sort_severity() {
        let profile = test_profile();
        // A graph where REQ-002 has no incoming satisfies edge → VACANCY or similar
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.node_list_state.sort = SortMode::Severity;
        let nodes = app.visible_nodes();
        // Nodes with findings should come before nodes without
        let has_findings = app.node_severity.values().any(|(_, count)| *count > 0);
        if has_findings
            && let (Some(first_with), Some(first_without)) = (
                nodes
                    .iter()
                    .position(|(id, _, _)| app.node_severity.get(*id).is_some_and(|(_, c)| *c > 0)),
                nodes
                    .iter()
                    .position(|(id, _, _)| app.node_severity.get(*id).is_none_or(|(_, c)| *c == 0)),
            )
        {
            assert!(first_with < first_without);
        }
    }

    #[test]
    fn schema_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        assert!(!app.show_schema);
        app.handle_key(KeyCode::Char('S'));
        assert!(app.show_schema);
        assert!(!app.schema_view_rows().is_empty());
        app.handle_key(KeyCode::Char('S'));
        assert!(!app.show_schema);
    }

    #[test]
    fn schema_cursor_moves_and_skips_separator() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.node_list_state.cursor, 1);
        app.handle_key(KeyCode::Char('S'));
        // 3 kinds, separator, 2 triples
        assert_eq!(app.schema_row_count(), 6);
        assert_eq!(app.schema_cursor, 0);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.schema_cursor, 2);
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.schema_cursor, 4, "skips the separator going down");
        app.handle_key(KeyCode::Char('k'));
        assert_eq!(app.schema_cursor, 2, "skips the separator going up");
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.schema_cursor, 5);
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.schema_cursor, 5, "clamps at the last row");
        app.handle_key(KeyCode::PageUp);
        assert_eq!(app.schema_cursor, 0);
        app.handle_key(KeyCode::Char('S'));
        assert_eq!(
            app.node_list_state.cursor, 1,
            "node-list cursor survives schema mode"
        );
    }

    #[test]
    fn markdown_preview_scroll_bounds_follow_rendered_lines() {
        let dir = std::env::temp_dir().join("lattice_tui_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let md = dir.join(format!("preview-{}.md", std::process::id()));
        let content = "# Title\n\nfirst paragraph line\nsecond paragraph line\n\n- one\n- two\n";
        std::fs::write(&md, content).unwrap();
        let raw = content.lines().count();
        let rendered = tui_markdown::from_str(content).lines.len();
        assert_ne!(
            raw, rendered,
            "fixture must render to a different line count"
        );

        let profile = test_profile();
        let graph = ingest_document(json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "REQ-001", "kind": "req", "attrs": {"title": "First"},
                 "provenance": {"file": md.to_str().unwrap(), "line": 3}}
            ],
            "edges": []
        }))
        .unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('p'));
        let pv = app.preview.as_ref().expect("preview opens");
        assert_eq!(pv.line_count(), rendered);
        app.handle_key(KeyCode::Char('G'));
        assert_eq!(app.preview.as_ref().unwrap().scroll, rendered - 1);
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.preview.as_ref().unwrap().scroll, rendered - 1);
    }

    #[test]
    fn schema_cursor_resets_on_enter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('S'));
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.schema_cursor, 1);
        app.handle_key(KeyCode::Char('S'));
        app.handle_key(KeyCode::Char('S'));
        assert_eq!(app.schema_cursor, 0);
    }

    // --- Group 6: Node detail ---

    #[test]
    fn node_detail_shows_attrs_and_edges() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Navigate to SPEC-001 (index 2: after REQ-001, REQ-002)
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Enter);
        assert!(app.show_detail);
        let rows = app.detail_rows();
        let has_attrs = rows
            .iter()
            .any(|r| matches!(r, DetailRow::Header(h) if *h == "Attributes"));
        assert!(has_attrs);
        let has_out = rows
            .iter()
            .any(|r| matches!(r, DetailRow::Header(h) if *h == "Outgoing edges"));
        assert!(has_out || rows.iter().any(|r| matches!(r, DetailRow::OutEdge { .. })));
    }

    #[test]
    fn expected_edges_shown() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // REQ-002 has no incoming 'satisfies' edge → expected but missing
        app.handle_key(KeyCode::Char('j')); // to REQ-002
        app.handle_key(KeyCode::Enter);
        let rows = app.detail_rows();
        let has_missing = rows
            .iter()
            .any(|r| matches!(r, DetailRow::MissingEdge { .. }));
        assert!(has_missing);
    }

    #[test]
    fn edge_follow_pushes_stack() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Go to SPEC-001 detail
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Enter);
        // Tab to detail, navigate to an edge row
        app.handle_key(KeyCode::Tab);
        assert!(app.detail_focused);
        // Find an OutEdge row
        let rows = app.detail_rows();
        let edge_idx = rows
            .iter()
            .position(|r| matches!(r, DetailRow::OutEdge { .. }));
        if let Some(idx) = edge_idx {
            app.detail_cursor = idx;
            app.handle_key(KeyCode::Enter);
            assert!(!app.view_stack.is_empty());
        }
    }

    #[test]
    fn edge_follow_vacancy() {
        let profile = profile_from(
            r#"
name: test
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds:
  refs:
    allowed: [[req, req]]
"#,
        );
        let graph = ingest_document(json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "REQ-001", "kind": "req", "attrs": {},
                 "provenance": {"file": "f.md", "line": 1}}
            ],
            "edges": [
                {"src": "REQ-001", "tgt": "REQ-999", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 2}}
            ]
        }))
        .unwrap();
        let mut app = make_app(&profile, &graph);
        // Follow edge to REQ-999 which doesn't exist
        app.handle_key(KeyCode::Enter); // show detail
        app.handle_key(KeyCode::Tab); // focus detail
        let rows = app.detail_rows();
        let edge_idx = rows
            .iter()
            .position(|r| matches!(r, DetailRow::OutEdge { .. }));
        if let Some(idx) = edge_idx {
            app.detail_cursor = idx;
            app.handle_key(KeyCode::Enter); // follow
            assert_eq!(app.view_stack.len(), 1);
            // Detail should show VACANCY
            let detail = app.detail_rows();
            let has_vacancy = detail
                .iter()
                .any(|r| matches!(r, DetailRow::Header(h) if h.contains("VACANCY")));
            assert!(has_vacancy);
        }
    }

    /// Two issues, `I-1 —[depends_on]→ I-2`, under a same-kind triple.
    fn same_kind_profile(orphan_ok: bool) -> Profile {
        profile_from(&format!(
            r#"
name: test
profile_version: "1.0.0"
node_kinds:
  issue:
    id_pattern: "^I-\\d+$"
    orphan_ok: {orphan_ok}
edge_kinds:
  depends_on:
    allowed: [[issue, issue]]
"#
        ))
    }

    fn same_kind_graph() -> LatticeGraph {
        ingest_document(json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "I-1", "kind": "issue", "attrs": {}, "provenance": {"file": "f", "line": 1}},
                {"id": "I-2", "kind": "issue", "attrs": {}, "provenance": {"file": "f", "line": 2}}
            ],
            "edges": [
                {"src": "I-1", "tgt": "I-2", "kind": "depends_on",
                 "provenance": {"file": "f", "line": 1}}
            ]
        }))
        .unwrap()
    }

    fn missing_sides(app: &App<'_>) -> Vec<bool> {
        app.detail_rows()
            .iter()
            .filter_map(|r| match r {
                DetailRow::MissingEdge { incoming, .. } => Some(*incoming),
                _ => None,
            })
            .collect()
    }

    // Requirement: browse/Node detail view
    // Scenario: Same-kind triple with only one side present
    #[test]
    fn missing_edge_names_the_missing_side_of_a_same_kind_triple() {
        let profile = same_kind_profile(false);
        let graph = same_kind_graph();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Enter); // I-1: has the outgoing edge only
        assert_eq!(missing_sides(&app), [true]);
        app.handle_key(KeyCode::Char('1'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Enter); // I-2: has the incoming edge only
        assert_eq!(missing_sides(&app), [false]);
    }

    // Requirement: browse/Node detail view
    // Scenario: Orphan-ok kind
    #[test]
    fn orphan_ok_kind_shows_no_missing_edges() {
        let profile = same_kind_profile(true);
        let graph = same_kind_graph();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Enter);
        assert!(missing_sides(&app).is_empty());
    }

    // Requirement: browse/View key with no node to anchor
    // Scenario: Tree key on an empty Findings view
    #[test]
    fn rooted_view_keys_on_an_empty_findings_view_say_why() {
        let profile = same_kind_profile(true);
        let graph = same_kind_graph();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3'));
        assert_eq!(app.top_view, TopView::Findings);
        for (key, view) in [('2', "Tree"), ('4', "Provenance"), ('5', "Graph")] {
            app.status_message = None;
            app.handle_key(KeyCode::Char(key));
            assert_eq!(app.top_view, TopView::Findings, "key {key}");
            let message = app.status_message.clone().unwrap_or_default();
            assert!(message.contains(view), "key {key}: {message:?}");
        }
    }

    // --- Group 7: Findings ---

    #[test]
    fn findings_list_basic() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        let _findings = app.visible_findings();
        // Just verify it doesn't panic and returns findings
    }

    #[test]
    fn unattached_findings_grouped() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        let (attached, unattached) = app.findings_attached();
        // All findings should be in one group or the other
        assert_eq!(
            attached.len() + unattached.len(),
            app.visible_findings().len()
        );
    }

    #[test]
    fn suppressed_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut issues = validate(&graph, &profile, false);
        if let Some(issue) = issues.first_mut() {
            issue.suppressed = true;
        }
        let mut app = App::new(&profile, &graph, issues, Vec::new());
        let before = app.visible_findings().len();
        app.show_suppressed = true;
        let after = app.visible_findings().len();
        assert!(after >= before);
    }

    #[test]
    fn finding_jump_to_node() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3')); // go to findings
        assert_eq!(app.top_view, TopView::Findings);
        // Enter on a finding with a node_id should jump
        let findings = app.visible_findings();
        if findings
            .iter()
            .any(|f| f.node_id.as_ref().is_some_and(|id| app.graph.has_node(id)))
        {
            let idx = findings
                .iter()
                .position(|f| f.node_id.as_ref().is_some_and(|id| app.graph.has_node(id)))
                .unwrap();
            app.findings_state.cursor = idx;
            app.handle_key(KeyCode::Enter);
            assert!(app.show_detail);
        }
    }

    #[test]
    fn finding_jump_moves_the_node_list_cursor_to_the_finding_node() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3'));
        let targets: Vec<(usize, String)> = app
            .visible_findings()
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.node_id.clone().map(|id| (i, id)))
            .filter(|(_, id)| app.graph.has_node(id))
            .collect();
        let rows: Vec<String> = app
            .visible_nodes()
            .iter()
            .map(|(id, _, _)| id.to_string())
            .collect();
        assert!(
            targets.iter().any(|(_, id)| rows.first() != Some(id)),
            "the fixture needs a finding on a node other than the first row"
        );
        for (idx, id) in targets {
            app.findings_state.cursor = idx;
            app.handle_key(KeyCode::Enter);
            assert_eq!(app.top_view, TopView::NodeList);
            assert_eq!(rows[app.node_list_state.cursor], id, "finding {idx}");
            app.handle_key(KeyCode::Char('q'));
            assert_eq!(app.top_view, TopView::Findings);
        }
    }

    #[test]
    fn finding_jump_roundtrip() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3')); // findings view
        let findings = app.visible_findings();
        if let Some(idx) = findings
            .iter()
            .position(|f| f.node_id.as_ref().is_some_and(|id| app.graph.has_node(id)))
        {
            app.findings_state.cursor = idx;
            app.handle_key(KeyCode::Enter); // jump to node
            assert_eq!(app.top_view, TopView::NodeList);
            assert!(!app.view_stack.is_empty());
            app.handle_key(KeyCode::Char('q')); // pop back
            assert_eq!(app.top_view, TopView::Findings);
            assert_eq!(app.findings_state.cursor, idx);
        }
    }

    // --- Group 8: Search ---

    #[test]
    fn search_basic() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('/'));
        assert_eq!(app.input_mode, InputMode::Search);
        for c in "REQ".chars() {
            app.handle_key(KeyCode::Char(c));
        }
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.current_state().search.is_some());
    }

    #[test]
    fn search_invalid_regex_shows_error() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('/'));
        for c in "[invalid".chars() {
            app.handle_key(KeyCode::Char(c));
        }
        app.handle_key(KeyCode::Enter);
        assert!(app.status_message.is_some());
    }

    #[test]
    fn search_cycle_matches() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Search for REQ
        app.handle_key(KeyCode::Char('/'));
        for c in "REQ".chars() {
            app.handle_key(KeyCode::Char(c));
        }
        app.handle_key(KeyCode::Enter);
        let first = app.current_state().cursor;
        app.handle_key(KeyCode::Char('n'));
        let second = app.current_state().cursor;
        // With two REQ nodes, n should move the cursor
        if app
            .visible_nodes()
            .iter()
            .filter(|(id, _, _)| id.contains("REQ"))
            .count()
            > 1
        {
            assert_ne!(first, second);
        }
    }

    // --- Edge follow from findings view ---

    #[test]
    fn stack_push_pop_preserves_cursor() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j')); // cursor = 1
        app.handle_key(KeyCode::Char('j')); // cursor = 2
        let saved = app.node_list_state.cursor;
        app.handle_key(KeyCode::Enter); // detail
        app.handle_key(KeyCode::Char('q')); // pop
        assert_eq!(app.node_list_state.cursor, saved);
    }

    // --- Jump to source ---

    /// A target directory holding `files`, unique to one test.
    fn target_with(name: &str, files: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lattice-jump-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            std::fs::write(dir.join(file), "x\n").unwrap();
        }
        dir
    }

    #[test]
    fn jump_to_source_no_editor_says_why() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        assert!(matches!(app.jump_to_source_with(false), Action::None));
        let message = app.status_message.as_deref().unwrap_or_default();
        assert!(message.contains("$EDITOR"), "{message}");
    }

    #[test]
    fn jump_to_source_with_editor_opens_the_selected_provenance() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        let dir = target_with("opens", &["reqs.md"]);
        app.set_target_dir(&dir);
        let expected = app
            .current_provenance()
            .expect("the first node has provenance")
            .clone();
        match app.jump_to_source_with(true) {
            Action::OpenEditor(opened) => {
                assert!(opened.file.ends_with(expected.file()), "{}", opened.file);
                assert_eq!(opened.line, expected.line());
            }
            _ => panic!("an editor is configured, so the jump opens it"),
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    // A finding jump focuses the detail on its first row, a section heading.
    // `e` there opens the node the detail shows, not nothing.
    #[test]
    fn jump_to_source_after_a_finding_jump_opens_the_node() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        let dir = target_with("finding-jump", &["reqs.md"]);
        app.set_target_dir(&dir);
        app.handle_key(KeyCode::Char('3'));
        app.findings_state.cursor = app
            .visible_findings()
            .iter()
            .position(|f| f.node_id.as_ref().is_some_and(|id| graph.has_node(id)))
            .expect("the fixture needs a finding on a node");
        app.handle_key(KeyCode::Enter);
        assert!(app.detail_focused);
        let expected = app
            .current_detail_node()
            .expect("the jump shows the finding's node")
            .provenance
            .clone();
        match app.jump_to_source_with(true) {
            Action::OpenEditor(opened) => {
                assert!(opened.file.ends_with(expected.file()), "{}", opened.file);
                assert_eq!(opened.line, expected.line());
            }
            _ => panic!("the detail shows a node, so the jump opens its source"),
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jump_to_source_with_nothing_selected_says_why() {
        // Every declared kind is used, so the register has no finding at all.
        let document = json!({"interface_version": "1.0", "nodes": [
            {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 1}},
            {"id": "SPEC-1", "kind": "spec", "attrs": {}, "provenance": {"file": "s.md", "line": 1}},
            {"id": "TEST-1", "kind": "test", "attrs": {}, "provenance": {"file": "t.md", "line": 1}}],
            "edges": [
            {"src": "SPEC-1", "tgt": "REQ-1", "kind": "satisfies", "provenance": {"file": "s.md", "line": 1}},
            {"src": "TEST-1", "tgt": "SPEC-1", "kind": "verifies", "provenance": {"file": "t.md", "line": 1}}]});
        let profile = test_profile();
        let graph = ingest_document(document).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3'));
        assert!(app.visible_findings().is_empty());
        assert!(matches!(app.jump_to_source_with(true), Action::None));
        let message = app.status_message.as_deref().unwrap_or_default();
        assert!(!message.is_empty(), "a jump that cannot happen says why");
    }

    // A provenance can name something that is not a work-tree file: a path on a
    // git branch (`entomologist-data:<id>`), an API locator. Handing it to the
    // editor opens an empty buffer for a file that does not exist.
    #[test]
    fn jump_to_source_refuses_a_provenance_that_is_not_a_work_tree_file() {
        let document = json!({"interface_version": "1.0", "nodes": [
            {"id": "REQ-001", "kind": "req", "attrs": {},
             "provenance": {"file": "entomologist-data:5a3f", "line": 0}}]});
        let profile = test_profile();
        let graph = ingest_document(document).unwrap();
        let mut app = make_app(&profile, &graph);
        let dir = target_with("refuses", &[]);
        app.set_target_dir(&dir);

        assert!(matches!(app.jump_to_source_with(true), Action::None));
        let message = app.status_message.as_deref().unwrap_or_default();
        assert!(message.contains("entomologist-data:5a3f"), "{message}");
        assert!(message.contains("not a file"), "{message}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    // A finding about the profile as a whole names no file; jump and preview say
    // so rather than offering `<profile>` as a path.
    #[test]
    fn jump_and_preview_on_a_profile_finding_say_it_has_no_file() {
        let profile = profile_from(
            "name: t\nprofile_version: \"1.0.0\"\nnode_kinds:\n  req:\n    id_pattern: \"^REQ-\\\\d+$\"\n\
             edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: UNTRACED, node_ids: [REQ-9]}\n",
        );
        let graph = ingest_document(json!({"interface_version": "1.0", "nodes": [
            {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 1}}]}))
        .unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3'));
        assert_eq!(app.visible_findings()[0].code, "SUPPRESS_UNUSED");

        assert!(matches!(app.jump_to_source_with(true), Action::None));
        let message = app.status_message.take().unwrap_or_default();
        assert_eq!(
            message,
            "SUPPRESS_UNUSED is about the profile, not a line in a file"
        );

        app.open_preview();
        assert!(app.preview.is_none());
        let message = app.status_message.take().unwrap_or_default();
        assert_eq!(
            message,
            "SUPPRESS_UNUSED is about the profile, not a line in a file"
        );
    }

    #[test]
    fn status_text_shows_counts() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        let status = app.status_text();
        assert!(status.contains("nodes"));
        assert!(status.contains("edges"));
    }

    // --- Tree view ---

    #[test]
    fn tree_root_from_node_list() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(app.top_view, TopView::Tree);
        assert!(app.tree_state.root.is_some());
        assert!(app.tree_report.is_some());
    }

    #[test]
    fn tree_expand_reveals_children() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Root at REQ-001 (cursor 0)
        app.handle_key(KeyCode::Char('2'));
        let before = app.visible_tree_rows().len();
        // Expand root
        app.handle_key(KeyCode::Char(' '));
        let after = app.visible_tree_rows().len();
        // REQ-001 has no outgoing edges in the test graph, so
        // try from SPEC-001 which has an outgoing satisfies edge
        let mut app2 = make_app(&profile, &graph);
        app2.handle_key(KeyCode::Char('j')); // cursor past reqs
        app2.handle_key(KeyCode::Char('j'));
        app2.handle_key(KeyCode::Char('2')); // root at SPEC-001
        let before2 = app2.visible_tree_rows().len();
        app2.handle_key(KeyCode::Char(' '));
        let after2 = app2.visible_tree_rows().len();
        assert!(
            after >= before || after2 > before2,
            "expanding a node with children should reveal them"
        );
    }

    #[test]
    fn tree_collapse_hides_descendants() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Navigate to TEST-001 (has outgoing verifies → SPEC-001)
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Char(' ')); // expand
        let expanded = app.visible_tree_rows().len();
        app.handle_key(KeyCode::Char('-')); // collapse
        let collapsed = app.visible_tree_rows().len();
        assert!(collapsed <= expanded);
    }

    #[test]
    fn tree_enter_pushes_detail() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Enter);
        assert!(!app.view_stack.is_empty());
        assert_eq!(app.view_stack.last().unwrap().source_view, TopView::Tree);
    }

    #[test]
    fn tree_expansion_resets_on_rebuild() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Char(' ')); // expand root
        assert!(!app.tree_state.expanded.is_empty());
        app.handle_key(KeyCode::Char('d')); // toggle direction → rebuild
        assert!(app.tree_state.expanded.is_empty());
    }

    #[test]
    fn tree_vacancy_and_taint_flags() {
        let profile = profile_from(
            r#"
name: test
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds:
  refs:
    allowed: [[req, req]]
"#,
        );
        let graph = ingest_document(json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "REQ-001", "kind": "req", "attrs": {},
                 "provenance": {"file": "f.md", "line": 1}},
                {"id": "REQ-002", "kind": "req", "attrs": {},
                 "provenance": {"file": "f.md", "line": 2}}
            ],
            "edges": [
                {"src": "REQ-001", "tgt": "GHOST", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 3}},
                {"src": "GHOST", "tgt": "REQ-002", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 4}}
            ]
        }))
        .unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2')); // root at REQ-001
        app.handle_key(KeyCode::Char(' ')); // expand root
        let ghost_idx = {
            let rows = app.visible_tree_rows();
            let ghost = rows.iter().find(|r| r.node_id == "GHOST");
            assert!(ghost.is_some(), "vacancy endpoint should appear");
            assert_eq!(ghost.unwrap().endpoint, Endpoint::Vacancy);
            rows.iter().position(|r| r.node_id == "GHOST").unwrap()
        };
        // REQ-002 is reachable only through GHOST → tainted
        // Need to expand GHOST first
        app.tree_state.cursor = ghost_idx;
        app.handle_key(KeyCode::Char(' ')); // expand GHOST
        let rows2 = app.visible_tree_rows();
        let req2 = rows2.iter().find(|r| r.node_id == "REQ-002");
        if let Some(r) = req2 {
            assert!(
                r.tainted,
                "REQ-002 is reachable only through vacancy → tainted"
            );
        }
    }

    #[test]
    fn tree_severity_matches_node_severity_map() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        let rows = app.visible_tree_rows();
        for row in rows.iter() {
            let expected = app.node_severity.get(row.node_id).copied();
            assert_eq!(row.severity, expected);
        }
    }

    #[test]
    fn tree_direction_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(app.tree_state.direction, Direction::Forward);
        app.handle_key(KeyCode::Char('d'));
        assert_eq!(app.tree_state.direction, Direction::Reverse);
        assert!(app.tree_report.is_some());
    }

    #[test]
    fn tree_direction_persists_across_reroot() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Char('d')); // reverse
        // Re-root: expand then move cursor to a child and press 2
        app.handle_key(KeyCode::Char(' ')); // expand
        if app.visible_tree_rows().len() > 1 {
            app.tree_state.cursor = 1;
            app.handle_key(KeyCode::Char('2')); // re-root
            assert_eq!(app.tree_state.direction, Direction::Reverse);
        }
    }

    #[test]
    fn tree_edge_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Root at TEST-001 (index 3)
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Char(' ')); // expand
        let before = app.visible_tree_rows().len();
        // Disable all edge kinds
        for i in 0..app.tree_state.edge_kind_filter.len() {
            app.tree_state.edge_kind_filter[i] = false;
        }
        app.rebuild_tree();
        let after = app.visible_tree_rows().len();
        assert!(after <= 1, "with no edge kinds, only root should remain");
        assert!(after < before || before == 1);
    }

    #[test]
    fn tree_search_match_cycling() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Root at REQ-001, expand to show more
        app.handle_key(KeyCode::Char('2'));
        // Search for the root node
        app.handle_key(KeyCode::Char('/'));
        for c in "REQ".chars() {
            app.handle_key(KeyCode::Char(c));
        }
        app.handle_key(KeyCode::Enter);
        assert!(app.tree_state.search.is_some());
    }

    #[test]
    fn tree_status_text_content() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        let status = app.status_text();
        assert!(status.contains("Tree"));
        assert!(status.contains("forward"));
        assert!(status.contains("reached"));
    }

    #[test]
    fn tree_state_preserved_across_view_switches() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2')); // enter tree
        app.handle_key(KeyCode::Char(' ')); // expand
        assert!(!app.tree_state.expanded.is_empty());
        app.handle_key(KeyCode::Char('d')); // set reverse
        app.handle_key(KeyCode::Char('1')); // switch to node list
        assert_eq!(app.top_view, TopView::NodeList);
        assert_eq!(app.tree_state.direction, Direction::Reverse);
        app.handle_key(KeyCode::Char('2')); // back to tree (re-roots)
        assert_eq!(app.top_view, TopView::Tree);
        assert_eq!(
            app.tree_state.direction,
            Direction::Reverse,
            "direction should persist"
        );
    }

    #[test]
    fn tree_stack_push_pop_from_tree() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2')); // enter tree
        let saved_cursor = app.tree_state.cursor;
        app.handle_key(KeyCode::Enter); // push detail
        assert_eq!(app.view_stack.last().unwrap().source_view, TopView::Tree);
        app.handle_key(KeyCode::Char('q')); // pop back
        assert_eq!(app.top_view, TopView::Tree);
        assert_eq!(app.tree_state.cursor, saved_cursor);
    }

    // --- SCC cluster collapse ---

    fn cyclic_profile() -> Profile {
        profile_from(
            r#"
name: test
profile_version: "1.0.0"
node_kinds:
  item:
    id_pattern: "^I-\\d+$"
edge_kinds:
  refs:
    allowed: [[item, item]]
"#,
        )
    }

    fn cyclic_document() -> serde_json::Value {
        json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "I-1", "kind": "item", "attrs": {},
                 "provenance": {"file": "f.md", "line": 1}},
                {"id": "I-2", "kind": "item", "attrs": {},
                 "provenance": {"file": "f.md", "line": 2}},
                {"id": "I-3", "kind": "item", "attrs": {},
                 "provenance": {"file": "f.md", "line": 3}},
                {"id": "I-4", "kind": "item", "attrs": {},
                 "provenance": {"file": "f.md", "line": 4}}
            ],
            "edges": [
                {"src": "I-1", "tgt": "I-2", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 5}},
                {"src": "I-2", "tgt": "I-3", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 6}},
                {"src": "I-3", "tgt": "I-1", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 7}},
                {"src": "I-1", "tgt": "I-4", "kind": "refs",
                 "provenance": {"file": "f.md", "line": 8}}
            ]
        })
    }

    #[test]
    fn scc_collapse_toggle() {
        let profile = cyclic_profile();
        let graph = ingest_document(cyclic_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        assert!(!app.tree_state.scc_collapse);
        app.handle_key(KeyCode::Char('z'));
        assert!(app.tree_state.scc_collapse);
        app.handle_key(KeyCode::Char('z'));
        assert!(!app.tree_state.scc_collapse);
    }

    #[test]
    fn scc_collapse_cluster_row() {
        let profile = cyclic_profile();
        let graph = ingest_document(cyclic_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2')); // root at I-1
        app.handle_key(KeyCode::Char(' ')); // expand root

        // Without SCC collapse: should show cycle markers
        assert!(app.visible_tree_rows().iter().all(|r| r.cluster.is_none()));

        // With SCC collapse
        app.handle_key(KeyCode::Char('z'));
        let rows_on = app.visible_tree_rows();
        let cluster = rows_on.iter().find(|r| r.cluster.is_some());
        if let Some(cr) = cluster {
            assert!(cr.cluster.as_ref().unwrap().len() > 1);
            assert!(cr.summary.as_ref().unwrap().contains("nodes"));
        }
    }

    #[test]
    fn scc_collapse_single_member_unaffected() {
        let profile = cyclic_profile();
        let graph = ingest_document(cyclic_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Char('z'));
        app.handle_key(KeyCode::Char(' ')); // expand
        let rows = app.visible_tree_rows();
        // I-4 is not part of a cycle — should appear normally
        let i4 = rows.iter().find(|r| r.node_id == "I-4");
        if let Some(r) = i4 {
            assert!(r.cluster.is_none());
        }
    }

    // --- Graph view ---

    #[test]
    fn graph_activation_from_node_list() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('5'));
        assert_eq!(app.top_view, TopView::Graph);
        assert!(app.graph_state.center.is_some());
        assert!(app.graph_layout.is_some());
    }

    #[test]
    fn graph_activation_from_tree() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2')); // tree
        app.handle_key(KeyCode::Char('5')); // graph
        assert_eq!(app.top_view, TopView::Graph);
    }

    #[test]
    fn graph_activation_from_provenance() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('4')); // provenance
        app.handle_key(KeyCode::Char('5')); // graph
        assert_eq!(app.top_view, TopView::Graph);
    }

    #[test]
    fn graph_activation_from_findings() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3')); // findings
        // Only works if cursor is on a finding with a valid node
        let findings = app.visible_findings();
        if findings
            .iter()
            .any(|f| f.node_id.as_ref().is_some_and(|id| app.graph.has_node(id)))
        {
            let idx = findings
                .iter()
                .position(|f| f.node_id.as_ref().is_some_and(|id| app.graph.has_node(id)))
                .unwrap();
            app.findings_state.cursor = idx;
            app.handle_key(KeyCode::Char('5'));
            assert_eq!(app.top_view, TopView::Graph);
        }
    }

    #[test]
    fn graph_recenter_push_pop() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Start on SPEC-001 (cursor 2)
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));
        let original_center = app.graph_state.center.clone().unwrap();
        assert!(app.graph_state.center_history.is_empty());

        // Move to a neighbor and re-center
        let rows = app.visible_graph_rows();
        if let Some(idx) = rows
            .iter()
            .position(|r| !r.is_center && r.overflow.is_none())
        {
            app.graph_state.cursor = idx;
            app.handle_key(KeyCode::Enter);
            assert_ne!(
                app.graph_state.center.as_deref(),
                Some(original_center.as_str())
            );
            assert_eq!(app.graph_state.center_history.len(), 1);

            // Pop back via Backspace; q quits
            app.handle_key(KeyCode::Backspace);
            assert_eq!(
                app.graph_state.center.as_deref(),
                Some(original_center.as_str())
            );
            assert!(app.graph_state.center_history.is_empty());
        }
    }

    #[test]
    fn graph_depth_radius_bounds() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('5'));
        assert_eq!(app.graph_state.depth_radius(), 2);

        app.handle_key(KeyCode::Char('+'));
        assert_eq!(app.graph_state.depth_radius(), 3);

        // Go back to 2, then 1, then try to go below
        app.handle_key(KeyCode::Char('-'));
        app.handle_key(KeyCode::Char('-'));
        assert_eq!(app.graph_state.depth_radius(), 1);
        app.handle_key(KeyCode::Char('-'));
        assert_eq!(app.graph_state.depth_radius(), 1, "cannot go below 1");
    }

    #[test]
    fn graph_kind_filter_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('5'));
        let before = app.visible_graph_rows().len();
        // Toggle off first kind
        app.graph_state.kind_filter[0] = false;
        let after = app.visible_graph_rows().len();
        assert!(after <= before);
    }

    #[test]
    fn graph_state_preserved_across_view_switches() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('5')); // graph
        app.graph_state.increase_depth_radius();
        let saved_center = app.graph_state.center.clone();

        app.handle_key(KeyCode::Char('1')); // node list
        assert_eq!(app.top_view, TopView::NodeList);
        assert_eq!(app.graph_state.depth_radius(), 3);
        assert_eq!(app.graph_state.center, saved_center);
    }

    #[test]
    fn graph_tab_pushes_detail() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));

        // Move to a non-center row
        let rows = app.visible_graph_rows();
        if let Some(idx) = rows
            .iter()
            .position(|r| !r.is_center && r.overflow.is_none())
        {
            app.graph_state.cursor = idx;
            app.handle_key(KeyCode::Tab);
            assert!(!app.view_stack.is_empty());
            assert_eq!(app.view_stack.last().unwrap().source_view, TopView::Graph);
            assert!(app.show_detail);

            // Tab again closes
            app.handle_key(KeyCode::Tab);
            assert!(app.view_stack.is_empty());
            assert!(!app.show_detail);
        }
    }

    // --- Help overlay ---

    #[test]
    fn help_overlay_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        assert_eq!(app.input_mode, InputMode::Normal);
        app.handle_key(KeyCode::Char('?'));
        assert_eq!(app.input_mode, InputMode::Help);
        // Any key dismisses
        app.handle_key(KeyCode::Char('a'));
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn view_bindings_per_view() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        let bindings = app.view_bindings();
        assert!(bindings.len() >= 2);
        assert_eq!(bindings[0].0, "Global");
        assert_eq!(bindings[1].0, "Node list");

        app.handle_key(KeyCode::Char('5'));
        if app.top_view == TopView::Graph {
            let bindings = app.view_bindings();
            assert_eq!(bindings[1].0, "Graph");
        }
    }

    #[test]
    fn tree_help_lists_the_cycle_collapse_toggle() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(app.top_view, TopView::Tree);

        let bindings = app.view_bindings();
        let (_, tree) = bindings
            .iter()
            .find(|(section, _)| *section == "Tree")
            .expect("the Tree view has a help section");
        assert!(tree.iter().any(|binding| binding.key == "z"));
    }

    // --- Which-key hints ---

    #[test]
    fn view_hints_non_empty() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        for view in [
            TopView::NodeList,
            TopView::Findings,
            TopView::Tree,
            TopView::Provenance,
            TopView::Graph,
        ] {
            app.top_view = view;
            assert!(!app.view_hints().is_empty(), "hints empty for {view:?}");
        }
    }

    // --- Breadcrumb and q-always-quits ---

    #[test]
    fn graph_q_always_quits() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));
        assert_eq!(app.top_view, TopView::Graph);

        // Re-center on a neighbor
        let rows = app.visible_graph_rows();
        if let Some(idx) = rows
            .iter()
            .position(|r| !r.is_center && r.overflow.is_none())
        {
            app.graph_state.cursor = idx;
            app.handle_key(KeyCode::Enter);
            assert!(!app.graph_state.center_history.is_empty());

            // q should quit, not pop history
            let action = app.handle_key(KeyCode::Char('q'));
            assert!(matches!(action, Action::Quit));
        }
    }

    #[test]
    fn graph_backspace_pops_history() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));
        let original_center = app.graph_state.center.clone().unwrap();

        let rows = app.visible_graph_rows();
        if let Some(idx) = rows
            .iter()
            .position(|r| !r.is_center && r.overflow.is_none())
        {
            app.graph_state.cursor = idx;
            app.handle_key(KeyCode::Enter);
            assert_eq!(app.graph_state.center_history.len(), 1);

            app.handle_key(KeyCode::Backspace);
            assert_eq!(
                app.graph_state.center.as_deref(),
                Some(original_center.as_str())
            );
            assert!(app.graph_state.center_history.is_empty());
        }
    }

    #[test]
    fn graph_backspace_noop_when_empty() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));
        let center = app.graph_state.center.clone();
        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.graph_state.center, center);
    }

    // --- Column mode ---

    #[test]
    fn column_mode_cycling() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));
        assert_eq!(app.graph_state.column_mode, ColumnMode::Default);
        app.handle_key(KeyCode::Char('C'));
        assert_eq!(app.graph_state.column_mode, ColumnMode::Full);
        app.handle_key(KeyCode::Char('C'));
        assert_eq!(app.graph_state.column_mode, ColumnMode::Compact);
        app.handle_key(KeyCode::Char('C'));
        assert_eq!(app.graph_state.column_mode, ColumnMode::Default);
    }

    // --- Markdown preview ---

    #[test]
    fn markdown_preview_is_never_dim() {
        // Requirement: browse/Accessibility baseline — dim, Markdown in the file preview.
        let body = "---\ntitle: Front matter\n---\n\n# Heading {#anchor}\n\n\
                    Text with a note[^1] and ![an image](x.png).\n\n[^1]: The footnote.\n";
        let dim_in = |text: &Text<'_>| {
            text.style.add_modifier.contains(Modifier::DIM)
                || text.lines.iter().any(|line| {
                    line.style.add_modifier.contains(Modifier::DIM)
                        || line
                            .spans
                            .iter()
                            .any(|span| span.style.add_modifier.contains(Modifier::DIM))
                })
        };
        // The input reaches tui-markdown's dimmed styles, so the check has teeth.
        assert!(
            dim_in(&tui_markdown::from_str(body)),
            "the sample renders nothing dim"
        );
        assert!(
            !dim_in(&render_markdown(body)),
            "the preview kept a dimmed style"
        );
    }

    // --- Search filter ---

    #[test]
    fn search_filter_toggles() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));

        // Set a search
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('R'));
        app.handle_key(KeyCode::Char('E'));
        app.handle_key(KeyCode::Char('Q'));
        app.handle_key(KeyCode::Enter);
        assert!(app.graph_state.search.is_some());
        assert!(!app.graph_state.search_filter);

        // Toggle filter
        app.toggle_search_filter();
        assert!(app.graph_state.search_filter);

        // Toggle off
        app.toggle_search_filter();
        assert!(!app.graph_state.search_filter);
    }

    #[test]
    fn search_filter_has_a_key_without_a_ctrl_chord() {
        // Requirement: browse/Accessibility baseline — everything reachable without Ctrl chords.
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('5'));
        app.handle_key(KeyCode::Char('/'));
        for ch in "REQ".chars() {
            app.handle_key(KeyCode::Char(ch));
        }
        app.handle_key(KeyCode::Enter);
        app.handle_key(KeyCode::Char('&'));
        assert!(app.graph_state.search_filter);
        app.handle_key(KeyCode::Char('&'));
        assert!(!app.graph_state.search_filter);

        // In the search prompt `&` is text, not the toggle.
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('&'));
        assert!(!app.graph_state.search_filter);
    }

    // --- Tab bar and view names ---

    #[test]
    fn views_constant_matches_key_dispatch() {
        assert_eq!(VIEWS[0], (TopView::NodeList, '1', "Nodes"));
        assert_eq!(VIEWS[1], (TopView::Tree, '2', "Tree"));
        assert_eq!(VIEWS[2], (TopView::Findings, '3', "Findings"));
        assert_eq!(VIEWS[3], (TopView::Provenance, '4', "Provenance"));
        assert_eq!(VIEWS[4], (TopView::Graph, '5', "Graph"));
    }

    #[test]
    fn tab_index_round_trips() {
        for (i, &(view, _, _)) in VIEWS.iter().enumerate() {
            assert_eq!(view.tab_index(), i);
        }
    }

    #[test]
    fn help_bindings_list_named_views() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let app = make_app(&profile, &graph);
        let bindings = app.view_bindings();
        let global = &bindings[0].1;
        let view_keys: Vec<&str> = global.iter().take(5).map(|b| b.key.as_str()).collect();
        assert_eq!(view_keys, vec!["1", "2", "3", "4", "5"]);
        let view_names: Vec<&str> = global.iter().take(5).map(|b| b.desc).collect();
        assert_eq!(
            view_names,
            vec!["Nodes", "Tree", "Findings", "Provenance", "Graph"]
        );
    }

    // --- Left/Right consistency ---

    #[test]
    fn right_opens_detail_left_closes_it() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        assert!(!app.show_detail);
        app.handle_key(KeyCode::Right);
        assert!(app.show_detail);
        app.handle_key(KeyCode::Right);
        assert!(app.detail_focused);
        app.handle_key(KeyCode::Left);
        assert!(!app.detail_focused);
        assert!(app.show_detail);
        app.handle_key(KeyCode::Left);
        assert!(!app.show_detail);
    }

    // --- Kind filter in all views ---

    #[test]
    fn f_activates_kind_filter_in_findings() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('3'));
        assert_eq!(app.top_view, TopView::Findings);
        app.handle_key(KeyCode::Char('f'));
        assert_eq!(app.input_mode, InputMode::KindFilter);
    }

    #[test]
    fn f_activates_kind_filter_in_tree() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(app.top_view, TopView::Tree);
        app.handle_key(KeyCode::Char('f'));
        assert_eq!(app.input_mode, InputMode::KindFilter);
    }

    #[test]
    fn findings_kind_filter_hides_by_node_kind() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        let all = app.visible_findings().len();
        assert!(all > 0, "test fixture should produce findings");
        let has_req_finding = all
            > app
                .visible_findings()
                .iter()
                .filter(|i| {
                    i.node_id
                        .as_ref()
                        .and_then(|id| app.graph.node(id))
                        .is_none_or(|n| n.kind != "req")
                })
                .count();
        assert!(
            has_req_finding,
            "fixture should have a req-attached finding"
        );
        app.findings_state.kind_filter[0] = false;
        let filtered = app.visible_findings();
        assert!(filtered.len() < all);
        assert!(filtered.iter().all(|i| {
            i.node_id
                .as_ref()
                .and_then(|id| app.graph.node(id))
                .is_none_or(|n| n.kind != "req")
        }));
    }

    #[test]
    fn left_pops_view_stack_before_view_back() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Switch to tree, root on SPEC-001
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(app.top_view, TopView::Tree);
        // Open detail via Enter
        app.handle_key(KeyCode::Enter);
        assert!(!app.view_stack.is_empty());
        // Left should pop view_stack first
        app.handle_key(KeyCode::Left);
        assert!(app.view_stack.is_empty());
    }

    /// Tree rooted on SPEC-001, Enter, follow its out edge, Left: one frame left on
    /// the stack, so the pane is drawn from the stack with focus in the tree.
    fn tree_with_one_stacked_frame<'a>(profile: &'a Profile, graph: &'a LatticeGraph) -> App<'a> {
        let mut app = make_app(profile, graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Enter);
        let edge_idx = app
            .detail_rows()
            .iter()
            .position(|r| matches!(r, DetailRow::OutEdge { .. }))
            .expect("SPEC-001 has an out edge");
        app.detail_cursor = edge_idx;
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.view_stack.len(), 2);
        app.handle_key(KeyCode::Left);
        assert_eq!(app.view_stack.len(), 1);
        app
    }

    #[test]
    fn tab_focuses_stacked_tree_detail_after_left() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = tree_with_one_stacked_frame(&profile, &graph);
        assert!(!app.detail_focused);
        app.handle_key(KeyCode::Tab);
        assert!(app.detail_focused);
    }

    #[test]
    fn right_follows_tree_row_in_stacked_tree_after_left() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = tree_with_one_stacked_frame(&profile, &graph);
        app.handle_key(KeyCode::Right);
        assert_eq!(app.view_stack.len(), 2);
        assert_eq!(app.view_stack.last().unwrap().source_view, TopView::Tree);
    }

    #[test]
    fn tree_kind_filter_hides_rows() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('2'));
        // Expand root to see children
        app.handle_key(KeyCode::Char(' '));
        let all = {
            let rows = app.visible_tree_rows();
            let has_req = rows
                .iter()
                .any(|r| app.graph.node(r.node_id).is_some_and(|n| n.kind == "req"));
            assert!(has_req, "expanded tree should contain a req node");
            rows.len()
        };
        assert!(all > 1, "tree should have root + children");
        app.tree_state.kind_filter[0] = false;
        let filtered = app.visible_tree_rows();
        assert!(filtered.len() < all);
        assert!(
            filtered
                .iter()
                .all(|r| app.graph.node(r.node_id).is_none_or(|n| n.kind != "req"))
        );
    }

    #[test]
    fn provenance_kind_filter_hides_rows() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.top_view, TopView::Provenance);
        let all = app.visible_provenance_indices().len();
        assert!(all > 0);
        app.provenance_state.kind_filter[0] = false;
        let filtered = app.visible_provenance_indices().len();
        assert!(filtered <= all);
    }

    #[test]
    fn kind_filter_enter_confirms_without_toggling() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        for key in ['f', ' ', 'j', ' '] {
            app.handle_key(KeyCode::Char(key));
        }
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.node_list_state.kind_filter, [false, false, true]);
    }

    #[test]
    fn f_in_provenance_opens_nothing_and_says_why() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.top_view, TopView::Provenance);
        app.handle_key(KeyCode::Char('f'));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("Provenance"))
        );
    }

    #[test]
    fn provenance_git_commits_empty_without_target() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.top_view, TopView::Provenance);
        assert!(app.git_commits.is_empty());
        assert!(app.target_dir.is_none());
    }

    #[test]
    fn provenance_git_commits_populated_with_target() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("lattice-app-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        };
        run(&["init", "-b", "main"]);
        run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ]);
        std::fs::write(dir.join("reqs.md"), "line1\nline2\n").unwrap();
        run(&["add", "reqs.md"]);
        run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add reqs",
        ]);

        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.set_target_dir(&dir);
        // REQ-001 at cursor 0 has provenance reqs.md:1
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.top_view, TopView::Provenance);
        assert!(app.git_fetch_pending(), "the fetch runs off the key path");
        assert!(
            app.git_commits.is_empty(),
            "nothing shown until git answers"
        );
        app.await_git_commits();
        assert!(
            !app.git_commits.is_empty(),
            "should have commits for reqs.md:1"
        );
        assert_eq!(app.git_commits[0].subject, "add reqs");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_git_fetch_never_lands_on_a_newer_node() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.set_target_dir(Path::new("/nonexistent/lattice-no-repo"));
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.git_commit_node.as_deref(), Some("REQ-001"));
        assert!(app.git_fetch_pending());
        // Stand in for a slow REQ-001 fetch.
        let (tx, rx) = mpsc::channel();
        app.git_fetch = Some(rx);
        app.handle_key(KeyCode::Char('1'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.git_commit_node.as_deref(), Some("REQ-002"));
        assert!(
            tx.send((
                "REQ-001".into(),
                git::LogFetch::Commits(vec![commit("old")])
            ))
            .is_ok()
        );
        assert!(app.poll_git_commits());
        assert!(app.git_commits.is_empty(), "REQ-001's result is discarded");
        assert!(
            app.git_fetch_pending(),
            "REQ-002's fetch starts in its place"
        );
        app.await_git_commits();
        assert!(app.git_commits.is_empty(), "no repo, so no commits");
    }

    #[test]
    fn provenance_git_fetch_times_out() {
        let dir = std::env::temp_dir().join(format!("lattice-app-git-hang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("git");
        crate::stub::write_stub(&shim, "#!/bin/sh\nexec sleep 30\n");

        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.set_target_dir(&dir);
        app.use_git(&shim, Duration::from_secs(1));
        let started = Instant::now();
        app.handle_key(KeyCode::Char('4'));
        assert!(app.git_fetch_pending());
        assert!(!app.git_fetch_timed_out);
        app.await_git_commits();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the bound, not the sleep, settles the fetch"
        );
        assert!(app.git_fetch_timed_out, "the panel reports the timeout");
        assert!(app.git_commits.is_empty(), "nothing is cached for the node");

        // Leaving the node starts a fresh fetch and clears the timed-out state.
        app.handle_key(KeyCode::Char('1'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.git_commit_node.as_deref(), Some("REQ-002"));
        assert!(!app.git_fetch_timed_out);
        assert!(app.git_fetch_pending());

        app.await_git_commits();
        std::fs::remove_dir_all(&dir).ok();
    }

    fn commit(subject: &str) -> git::GitCommit {
        git::GitCommit {
            hash: "abc".into(),
            parents: Vec::new(),
            author: "t".into(),
            date: "d".into(),
            subject: subject.into(),
        }
    }

    #[test]
    fn f_edge_kind_filter_in_provenance() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.top_view, TopView::Provenance);
        let before = app.visible_provenance_indices().len();
        assert!(before > 0);
        app.handle_key(KeyCode::Char('F'));
        assert_eq!(app.input_mode, InputMode::EdgeKindFilter);
        for i in 0..app.provenance_state.edge_kind_filter.len() {
            app.provenance_state.edge_kind_filter[i] = false;
        }
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.input_mode, InputMode::Normal);
        let after = app.visible_provenance_indices().len();
        assert!(
            after <= 1,
            "with no edge kinds enabled, provenance should collapse to root or empty"
        );
        assert!(after < before || before <= 1);
    }

    #[test]
    fn d_toggles_direction_in_graph() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        // Center on SPEC-001 (index 2): has incoming (TEST-001) and outgoing (REQ-001)
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('5'));
        assert_eq!(app.top_view, TopView::Graph);
        assert_eq!(app.graph_state.direction, Direction::Forward);
        let fwd_ids: Vec<String> = app
            .visible_graph_rows()
            .iter()
            .map(|r| r.id.to_string())
            .collect();
        app.handle_key(KeyCode::Char('d'));
        assert_eq!(app.graph_state.direction, Direction::Reverse);
        let rev_ids: Vec<String> = app
            .visible_graph_rows()
            .iter()
            .map(|r| r.id.to_string())
            .collect();
        // Sugiyama collects both BFS directions: all 3 connected nodes visible.
        assert_eq!(fwd_ids.len(), 3, "Sugiyama shows all reachable nodes");
        assert_eq!(rev_ids.len(), 3, "reverse also shows all reachable nodes");
        assert!(
            fwd_ids.iter().any(|id| id == "TEST-001") && fwd_ids.iter().any(|id| id == "REQ-001"),
            "forward sees both ends of the chain"
        );
        assert_ne!(
            fwd_ids, rev_ids,
            "direction toggle should change layer ordering"
        );
        app.handle_key(KeyCode::Char('d'));
        assert_eq!(app.graph_state.direction, Direction::Forward);
    }

    // --- Row cache transitions: every state change that shapes a view's rows ---

    fn tree_ids(app: &App<'_>) -> Vec<String> {
        app.visible_tree_rows()
            .iter()
            .map(|r| r.node_id.to_string())
            .collect()
    }

    fn lane_ids(app: &App<'_>) -> Vec<String> {
        let layout = app.provenance_layout.as_ref().expect("lane layout");
        app.visible_provenance_indices()
            .iter()
            .map(|&ri| layout.rows[ri].id.clone())
            .collect()
    }

    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    fn graph_ids(app: &App<'_>) -> Vec<String> {
        app.visible_graph_rows()
            .iter()
            .map(|r| r.id.to_string())
            .collect()
    }

    /// Press `key` with the node-list cursor on the n-th node first.
    fn press_at(app: &mut App<'_>, n: usize, key: char) {
        for _ in 0..n {
            app.handle_key(KeyCode::Char('j'));
        }
        app.handle_key(KeyCode::Char(key));
    }

    #[test]
    fn tree_rows_follow_expand_and_collapse() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '2'); // root TEST-001
        assert_eq!(tree_ids(&app), ["TEST-001"]);
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app), ["TEST-001", "SPEC-001"]);
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app), ["TEST-001", "SPEC-001", "REQ-001"]);
        app.handle_key(KeyCode::Char('-'));
        assert_eq!(tree_ids(&app), ["TEST-001", "SPEC-001"]);
        app.handle_key(KeyCode::Char('k'));
        app.handle_key(KeyCode::Char('-'));
        assert_eq!(tree_ids(&app), ["TEST-001"]);
    }

    #[test]
    fn tree_rows_follow_reroot() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '2');
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('2')); // re-root at the cursor row
        assert_eq!(app.tree_state.root.as_deref(), Some("SPEC-001"));
        assert_eq!(tree_ids(&app), ["SPEC-001"]);
        assert_eq!(app.tree_state.cursor, 0);
    }

    #[test]
    fn tree_rows_follow_direction() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '2');
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app).len(), 2);
        app.handle_key(KeyCode::Char('d')); // nothing points at TEST-001
        assert_eq!(tree_ids(&app), ["TEST-001"]);
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app), ["TEST-001"]);
        app.handle_key(KeyCode::Char('d'));
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app), ["TEST-001", "SPEC-001"]);
    }

    #[test]
    fn tree_rows_follow_edge_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '2');
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app).len(), 2);
        // Edge kinds in key order: satisfies, verifies. Drop verifies.
        app.handle_key(KeyCode::Char('F'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Enter);
        assert_eq!(tree_ids(&app), ["TEST-001"]);
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app), ["TEST-001"], "no verifies edge to follow");
    }

    #[test]
    fn tree_rows_follow_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '2');
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app).len(), 3);
        // Kinds in key order: req, spec, test. Space toggles the slot; Enter closes.
        app.handle_key(KeyCode::Char('f'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Enter);
        assert!(!app.tree_state.kind_filter[1]);
        assert_eq!(tree_ids(&app), ["TEST-001"], "spec and its subtree hidden");
        app.handle_key(KeyCode::Char('f'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Enter);
        assert_eq!(tree_ids(&app).len(), 3, "expansion survives the filter");
    }

    #[test]
    fn tree_rows_follow_scc_collapse() {
        let profile = test_profile();
        let graph = ingest_document(json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "REQ-001", "kind": "req", "attrs": {"title": "a"},
                 "provenance": {"file": "reqs.md", "line": 1}},
                {"id": "SPEC-001", "kind": "spec", "attrs": {"title": "b"},
                 "provenance": {"file": "specs.md", "line": 1}}
            ],
            "edges": [
                {"src": "SPEC-001", "tgt": "REQ-001", "kind": "satisfies",
                 "provenance": {"file": "specs.md", "line": 2}},
                {"src": "REQ-001", "tgt": "SPEC-001", "kind": "satisfies",
                 "provenance": {"file": "reqs.md", "line": 2}}
            ]
        }))
        .unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 1, '2'); // root SPEC-001, on the cycle
        app.handle_key(KeyCode::Char(' '));
        assert!(app.visible_tree_rows().iter().all(|r| r.cluster.is_none()));
        let before = tree_ids(&app);
        app.handle_key(KeyCode::Char('z'));
        assert!(
            app.visible_tree_rows().iter().any(|r| r.cluster.is_some()),
            "{:?}",
            tree_ids(&app)
        );
        app.handle_key(KeyCode::Char('z'));
        assert!(app.visible_tree_rows().iter().all(|r| r.cluster.is_none()));
        assert_eq!(tree_ids(&app), before);
    }

    #[test]
    fn tree_rows_follow_view_switch() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '2');
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(tree_ids(&app).len(), 2);
        app.handle_key(KeyCode::Char('1'));
        app.handle_key(KeyCode::Char('k')); // node list cursor to SPEC-001
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(tree_ids(&app), ["SPEC-001"]);
    }

    #[test]
    fn provenance_rows_follow_reroot() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '4');
        let before = lane_ids(&app);
        assert_eq!(sorted(before.clone()), ["REQ-001", "SPEC-001", "TEST-001"]);
        app.handle_key(KeyCode::Char('j'));
        let target = before[1].clone();
        app.handle_key(KeyCode::Char('4'));
        assert_eq!(app.provenance_state.root.as_deref(), Some(target.as_str()));
        assert_ne!(lane_ids(&app), before);
        assert_eq!(app.provenance_state.cursor, 0);
    }

    #[test]
    fn provenance_rows_follow_direction() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 2, '4'); // root SPEC-001
        assert_eq!(sorted(lane_ids(&app)), ["REQ-001", "SPEC-001"]);
        app.handle_key(KeyCode::Char('d'));
        assert_eq!(sorted(lane_ids(&app)), ["SPEC-001", "TEST-001"]);
    }

    #[test]
    fn provenance_rows_follow_edge_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '4');
        assert_eq!(lane_ids(&app).len(), 3);
        app.handle_key(KeyCode::Char('F'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Enter);
        assert_eq!(lane_ids(&app), ["TEST-001"]);
    }

    #[test]
    fn provenance_rows_follow_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '4');
        assert_eq!(lane_ids(&app).len(), 3);
        app.provenance_state.kind_filter[0] = false; // req
        assert_eq!(sorted(lane_ids(&app)), ["SPEC-001", "TEST-001"]);
        app.provenance_state.kind_filter[0] = true;
        assert_eq!(lane_ids(&app).len(), 3);
    }

    #[test]
    fn graph_rows_follow_recenter_and_history() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        let before = graph_ids(&app);
        assert_eq!(before.len(), 3);
        let center_of = |app: &App<'_>| -> String {
            app.visible_graph_rows()
                .iter()
                .find(|r| r.is_center)
                .map(|r| r.id.to_string())
                .expect("a center row")
        };
        assert_eq!(center_of(&app), "TEST-001");
        // Move off the center and re-center there.
        let (i, target) = {
            let rows = app.visible_graph_rows();
            let i = rows.iter().position(|r| !r.is_center).unwrap();
            (i, rows[i].id.to_string())
        };
        app.graph_state.cursor = i;
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.graph_state.center.as_deref(), Some(target.as_str()));
        assert_eq!(center_of(&app), target);
        app.handle_key(KeyCode::Backspace);
        assert_eq!(center_of(&app), "TEST-001");
        assert_eq!(graph_ids(&app), before);
    }

    #[test]
    fn graph_rows_follow_depth_radius() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        assert_eq!(graph_ids(&app).len(), 3);
        app.handle_key(KeyCode::Char('-'));
        assert_eq!(sorted(graph_ids(&app)), ["SPEC-001", "TEST-001"]);
        app.handle_key(KeyCode::Char('+'));
        assert_eq!(graph_ids(&app).len(), 3);
    }

    #[test]
    fn graph_rows_follow_direction() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        let forward = graph_ids(&app);
        app.handle_key(KeyCode::Char('d'));
        let reverse = graph_ids(&app);
        assert_eq!(sorted(forward.clone()), sorted(reverse.clone()));
        assert_ne!(forward, reverse, "layers flip with the direction");
        app.handle_key(KeyCode::Char('d'));
        assert_eq!(graph_ids(&app), forward);
    }

    #[test]
    fn graph_rows_follow_edge_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        assert_eq!(graph_ids(&app).len(), 3);
        app.handle_key(KeyCode::Char('F'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Enter);
        assert_eq!(graph_ids(&app), ["TEST-001"]);
    }

    #[test]
    fn graph_rows_follow_kind_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        app.handle_key(KeyCode::Char('f'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' ')); // spec off
        app.handle_key(KeyCode::Enter);
        assert_eq!(sorted(graph_ids(&app)), ["REQ-001", "TEST-001"]);
        app.handle_key(KeyCode::Char('f'));
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char(' '));
        app.handle_key(KeyCode::Enter);
        assert_eq!(graph_ids(&app).len(), 3);
    }

    #[test]
    fn graph_rows_follow_search_filter() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        app.handle_key(KeyCode::Char('/'));
        for c in "REQ".chars() {
            app.handle_key(KeyCode::Char(c));
        }
        app.handle_key(KeyCode::Enter);
        assert_eq!(graph_ids(&app).len(), 3, "a search alone hides nothing");
        app.toggle_search_filter();
        assert_eq!(graph_ids(&app), ["REQ-001"]);
        app.toggle_search_filter();
        assert_eq!(graph_ids(&app).len(), 3);
        app.toggle_search_filter();
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Enter); // clear the search: the filter goes with it
        assert!(!app.graph_state.search_filter);
        assert_eq!(graph_ids(&app).len(), 3);
    }

    #[test]
    fn graph_cursor_clamps_to_the_current_rows() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        press_at(&mut app, 3, '5');
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.graph_state.cursor, 2);
        app.handle_key(KeyCode::Char('-')); // two rows now
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.graph_state.cursor, 1);
    }

    #[test]
    fn schema_rows_are_the_same_across_view_switches() {
        let profile = test_profile();
        let graph = ingest_document(test_document()).unwrap();
        let mut app = make_app(&profile, &graph);
        app.handle_key(KeyCode::Char('S'));
        let count = app.schema_row_count();
        assert!(count > 0);
        app.handle_key(KeyCode::Char('S'));
        app.handle_key(KeyCode::Char('2'));
        app.handle_key(KeyCode::Char('1'));
        app.handle_key(KeyCode::Char('S'));
        assert_eq!(app.schema_row_count(), count);
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.schema_cursor, count - 1);
    }

    mod owner_action {
        use super::*;

        /// The test profile with owner actions on `req`, `comment` running
        /// `run_comment`. `spec` and `test` declare none.
        pub fn action_profile(run_comment: &str) -> Profile {
            profile_from(&format!(
                r#"
name: test
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: title
    attrs:
      title: {{ type: string }}
      number: {{ type: int }}
    actions:
      comment: {{ run: {run_comment}, mode: interactive }}
      show: {{ run: ["tool", "show", "{{id}}"], mode: capture }}
      web: {{ run: ["tool", "{{attrs.number}}"], mode: detached }}
      edit: {{ run: ["tool", "edit", "{{id}}"], mode: interactive }}
  spec:
    id_pattern: "^SPEC-\\d+$"
    summary_attr: title
    attrs:
      title: {{ type: string }}
  test:
    id_pattern: "^TEST-\\d+$"
    attrs:
      title: {{ type: string }}
edge_kinds:
  satisfies:
    allowed: [[spec, req]]
  verifies:
    allowed: [[test, spec]]
"#
            ))
        }

        pub const COMMENT: &str = r#"["tool", "comment", "{id}"]"#;

        /// A directory holding an executable `tool`, for `path_env`.
        pub fn tool_dir(name: &str) -> PathBuf {
            let dir =
                std::env::temp_dir().join(format!("lattice-owner-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let tool = dir.join("tool");
            #[cfg(unix)]
            crate::stub::write_stub(&tool, "#!/bin/sh\n");
            #[cfg(not(unix))]
            std::fs::write(&tool, "#!/bin/sh\n").unwrap();
            dir
        }

        pub fn with_tool(app: &mut App<'_>, name: &str) {
            app.path_env = Some(tool_dir(name).into_os_string());
        }

        pub fn status(app: &App<'_>) -> String {
            app.status_message.clone().unwrap_or_default()
        }

        pub fn argv(action: Action) -> Vec<String> {
            match action {
                Action::RunOwner(run) => run.argv,
                _ => panic!("expected an owner run"),
            }
        }

        /// Requirement: browse-owner-actions/Action keys; browse-owner-actions/Selected node for an action
        mod keys {
            use super::*;

            #[test]
            fn a_declared_verb_asks_first_then_runs_on_y() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "keys-declared");
                assert!(matches!(app.handle_key(KeyCode::Char('c')), Action::None));
                let pending = app.confirm.as_ref().expect("first use asks");
                assert_eq!(pending.run.display(), "tool comment REQ-001");
                assert_eq!(
                    argv(app.handle_key(KeyCode::Char('y'))),
                    ["tool", "comment", "REQ-001"]
                );
            }

            #[test]
            fn an_undeclared_verb_names_the_kind_and_verb() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                assert!(matches!(app.handle_key(KeyCode::Char('s')), Action::None));
                assert!(app.confirm.is_none());
                let text = status(&app);
                assert!(text.contains("state") && text.contains("req"), "{text}");
            }

            #[test]
            fn a_finding_without_a_node_selects_nothing() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                app.issues = vec![Issue::new(
                    Severity::Error,
                    "PARSE_ERROR",
                    "bad row",
                    Provenance::new("reqs.md", 9),
                    None,
                )];
                app.handle_key(KeyCode::Char('3'));
                assert!(matches!(app.handle_key(KeyCode::Char('c')), Action::None));
                assert!(
                    status(&app).contains("no node selected"),
                    "{}",
                    status(&app)
                );
            }

            #[test]
            fn a_finding_with_a_node_runs_against_that_node() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "keys-finding");
                app.issues = vec![Issue::new(
                    Severity::Warning,
                    "VACANCY",
                    "no edge",
                    Provenance::new("reqs.md", 5),
                    Some("REQ-002".into()),
                )];
                app.handle_key(KeyCode::Char('3'));
                app.handle_key(KeyCode::Char('c'));
                let pending = app.confirm.as_ref().expect("first use asks");
                assert_eq!(pending.run.argv, ["tool", "comment", "REQ-002"]);
            }

            #[test]
            fn a_missing_program_is_reported_without_asking() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                app.path_env = Some(tool_dir("keys-missing").join("empty").into_os_string());
                assert!(matches!(app.handle_key(KeyCode::Char('c')), Action::None));
                assert!(app.confirm.is_none());
                assert!(
                    status(&app).contains("tool was not found"),
                    "{}",
                    status(&app)
                );
            }

            #[test]
            fn an_unresolvable_placeholder_is_reported_without_asking() {
                let profile = action_profile(r#"["tool", "{attrs.number}"]"#);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "keys-placeholder");
                app.handle_key(KeyCode::Char('c'));
                assert!(app.confirm.is_none());
                assert!(status(&app).contains("number"), "{}", status(&app));
            }

            #[test]
            fn e_runs_a_declared_edit_action() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "keys-edit");
                app.handle_key(KeyCode::Char('e'));
                let pending = app.confirm.as_ref().expect("edit is an owner action here");
                assert_eq!(pending.run.argv, ["tool", "edit", "REQ-001"]);
            }

            #[test]
            fn e_without_an_edit_action_keeps_the_editor() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                // SPEC-001 is third in kind order; spec declares no actions.
                for _ in 0..2 {
                    app.handle_key(KeyCode::Char('j'));
                }
                app.edit();
                assert!(app.confirm.is_none());
            }

            #[test]
            fn reload_runs_no_action() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                assert!(matches!(
                    app.handle_key(KeyCode::Char('r')),
                    Action::Reload(Reload::Register)
                ));
                assert!(app.confirm.is_none());
            }
        }

        /// Requirement: browse-owner-actions/Launcher menu
        mod menu {
            use super::*;

            #[test]
            fn the_menu_lists_every_verb_with_its_argv_or_refusal() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "menu-list");
                app.handle_key(KeyCode::Char('!'));
                let rows = app.launcher_rows();
                let verbs: Vec<&str> = rows.iter().map(|(v, _)| v.as_str()).collect();
                assert_eq!(verbs, ["comment", "show", "web", "edit"]);
                assert_eq!(rows[1].1, Ok("tool show REQ-001".to_owned()));
                // REQ-001 carries no `number`.
                assert!(rows[2].1.as_ref().unwrap_err().contains("number"));
            }

            #[test]
            fn enter_runs_the_row_without_asking() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "menu-enter");
                app.handle_key(KeyCode::Char('!'));
                app.handle_key(KeyCode::Char('j'));
                assert_eq!(
                    argv(app.handle_key(KeyCode::Enter)),
                    ["tool", "show", "REQ-001"]
                );
                assert!(app.launcher.is_none());
            }

            #[test]
            fn esc_closes_the_menu_without_running() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                app.handle_key(KeyCode::Char('!'));
                assert!(matches!(app.handle_key(KeyCode::Esc), Action::None));
                assert!(app.launcher.is_none() && app.confirm.is_none());
            }

            #[test]
            fn a_kind_without_actions_gets_a_message_not_a_menu() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                for _ in 0..2 {
                    app.handle_key(KeyCode::Char('j'));
                }
                app.handle_key(KeyCode::Char('!'));
                assert!(app.launcher.is_none());
                assert!(
                    status(&app).contains("no actions for kind spec"),
                    "{}",
                    status(&app)
                );
            }

            #[test]
            fn q_quits_from_the_menu() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                app.handle_key(KeyCode::Char('!'));
                assert!(matches!(app.handle_key(KeyCode::Char('Q')), Action::Quit));
            }
        }

        /// Requirement: browse-owner-actions/Capture mode
        mod capture {
            use super::*;

            #[test]
            fn the_output_pane_scrolls_and_closes() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                let text: String = (1..=200).map(|i| format!("line {i}\n")).collect();
                app.output = Some(OutputPane::new("t".into(), &text, false, "", false));
                let scroll = |app: &App<'_>| app.output.as_ref().unwrap().scroll;
                app.handle_key(KeyCode::Char('j'));
                assert_eq!(scroll(&app), 1);
                app.handle_key(KeyCode::Char('G'));
                assert_eq!(scroll(&app), 199);
                app.handle_key(KeyCode::Char('k'));
                assert_eq!(scroll(&app), 198);
                app.handle_key(KeyCode::Char('g'));
                assert_eq!(scroll(&app), 0);
                app.handle_key(KeyCode::Esc);
                assert!(app.output.is_none());
                assert_eq!(app.node_list_state.cursor, 0, "keys went to the pane");
            }
        }

        /// Requirement: browse-owner-actions/First-use confirmation
        mod confirm {
            use super::*;

            #[test]
            fn a_confirmed_action_runs_directly_on_another_node() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "confirm-direct");
                app.handle_key(KeyCode::Char('c'));
                app.handle_key(KeyCode::Char('y'));
                app.handle_key(KeyCode::Char('j'));
                assert_eq!(
                    argv(app.handle_key(KeyCode::Char('c'))),
                    ["tool", "comment", "REQ-002"]
                );
            }

            #[test]
            fn any_other_key_cancels_and_the_next_press_asks_again() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "confirm-cancel");
                app.handle_key(KeyCode::Char('c'));
                assert!(matches!(app.handle_key(KeyCode::Char('n')), Action::None));
                assert!(app.confirm.is_none());
                app.handle_key(KeyCode::Char('c'));
                assert!(app.confirm.is_some(), "a cancelled action is not confirmed");
            }

            #[test]
            fn a_run_from_the_menu_confirms_the_action() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "confirm-menu");
                app.handle_key(KeyCode::Char('!'));
                assert!(matches!(
                    app.handle_key(KeyCode::Enter),
                    Action::RunOwner(_)
                ));
                assert!(matches!(
                    app.handle_key(KeyCode::Char('c')),
                    Action::RunOwner(_)
                ));
            }

            #[test]
            fn q_quits_from_the_prompt() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "confirm-quit");
                app.handle_key(KeyCode::Char('c'));
                assert!(matches!(app.handle_key(KeyCode::Char('Q')), Action::Quit));
            }

            #[test]
            fn confirmations_survive_a_reload() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "confirm-reload");
                app.handle_key(KeyCode::Char('c'));
                app.handle_key(KeyCode::Char('y'));
                let path = app.path_env.clone();
                let mut app = reloaded(app, &profile, &graph);
                app.path_env = path;
                assert!(matches!(
                    app.handle_key(KeyCode::Char('c')),
                    Action::RunOwner(_)
                ));
            }

            #[test]
            fn a_changed_template_asks_again() {
                let profile = action_profile(COMMENT);
                let graph = ingest_document(test_document()).unwrap();
                let mut app = make_app(&profile, &graph);
                with_tool(&mut app, "confirm-changed");
                app.handle_key(KeyCode::Char('c'));
                app.handle_key(KeyCode::Char('y'));
                let path = app.path_env.clone();
                let edited = action_profile(r#"["tool", "comment", "--", "{id}"]"#);
                let mut app = reloaded(app, &edited, &graph);
                app.path_env = path;
                assert!(matches!(app.handle_key(KeyCode::Char('c')), Action::None));
                let pending = app.confirm.as_ref().expect("an edited template asks again");
                assert_eq!(pending.run.display(), "tool comment -- REQ-001");
            }
        }
    }
}

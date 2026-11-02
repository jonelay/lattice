use std::borrow::Cow;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use latt_core::types::{Endpoint, LayoutNode, Visit};

use ratatui::widgets::Clear;

use super::app::{
    App, Binding, ColumnMode, DetailRow, InputMode, OutputPane, Preview, SchemaRow, TopView, VIEWS,
};
use super::git;

const EDGE_COLORS: &[Color] = &[
    Color::Green,
    Color::Magenta,
    Color::Yellow,
    Color::Cyan,
    Color::Blue,
    Color::Red,
    Color::LightGreen,
    Color::LightMagenta,
];

fn edge_color(edge_kinds: &[&str], kind: &str) -> Color {
    edge_kinds
        .iter()
        .position(|k| *k == kind)
        .map_or(Color::Reset, |i| EDGE_COLORS[i % EDGE_COLORS.len()])
}

/// Secondary text: the terminal's own foreground at reduced intensity, so it
/// stays legible on dark and light themes alike, where a fixed gray vanishes on one.
fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Emphasis in the terminal's own foreground.
fn strong() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// A pane border: bold when the pane has focus, dim when it does not — a
/// weight difference, so focus survives `NO_COLOR` and monochrome terminals.
fn border(focused: bool) -> Style {
    if focused { strong() } else { dim() }
}

/// A pane title over `border`: the border's weight without its dim, so an
/// unfocused pane's title is text at full intensity. Block titles are drawn
/// over the border style, and a patch only adds modifiers, so dim must be
/// removed explicitly.
fn title_style(border: Style) -> Style {
    border.remove_modifier(Modifier::DIM)
}

/// A search match that is not under the cursor: underlined as well as
/// colored, so the match does not rest on color alone.
fn matched() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::UNDERLINED)
}

/// Show the terminal cursor on the selection of a bordered pane: `cell` is the
/// (row, column) counted from the pane's first inner cell, `None` when the
/// pane has nothing selected. Screen readers and magnifiers follow the
/// terminal cursor, not a highlight bar (WCAG2ICT Appendix B), so every frame
/// places it; the position is clamped to the pane and the frame because
/// ratatui does not clamp it.
fn place_cursor(frame: &mut Frame, area: Rect, cell: Option<(usize, usize)>) {
    let frame_area = frame.area();
    if frame_area.is_empty() {
        return;
    }
    let inner = area.inner(Margin::new(1, 1));
    let pane = if inner.is_empty() { area } else { inner };
    let (row, col) = cell.unwrap_or((0, 0));
    let along = |start: u16, offset: usize, len: u16| {
        let offset = u16::try_from(offset).unwrap_or(u16::MAX);
        start.saturating_add(offset.min(len.saturating_sub(1)))
    };
    let x = along(pane.x, col, pane.width).min(frame_area.right() - 1);
    let y = along(pane.y, row, pane.height).min(frame_area.bottom() - 1);
    frame.set_cursor_position((x, y));
}

/// The selected row of a list pane scrolled to `offset`, for [`place_cursor`].
fn selected_row(cursor: usize, len: usize, offset: usize) -> Option<(usize, usize)> {
    (len > 0).then(|| (cursor.saturating_sub(offset), 0))
}

/// The tab and status bars: the terminal's own colors reversed.
fn bar() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Reset every cell to the terminal's default foreground and background,
/// keeping modifiers; how browse honors `NO_COLOR`. Applied to the whole
/// frame, it also reaches styles browse does not choose, such as rendered Markdown.
pub fn strip_color(buffer: &mut Buffer) {
    for cell in &mut buffer.content {
        cell.set_fg(Color::Reset).set_bg(Color::Reset);
    }
}

/// The ASCII stand-in for each glyph browse or its widgets draw, one cell for
/// one cell so the layout is unchanged.
fn ascii_glyph(symbol: &str) -> Option<&'static str> {
    Some(match symbol {
        "─" | "━" | "═" | "—" => "-",
        "│" | "┃" | "║" => "|",
        "┌" | "┐" | "└" | "┘" | "╭" | "╮" | "╰" | "╯" | "├" | "┤" | "┬" | "┴" | "┼" => {
            "+"
        }
        "▲" | "↑" => "^",
        "▼" | "↓" => "v",
        "█" | "■" => "#",
        "→" | "▶" | "⟩" => ">",
        "←" | "⟨" => "<",
        "●" | "◆" | "•" | "▪" => "*",
        "◌" | "◦" => "o",
        "⊘" => "x",
        "…" => ".",
        "╱" => "/",
        "╲" => "\\",
        "↝" => "~",
        "↻" => "@",
        _ => return None,
    })
}

/// Map every cell through the glyph table to its ASCII stand-in, for a
/// terminal whose locale is not UTF-8. The pass cannot tell browse's glyphs
/// from register text: a `→` in a summary becomes `>` too, and characters
/// outside the table are left as they are.
pub fn ascii_glyphs(buffer: &mut Buffer) {
    for cell in &mut buffer.content {
        if let Some(ascii) = ascii_glyph(cell.symbol()) {
            cell.set_symbol(ascii);
        }
    }
}

pub fn draw(frame: &mut Frame, app: &App<'_>) {
    let edge_kinds = app.edge_kind_names();

    let mut constraints = vec![Constraint::Length(1), Constraint::Min(3)];
    let tree_detail = app.top_view == TopView::Tree && !app.view_stack.is_empty();
    let provenance_detail = app.top_view == TopView::Provenance && !app.view_stack.is_empty();
    let graph_detail = app.top_view == TopView::Graph && !app.view_stack.is_empty();
    if (app.show_detail && app.top_view == TopView::NodeList)
        || tree_detail
        || provenance_detail
        || graph_detail
    {
        constraints.insert(2, Constraint::Percentage(50));
        constraints[1] = Constraint::Percentage(50);
    }
    constraints.push(Constraint::Length(1));
    let chunks = Layout::vertical(constraints).split(frame.area());

    draw_tab_bar(frame, app, chunks[0]);
    let mut chunk_idx = 1;

    if app.input_mode == InputMode::KindFilter {
        draw_kind_filter(frame, app, chunks[chunk_idx]);
    } else if app.input_mode == InputMode::EdgeKindFilter {
        draw_edge_kind_filter(frame, app, chunks[chunk_idx]);
    } else {
        match app.top_view {
            TopView::NodeList if app.show_schema => {
                draw_schema_view(frame, app, chunks[chunk_idx]);
            }
            TopView::NodeList => draw_node_list(frame, app, chunks[chunk_idx]),
            TopView::Findings => draw_findings(frame, app, chunks[chunk_idx]),
            TopView::Tree => draw_tree(frame, app, chunks[chunk_idx], &edge_kinds),
            TopView::Provenance => draw_provenance(frame, app, chunks[chunk_idx], &edge_kinds),
            TopView::Graph => draw_graph(frame, app, chunks[chunk_idx], &edge_kinds),
        }
    }
    chunk_idx += 1;

    let show_tree_detail = app.top_view == TopView::Tree && !app.view_stack.is_empty();
    let show_provenance_detail = app.top_view == TopView::Provenance && !app.view_stack.is_empty();
    let show_graph_detail = app.top_view == TopView::Graph && !app.view_stack.is_empty();
    if (app.show_detail && app.top_view == TopView::NodeList)
        || show_tree_detail
        || show_provenance_detail
        || show_graph_detail
    {
        draw_detail(frame, app, chunks[chunk_idx], &edge_kinds);
        chunk_idx += 1;
    }

    let show_hint = app.input_mode == InputMode::Normal
        && app.last_key_instant.elapsed() >= std::time::Duration::from_secs(1)
        && app.preview.is_none()
        && app.launcher.is_none()
        && app.confirm.is_none()
        && app.output.is_none();
    if show_hint {
        let hint_area = Rect {
            x: chunks[chunk_idx].x,
            y: chunks[chunk_idx].y.saturating_sub(1),
            width: chunks[chunk_idx].width,
            height: 1,
        };
        if hint_area.y > 0 {
            let hint = Paragraph::new(Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(app.view_hints(), Style::default()),
            ]));
            frame.render_widget(hint, hint_area);
        }
    }

    draw_status_line(frame, app, chunks[chunk_idx]);

    if let Some(ref preview) = app.preview {
        draw_preview(frame, preview, frame.area());
    }
    if let Some(ref pane) = app.output {
        draw_output(frame, pane, frame.area());
    }
    if app.launcher.is_some() {
        draw_launcher(frame, app, frame.area());
    }
    if let Some(ref confirm) = app.confirm {
        draw_confirm(frame, &confirm.run.display(), frame.area());
    }

    if app.input_mode == InputMode::Help {
        draw_help_overlay(frame, app, &edge_kinds, frame.area());
    }
}

fn draw_tab_bar(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let active = app.top_view.tab_index();
    let mut spans = Vec::new();
    for (i, &(_, key, name)) in VIEWS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let label = format!(" {key} {name} ");
        if i == active {
            spans.push(Span::styled(
                label,
                Style::default()
                    .remove_modifier(Modifier::REVERSED)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
        } else {
            spans.push(Span::raw(label));
        }
    }
    let tab_line = Paragraph::new(Line::from(spans)).style(bar());
    frame.render_widget(tab_line, area);
}

fn scroll_offset(cursor: usize, total: usize, viewport: usize) -> usize {
    if total <= viewport || cursor < viewport / 2 {
        0
    } else if cursor + viewport / 2 >= total {
        total.saturating_sub(viewport)
    } else {
        cursor.saturating_sub(viewport / 2)
    }
}

fn draw_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize, style: Style) {
    let viewport = area.height.saturating_sub(2) as usize;
    if total <= viewport {
        return;
    }
    let mut state = ScrollbarState::new(total - viewport + 1)
        .position(offset)
        .viewport_content_length(viewport);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).style(style),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

fn draw_node_list(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let nodes = app.visible_nodes();
    let cursor = app
        .current_state()
        .cursor
        .min(nodes.len().saturating_sub(1));
    let search = app.current_state().search.as_ref();
    let viewport = area.height.saturating_sub(2) as usize;
    let offset = scroll_offset(cursor, nodes.len(), viewport);

    let visible: Vec<ListItem> = nodes
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .map(|(i, (id, kind, summary))| {
            let summary_text = summary.as_deref().unwrap_or("");
            let is_match = search.is_some_and(|re| {
                re.is_match(id) || (!summary_text.is_empty() && re.is_match(summary_text))
            });
            let mut spans = vec![
                Span::styled(format!("[{kind}] "), Style::default().fg(Color::Cyan)),
                Span::raw(*id),
            ];
            // An ID that ends with its summary (a title used as the ID) already shows it.
            if !summary_text.is_empty() && !id.ends_with(summary_text) {
                spans.push(Span::styled(format!(" — {summary_text}"), Style::default()));
            }
            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if i == cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            } else if is_match {
                item = item.style(matched());
            }
            item
        })
        .collect();

    let focused = !app.detail_focused;
    let border_style = border(focused);
    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_style(title_style(border_style))
            .title(" Nodes "),
    );
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, nodes.len(), offset, border_style);
    if focused {
        place_cursor(frame, area, selected_row(cursor, nodes.len(), offset));
    }
}

fn draw_schema_view(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let rows = app.schema_view_rows();
    let cursor = app.schema_cursor.min(rows.len().saturating_sub(1));
    let viewport = area.height.saturating_sub(2) as usize;
    let offset = scroll_offset(cursor, rows.len(), viewport);
    let visible: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .map(|(i, row)| {
            let item = schema_row_item(row);
            if i == cursor {
                item.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect();
    let border_style = border(true);
    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_style(title_style(border_style))
            .title(" Schema "),
    );
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, rows.len(), offset, border_style);
    place_cursor(frame, area, selected_row(cursor, rows.len(), offset));
}

fn schema_row_item(row: &SchemaRow) -> ListItem<'_> {
    match row {
        SchemaRow::Kind { name, count } => {
            let style = Style::default();
            ListItem::new(Line::from(vec![
                Span::styled(name.as_str(), Style::default().fg(Color::Cyan)),
                Span::styled(format!("  ({count})"), style),
            ]))
        }
        SchemaRow::Triple { triple, count } => {
            let style = if *count == 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![Span::styled(
                format!(
                    "  {} —[{}]→ {}  ({count})",
                    triple.src_kind, triple.edge_kind, triple.tgt_kind
                ),
                style,
            )]))
        }
        SchemaRow::Separator => ListItem::new(Line::from("  ───")),
    }
}

fn draw_findings(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let findings = app.visible_findings();
    let search = app.current_state().search.as_ref();
    let cursor = app
        .current_state()
        .cursor
        .min(findings.len().saturating_sub(1));

    let mut all_items: Vec<ListItem> = Vec::new();
    let mut saw_unattached = false;
    let mut cursor_item_idx = 0;

    for (i, issue) in findings.iter().enumerate() {
        let is_attached = issue
            .node_id
            .as_ref()
            .is_some_and(|id| app.graph.has_node(id));
        if !is_attached && !saw_unattached {
            saw_unattached = true;
            all_items.push(ListItem::new(Line::from(Span::styled(
                "── Unattached ──",
                strong(),
            ))));
        }
        if i == cursor {
            cursor_item_idx = all_items.len();
        }
        let is_match = search.is_some_and(|re| {
            re.is_match(&issue.code)
                || re.is_match(&issue.message)
                || issue.node_id.as_ref().is_some_and(|id| re.is_match(id))
        });
        all_items.push(finding_item(issue, i == cursor, is_match));
        if let Some(note) = &issue.note {
            all_items.push(ListItem::new(note_line("        ", note)));
        }
    }

    let viewport = area.height.saturating_sub(2) as usize;
    let total = all_items.len();
    let offset = scroll_offset(cursor_item_idx, total, viewport);
    let visible: Vec<ListItem> = all_items.into_iter().skip(offset).take(viewport).collect();

    let list = List::new(visible).block(Block::default().borders(Borders::ALL).title(" Findings "));
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, total, offset, Style::default());
    // Findings draws no detail pane, so it keeps the cursor whatever the focus.
    place_cursor(frame, area, selected_row(cursor_item_idx, total, offset));
}

fn finding_item(issue: &latt_core::types::Issue, selected: bool, is_match: bool) -> ListItem<'_> {
    let sev_style = severity_style(issue.severity);
    let node_text = issue
        .node_id
        .as_deref()
        .map(|id| format!(" [{id}]"))
        .unwrap_or_default();
    let suppressed = if issue.suppressed {
        " (suppressed)"
    } else {
        ""
    };
    let line = Line::from(vec![
        Span::styled(format!("{:7}", issue.severity.as_upper()), sev_style),
        Span::styled(format!(" {}", issue.code), strong()),
        Span::styled(node_text, Style::default().fg(Color::Cyan)),
        Span::raw(format!(" {}{suppressed}", issue.message)),
    ]);
    let mut item = ListItem::new(line);
    if selected {
        item = item.style(Style::default().add_modifier(Modifier::REVERSED));
    } else if is_match {
        item = item.style(matched());
    }
    item
}

/// A finding's note, indented under it: `note: `, `(expired) ` when it has
/// expired, the text, then `by` and `until` in brackets.
fn note_line(indent: &str, note: &latt_core::types::Note) -> Line<'static> {
    let mut text = format!("{indent}note: ");
    if note.expired {
        text.push_str("(expired) ");
    }
    text.push_str(&note.text);
    let mut qualifiers: Vec<String> = note.by.iter().cloned().collect();
    if let Some(until) = &note.until {
        qualifiers.push(match until {
            latt_core::types::Until::Date(date) => format!("until {date}"),
            latt_core::types::Until::Pathway { pathway, position } => {
                format!("until {pathway} {position}")
            }
            _ => "until …".to_owned(),
        });
    }
    if !qualifiers.is_empty() {
        text.push_str(&format!(" [{}]", qualifiers.join(", ")));
    }
    let style = if note.expired {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    Line::from(Span::styled(text, style))
}

fn severity_style(severity: latt_core::types::Severity) -> Style {
    match severity {
        latt_core::types::Severity::Error => Style::default().fg(Color::Red),
        latt_core::types::Severity::Warning => Style::default().fg(Color::Yellow),
        latt_core::types::Severity::Info => Style::default().fg(Color::Blue),
        latt_core::types::Severity::Hint => Style::default(),
    }
}

fn draw_detail(frame: &mut Frame, app: &App<'_>, area: Rect, edge_kinds: &[&str]) {
    let rows = app.detail_rows();
    let cursor = app.detail_cursor.min(rows.len().saturating_sub(1));

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let line = match row {
                DetailRow::Header(text) => {
                    Line::from(Span::styled(format!("── {text} ──"), strong()))
                }
                DetailRow::Attr(key, val) => Line::from(vec![
                    Span::styled(format!("  {key}: "), Style::default().fg(Color::Cyan)),
                    Span::raw(val.as_ref()),
                ]),
                DetailRow::OutEdge { kind, tgt, .. } => {
                    let color = edge_color(edge_kinds, kind);
                    Line::from(vec![
                        Span::styled(format!("  —[{kind}]→ "), Style::default().fg(color)),
                        Span::raw(*tgt),
                    ])
                }
                DetailRow::InEdge { kind, src, .. } => {
                    let color = edge_color(edge_kinds, kind);
                    Line::from(vec![
                        Span::styled(format!("  ←[{kind}]— "), Style::default().fg(color)),
                        Span::raw(*src),
                    ])
                }
                DetailRow::MissingEdge { triple, incoming } => Line::from(Span::styled(
                    if *incoming {
                        format!(
                            "  ⊘ no incoming ←[{}]— {}",
                            triple.edge_kind, triple.src_kind
                        )
                    } else {
                        format!(
                            "  ⊘ no outgoing —[{}]→ {}",
                            triple.edge_kind, triple.tgt_kind
                        )
                    },
                    Style::default(),
                )),
                DetailRow::Pathway {
                    name,
                    order,
                    current,
                    node_position,
                } => {
                    let positions: Vec<Span> = order
                        .iter()
                        .map(|pos| {
                            let marker = if pos == node_position { "▶" } else { " " };
                            let style = if pos == current {
                                Style::default()
                                    .fg(Color::Green)
                                    .add_modifier(Modifier::BOLD)
                            } else if pos == node_position {
                                Style::default().fg(Color::Yellow)
                            } else {
                                Style::default()
                            };
                            Span::styled(format!("{marker}{pos} "), style)
                        })
                        .collect();
                    let mut spans = vec![Span::styled(
                        format!("  {name}: "),
                        Style::default().fg(Color::Cyan),
                    )];
                    spans.extend(positions);
                    Line::from(spans)
                }
                DetailRow::Finding {
                    severity,
                    code,
                    message,
                    ..
                } => {
                    let sev_style = severity_style(*severity);
                    Line::from(vec![
                        Span::styled(format!("  {:7}", severity.as_upper()), sev_style),
                        Span::styled(format!(" {code}"), strong()),
                        Span::raw(format!(" {message}")),
                    ])
                }
                DetailRow::Note(note) => note_line("          ", note),
            };
            let mut item = ListItem::new(line);
            if app.detail_focused && i == cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();

    let focused = app.detail_focused;
    let border_style = border(focused);
    let viewport = area.height.saturating_sub(2) as usize;
    let offset = scroll_offset(cursor, items.len(), viewport);
    let visible: Vec<ListItem> = items.into_iter().skip(offset).take(viewport).collect();

    let title = app
        .current_detail_id()
        .map(|id| format!(" {id} "))
        .unwrap_or_else(|| " Detail ".to_string());
    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_style(title_style(border_style))
            .title(title),
    );
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, rows.len(), offset, border_style);
    if focused {
        place_cursor(frame, area, selected_row(cursor, rows.len(), offset));
    }
}

fn draw_kind_filter(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let (filter_cursor, filter) = app.kind_filter_parts();
    let kinds: Vec<(&str, bool)> = app
        .profile
        .node_kinds()
        .keys()
        .zip(filter.iter())
        .map(|(name, enabled)| (name.as_str(), *enabled))
        .collect();
    let cursor = filter_cursor.min(kinds.len().saturating_sub(1));

    let items: Vec<ListItem> = kinds
        .iter()
        .enumerate()
        .map(|(i, (name, enabled))| {
            let check = if *enabled { "[x]" } else { "[ ]" };
            let line = Line::from(vec![
                Span::raw(format!("  {check} ")),
                Span::styled(*name, Style::default().fg(Color::Cyan)),
            ]);
            let mut item = ListItem::new(line);
            if i == cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Kind Filter (Space to toggle, Enter/Esc to close) "),
    );
    frame.render_widget(list, area);
    place_cursor(frame, area, selected_row(cursor, kinds.len(), 0));
}

fn draw_tree(frame: &mut Frame, app: &App<'_>, area: Rect, edge_kinds: &[&str]) {
    let rows = app.visible_tree_rows();
    let cursor = app.tree_state.cursor.min(rows.len().saturating_sub(1));
    let search = app.tree_state.search.as_ref();

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let indent = "  ".repeat(row.depth);
            let is_match = search.is_some_and(|re| {
                re.is_match(row.node_id) || row.summary.as_ref().is_some_and(|s| re.is_match(s))
            });
            let expanded = app.tree_state.expanded.contains(&row.step_index);
            let is_cluster = row.cluster.is_some();
            let marker = if is_cluster {
                if expanded { "▼ ⟨" } else { "▶ ⟨" }
            } else if row.visit == Visit::Cycle {
                "↻ "
            } else if row.visit == Visit::Reconvergence {
                "↝ "
            } else if expanded {
                "▼ "
            } else {
                "▶ "
            };

            let mut spans = vec![Span::raw(format!("{indent}{marker}"))];

            if let Some(ek) = row.edge_kind {
                let color = edge_color(edge_kinds, ek);
                spans.push(Span::styled(format!("[{ek}] "), Style::default().fg(color)));
            }

            let node_style = match row.endpoint {
                Endpoint::Vacancy => Style::default(),
                Endpoint::Declared if row.tainted => Style::default().fg(Color::Yellow),
                Endpoint::Declared => Style::default(),
            };
            spans.push(Span::styled(row.node_id, node_style));

            if row.endpoint == Endpoint::Vacancy {
                spans.push(Span::styled(" ⊘", Style::default()));
            }
            if row.tainted {
                spans.push(Span::styled(
                    " ~tainted",
                    Style::default().fg(Color::Yellow),
                ));
            }

            if let Some(ref s) = row.summary
                && !row.node_id.ends_with(s.as_ref() as &str)
            {
                spans.push(Span::styled(format!(" — {s}"), Style::default()));
            }

            if let Some((sev, count)) = row.severity
                && count > 0
            {
                let sev_style = severity_style(sev);
                spans.push(Span::styled(format!(" [{count}]"), sev_style));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if i == cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            } else if is_match {
                item = item.style(matched());
            }
            item
        })
        .collect();

    let viewport = area.height.saturating_sub(2) as usize;
    let offset = scroll_offset(cursor, items.len(), viewport);
    let visible: Vec<ListItem> = items.into_iter().skip(offset).take(viewport).collect();

    let focused = app.view_stack.is_empty();
    let border_style = border(focused);
    let root = app.tree_state.root.as_deref().unwrap_or("(none)");
    let title = format!(" Tree: {root} ");
    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_style(title_style(border_style))
            .title(title),
    );
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, rows.len(), offset, border_style);
    if !app.detail_focused {
        place_cursor(frame, area, selected_row(cursor, rows.len(), offset));
    }
}

fn draw_provenance(frame: &mut Frame, app: &App<'_>, area: Rect, _edge_kinds: &[&str]) {
    let Some(ref layout) = app.provenance_layout else {
        let empty = Paragraph::new(" No lane layout computed")
            .block(Block::default().borders(Borders::ALL).title(" Provenance "));
        frame.render_widget(empty, area);
        place_cursor(frame, area, None);
        return;
    };

    let pending = app.git_fetch_pending();
    let (lane_area, commit_area) =
        if app.git_commits.is_empty() && !pending && !app.git_fetch_timed_out {
            (area, None)
        } else {
            let commit_height = (app.git_commits.len().max(1) as u16 + 2)
                .min(8)
                .min(area.height / 3);
            let chunks = Layout::vertical([Constraint::Min(4), Constraint::Length(commit_height)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        };
    let area = lane_area;

    let visible_indices = app.visible_provenance_indices();
    let cursor = app
        .provenance_state
        .cursor
        .min(visible_indices.len().saturating_sub(1));
    let search = app.provenance_state.search.as_ref();
    let h_scroll = app.provenance_state.h_scroll;

    let items: Vec<ListItem> = visible_indices
        .iter()
        .enumerate()
        .map(|(i, &row_idx)| {
            let row = &layout.rows[row_idx];
            let is_match = search.is_some_and(|re| re.is_match(&row.id));

            let marker = if row.cluster { '◆' } else { '●' };
            let chars = lane_chars(row.lane, &row.parent_lanes, &[], layout.peak_lanes, marker);
            let mut spans = lane_spans(chars.into_iter().skip(h_scroll));
            spans.push(Span::raw(" "));

            // Node info; an undeclared node is marked by `◌`, not by style.
            if row.cluster {
                let member_str = row.members.join(", ");
                spans.push(Span::styled(
                    format!("[{} nodes: {}]", row.members.len(), member_str),
                    Style::default().fg(Color::Cyan),
                ));
            } else {
                spans.push(Span::raw(row.id.as_str()));
            }

            if !app.graph.has_node(&row.id) && !row.cluster {
                spans.push(Span::styled(" ◌", Style::default()));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if i == cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            } else if is_match {
                item = item.style(matched());
            }
            item
        })
        .collect();

    let viewport = area.height.saturating_sub(2) as usize;
    let offset = scroll_offset(cursor, items.len(), viewport);
    let visible: Vec<ListItem> = items.into_iter().skip(offset).take(viewport).collect();

    let focused = app.view_stack.is_empty();
    let border_style = border(focused);
    let root = app.provenance_state.root.as_deref().unwrap_or("(none)");
    let dir = match app.provenance_state.direction {
        latt_core::query::Direction::Forward => "→",
        latt_core::query::Direction::Reverse => "←",
    };
    let visible_count = visible_indices.len();
    let total_count = layout.rows.len();
    let count_label = if visible_count < total_count {
        format!("{visible_count}/{total_count} nodes")
    } else {
        format!("{total_count} nodes")
    };
    let title = format!(
        " Lane: {root} {dir} ({count_label}, {} lanes) ",
        layout.peak_lanes
    );
    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_style(title_style(border_style))
            .title(title),
    );
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, visible_indices.len(), offset, border_style);
    if !app.detail_focused {
        place_cursor(
            frame,
            area,
            selected_row(cursor, visible_indices.len(), offset),
        );
    }

    if let Some(commit_area) = commit_area {
        let lanes = &app.git_lanes;
        let width = lanes.iter().map(git::CommitRow::width).max().unwrap_or(1);
        let items: Vec<ListItem> = app
            .git_commits
            .iter()
            .zip(lanes)
            .map(|(c, row)| {
                let hash = if c.hash.len() >= 8 {
                    &c.hash[..8]
                } else {
                    &c.hash
                };
                let date = c.date.split(' ').next().unwrap_or(&c.date);
                let chars = lane_chars(row.lane, &row.linked_lanes, &row.open_lanes, width, '●');
                let mut spans = lane_spans(chars.into_iter());
                spans.extend([
                    Span::styled(format!(" {hash} "), Style::default().fg(Color::Yellow)),
                    Span::styled(format!("{date} "), Style::default()),
                    Span::styled(format!("{} ", c.author), Style::default().fg(Color::Cyan)),
                    Span::raw(&c.subject),
                ]);
                ListItem::new(Line::from(spans))
            })
            .collect();
        let title = if pending {
            " Git (fetching…) ".to_string()
        } else if app.git_fetch_timed_out {
            " Git (timed out) ".to_string()
        } else {
            format!(" Git ({}) ", app.git_commits.len())
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(dim())
            .title_style(title_style(dim()))
            .title(title);
        frame.render_widget(List::new(items).block(block), commit_area);
    }
}

/// Box-drawing cells for one lane row: `linked` lanes join this row's node
/// with a horizontal run, `open` lanes pass straight through. The flag marks
/// the row's own node marker, drawn at full intensity; every other cell is dim.
fn lane_chars(
    lane: usize,
    linked: &[usize],
    open: &[usize],
    width: usize,
    marker: char,
) -> Vec<(char, bool)> {
    let mut chars = vec![(' ', false); width];
    for &l in open.iter().chain(linked) {
        if l < width {
            chars[l] = ('│', false);
        }
    }
    if lane >= width {
        return chars;
    }
    chars[lane] = (marker, true);
    for &l in linked {
        if l >= width || l == lane {
            continue;
        }
        let (lo, hi) = if l < lane {
            (l + 1, lane)
        } else {
            (lane + 1, l)
        };
        for entry in &mut chars[lo..hi] {
            match entry.0 {
                ' ' => *entry = ('─', false),
                '│' => *entry = ('┼', false),
                _ => {}
            }
        }
        if chars[l].0 == '│' {
            chars[l] = (if l < lane { '├' } else { '┤' }, false);
        }
    }
    chars
}

fn lane_spans(chars: impl Iterator<Item = (char, bool)>) -> Vec<Span<'static>> {
    chars
        .map(|(ch, marker)| {
            let style = if marker { Style::default() } else { dim() };
            Span::styled(lane_glyph(ch), style)
        })
        .collect()
}

/// The lane alphabet as static text, so a lane cell allocates nothing; any
/// other character is owned.
fn lane_glyph(ch: char) -> Cow<'static, str> {
    Cow::Borrowed(match ch {
        ' ' => " ",
        '│' => "│",
        '─' => "─",
        '┼' => "┼",
        '├' => "├",
        '┤' => "┤",
        '●' => "●",
        '◆' => "◆",
        _ => return Cow::Owned(ch.to_string()),
    })
}

/// One graph node line under construction, with its width in cells kept as
/// spans are added instead of re-counted from every span per placement.
#[derive(Default)]
struct NodeLine<'a> {
    spans: Vec<Span<'a>>,
    width: usize,
}

impl<'a> NodeLine<'a> {
    fn push(&mut self, span: Span<'a>) {
        self.width += span.content.chars().count();
        self.spans.push(span);
    }
}

/// The most cells a neighbour's label takes in the graph view; the center is shown
/// whole. A register whose IDs are paths would otherwise lay one layer out wider
/// than any terminal.
const GRAPH_LABEL_CELLS: usize = 32;

/// Return `id` as the graph view labels it: whole when it fits, else `…` and its
/// tail, where a path-shaped ID keeps the part that tells siblings apart.
fn graph_label(id: &str) -> Cow<'_, str> {
    let len = id.chars().count();
    if len <= GRAPH_LABEL_CELLS {
        return Cow::Borrowed(id);
    }
    let tail: String = id.chars().skip(len - (GRAPH_LABEL_CELLS - 1)).collect();
    Cow::Owned(format!("…{tail}"))
}

/// Return how many cells to scroll the graph right so the selected node, starting
/// at cell `selected`, sits mid-view; 0 when every line fits.
fn graph_x_offset(lines: &[Line<'_>], selected: Option<usize>, width: u16) -> usize {
    // Two borders and the scrollbar column.
    let inner = usize::from(width).saturating_sub(3);
    let widest = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    if widest <= inner {
        return 0;
    }
    let want = (selected.unwrap_or(0) + GRAPH_LABEL_CELLS / 2).saturating_sub(inner / 2);
    want.min(widest - inner)
}

/// Drop the first `skip` cells of `line`, keeping each span's style.
fn crop_line(line: Line<'_>, skip: usize) -> Line<'_> {
    if skip == 0 {
        return line;
    }
    let mut left = skip;
    let mut spans = Vec::new();
    for span in line.spans {
        let n = span.content.chars().count();
        if left >= n {
            left -= n;
            continue;
        }
        if left > 0 {
            let rest: String = span.content.chars().skip(left).collect();
            spans.push(Span::styled(rest, span.style));
            left = 0;
        } else {
            spans.push(span);
        }
    }
    Line::from(spans)
}

fn draw_graph(frame: &mut Frame, app: &App<'_>, area: Rect, edge_kinds: &[&str]) {
    let has_breadcrumb = !app.graph_state.center_history.is_empty();
    let (breadcrumb_area, graph_area) = if has_breadcrumb {
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    if let Some(bc_area) = breadcrumb_area {
        let width = bc_area.width as usize;
        let current = app.graph_state.center.as_deref().unwrap_or("?");
        let mut parts: Vec<&str> = app
            .graph_state
            .center_history
            .iter()
            .map(String::as_str)
            .collect();
        parts.push(current);
        let mut text = parts.join(" > ");
        if text.len() > width {
            while parts.len() > 2 && text.len() > width {
                parts.remove(0);
                text = format!("… > {}", parts.join(" > "));
            }
        }
        let breadcrumb = Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(text, Style::default()),
        ]));
        frame.render_widget(breadcrumb, bc_area);
    }

    let area = graph_area;
    let rows = app.visible_graph_rows();
    let cursor = app.graph_state.cursor.min(rows.len().saturating_sub(1));
    let search = app.graph_state.search.as_ref();
    let centrality_threshold = 0.5;

    // Group rows by layer for grid rendering.
    let mut layer_groups: Vec<Vec<(usize, &super::app::MapRowRef<'_>)>> = Vec::new();
    for (row_idx, row) in rows.iter().enumerate() {
        while layer_groups.len() <= row.layer {
            layer_groups.push(Vec::new());
        }
        layer_groups[row.layer].push((row_idx, row));
    }

    let col_mode = app.graph_state.column_mode;

    // Compute per-column display widths from actual rendered content.
    let node_cell_width = |row: &super::app::MapRowRef<'_>| -> usize {
        let mut w = 0usize;
        if row.is_center {
            w += 1 + row.id.chars().count(); // ● + id, shown whole
        } else {
            if col_mode != ColumnMode::Compact
                && let Some(ek) = row.edge_kind
            {
                w += ek.len() + 2; // [ek]
            }
            if col_mode == ColumnMode::Full {
                w += row.kind.len() + 3; // (kind)_
            }
            w += graph_label(row.id).chars().count();
        }
        if row.endpoint == Endpoint::Vacancy {
            w += 2; // _◌ (1 space + 1 display cell)
        }
        if row.cluster {
            w += 2; // _◆
        }
        w
    };

    // Include virtual nodes from the layout in col_widths computation.
    let layout = app.graph_layout.as_ref();
    let max_x = if let Some(lo) = layout {
        lo.x_positions
            .iter()
            .flat_map(|row| row.iter())
            .copied()
            .max()
            .unwrap_or(0)
            + 1
    } else {
        rows.iter().map(|r| r.col + 1).max().unwrap_or(0)
    };
    let mut col_widths: Vec<usize> = vec![3; max_x];
    for row in &rows {
        let w = node_cell_width(row) + 2; // 2 cells of padding
        if row.col < col_widths.len() && w > col_widths[row.col] {
            col_widths[row.col] = w;
        }
    }

    // Compute the character position for a given x-column index.
    let col_char_pos = |col: usize| -> usize { col_widths.iter().take(col).sum::<usize>() + 1 };

    // Build items: interleave node-layer lines with connector lines.
    let mut items: Vec<Line> = Vec::new();
    // Map from display-item index to the layer index (for cursor tracking).
    let mut item_layer: Vec<usize> = Vec::new();
    // The selected node's display item and first cell, for the terminal cursor.
    let mut selected_at: Option<(usize, usize)> = None;

    let num_layers = layout
        .as_ref()
        .map_or(layer_groups.len(), |lo| lo.layers.len());

    // Index rows by (layer, x-col) for single-pass rendering with layout nodes.
    let mut row_by_pos: std::collections::HashMap<
        (usize, usize),
        (usize, &super::app::MapRowRef<'_>),
    > = std::collections::HashMap::new();
    for (row_idx, row) in rows.iter().enumerate() {
        row_by_pos.insert((row.layer, row.col), (row_idx, row));
    }

    // Visible components: only Real nodes that survived kind/search filtering.
    let mut visible_comp: std::collections::HashSet<usize> = std::collections::HashSet::new();
    if let Some(lo) = layout {
        for (li, layer) in lo.layers.iter().enumerate() {
            for (idx, node) in layer.iter().enumerate() {
                if let LayoutNode::Real(ci) = node {
                    let x = lo.x_positions[li][idx];
                    if row_by_pos.contains_key(&(li, x)) {
                        visible_comp.insert(*ci);
                    }
                }
            }
        }
    }

    // Precompute edge set for O(1) connectivity lookup in connector rendering.
    let edge_set: std::collections::HashSet<(usize, usize)> = layout
        .map(|lo| {
            lo.edges
                .iter()
                .map(|e| (e.src_component, e.tgt_component))
                .collect()
        })
        .unwrap_or_default();

    for li in 0..num_layers {
        // -- Node line for this layer: single pass in index (x) order --
        let mut line = NodeLine::default();
        line.push(Span::raw(" "));

        if let Some(lo) = layout {
            // Walk layout nodes in index order (which is x order).
            for (idx, node) in lo.layers[li].iter().enumerate() {
                let x = lo.x_positions[li][idx];
                match node {
                    LayoutNode::Virtual(ei) => {
                        if !visible_comp.contains(&lo.edges[*ei].src_component)
                            || !visible_comp.contains(&lo.edges[*ei].tgt_component)
                        {
                            continue;
                        }
                        let target = col_char_pos(x);
                        let current = line.width;
                        if target > current {
                            line.push(Span::raw(" ".repeat(target - current)));
                        }
                        line.push(Span::styled("│", dim()));
                    }
                    LayoutNode::Real(_) => {
                        // Find the matching visible row at this position.
                        if let Some(&(gi, row)) = row_by_pos.get(&(li, x)) {
                            let col_start: usize = col_widths.iter().take(row.col).sum();
                            let current_cells = line.width;
                            if col_start + 1 > current_cells {
                                line.push(Span::raw(" ".repeat(col_start + 1 - current_cells)));
                            }

                            let is_match = search.is_some_and(|re| re.is_match(row.id));
                            if gi == cursor {
                                selected_at = Some((items.len(), line.width));
                            }
                            let centrality =
                                app.centrality_cache.get(row.id).copied().unwrap_or(0.0);
                            let mut node_style = Style::default();
                            if centrality >= centrality_threshold {
                                node_style = node_style.add_modifier(Modifier::BOLD);
                            }
                            if row.endpoint == Endpoint::Vacancy {
                                node_style = Style::default();
                            }
                            if gi == cursor {
                                node_style = node_style.add_modifier(Modifier::REVERSED);
                            } else if is_match {
                                node_style = node_style.patch(matched());
                            }

                            if row.is_center {
                                line.push(Span::styled("●", node_style));
                                line.push(Span::styled(row.id, node_style));
                            } else {
                                if col_mode != ColumnMode::Compact
                                    && let Some(ek) = row.edge_kind
                                {
                                    let ek_color = edge_color(edge_kinds, ek);
                                    line.push(Span::styled(
                                        format!("[{ek}]"),
                                        Style::default().fg(ek_color),
                                    ));
                                }
                                if col_mode == ColumnMode::Full {
                                    line.push(Span::styled(
                                        format!("({}) ", row.kind),
                                        Style::default().fg(Color::Cyan),
                                    ));
                                }
                                line.push(Span::styled(graph_label(row.id), node_style));
                            }

                            if row.endpoint == Endpoint::Vacancy {
                                line.push(Span::styled(" ◌", Style::default()));
                            }
                            if row.cluster {
                                line.push(Span::styled(" ◆", Style::default().fg(Color::Cyan)));
                            }
                        }
                        // else: filtered out — skip
                    }
                }
            }
        } else if li < layer_groups.len() {
            // Fallback: no layout, render from layer_groups directly.
            for &(gi, row) in &layer_groups[li] {
                let col_start: usize = col_widths.iter().take(row.col).sum();
                let current_cells = line.width;
                if col_start + 1 > current_cells {
                    line.push(Span::raw(" ".repeat(col_start + 1 - current_cells)));
                }

                let is_match = search.is_some_and(|re| re.is_match(row.id));
                if gi == cursor {
                    selected_at = Some((items.len(), line.width));
                }
                let centrality = app.centrality_cache.get(row.id).copied().unwrap_or(0.0);
                let mut node_style = Style::default();
                if centrality >= centrality_threshold {
                    node_style = node_style.add_modifier(Modifier::BOLD);
                }
                if row.endpoint == Endpoint::Vacancy {
                    node_style = Style::default();
                }
                if gi == cursor {
                    node_style = node_style.add_modifier(Modifier::REVERSED);
                } else if is_match {
                    node_style = node_style.patch(matched());
                }

                if row.is_center {
                    line.push(Span::styled("●", node_style));
                    line.push(Span::styled(row.id, node_style));
                } else {
                    if col_mode != ColumnMode::Compact
                        && let Some(ek) = row.edge_kind
                    {
                        let ek_color = edge_color(edge_kinds, ek);
                        line.push(Span::styled(
                            format!("[{ek}]"),
                            Style::default().fg(ek_color),
                        ));
                    }
                    if col_mode == ColumnMode::Full {
                        line.push(Span::styled(
                            format!("({}) ", row.kind),
                            Style::default().fg(Color::Cyan),
                        ));
                    }
                    line.push(Span::styled(graph_label(row.id), node_style));
                }

                if row.endpoint == Endpoint::Vacancy {
                    line.push(Span::styled(" ◌", Style::default()));
                }
                if row.cluster {
                    line.push(Span::styled(" ◆", Style::default().fg(Color::Cyan)));
                }
            }
        }

        items.push(Line::from(line.spans));
        item_layer.push(li);

        // -- Connector line between this layer and the next --
        if let Some(lo) = layout
            && li + 1 < lo.layers.len()
        {
            let total_width: usize = col_widths.iter().sum::<usize>() + 2;
            let mut buf = vec![' '; total_width];

            // Character positions of the next layer's nodes, once per layer
            // pair rather than once per (node, next node) pair.
            let tgt_xs: Vec<usize> = lo.x_positions[li + 1]
                .iter()
                .map(|&x| col_char_pos(x))
                .collect();

            // For each node in this layer, find edges to the next layer.
            for (idx, node) in lo.layers[li].iter().enumerate() {
                let src_x = col_char_pos(lo.x_positions[li][idx]);

                // Find matching nodes in the next layer.
                for (nidx, nnode) in lo.layers[li + 1].iter().enumerate() {
                    let tgt_x = tgt_xs[nidx];
                    let connected = match (node, nnode) {
                        (LayoutNode::Real(ci), LayoutNode::Real(cj)) => {
                            visible_comp.contains(ci)
                                && visible_comp.contains(cj)
                                && edge_set.contains(&(*ci, *cj))
                        }
                        (LayoutNode::Real(ci), LayoutNode::Virtual(ei)) => {
                            lo.edges[*ei].src_component == *ci
                                && visible_comp.contains(ci)
                                && visible_comp.contains(&lo.edges[*ei].tgt_component)
                        }
                        (LayoutNode::Virtual(ei), LayoutNode::Real(cj)) => {
                            lo.edges[*ei].tgt_component == *cj
                                && visible_comp.contains(&lo.edges[*ei].src_component)
                                && visible_comp.contains(cj)
                        }
                        (LayoutNode::Virtual(ei), LayoutNode::Virtual(ej)) => {
                            ei == ej
                                && visible_comp.contains(&lo.edges[*ei].src_component)
                                && visible_comp.contains(&lo.edges[*ei].tgt_component)
                        }
                    };
                    if !connected {
                        continue;
                    }

                    if src_x == tgt_x {
                        if src_x < buf.len() {
                            buf[src_x] = '│';
                        }
                    } else {
                        let (lo_x, hi_x) = if src_x < tgt_x {
                            (src_x, tgt_x)
                        } else {
                            (tgt_x, src_x)
                        };
                        // Place diagonal indicators at endpoints.
                        if lo_x < buf.len() {
                            let ch = if src_x < tgt_x { '╲' } else { '╱' };
                            buf[lo_x] = if buf[lo_x] != ' ' { '┼' } else { ch };
                        }
                        if hi_x < buf.len() {
                            let ch = if src_x < tgt_x { '╲' } else { '╱' };
                            buf[hi_x] = if buf[hi_x] != ' ' { '┼' } else { ch };
                        }
                    }
                }
            }

            let line: String = buf.into_iter().collect();
            let trimmed = line.trim_end();
            if !trimmed.trim().is_empty() {
                items.push(Line::from(vec![Span::styled(trimmed.to_string(), dim())]));
            } else {
                items.push(Line::from(""));
            }
            item_layer.push(li); // connector belongs to same layer for scrolling
        }
    }

    // Scroll by layer: find which layer the cursor is in.
    let cursor_layer = rows.get(cursor).map_or(0, |r| r.layer);
    // Find the display-item index for this cursor layer's node line.
    let cursor_item = item_layer
        .iter()
        .position(|&l| l == cursor_layer)
        .unwrap_or(0);
    let viewport = area.height.saturating_sub(2) as usize;
    let total = items.len();
    let offset = scroll_offset(cursor_item, total, viewport);
    let x_offset = graph_x_offset(&items, selected_at.map(|(_, col)| col), area.width);
    let visible: Vec<ListItem> = items
        .into_iter()
        .skip(offset)
        .take(viewport)
        .map(|line| ListItem::new(crop_line(line, x_offset)))
        .collect();

    let focused = app.view_stack.is_empty();
    let border_style = border(focused);
    let center = app.graph_state.center.as_deref().unwrap_or("(none)");
    let dir = match app.graph_state.direction {
        latt_core::query::Direction::Forward => "→",
        latt_core::query::Direction::Reverse => "←",
    };
    let title = format!(" Graph: {center} {dir} ");
    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_style(title_style(border_style))
            .title(title),
    );
    frame.render_widget(list, area);
    draw_scrollbar(frame, area, total, offset, border_style);
    if !app.detail_focused {
        let cell = selected_at
            .map(|(item, col)| (item.saturating_sub(offset), col.saturating_sub(x_offset)));
        place_cursor(frame, area, cell);
    }
}

fn draw_edge_kind_filter(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let (filter, filter_cursor) = if app.top_view == TopView::Provenance {
        (
            &app.provenance_state.edge_kind_filter,
            app.provenance_state.filter_cursor,
        )
    } else if app.top_view == TopView::Graph {
        (
            &app.graph_state.edge_kind_filter,
            app.graph_state.filter_cursor,
        )
    } else {
        (
            &app.tree_state.edge_kind_filter,
            app.tree_state.filter_cursor,
        )
    };
    let kinds: Vec<(&str, bool)> = app
        .profile
        .edge_kinds()
        .keys()
        .zip(filter.iter())
        .map(|(name, enabled)| (name.as_str(), *enabled))
        .collect();
    let cursor = filter_cursor.min(kinds.len().saturating_sub(1));

    let items: Vec<ListItem> = kinds
        .iter()
        .enumerate()
        .map(|(i, (name, enabled))| {
            let check = if *enabled { "[x]" } else { "[ ]" };
            let line = Line::from(vec![
                Span::raw(format!("  {check} ")),
                Span::styled(*name, Style::default().fg(Color::Cyan)),
            ]);
            let mut item = ListItem::new(line);
            if i == cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Edge-Kind Filter (Space to toggle, Enter/Esc to close) "),
    );
    frame.render_widget(list, area);
    place_cursor(frame, area, selected_row(cursor, kinds.len(), 0));
}

fn draw_preview(frame: &mut Frame, preview: &Preview, area: Rect) {
    let margin_h = area.width / 8;
    let margin_v = area.height / 8;
    let popup = Rect {
        x: area.x + margin_h,
        y: area.y + margin_v,
        width: area.width.saturating_sub(margin_h * 2),
        height: area.height.saturating_sub(margin_v * 2),
    };
    frame.render_widget(Clear, popup);
    place_cursor(frame, popup, None);

    let title = format!(" {} ", preview.file);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title);

    if let Some(rendered) = &preview.rendered {
        let visible_height = popup.height.saturating_sub(2) as usize;
        let md_lines: Vec<Line> = rendered
            .lines
            .iter()
            .skip(preview.scroll)
            .take(visible_height)
            .cloned()
            .collect();
        let paragraph = Paragraph::new(md_lines).style(rendered.style).block(block);
        frame.render_widget(paragraph, popup);
    } else {
        let visible_height = popup.height.saturating_sub(2) as usize;
        let items: Vec<ListItem> = preview
            .lines
            .iter()
            .enumerate()
            .skip(preview.scroll)
            .take(visible_height)
            .map(|(i, line)| {
                let lineno = format!("{:>4} ", i + 1);
                let is_center = i == preview.center_line;
                let style = if is_center {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(lineno, Style::default()),
                    Span::styled(line.as_str(), style),
                ]))
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_widget(list, popup);
    }
}

/// The middle of `area`, `width`×`height` at most, inside a one-cell margin.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn overlay_block(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::BOLD))
        .title(title)
}

fn draw_output(frame: &mut Frame, pane: &OutputPane, area: Rect) {
    let margin_h = area.width / 8;
    let margin_v = area.height / 8;
    let popup = Rect {
        x: area.x + margin_h,
        y: area.y + margin_v,
        width: area.width.saturating_sub(margin_h * 2),
        height: area.height.saturating_sub(margin_v * 2),
    };
    frame.render_widget(Clear, popup);
    place_cursor(frame, popup, None);
    let visible = popup.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = pane
        .lines
        .iter()
        .skip(pane.scroll)
        .take(visible)
        .map(|l| Line::raw(l.as_str()))
        .collect();
    let block = overlay_block(format!(" {} ", pane.title));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_launcher(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let rows = app.launcher_rows();
    let cursor = app.launcher.as_ref().map_or(0, |l| l.cursor);
    let verb_width = rows
        .iter()
        .map(|(v, _)| v.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, (verb, row))| {
            let (text, style) = match row {
                Ok(argv) => (argv.clone(), Style::default()),
                Err(why) => (why.clone(), Style::default().add_modifier(Modifier::DIM)),
            };
            let line = Line::from(vec![
                Span::raw(format!(" {verb:<verb_width$}")),
                Span::styled(text, style),
            ]);
            if i == cursor {
                line.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .collect();
    let width = lines.iter().map(Line::width).max().unwrap_or(0) + 4;
    let height = lines.len() + 2;
    let popup = centered(
        area,
        u16::try_from(width.max(30)).unwrap_or(u16::MAX),
        u16::try_from(height).unwrap_or(u16::MAX),
    );
    frame.render_widget(Clear, popup);
    place_cursor(frame, popup, Some((cursor, 0)));
    let block = overlay_block(" Owner actions — Enter runs, Esc closes ".to_owned());
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_confirm(frame: &mut Frame, argv: &str, area: Rect) {
    let lines = vec![
        Line::raw(format!(" run: {argv}")),
        Line::raw(""),
        Line::raw(" y runs it; any other key cancels"),
    ];
    let width = lines.iter().map(Line::width).max().unwrap_or(0) + 4;
    let popup = centered(area, u16::try_from(width).unwrap_or(u16::MAX), 5);
    frame.render_widget(Clear, popup);
    place_cursor(frame, popup, None);
    let block = overlay_block(" First use of this action ".to_owned());
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_help_overlay(frame: &mut Frame, app: &App<'_>, edge_kinds: &[&str], area: Rect) {
    let margin_h = 4u16.min(area.width / 6);
    let margin_v = 2u16.min(area.height / 6);
    let popup = Rect {
        x: area.x + margin_h,
        y: area.y + margin_v,
        width: area.width.saturating_sub(margin_h * 2),
        height: area.height.saturating_sub(margin_v * 2),
    };
    frame.render_widget(Clear, popup);

    let sections = app.view_bindings();
    let key_width = sections
        .iter()
        .flat_map(|(_, bindings)| bindings)
        .map(|b| b.key.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let mut lines: Vec<Line> = Vec::new();

    for (title, bindings) in &sections {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            format!("  {title}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));
        for Binding { key, desc } in bindings {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("    {key:<key_width$}"),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(*desc),
            ]));
        }
    }

    // Edge-kind legend section
    if !edge_kinds.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  Edge kinds",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));
        for (i, kind) in edge_kinds.iter().enumerate() {
            let color = EDGE_COLORS[i % EDGE_COLORS.len()];
            lines.push(Line::from(vec![
                Span::styled("    ■ ", Style::default().fg(color)),
                Span::raw(*kind),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  Screen readers",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(
        "    browse redraws the screen; the other commands take",
    ));
    lines.push(Line::from("    --format plain to read line by line,"));
    lines.push(Line::from("    --format json for structure"));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Help — j/k, g/G, PgDn/PgUp scroll; any other key closes ");
    let visible = popup.height.saturating_sub(2) as usize;
    let max = lines.len().saturating_sub(visible);
    app.help_scroll_max.set(max);
    let scroll = app.help_scroll.min(max) as u16;
    let paragraph = Paragraph::new(lines).block(block).scroll((scroll, 0));
    frame.render_widget(paragraph, popup);
    place_cursor(frame, popup, None);
}

fn draw_status_line(frame: &mut Frame, app: &App<'_>, area: Rect) {
    let text = app.status_text();
    let mode_style = match app.input_mode {
        InputMode::Search => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .remove_modifier(Modifier::REVERSED),
        InputMode::KindFilter | InputMode::EdgeKindFilter => Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .remove_modifier(Modifier::REVERSED),
        _ => bar(),
    };
    let mode_tag = match app.input_mode {
        InputMode::Search => " SEARCH ",
        InputMode::KindFilter | InputMode::EdgeKindFilter => " FILTER ",
        _ => "",
    };
    let line = if mode_tag.is_empty() {
        Line::from(Span::raw(text))
    } else {
        Line::from(vec![
            Span::styled(mode_tag, mode_style.add_modifier(Modifier::BOLD)),
            Span::raw(text),
        ])
    };
    // The prompt ends in a drawn block; the terminal cursor sits on it.
    let prompt_end = line.width().saturating_sub(1);
    let status = Paragraph::new(line).style(bar());
    frame.render_widget(status, area);
    if app.input_mode == InputMode::Search {
        place_cursor(frame, area, Some((0, prompt_end)));
    }
}

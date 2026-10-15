//! What each tab draws inside the shell's panes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

use crate::app::{self, ActFilter, App, BACKLOG, CHAT, LOGS, Mode, Pane, RowRef, SKILLS, WORKSPACES};
use crate::flow::{self, Graph};
use crate::data::{self, FieldKind, Item, Msg, SessionKind, Status, WsState};
use crate::theme::{self, INK, Palette, SEL_BG};
use crate::ui::panel;

/// What the tab is for, and the few things you usually do there.
pub fn about(tab: usize) -> (&'static str, Vec<(&'static str, &'static str)>) {
    match tab {
        WORKSPACES => (
            "Every saved workspace, its projects, components (databases, caches…) and requirements. Open one; the others keep running.",
            vec![("o", "open"), ("s", "stop (twice: force)"), ("r", "resume"), ("N", "new…"), ("S-←→", "sections")],
        ),
        BACKLOG => (
            "The open workspace's items. Pick one; its body, runs and dependency graph open on the right.",
            vec![("N", "new item"), ("/", "filter"), ("h", "done items"), ("g", "grouping"), ("Tab", "next pane"), ("S-←→", "sections")],
        ),
        CHAT => (
            "The workspace and project coordinators, and every agent working on an item. Item chats are read-only until they ask you.",
            vec![("i", "write"), ("a", "allow"), ("r", "reject"), ("Tab", "next pane"), ("z", "zoom")],
        ),
        SKILLS => (
            "Skills and templates agents can load, and per-agent help. Attach a skill to files with a glob.",
            vec![("Enter", "open"), ("e", "edit"), ("t", "attach"), ("i", "import")],
        ),
        LOGS => (
            "htui's own log, newest at the bottom. Pick a source on the left; new lines stream in while following.",
            vec![("v", "level"), ("F", "follow"), ("↑↓", "scroll"), ("G", "jump to the end")],
        ),
        _ => (
            "How htui is set up on this box: agents, queue limits, storage and secrets. Workspaces live in their own tab (w).",
            vec![("Enter", "edit field"), ("Esc", "back to sections"), ("Tab", "next pane"), ("/", "find a setting")],
        ),
    }
}

fn list_title(tab: usize) -> &'static str {
    ["workspaces", "items", "chats", "skills", "sources", "sections"][tab]
}

pub fn draw_list(f: &mut Frame, app: &App, area: Rect, pal: Palette) {
    let t = app.cur();
    let focused = t.focus == Pane::List;
    let rows = t.tree.rows();
    let entries = rows.iter().filter(|r| matches!(r, RowRef::Entry(_))).count();
    let pos = rows[..(t.tree.cursor + 1).min(rows.len())].iter().filter(|r| matches!(r, RowRef::Entry(_))).count();

    let all: &[(&str, &str)] = match app.tab {
        WORKSPACES => &[("o", "open"), ("s", "stop"), ("←→", "fold")],
        BACKLOG => &[("h", "done"), ("g", "group"), ("/", "filter"), ("←→", "fold")],
        _ => &[("/", "filter"), ("←→", "fold"), ("Enter", "open")],
    };
    // As many hints as fit in the border; a narrow list keeps the first ones.
    let mut n = all.len();
    while n > 0 && crate::ui::hint_line(&all[..n], pal).width() as u16 + 4 > area.width {
        n -= 1;
    }
    let hints = &all[..n];
    let mut title = vec![Span::styled(list_title(app.tab), theme::bold())];
    if app.tab == BACKLOG {
        title.push(Span::styled(format!(" by {}", app.group_by.name()), theme::dim()));
    }
    let mut block = panel(
        title,
        Some(Line::from(Span::styled(format!(" {pos}/{entries} "), theme::dim()))),
        hints,
        focused,
        pal,
    );
    let filtering = matches!(app.mode, Mode::Filter);
    if filtering || !t.tree.filter.is_empty() {
        let cursor = if filtering { "▏" } else { "" };
        block = block.title_bottom(Line::from(vec![
            Span::styled(" / ", pal.accent().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{cursor} ", t.tree.filter), theme::bold()),
        ]));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);

    if rows.is_empty() {
        let text = if t.tree.filter.is_empty() { " empty · N creates the first one" } else { " nothing matches · Esc clears" };
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, theme::dim()))), inner);
        return;
    }
    for (screen, (i, row)) in rows.iter().enumerate().skip(t.tree.offset).take(inner.height as usize).enumerate() {
        let rect = Rect::new(inner.x, inner.y + screen as u16, inner.width, 1);
        let selected = i == t.tree.cursor;
        let marker = if selected { Span::styled("▌", if focused { pal.accent() } else { theme::dim() }) } else { Span::raw(" ") };
        let mut spans = vec![marker];
        match *row {
            RowRef::Group(gi) => {
                let g = &t.tree.groups[gi];
                let open = g.open || !t.tree.filter.is_empty();
                spans.push(Span::styled(if open { "▾ " } else { "▸ " }, theme::dim()));
                if app.tab == WORKSPACES {
                    spans.extend(workspace_group(app, gi));
                } else {
                    spans.push(Span::styled(g.name.clone(), theme::bold()));
                    spans.push(Span::styled(format!("  {}", t.tree.count(gi)), theme::dim()));
                }
            }
            RowRef::Entry(idx) => spans.extend(entry_spans(app, idx, pal)),
        }
        let mut line = crate::ui::fit(Line::from(spans), inner.width);
        if selected {
            line = line.style(Style::new().bg(SEL_BG));
        }
        f.render_widget(line, rect);
    }
    // Under the last row: what `h` hides, and how to see it.
    let shown = rows.len().saturating_sub(t.tree.offset);
    let done = app::finished_count(&app.data, app.workspace);
    if app.tab == BACKLOG && done > 0 && (shown as u16) + 1 < inner.height {
        let text = if app.show_done { format!(" {done} done shown · h hides") } else { format!(" {done} done hidden · h shows") };
        let y = inner.y + shown as u16 + 1;
        f.render_widget(crate::ui::fit(Line::from(Span::styled(text, theme::dim())), inner.width), Rect::new(inner.x, y, inner.width, 1));
    }
}

pub fn ws_state_span(app: &App, ws: usize) -> Span<'static> {
    let (glyph, text, status) = match app.data.workspaces[ws].state {
        WsState::Open => ("●", "open".to_string(), Status::Done),
        WsState::Background => ("◐", format!("background · {} running", app.running(ws)), Status::InProgress),
        WsState::Stopping => ("◌", "stopping…".to_string(), Status::AwaitingApproval),
        WsState::Stopped => ("○", "stopped".to_string(), Status::Queued),
    };
    Span::styled(format!("{glyph} {text}"), Style::new().fg(theme::status_color(status)))
}

fn workspace_group(app: &App, ws: usize) -> Vec<Span<'static>> {
    let w = &app.data.workspaces[ws];
    vec![Span::styled(format!("{}  ", w.name), theme::bold()), ws_state_span(app, ws)]
}

fn entry_spans(app: &App, idx: usize, pal: Palette) -> Vec<Span<'static>> {
    let d = &app.data;
    match app.tab {
        WORKSPACES => {
            let (_, project) = app.ws_nodes[idx];
            match project {
                None => vec![Span::raw("  "), Span::styled("◇ ", pal.accent()), Span::styled("overview", theme::text())],
                Some(p) => vec![Span::raw("  "), Span::styled("▸ ", theme::dim()), Span::styled(p, theme::text())],
            }
        }
        BACKLOG => {
            let item = &d.items[idx];
            vec![
                Span::raw("  "),
                theme::glyph(item.status),
                Span::raw(" "),
                Span::styled(format!("{:<9}", item.key), pal.key(item.key)),
                Span::styled(item.title, theme::text()),
            ]
        }
        CHAT => {
            let s = &d.sessions[idx];
            let (icon, note) = match (s.kind, s.state) {
                (SessionKind::Global, _) => ("✦", ""),
                (SessionKind::Workspace, _) => ("◆", ""),
                (SessionKind::Project, _) => ("◇", ""),
                (SessionKind::Archived(..), _) => ("▤", ""),
                (SessionKind::Item(_), Status::AwaitingApproval) => ("", "asks you"),
                (SessionKind::Item(_), Status::InProgress) => ("", "view only"),
                (SessionKind::Item(_), _) => ("", ""),
            };
            let mut spans = vec![Span::raw("  ")];
            if icon.is_empty() {
                spans.push(theme::glyph(s.state));
            } else {
                spans.push(Span::styled(icon, pal.accent()));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(s.name, theme::text()));
            if !note.is_empty() {
                let style = if note == "asks you" { Style::new().fg(theme::status_color(Status::AwaitingApproval)) } else { theme::dim() };
                spans.push(Span::styled(format!("  {note}"), style));
            }
            spans
        }
        SKILLS => vec![Span::raw("  "), Span::styled(d.skills[idx].name, theme::text())],
        LOGS => {
            let name = if idx == 0 { "all" } else { data::LOG_TARGETS[idx - 1].1 };
            let count = d.htui_log.iter().filter(|r| idx == 0 || r.target == data::LOG_TARGETS[idx - 1].0).count();
            let errors = d.htui_log.iter().filter(|r| (idx == 0 || r.target == data::LOG_TARGETS[idx - 1].0) && r.level >= data::Level::Warn).count();
            let mut spans = vec![Span::raw("  "), Span::styled(format!("{name:<14}"), theme::text()), Span::styled(format!("{count:>4}"), theme::dim())];
            if errors > 0 {
                spans.push(Span::styled(format!("  ⚠{errors}"), Style::new().fg(theme::status_color(Status::AwaitingApproval))));
            }
            spans
        }
        _ => vec![Span::raw("  "), Span::styled(d.sections[idx].name, theme::text())],
    }
}

/// What the detail pane tells the shell after drawing.
#[derive(Default)]
pub struct DetailOut {
    /// Screen rows of selectable rows (settings fields, components), for the mouse.
    pub field_rows: Vec<(u16, usize)>,
    /// Where a rataflow graph went, for the mouse.
    pub flow: Option<Rect>,
    pub max_scroll: u16,
    /// Sub-tab chips in the top border: (x0, x1, y, index), for the mouse.
    pub subtabs: Vec<(u16, u16, u16, usize)>,
    /// Some: the detail scrolled itself to keep its cursor in view.
    pub scroll_to: Option<u16>,
}

pub fn draw_detail(f: &mut Frame, app: &App, area: Rect, pal: Palette) -> DetailOut {
    let t = app.cur();
    let focused = t.focus == Pane::Detail;
    let Some(idx) = app.selected_entry() else {
        let block = panel(vec![Span::styled("nothing selected", theme::dim())], None, &[], focused, pal);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let empty = t.tree.rows().is_empty();
        let text = vec![
            Line::raw(""),
            Line::from(Span::styled(
                if empty { "  Nothing here yet. Press N to create the first one." } else { "  Pick something in the list on the left." },
                theme::text(),
            )),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  "),
                theme::chip("↑↓", pal.accent),
                Span::styled(" move   ", theme::text()),
                theme::chip("→", pal.accent),
                Span::styled(" unfold a group   ", theme::text()),
                theme::chip("N", pal.accent),
                Span::styled(" create one", theme::text()),
            ]),
        ];
        f.render_widget(Paragraph::new(text), inner);
        return DetailOut::default();
    };
    match app.tab {
        WORKSPACES => crate::wsview::ws_detail(f, app, idx, area, focused, pal),
        BACKLOG => backlog_detail(f, app, &app.data.items[idx], area, focused, pal),
        CHAT => {
            chat_detail(f, app, idx, area, focused, pal);
            DetailOut::default()
        }
        SKILLS => skill_detail(f, app, idx, area, focused, pal),
        LOGS => crate::wsview::logs_detail(f, app, idx, area, focused, pal),
        _ => DetailOut { field_rows: settings_detail(f, app, idx, area, focused, pal), ..DetailOut::default() },
    }
}

/// Where each sub-tab chip lands when `subtab_line` is the right-aligned title of `area`.
pub fn subtab_hits(area: Rect, names: &[&str]) -> Vec<(u16, u16, u16, usize)> {
    let width: u16 = 6 + names.iter().map(|n| n.chars().count() as u16 + 2).sum::<u16>();
    let mut x = (area.x + area.width).saturating_sub(1 + width) + 3;
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let w = n.chars().count() as u16 + 2;
            let hit = (x, x + w, area.y, i);
            x += w;
            hit
        })
        .collect()
}

/// The cache key of the graph the current tab shows, if it shows one.
pub fn flow_key(app: &App) -> Option<String> {
    let t = app.cur();
    let (scheme, opts) = (app.scheme.id(), app.graph.id());
    match app.tab {
        BACKLOG => {
            let item = &app.data.items[t.tree.selected()?];
            match t.subtab {
                1 if !t.runs_table => Some(format!("runs:{}:{scheme}:{opts}", item.key)),
                2 => Some(format!("graph:{}:{scheme}:{opts}", item.key)),
                _ => None,
            }
        }
        WORKSPACES if t.subtab == 2 && t.req_graph => {
            let (ws, project) = app.ws_nodes[app.selected_entry()?];
            Some(format!("reqs:{ws}:{}:{scheme}:{opts}", project.unwrap_or("-")))
        }
        _ => None,
    }
}

fn build_graph(app: &App, key: &str, pal: Palette) -> (Graph, Vec<&'static str>) {
    let d = &app.data;
    match key.split(':').next() {
        Some("runs") | Some("graph") => {
            let item = app.cur().tree.selected().map_or(&d.items[0], |i| &d.items[i]);
            if key.starts_with("runs") {
                (flow::run_graph(item, pal, app.graph), flow::RUN_KINDS.to_vec())
            } else {
                flow::item_graph(&d.items, &d.reqs, item, pal, app.graph)
            }
        }
        _ => {
            let (ws, project) = app.selected_entry().map_or((app.workspace, None), |i| app.ws_nodes[i]);
            let reqs: Vec<_> = d.reqs.iter().filter(|r| r.ws == ws && (project.is_none() || r.project == project)).collect();
            flow::req_graph(&d.items, &reqs, pal, app.graph)
        }
    }
}

/// Draws the current tab's graph into `canvas`, with the minimap and the legend when they are on.
pub fn draw_graph(f: &mut Frame, app: &App, canvas: Rect, pal: Palette) -> Option<Rect> {
    let key = flow_key(app)?;
    let mut flows = app.flows.borrow_mut();
    let (graph, kinds) = flows.entry(key.clone()).or_insert_with(|| build_graph(app, &key, pal));
    // A faint dot grid that moves with the view, so panning and dragging read as moving a canvas.
    let dots = rataflow::Background::new(graph)
        .variant(rataflow::BackgroundVariant::Dots)
        .gap(6, 3)
        .style(rataflow::BackgroundStyle::default().with_pattern_color(theme::FAINT).with_bg_color(None));
    f.render_widget(dots, canvas);
    f.render_widget(&mut *graph, canvas);
    if app.graph.minimap && canvas.width > 40 && canvas.height > 10 {
        f.render_widget(rataflow::MiniMap::new(graph).position(rataflow::MiniMapPosition::TopRight).size(24, 8), canvas);
    }
    if app.graph.labels.legend() && !kinds.is_empty() {
        let lines = flow::legend_lines(kinds);
        let w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 2;
        let h = lines.len() as u16 + 2;
        if canvas.width > w + 2 && canvas.height > h + 2 {
            let rect = Rect::new(canvas.x + 1, canvas.bottom() - h, w, h);
            let block = Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::new().fg(theme::FAINT))
                .title(Span::styled(" links ", theme::dim()))
                .title_bottom(Line::from(vec![Span::styled(" L ", pal.accent().add_modifier(Modifier::BOLD)), Span::styled("changes ", theme::dim())]).right_aligned());
            f.render_widget(ratatui::widgets::Clear, rect);
            f.render_widget(Paragraph::new(lines).block(block), rect);
        }
    }
    Some(canvas)
}

/// Hints for a pane showing a graph.
pub const GRAPH_HINTS: &[(&str, &str)] =
    &[("←→↑↓", "node"), ("Enter", "open"), ("M / drag", "move"), ("+ -", "zoom"), ("L", "links"), ("r", "re-layout"), ("S-←→", "section")];

pub fn graph_hints(app: &App) -> &'static [(&'static str, &'static str)] {
    if app.moving { MOVE_HINTS } else { GRAPH_HINTS }
}

/// Hints while a node is being moved.
pub const MOVE_HINTS: &[(&str, &str)] = &[("←→↑↓", "move the node"), ("Enter / Esc", "done"), ("r", "re-layout after")];

/// Sub-tabs as chips, for a pane's top border.
pub fn subtab_line(names: &[&str], active: usize, pal: Palette) -> Line<'static> {
    let mut tabs = vec![Span::styled(" [ ", theme::dim())];
    for (i, name) in names.iter().enumerate() {
        let style = if i == active { Style::new().fg(INK).bg(pal.accent).add_modifier(Modifier::BOLD) } else { theme::dim() };
        tabs.push(Span::styled(format!(" {name} "), style));
    }
    tabs.push(Span::styled(" ] ", theme::dim()));
    Line::from(tabs)
}

fn backlog_detail(f: &mut Frame, app: &App, item: &Item, area: Rect, focused: bool, pal: Palette) -> DetailOut {
    let t = app.cur();
    let hints: &[(&str, &str)] = match t.subtab {
        1 if !t.runs_table => graph_hints(app),
        1 => &[("v", "graph"), ("S-←→", "section")],
        2 => graph_hints(app),
        _ => &[("S-←→", "section"), ("↑↓", "scroll"), ("e", "edit"), ("z", "zoom")],
    };
    let block = panel(
        vec![Span::styled(item.key, pal.key(item.key)), Span::styled(format!(" · {}", item.title), theme::bold())],
        Some(subtab_line(&app::BACKLOG_SUBTABS, t.subtab, pal)),
        hints,
        focused,
        pal,
    );

    let mut lines = vec![Line::from(vec![
        Span::raw(" "),
        theme::status_span(item.status),
        Span::styled("   project ", theme::dim()),
        Span::styled(item.project, theme::text()),
        Span::styled("   priority ", theme::dim()),
        Span::styled(item.priority, theme::text()),
        Span::styled("   depends on ", theme::dim()),
        Span::styled(if item.deps.is_empty() { "nothing".to_string() } else { item.deps.join(", ") }, theme::text()),
    ])];
    lines.push(Line::raw(""));

    let subtabs = subtab_hits(area, &app::BACKLOG_SUBTABS);
    // Runs (as a graph) and Graph are rataflow canvases under the status line.
    if (t.subtab == 1 && !t.runs_table) || t.subtab == 2 {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [head, canvas] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
        f.render_widget(Paragraph::new(lines), Rect { x: head.x + 1, width: head.width.saturating_sub(1), ..head });
        let flow = draw_graph(f, app, canvas, pal);
        return DetailOut { flow, subtabs, ..DetailOut::default() };
    }

    match t.subtab {
        0 => lines.extend(markdown(item.body, pal)),
        1 => {
            let runs = data::runs(item);
            if runs.is_empty() {
                lines.push(Line::from(Span::styled(" No runs yet.", theme::text())));
            } else {
                lines.push(Line::from(Span::styled(format!(" {:<13}{:<14}{:<16}{:<10}{}", "step", "status", "agent", "took", "cost"), theme::dim())));
                for run in runs {
                    lines.push(Line::from(vec![
                        Span::styled(format!(" {:<13}", run.step), theme::bold()),
                        Span::styled(
                            format!("{:<14}", format!("{} {}", theme::status_glyph(run.status), run.status.label())),
                            Style::new().fg(theme::status_color(run.status)),
                        ),
                        Span::styled(format!("{:<16}", run.agent), theme::text()),
                        Span::styled(format!("{:<10}", run.took), theme::text()),
                        Span::styled(run.cost, theme::text()),
                    ]));
                }
            }
        }
        3 => {
            lines.push(Line::from(Span::styled(" docs/decisions/ ", theme::dim())));
            lines.push(Line::from(vec![Span::styled("   ▸ ", theme::dim()), Span::styled(format!("{}.md", item.key), theme::text())]));
            lines.push(Line::from(vec![Span::styled("   ▸ ", theme::dim()), Span::styled("plan.md", theme::text())]));
        }
        _ => lines.push(Line::from(Span::styled(" No notes. Press e to add one.", theme::text()))),
    }
    DetailOut { max_scroll: body(f, block, area, lines, t.detail_scroll), subtabs, ..DetailOut::default() }
}

/// Renders `lines` in `block`, one column in from the border, so wrapped lines keep the indent.
/// Scrolling stops when the last line reaches the bottom; returns that limit.
pub fn body(f: &mut Frame, block: Block, area: Rect, lines: Vec<Line<'static>>, scroll: u16) -> u16 {
    let inner = block.inner(area);
    f.render_widget(block, area);
    let text_width = inner.width.saturating_sub(2).max(1) as usize;
    let total: usize = lines.iter().map(|l| l.width().max(1).div_ceil(text_width)).sum();
    let max_scroll = total.saturating_sub(inner.height as usize) as u16;
    let scroll = scroll.min(max_scroll);
    if max_scroll > 0 {
        let mut state = ScrollbarState::new(max_scroll as usize + 1).position(scroll as usize);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(Style::new().fg(theme::FAINT))
            .thumb_symbol("┃")
            .thumb_style(theme::dim());
        f.render_stateful_widget(bar, Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area }, &mut state);
    }
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    // The padding replaces each line's leading space; a lone " " is a selection gutter and stays.
    let lines: Vec<Line> = lines
        .into_iter()
        .map(|mut l| {
            if let Some(first) = l.spans.first_mut()
                && first.content.len() > 1
                && let Some(rest) = first.content.strip_prefix(' ')
            {
                first.content = rest.to_string().into();
            }
            l
        })
        .collect();
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)), padded);
    max_scroll
}

/// Just enough markdown: headings, bullets, `code`.
pub fn markdown(text: &str, pal: Palette) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| {
            if let Some(h) = l.strip_prefix("# ") {
                Line::from(Span::styled(format!(" {h}"), pal.accent().add_modifier(Modifier::BOLD)))
            } else if let Some(b) = l.strip_prefix("- ") {
                let mut spans = vec![Span::styled("   • ", pal.accent())];
                spans.extend(inline_code(b));
                Line::from(spans)
            } else {
                let mut spans = vec![Span::raw(" ")];
                spans.extend(inline_code(l));
                Line::from(spans)
            }
        })
        .collect()
}

fn inline_code(text: &str) -> Vec<Span<'static>> {
    text.split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                Span::styled(part.to_string(), Style::new().fg(theme::TEXT).bg(SEL_BG))
            } else {
                Span::styled(part.to_string(), theme::text())
            }
        })
        .collect()
}

/// Whether the chat takes a message now: coordinators always, item runs only when they ask.
pub fn chat_writable(s: &data::Session) -> bool {
    match s.kind {
        SessionKind::Global | SessionKind::Workspace | SessionKind::Project => true,
        SessionKind::Item(_) => s.state == Status::AwaitingApproval,
        SessionKind::Archived(..) => false,
    }
}

fn chat_detail(f: &mut Frame, app: &App, idx: usize, area: Rect, focused: bool, pal: Palette) {
    let s = &app.data.sessions[idx];
    let pending = s.msgs.iter().any(|m| matches!(m, Msg::Permission { answer: None, .. }));
    let hints: &[(&str, &str)] = if pending {
        &[("a", "allow"), ("A", "always"), ("r", "reject")]
    } else if chat_writable(s) {
        &[("i", "write"), ("z", "zoom")]
    } else {
        &[("z", "zoom")]
    };
    let badge = match (s.kind, s.state) {
        (SessionKind::Global, _) => Span::styled(" settings · skills · agents · catalogue ", Style::new().fg(INK).bg(pal.accent)),
        (SessionKind::Archived(..), _) => Span::styled(" archived · read only ", Style::new().fg(INK).bg(theme::DIM)),
        (SessionKind::Workspace, _) => Span::styled(" workspace coordinator ", Style::new().fg(INK).bg(pal.accent)),
        (SessionKind::Project, _) => Span::styled(" project coordinator ", Style::new().fg(INK).bg(pal.accent)),
        (SessionKind::Item(_), Status::AwaitingApproval) => {
            Span::styled(" asks you ", Style::new().fg(INK).bg(theme::status_color(Status::AwaitingApproval)))
        }
        (SessionKind::Item(_), Status::InProgress) => Span::styled(" view only · running ", Style::new().fg(INK).bg(theme::DIM)),
        (SessionKind::Item(_), _) => Span::styled(" finished ", Style::new().fg(INK).bg(theme::DIM)),
    };
    let block = panel(
        vec![Span::styled(s.name, theme::bold()), Span::raw(" "), badge],
        Some(Line::from(vec![Span::styled(format!(" {} · {} ", s.agent, s.model), pal.accent()), Span::styled(format!("· {} ", s.project), theme::dim())])),
        hints,
        focused,
        pal,
    );
    let inner = block.inner(area);
    let agent_color = theme::kind_color("MOD");

    let mut lines: Vec<Line> = Vec::new();
    if s.msgs.is_empty() {
        lines.push(Line::from(Span::styled(" No messages yet. Press i to talk to this coordinator.", theme::dim())));
    }
    for msg in &s.msgs {
        match msg {
            Msg::You(text) => {
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![Span::styled(" ▍", pal.accent()), Span::styled("you", pal.accent().add_modifier(Modifier::BOLD))]));
                lines.push(Line::from(vec![Span::styled(" ▍", pal.accent()), Span::styled(text.clone(), theme::text())]));
            }
            Msg::Agent(text) => {
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled(" ▍", Style::new().fg(agent_color)),
                    Span::styled(s.agent, Style::new().fg(agent_color).add_modifier(Modifier::BOLD)),
                ]));
                for l in text.lines() {
                    lines.push(Line::from(vec![Span::styled(" ▍", Style::new().fg(agent_color)), Span::styled(l.to_string(), theme::text())]));
                }
            }
            Msg::Tool { name, arg, ok, took } => {
                let status = if *ok { Status::Done } else { Status::Failed };
                lines.push(Line::from(vec![
                    Span::styled("   ▸ ", theme::dim()),
                    Span::styled(format!("{name:<8}"), theme::bold()),
                    Span::styled(arg.to_string(), theme::text()),
                    Span::raw("  "),
                    theme::glyph(status),
                    Span::styled(format!(" {took}"), theme::dim()),
                ]));
            }
            Msg::Permission { what, answer } => {
                let warn = Style::new().fg(theme::status_color(Status::AwaitingApproval));
                lines.push(Line::raw(""));
                match answer {
                    None => {
                        lines.push(Line::from(vec![
                            Span::styled(" ┃ ⚠ ", warn),
                            Span::styled(format!("{} wants to ", s.agent), theme::bold()),
                            Span::styled(what.to_string(), Style::new().fg(theme::TEXT).bg(SEL_BG)),
                        ]));
                        lines.push(Line::from(vec![
                            Span::styled(" ┃   ", warn),
                            theme::chip("a", theme::status_color(Status::Done)),
                            Span::styled(" allow once   ", theme::text()),
                            theme::chip("A", theme::status_color(Status::Done)),
                            Span::styled(" always   ", theme::text()),
                            theme::chip("r", theme::status_color(Status::Failed)),
                            Span::styled(" reject", theme::text()),
                        ]));
                    }
                    Some(a) => lines.push(Line::from(vec![Span::styled("   ✓ ", theme::dim()), Span::styled(format!("{what} — {a}"), theme::dim())])),
                }
            }
        }
    }
    // Stick to the bottom, like a terminal; wrapped lines keep a one-column indent.
    f.render_widget(block, area);
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let lines: Vec<Line> = lines
        .into_iter()
        .map(|mut l| {
            if let Some(first) = l.spans.first_mut()
                && let Some(rest) = first.content.strip_prefix(' ')
            {
                first.content = rest.to_string().into();
            }
            l
        })
        .collect();
    let width = padded.width.max(1) as usize;
    let total: usize = lines.iter().map(|l| l.width().max(1).div_ceil(width)).sum();
    let scroll = total.saturating_sub(padded.height as usize) as u16;
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)), padded);
}

pub fn draw_composer(f: &mut Frame, app: &App, area: Rect, pal: Palette) {
    let focused = app.cur().focus == Pane::Composer;
    let writing = matches!(app.mode, Mode::Compose);
    let session = app.cur().tree.selected().map(|i| &app.data.sessions[i]);
    let writable = session.is_some_and(chat_writable);
    let asks = session.is_some_and(|s| matches!(s.kind, SessionKind::Item(_)) && s.state == Status::AwaitingApproval);
    let archived = session.is_some_and(|s| matches!(s.kind, SessionKind::Archived(..)));
    let title = if archived { "archived" } else if !writable { "🔒 view only" } else if asks { "answer" } else { "message" };
    let block = panel(
        vec![Span::styled(title, theme::bold())],
        None,
        if writing { &[("Enter", "send"), ("Esc", "stop writing")] } else if writable { &[("i", "write")] } else { &[] },
        focused || writing,
        pal,
    );
    let line = match session {
        _ if writing => Line::from(vec![Span::styled(" › ", pal.accent()), Span::styled(app.compose.clone(), theme::text()), Span::styled("▏", pal.accent())]),
        Some(_) if archived => Line::from(Span::styled(" This step finished. Its transcript is kept here, read only.", theme::dim())),
        Some(s) if !writable => Line::from(Span::styled(
            format!(" {} is working on {} on its own. This unlocks when it asks you something.", s.agent, s.name),
            theme::dim(),
        )),
        Some(s) if asks => Line::from(vec![
            Span::styled(" ⚠ ", Style::new().fg(theme::status_color(Status::AwaitingApproval))),
            Span::styled(format!("{} is waiting for you · press i to answer", s.agent), theme::text()),
        ]),
        Some(s) => Line::from(Span::styled(format!(" press i to write to the {}…", s.name), theme::dim())),
        None => Line::raw(""),
    };
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn skill_detail(f: &mut Frame, app: &App, idx: usize, area: Rect, focused: bool, pal: Palette) -> DetailOut {
    let s = &app.data.skills[idx];
    let block = panel(
        vec![Span::styled(s.name, theme::bold())],
        Some(Line::from(Span::styled(format!(" {} ", s.group), theme::dim()))),
        &[("e", "edit"), ("t", "attach")],
        focused,
        pal,
    );
    let mut lines = vec![
        Line::from(Span::styled(format!(" {}", s.desc), theme::text())),
        Line::raw(""),
        Line::from(vec![Span::styled(" attached to  ", theme::dim()), Span::styled(s.attach, theme::bold())]),
        Line::raw(""),
    ];
    for l in s.body.lines() {
        lines.push(Line::from(vec![Span::styled(" │ ", pal.accent()), Span::styled(l, theme::text())]));
    }
    DetailOut { max_scroll: body(f, block, area, lines, app.cur().detail_scroll), ..DetailOut::default() }
}

fn settings_detail(f: &mut Frame, app: &App, idx: usize, area: Rect, focused: bool, pal: Palette) -> Vec<(u16, usize)> {
    let t = app.cur();
    let section = &app.data.sections[idx];
    let editing = matches!(app.mode, Mode::Edit(_));
    let hints: &[(&str, &str)] = if editing {
        &[("Enter", "save"), ("Esc", "cancel")]
    } else if section.name == "Agents" {
        &[("n", "add"), ("a", "authenticate"), ("r", "probe")]
    } else {
        &[("Enter", "edit"), ("Esc", "sections")]
    };
    let block = panel(vec![Span::styled(section.name, theme::bold())], None, hints, focused || editing, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![Line::from(Span::styled(format!(" {}", section.about), theme::dim())), Line::raw("")];
    let mut field_rows = Vec::new();

    if section.name == "Agents" {
        lines.push(Line::from(Span::styled(
            format!("   {:<10}{:<11}{:<15}{:<15}{:<16}{}", "name", "transport", "billing", "models", "status", "version"),
            theme::dim(),
        )));
        for (i, a) in app.data.agents.iter().enumerate() {
            let label = match a.state {
                Status::Done => "ready",
                Status::AwaitingApproval => "needs login",
                _ => "missing",
            };
            field_rows.push((inner.y + lines.len() as u16, i));
            let selected = focused && i == t.detail_cursor;
            let mut line = Line::from(vec![
                Span::styled(if selected { " ▌ " } else { "   " }, pal.accent()),
                Span::styled(format!("{:<10}", a.name), theme::bold()),
                Span::styled(format!("{:<11}", a.transport), theme::text()),
                Span::styled(format!("{:<15}", a.billing), theme::text()),
                Span::styled(format!("{:<15}", a.models), theme::text()),
                Span::styled(format!("{:<16}", format!("{} {label}", theme::status_glyph(a.state))), Style::new().fg(theme::status_color(a.state))),
                Span::styled(a.version, theme::dim()),
            ]);
            if selected {
                line = line.style(Style::new().bg(SEL_BG));
            }
            lines.push(line);
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(" ⚠ gemini needs a login. ", Style::new().fg(theme::status_color(Status::AwaitingApproval))),
            Span::styled("Select it and press ", theme::text()),
            theme::chip("a", pal.accent),
            Span::styled(" to authenticate.", theme::text()),
        ]));
    } else {
        for (i, field) in section.fields.iter().enumerate() {
            let selected = (focused || editing) && i == t.detail_cursor;
            field_rows.push((inner.y + lines.len() as u16, i));
            let mut spans = vec![
                Span::styled(if selected { " ▌ " } else { "   " }, pal.accent()),
                Span::styled(format!("{:<18}", field.label), if selected { pal.accent().add_modifier(Modifier::BOLD) } else { theme::dim() }),
            ];
            match (&app.mode, selected) {
                (Mode::Edit(buf), true) => {
                    let shown = if matches!(field.kind, FieldKind::Secret) { "•".repeat(buf.chars().count()) } else { buf.clone() };
                    spans.push(Span::styled(format!(" {shown}"), Style::new().fg(theme::TEXT).bg(SEL_BG)));
                    spans.push(Span::styled("▏", pal.accent()));
                }
                _ => spans.extend(field_value(field, pal)),
            }
            let mut line = Line::from(spans);
            if selected && !editing {
                line = line.style(Style::new().bg(SEL_BG));
            }
            lines.push(line);
            // The help sits under the field it explains, not at the bottom of the pane.
            if selected && !field.help.is_empty() {
                lines.push(Line::from(Span::styled(format!("   {:<18}↳ {}", "", field.help), theme::dim())));
            }
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
    field_rows
}

pub fn field_value(field: &data::Field, pal: Palette) -> Vec<Span<'static>> {
    match field.kind {
        FieldKind::Action(_) => vec![Span::styled("▶ ", pal.accent()), Span::styled(field.value.clone(), theme::bold())],
        FieldKind::Toggle => {
            let on = field.value == "on";
            let status = if on { Status::Done } else { Status::Queued };
            vec![Span::styled(if on { "● on" } else { "○ off" }, Style::new().fg(theme::status_color(status)))]
        }
        FieldKind::Choice(_) => vec![Span::styled("‹ ", pal.accent()), Span::styled(field.value.clone(), theme::bold()), Span::styled(" ›", pal.accent())],
        FieldKind::Secret => vec![Span::styled("•••••••• ", theme::text()), Span::styled("stored", theme::dim())],
        FieldKind::ReadOnly => vec![Span::styled(field.value.clone(), theme::dim())],
        FieldKind::Text => vec![Span::styled(field.value.clone(), theme::text())],
    }
}

/// The activity events the filter lets through: "needs you" pinned on top, then the rest; newest
/// first in both.
pub fn activity_events(app: &App) -> Vec<usize> {
    let log = &app.data.log;
    let keep = |i: &usize| {
        let e = &log[*i];
        match app.act_filter {
            ActFilter::All => true,
            ActFilter::NeedsYou => e.status == Status::AwaitingApproval,
            ActFilter::Failures => e.status == Status::Failed,
            ActFilter::ThisWorkspace => e.ws == app.workspace,
        }
    };
    let pinned = (0..log.len()).rev().filter(|i| log[*i].status == Status::AwaitingApproval).filter(keep);
    let rest = (0..log.len()).rev().filter(|i| log[*i].status != Status::AwaitingApproval).filter(keep);
    pinned.chain(rest).collect()
}

/// Returns the screen row of each event drawn.
pub fn draw_activity(f: &mut Frame, app: &App, area: Rect, pal: Palette) -> Vec<(u16, usize)> {
    let focused = app.cur().focus == Pane::Activity;
    let events = activity_events(app);
    let needs = events.iter().filter(|i| app.data.log[**i].status == Status::AwaitingApproval).count();
    let mut title = vec![Span::styled("activity", theme::bold())];
    if needs > 0 {
        title.push(Span::styled(format!("  ⚠ {needs} need you"), Style::new().fg(theme::status_color(Status::AwaitingApproval))));
    }
    let block = panel(
        title,
        Some(Line::from(vec![Span::styled(" f ", pal.accent().add_modifier(Modifier::BOLD)), Span::styled(format!("{} ", app.act_filter.name()), theme::dim())])),
        &[("↑↓", "select"), ("Enter", "go to it"), ("f", "filter"), ("F3", "hide")],
        focused,
        pal,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let height = inner.height as usize;
    // Newest at the top; the view follows the cursor down.
    let start = app.act_cursor.saturating_sub(height.saturating_sub(1));
    let mut rows = Vec::new();
    for (screen, pos) in (start..events.len()).take(height).enumerate() {
        let e = &app.data.log[events[pos]];
        let y = inner.y + screen as u16;
        rows.push((y, pos));
        let selected = focused && pos == app.act_cursor;
        let pin = if e.status == Status::AwaitingApproval { "📌" } else { "  " };
        let mut line = crate::ui::fit(
            Line::from(vec![
                Span::styled(if selected { "▌" } else { " " }, pal.accent()),
                Span::styled(format!("{} ", e.at), theme::dim()),
                theme::glyph(e.status),
                Span::raw(" "),
                Span::styled(e.text.clone(), theme::text()),
                Span::styled(format!(" {pin}"), theme::dim()),
                Span::styled(if e.target.is_some() && selected { "  ⏎ go" } else { "" }, pal.accent()),
            ]),
            inner.width,
        );
        if selected {
            line = line.style(Style::new().bg(SEL_BG));
        }
        f.render_widget(line, Rect::new(inner.x, y, inner.width, 1));
    }
    if events.is_empty() {
        f.render_widget(Line::from(Span::styled(" nothing for this filter · f changes it", theme::dim())), inner);
    }
    rows
}

/// Settings tab's field count for the selected section (Agents is a table).
pub fn settings_rows(app: &App, idx: usize) -> usize {
    if app.data.sections[idx].name == "Agents" { app.data.agents.len() } else { app.data.sections[idx].fields.len() }
}

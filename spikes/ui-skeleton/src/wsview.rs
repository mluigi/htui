//! The Workspaces tab (overview, components, requirements) and the Logs tab.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::app::{self, App};
use crate::data::{self, Level, Status, Usage, WsState};
use crate::theme::{self, Palette, SEL_BG};
use crate::ui::panel;
use crate::views::{DetailOut, GRAPH_HINTS, body, draw_graph, subtab_hits, subtab_line, ws_state_span};

fn heading(text: &str, pal: Palette) -> Line<'static> {
    Line::from(Span::styled(format!(" {text}"), pal.accent().add_modifier(Modifier::BOLD)))
}

pub fn ws_detail(f: &mut Frame, app: &App, idx: usize, area: Rect, focused: bool, pal: Palette) -> DetailOut {
    let t = app.cur();
    let (ws, project) = app.ws_nodes[idx];
    let w = &app.data.workspaces[ws];
    // The panel's hints say what can be done with the workspace in its current state.
    let hints: &[(&str, &str)] = match (t.subtab, project, w.state) {
        (0, None, WsState::Background) => &[("o", "open"), ("s", "stop"), ("N", "new…"), ("S-←→", "section")],
        (0, None, WsState::Stopping) => &[("s", "force stop"), ("S-←→", "section")],
        (0, None, WsState::Stopped) => &[("o", "open"), ("N", "new…"), ("S-←→", "section")],
        (0, ..) => &[("N", "new…"), ("S-←→", "section")],
        (1, None, _) => &[("↑↓", "select"), ("N", "add component"), ("d", "remove")],
        (1, Some(_), _) => &[("↑↓", "select"), ("Enter", "shared / own / none"), ("N", "add own")],
        _ if t.req_graph => GRAPH_HINTS,
        _ => &[("↑↓", "select"), ("Enter", "go to item"), ("v", "graph"), ("N", "new…"), ("S-←→", "section")],
    };
    let mut title = vec![Span::styled(project.unwrap_or(w.name), theme::bold())];
    if project.is_some() {
        title.push(Span::styled(format!(" · project in {}", w.name), theme::dim()));
    } else {
        title.push(Span::styled(" · workspace", theme::dim()));
    }
    let block = panel(title, Some(subtab_line(&app::WS_SUBTABS, t.subtab, pal)), hints, focused, pal);
    let inner = block.inner(area);
    let subtabs = subtab_hits(area, &app::WS_SUBTABS);
    if t.subtab == 2 && t.req_graph {
        f.render_widget(block, area);
        let head = Line::from(vec![
            Span::styled(" requirements as a graph · ", theme::dim()),
            Span::styled("v", pal.accent().add_modifier(Modifier::BOLD)),
            Span::styled(" back to the list", theme::dim()),
        ]);
        f.render_widget(head, Rect { height: 1, ..inner });
        let canvas = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
        let flow = draw_graph(f, app, canvas, pal);
        return DetailOut { flow, subtabs, ..DetailOut::default() };
    }

    let mut rows = Vec::new();
    let mut scroll_to = None;
    let lines = match t.subtab {
        0 => match project {
            None => workspace_overview(app, ws, pal),
            Some(p) => project_overview(app, ws, p, pal),
        },
        1 => components(app, ws, project, focused, t.detail_cursor, pal, &mut rows),
        _ => {
            let (lines, sel) = requirements(app, ws, project, focused.then_some(t.detail_cursor), pal);
            // Keep the selected row in view.
            if let Some((line, _)) = sel.get(t.detail_cursor) {
                let (line, h) = (*line as u16, inner.height);
                let cur = t.detail_scroll;
                scroll_to = Some(if line < cur { line } else if line >= cur + h { line + 1 - h } else { cur });
            }
            rows = sel.iter().enumerate().map(|(i, (line, _))| (*line, i)).collect();
            lines
        }
    };
    let scroll = scroll_to.unwrap_or(t.detail_scroll);
    let max_scroll = body(f, block, area, lines, scroll);
    let scroll = scroll.min(max_scroll);
    let field_rows = rows
        .into_iter()
        .filter_map(|(line, i)| {
            let y = inner.y as i32 + line as i32 - scroll as i32;
            (y >= inner.y as i32 && y < inner.bottom() as i32).then_some((y as u16, i))
        })
        .collect();
    DetailOut { field_rows, flow: None, max_scroll, subtabs, scroll_to }
}

fn workspace_overview(app: &App, ws: usize, pal: Palette) -> Vec<Line<'static>> {
    let w = &app.data.workspaces[ws];
    let items = app.data.items.iter().filter(|i| w.projects.contains(&i.project)).count();
    let mut lines = vec![
        Line::from(vec![
            Span::raw(" "),
            ws_state_span(app, ws),
            Span::styled(if w.state == WsState::Stopping { " · s again forces" } else { "" }, theme::dim()),
            Span::styled("   path ", theme::dim()),
            Span::styled(w.path, theme::text()),
            Span::styled(format!("   {} projects · {items} items · opened {}", w.projects.len(), w.opened), theme::dim()),
        ]),
        Line::raw(""),
    ];
    lines.push(heading("description", pal));
    lines.push(Line::from(Span::styled(format!(" {}", w.description), theme::text())));
    lines.push(Line::raw(""));
    lines.push(heading("goals", pal));
    if w.goals.is_empty() {
        lines.push(Line::from(Span::styled(" none yet · e to add", theme::dim())));
    }
    for g in w.goals {
        lines.push(Line::from(vec![Span::styled("   • ", pal.accent()), Span::styled(g.to_string(), theme::text())]));
    }
    lines.push(Line::raw(""));
    lines.push(heading("projects", pal));
    if w.projects.is_empty() {
        lines.push(Line::from(Span::styled(" none yet · P adds a project from a folder or a git URL", theme::dim())));
    }
    for p in &w.projects {
        let its: Vec<_> = app.data.items.iter().filter(|i| i.project == *p).collect();
        let running = its.iter().filter(|i| i.status == Status::InProgress).count();
        let needs = its.iter().filter(|i| i.status == Status::AwaitingApproval).count();
        let repo = app.data.projects.iter().find(|x| x.name == *p).map_or("", |x| x.repo);
        lines.push(Line::from(vec![
            Span::styled(format!("   {p:<10}"), theme::bold()),
            Span::styled(format!("{:>2} items   ", its.len()), theme::dim()),
            theme::glyph(Status::InProgress),
            Span::styled(format!(" {running}   "), theme::text()),
            theme::glyph(Status::AwaitingApproval),
            Span::styled(format!(" {needs}   "), theme::text()),
            Span::styled(repo.to_string(), theme::dim()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(heading("components", pal));
    let comps: Vec<_> = app.data.components.iter().filter(|c| c.ws == ws).collect();
    if comps.is_empty() {
        lines.push(Line::from(Span::styled(" none · ] then N adds a database, cache, queue…", theme::dim())));
    }
    for c in comps {
        lines.push(Line::from(vec![
            Span::raw("   "),
            theme::glyph(c.status),
            Span::styled(format!(" {:<10}", c.name), theme::bold()),
            Span::styled(format!("{} {}  :{}", c.ctype, c.kind, c.port), theme::text()),
        ]));
    }
    lines
}

fn project_overview(app: &App, ws: usize, p: &str, pal: Palette) -> Vec<Line<'static>> {
    let proj = app.data.projects.iter().find(|x| x.name == p);
    let its: Vec<_> = app.data.items.iter().filter(|i| i.project == p).collect();
    let count = |s: Status| its.iter().filter(|i| i.status == s).count();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" repo ", theme::dim()),
            Span::styled(proj.map_or("", |x| x.repo).to_string(), theme::text()),
            Span::styled(format!("   in workspace {}", app.data.workspaces[ws].name), theme::dim()),
        ]),
        Line::raw(""),
        heading("description", pal),
        Line::from(Span::styled(format!(" {}", proj.map_or("", |x| x.description)), theme::text())),
        Line::raw(""),
        heading("items", pal),
        Line::from(vec![
            Span::styled(format!(" {} items   ", its.len()), theme::text()),
            theme::status_span(Status::InProgress),
            Span::styled(format!(" {}   ", count(Status::InProgress)), theme::text()),
            theme::status_span(Status::AwaitingApproval),
            Span::styled(format!(" {}   ", count(Status::AwaitingApproval)), theme::text()),
            theme::status_span(Status::Done),
            Span::styled(format!(" {}", count(Status::Done)), theme::text()),
        ]),
        Line::raw(""),
        heading("coordinator", pal),
        Line::from(vec![Span::styled(format!(" ◇ {p} coordinator"), theme::text()), Span::styled("   in the Chat tab", theme::dim())]),
        Line::raw(""),
        heading("components", pal),
    ];
    for c in app.data.components.iter().filter(|c| c.ws == ws) {
        let usage = c.usage.iter().find(|(name, _)| *name == p).map_or(Usage::Unused, |(_, u)| *u);
        lines.push(Line::from(vec![
            Span::styled(format!("   {:<10}", c.name), theme::bold()),
            Span::styled(format!("{:<10}", c.kind), theme::text()),
            Span::styled(usage.label(), if usage == Usage::Unused { theme::dim() } else { theme::text() }),
        ]));
    }
    lines
}

/// Workspace level: its components and the generated compose file. Project level: how this project
/// uses each one (the workspace's instance, its own, or none). `rows` gets (line, row) per table row.
fn components(
    app: &App,
    ws: usize,
    project: Option<&str>,
    focused: bool,
    cursor: usize,
    pal: Palette,
    rows: &mut Vec<(usize, usize)>,
) -> Vec<Line<'static>> {
    let comps: Vec<_> = app.data.components.iter().filter(|c| c.ws == ws).collect();
    let mut lines = Vec::new();
    match project {
        None => {
            lines.push(Line::from(Span::styled(
                format!("  {:<12}{:<16}{:<11}{:<22}{:<8}{:<12}{}", "name", "type", "kind", "image", "port", "status", "used by"),
                theme::dim(),
            )));
            for (i, c) in comps.iter().enumerate() {
                let used: Vec<String> = c
                    .usage
                    .iter()
                    .filter(|(_, u)| *u != Usage::Unused)
                    .map(|(p, u)| if *u == Usage::Shared { p.to_string() } else { format!("{p} (own)") })
                    .collect();
                let label = match c.status {
                    Status::InProgress => "running",
                    Status::Queued => "stopped",
                    _ => c.status.label(),
                };
                rows.push((lines.len(), i));
                let selected = focused && i == cursor;
                let mut line = Line::from(vec![
                    Span::styled(if selected { "▌" } else { " " }, pal.accent()),
                    Span::styled(format!("{:<12}", c.name), theme::bold()),
                    Span::styled(format!("{:<16}", c.ctype), theme::text()),
                    Span::styled(format!("{:<11}", c.kind), theme::text()),
                    Span::styled(format!("{:<22}", c.image), theme::dim()),
                    Span::styled(format!("{:<8}", format!(":{}", c.port)), theme::text()),
                    Span::styled(format!("{:<12}", format!("{} {label}", theme::status_glyph(c.status))), Style::new().fg(theme::status_color(c.status))),
                    Span::styled(used.join(", "), theme::text()),
                ]);
                if selected {
                    line = line.style(Style::new().bg(SEL_BG));
                }
                lines.push(line);
            }
            if comps.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(" No components yet. ", theme::text()),
                    theme::chip("N", pal.accent),
                    Span::styled(" adds a database, cache, queue, search engine or any image.", theme::text()),
                ]));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(" docker-compose.yml", pal.accent().add_modifier(Modifier::BOLD)),
                Span::styled(format!("  generated in {}/.htui/ · ports on localhost", app.data.workspaces[ws].path), theme::dim()),
            ]));
            lines.extend(compose_lines(&services(app, ws, None), pal));
        }
        Some(p) => {
            lines.push(Line::from(Span::styled(format!(" How {p} uses the workspace's components. Enter cycles: the workspace's · its own · none."), theme::dim())));
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(format!("  {:<12}{:<11}{:<10}{}", "component", "kind", "port", p), theme::dim())));
            for (i, c) in comps.iter().enumerate() {
                let usage = c.usage.iter().find(|(name, _)| *name == p).map_or(Usage::Unused, |(_, u)| *u);
                let port = match usage {
                    Usage::Own(port) => port,
                    _ => c.port,
                };
                rows.push((lines.len(), i));
                let selected = focused && i == cursor;
                let mut line = Line::from(vec![
                    Span::styled(if selected { "▌" } else { " " }, pal.accent()),
                    Span::styled(format!("{:<12}", c.name), theme::bold()),
                    Span::styled(format!("{:<11}", c.kind), theme::text()),
                    Span::styled(format!("{:<10}", if usage == Usage::Unused { "—".to_string() } else { format!(":{port}") }), theme::text()),
                    Span::styled("‹ ", pal.accent()),
                    Span::styled(usage.label(), if usage == Usage::Unused { theme::dim() } else { theme::bold() }),
                    Span::styled(" ›", pal.accent()),
                ]);
                if selected {
                    line = line.style(Style::new().bg(SEL_BG));
                }
                lines.push(line);
                if selected && usage != Usage::Unused {
                    let scheme = if c.kind == "postgres" { "postgres" } else { c.kind };
                    let var = format!("{}_URL", c.name.to_uppercase());
                    lines.push(Line::from(Span::styled(format!("   ↳ {p} gets {var}={scheme}://localhost:{port}/{p}"), theme::dim())));
                }
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![Span::raw(" "), theme::chip("N", pal.accent), Span::styled(format!(" adds a component only {p} uses"), theme::text())]));
        }
    }
    lines
}

/// One service of the generated compose file.
pub struct Service {
    pub name: String,
    pub image: String,
    pub host_port: u16,
    pub port: u16,
    pub kind: String,
    /// For cloud emulators: the services it emulates.
    pub services: Vec<&'static str>,
    pub comment: Option<String>,
    pub new: bool,
}

/// The services the workspace's compose file runs: each component, plus one per project-own instance.
pub fn services(app: &App, ws: usize, extra: Option<Service>) -> Vec<Service> {
    let mut out = Vec::new();
    for c in app.data.components.iter().filter(|c| c.ws == ws) {
        let shared_by: Vec<&str> = c.usage.iter().filter(|(_, u)| *u == Usage::Shared).map(|(p, _)| *p).collect();
        out.push(Service {
            name: c.name.clone(),
            image: c.image.clone(),
            host_port: c.port,
            port: c.port,
            kind: c.kind.to_string(),
            services: c.services.clone(),
            comment: (!shared_by.is_empty()).then(|| format!("shared by {}", shared_by.join(", "))),
            new: false,
        });
        for (p, u) in &c.usage {
            if let Usage::Own(port) = u {
                out.push(Service {
                    name: format!("{p}-{}", c.name),
                    image: c.image.clone(),
                    host_port: *port,
                    port: c.port,
                    kind: c.kind.to_string(),
                    services: c.services.clone(),
                    comment: Some(format!("{p}'s own instance")),
                    new: false,
                });
            }
        }
    }
    out.extend(extra);
    out
}

pub fn compose_lines(services: &[Service], pal: Palette) -> Vec<Line<'static>> {
    let key = Style::new().fg(theme::kind_color("ANA"));
    let added = Style::new().fg(theme::status_color(Status::Done));
    let mut lines = vec![Line::from(Span::styled("   services:", key))];
    let mut volumes = Vec::new();
    for s in services {
        let gutter = if s.new { Span::styled(" + ", added) } else { Span::raw("   ") };
        let mut push = |text: Vec<Span<'static>>| {
            let mut spans = vec![gutter.clone()];
            spans.extend(text);
            lines.push(Line::from(spans));
        };
        let mut head = vec![Span::styled(format!("  {}:", s.name), key.add_modifier(Modifier::BOLD))];
        if let Some(c) = &s.comment {
            head.push(Span::styled(format!("   # {c}"), theme::dim()));
        }
        push(head);
        push(vec![Span::styled("    image: ", key), Span::styled(s.image.clone(), theme::text())]);
        push(vec![Span::styled("    ports: ", key), Span::styled(format!("[\"{}:{}\"]", s.host_port, s.port), theme::text())]);
        let env = match s.kind.as_str() {
            "postgres" => Some("POSTGRES_PASSWORD"),
            "mysql" | "mariadb" => Some("MYSQL_ROOT_PASSWORD"),
            "mongodb" => Some("MONGO_INITDB_ROOT_PASSWORD"),
            _ => None,
        };
        if !s.services.is_empty() {
            push(vec![Span::styled("    environment:", key)]);
            push(vec![Span::styled("      SERVICES: ", key), Span::styled(s.services.join(","), theme::text())]);
        }
        if let Some(var) = env {
            let secret = format!("{}_PASSWORD", s.name.to_uppercase().replace('-', "_"));
            push(vec![Span::styled("    environment:", key)]);
            push(vec![
                Span::styled(format!("      {var}: "), key),
                Span::styled(format!("${{{secret}}}"), theme::text()),
                Span::styled("   # from Secrets", theme::dim()),
            ]);
            let path = match s.kind.as_str() {
                "postgres" => "/var/lib/postgresql/data",
                "mongodb" => "/data/db",
                _ => "/var/lib/mysql",
            };
            push(vec![Span::styled("    volumes: ", key), Span::styled(format!("[\"{}-data:{path}\"]", s.name), theme::text())]);
            volumes.push((format!("{}-data", s.name), s.new));
        }
    }
    if !volumes.is_empty() {
        lines.push(Line::from(Span::styled("   volumes:", key)));
        for (v, new) in volumes {
            let gutter = if new { Span::styled(" + ", added) } else { Span::raw("   ") };
            lines.push(Line::from(vec![gutter, Span::styled(format!("  {v}:"), key)]));
        }
    }
    let _ = pal;
    lines
}

/// A selectable row of the requirements list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReqSel {
    Req(&'static str),
    Item(&'static str),
}

/// The requirements list, and its selectable rows as (line, what). `cursor` marks one row.
pub fn requirements(app: &App, ws: usize, project: Option<&str>, cursor: Option<usize>, pal: Palette) -> (Vec<Line<'static>>, Vec<(usize, ReqSel)>) {
    let mut sel = Vec::new();
    let reqs: Vec<_> = app
        .data
        .reqs
        .iter()
        .filter(|r| r.ws == ws && (project.is_none() || r.project == project))
        .collect();
    let mut lines = vec![Line::from(Span::styled(
        " What this must do, and which items cover it. ● covered · ◐ in progress · ○ nobody on it yet.",
        theme::dim(),
    ))];
    if reqs.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::raw(" "), theme::chip("N", pal.accent), Span::styled(" adds the first requirement", theme::text())]));
    }
    let mut groups: Vec<Option<&str>> = Vec::new();
    for r in &reqs {
        if !groups.contains(&r.project) {
            groups.push(r.project);
        }
    }
    groups.sort_by_key(|g| g.is_some());
    for g in groups {
        lines.push(Line::raw(""));
        lines.push(heading(&g.map_or("workspace-wide".to_string(), |p| p.to_string()), pal));
        let in_group: Vec<_> = reqs.iter().copied().filter(|r| r.project == g).collect();
        // Top level first; each requirement's children under it, as a tree.
        let roots: Vec<_> = in_group.iter().copied().filter(|r| r.parent.is_none_or(|p| !in_group.iter().any(|x| x.id == p))).collect();
        for r in roots {
            req_lines(app, r, &in_group, "", true, true, pal, cursor, &mut lines, &mut sel);
        }
    }
    (lines, sel)
}

/// One requirement and, under it, its children, drawn with tree connectors.
#[allow(clippy::too_many_arguments)]
fn req_lines(
    app: &App,
    r: &data::Requirement,
    scope: &[&data::Requirement],
    prefix: &str,
    last: bool,
    root: bool,
    pal: Palette,
    cursor: Option<usize>,
    lines: &mut Vec<Line<'static>>,
    sel: &mut Vec<(usize, ReqSel)>,
) {
    let selected = |sel: &Vec<(usize, ReqSel)>| cursor == Some(sel.len());
    let branch = if root { "" } else if last { "└─ " } else { "├─ " };
    let highlighted = selected(sel);
    sel.push((lines.len(), ReqSel::Req(r.id)));
    let mut head = Line::from(vec![
        Span::styled(format!("   {prefix}{branch}"), theme::dim()),
        theme::glyph(r.status),
        Span::raw(" "),
        Span::styled(format!("{:<8}", r.id), Style::new().fg(theme::status_color(Status::AwaitingApproval)).add_modifier(Modifier::BOLD)),
        Span::styled(r.title, theme::bold()),
        Span::styled(format!("   {}", r.area), theme::dim()),
    ]);
    if highlighted {
        head = head.style(Style::new().bg(SEL_BG));
    }
    lines.push(head);
    let child_prefix = if root { String::new() } else { format!("{prefix}{}", if last { "   " } else { "│  " }) };
    let pad = format!("   {child_prefix}{}           ", if scope.iter().any(|c| c.parent == Some(r.id)) { "│" } else { " " });
    lines.push(Line::from(Span::styled(format!("{pad}{}", r.text), theme::dim())));
    // Each covering item on its own row, so it can be picked and opened.
    if r.items.is_empty() {
        lines.push(Line::from(Span::styled(format!("{pad}nobody covers it yet"), theme::dim())));
    }
    for key in r.items {
        let on = selected(sel);
        sel.push((lines.len(), ReqSel::Item(key)));
        let mut spans = vec![Span::styled(format!("{pad}↳ "), theme::dim())];
        match app.data.items.iter().find(|i| i.key == *key) {
            Some(item) => spans.extend([
                theme::glyph(item.status),
                Span::raw(" "),
                Span::styled(format!("{key:<9}"), pal.key(key)),
                Span::styled(item.title, theme::text()),
            ]),
            None => spans.extend([Span::styled(format!("· {key:<9}"), pal.key(key)), Span::styled("another workspace", theme::dim())]),
        }
        let mut line = Line::from(spans);
        if on {
            line = line.style(Style::new().bg(SEL_BG));
        }
        lines.push(line);
    }
    let children: Vec<_> = scope.iter().copied().filter(|c| c.parent == Some(r.id)).collect();
    for (i, c) in children.iter().enumerate() {
        req_lines(app, c, scope, &child_prefix, i + 1 == children.len(), false, pal, cursor, lines, sel);
    }
}

/// htui's log lines for source `idx` (0 = all) at or above the chosen level.
pub fn log_lines(app: &App, idx: usize) -> Vec<usize> {
    app.data
        .htui_log
        .iter()
        .enumerate()
        .filter(|(_, r)| r.level >= app.logs.min_level && (idx == 0 || r.target == data::LOG_TARGETS[idx - 1].0))
        .map(|(i, _)| i)
        .collect()
}

pub fn logs_detail(f: &mut Frame, app: &App, idx: usize, area: Rect, focused: bool, pal: Palette) -> DetailOut {
    let source = if idx == 0 { "all sources" } else { data::LOG_TARGETS[idx - 1].0 };
    let follow = if app.logs.follow {
        Span::styled("● following ", Style::new().fg(theme::status_color(Status::Done)))
    } else {
        Span::styled("○ paused · G resumes ", Style::new().fg(theme::status_color(Status::AwaitingApproval)))
    };
    let block = panel(
        vec![Span::styled("log", theme::bold()), Span::styled(format!(" · {source}"), theme::dim())],
        Some(Line::from(vec![
            Span::styled(" level ≥ ", theme::dim()),
            Span::styled(format!("{} ", app.logs.min_level.name()), theme::bold()),
            Span::styled("· ", theme::dim()),
            follow,
        ])),
        &[("v", "level"), ("F", "follow"), ("↑↓", "scroll"), ("G", "end")],
        focused,
        pal,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let recs = log_lines(app, idx);
    let height = inner.height as usize;
    let max_top = recs.len().saturating_sub(height);
    let top = if app.logs.follow { max_top } else { app.logs.top.min(max_top) };
    let lines: Vec<Line> = recs
        .iter()
        .skip(top)
        .take(height)
        .map(|i| {
            let r = &app.data.htui_log[*i];
            let level_style = match r.level {
                Level::Error => Style::new().fg(theme::status_color(Status::Failed)).add_modifier(Modifier::BOLD),
                Level::Warn => Style::new().fg(theme::status_color(Status::AwaitingApproval)),
                Level::Info => Style::new().fg(theme::status_color(Status::InProgress)),
                Level::Debug => theme::dim(),
                Level::Trace => Style::new().fg(theme::FAINT),
            };
            let msg_style = if r.level >= Level::Warn { theme::bold() } else { theme::text() };
            crate::ui::fit(
                Line::from(vec![
                    Span::styled(format!(" {} ", r.at), theme::dim()),
                    Span::styled(format!("{:<6}", r.level.name()), level_style),
                    Span::styled(format!("{:<13}", r.target), Style::new().fg(theme::kind_color("ANA"))),
                    Span::styled(r.msg.clone(), msg_style),
                ]),
                inner.width.saturating_sub(1),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    if max_top > 0 {
        let mut state = ScrollbarState::new(max_top + 1).position(top);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(Style::new().fg(theme::FAINT))
            .thumb_symbol("┃")
            .thumb_style(theme::dim());
        f.render_stateful_widget(bar, Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area }, &mut state);
    }
    DetailOut::default()
}

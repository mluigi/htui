//! THROWAWAY SPIKE: a btop-style skeleton of htui for trying layouts and colours.
//! Mock data only. Run: `cargo run --manifest-path spikes/ui-skeleton/Cargo.toml`.

#![allow(clippy::type_complexity)] // a throwaway spike: tuples over named types

mod app;
mod data;
mod flow;
mod forms;
mod modal;
mod theme;
mod ui;
mod views;
mod wsview;

use std::io::stdout;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};

use app::{App, BACKLOG, CHAT, Drag, LOGS, Mode, Pane, RowRef, SETTINGS, TABS, WORKSPACES};
use data::{FieldKind, Item, Level, LogLine, Msg, SessionKind, Status, Usage, WsState};
use modal::{Modal, Outcome};
use wsview::ReqSel;

fn main() -> std::io::Result<()> {
    // `--dump <tab 1-6> [preset 0-3] [WxH]`: print one frame as text and exit (no TTY needed).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--dump") {
        return dump(&args[1..]);
    }
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let mut app = App::new();
    let result = run(&mut terminal, &mut app);
    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    app.save();
    if let Some(bye) = &app.goodbye {
        println!("{bye}");
    }
    result
}

fn dump(args: &[String]) -> std::io::Result<()> {
    let mut app = App::new();
    let tab = args.first().and_then(|a| a.parse::<usize>().ok()).unwrap_or(2).clamp(1, TABS.len()) - 1;
    switch_tab(&mut app, tab);
    if let Some(p) = args.get(1).and_then(|a| a.parse::<usize>().ok()).and_then(|p| app::Preset::ALL.get(p)) {
        app.cur_mut().layout.apply(*p);
    }
    // Extra keys to replay before drawing, from the list: `_` down, `^` up, `!` Enter, `~` Tab,
    // `<` `>` left / right; e.g. `--dump 2 0 120x30 '__!]]'` opens MOD-67's graph.
    if let Some(keys) = args.get(3) {
        for c in keys.chars() {
            let code = match c {
                '>' => KeyCode::Right,
                '<' => KeyCode::Left,
                '_' => KeyCode::Down,
                '^' => KeyCode::Up,
                '!' => KeyCode::Enter,
                '~' => KeyCode::Tab,
                c => KeyCode::Char(c),
            };
            on_key(&mut app, KeyEvent::new(code, KeyModifiers::NONE));
            app.check_back();
        }
    } else {
        app.cur_mut().focus = Pane::Detail;
    }
    let (w, h) = args
        .get(2)
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .unwrap_or((120, 36));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).map_err(std::io::Error::other)?;
    // Twice: rataflow fits its view on the first draw.
    terminal.draw(|f| ui::draw(f, &mut app)).map_err(std::io::Error::other)?;
    terminal.draw(|f| ui::draw(f, &mut app)).map_err(std::io::Error::other)?;
    let buf = terminal.backend().buffer();
    for y in 0..h {
        let line: String = (0..w).map(|x| buf[(x, y)].symbol()).collect();
        println!("{}", line.trim_end());
    }
    Ok(())
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    while !app.quit {
        tick(app);
        terminal.draw(|f| ui::draw(f, app))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        // Drain everything queued, so a mouse drag does not lag behind (rataflow's advice).
        loop {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => on_key(app, key),
                Event::Mouse(mouse) => on_mouse(app, mouse),
                _ => {}
            }
            app.check_back();
            if app.quit || !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
    Ok(())
}

/// Time passing: the mock model answers, stops finish, log lines and events stream in.
fn tick(app: &mut App) {
    if app.flash.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_millis(2600)) {
        app.flash = None;
    }
    if let Some(Modal::NewItem(n)) = &mut app.modal {
        n.tick(&app.data);
    }
    let mut catalogue_done = false;
    if let Some(Modal::AddComponent(w)) = &mut app.modal
        && w.updating.is_some_and(|at| at.elapsed() > Duration::from_millis(1200))
    {
        w.updating = None;
        catalogue_done = true;
    }
    if catalogue_done {
        let cat = &mut app.data.catalogue;
        cat.updated = "just now".into();
        for t in &mut cat.types {
            for k in &mut t.kinds {
                let new = match k.name {
                    "postgres" => Some("18"),
                    "kafka" => Some("4.1.0"),
                    "valkey" => Some("8.2"),
                    _ => None,
                };
                if let Some(v) = new
                    && !k.versions.contains(&v)
                {
                    k.versions.insert(0, v);
                }
            }
        }
        app.flash("catalogue updated: postgres 18, kafka 4.1.0, valkey 8.2");
    }
    let done: Vec<usize> = app.stopping.iter().filter(|(_, at)| at.elapsed() > Duration::from_secs(3)).map(|(w, _)| *w).collect();
    for w in done {
        app.stopping.remove(&w);
        app.data.workspaces[w].state = WsState::Stopped;
        let name = app.data.workspaces[w].name;
        app.flash(format!("{name} stopped gracefully"));
    }
    if app.logs.last.elapsed() > Duration::from_millis(1500) {
        app.logs.last = Instant::now();
        let n = app.logs.next;
        app.data.htui_log.push(data::mock_log(n));
        app.logs.next += 1;
    }
    if app.act_last.elapsed() > Duration::from_secs(6) {
        app.act_last = Instant::now();
        const LIVE: [(usize, Status, &str, Option<&str>); 4] = [
            (0, Status::InProgress, "run TOOL-8 · implement step 4/5", Some("TOOL-8")),
            (1, Status::InProgress, "Platform · run FEAT-3 · tests running", Some("FEAT-3")),
            (0, Status::Done, "run FIX-1 · new test passes", Some("FIX-1")),
            (0, Status::InProgress, "worker DESKTOP-HTUI picked MOD-107", Some("MOD-107")),
        ];
        let (ws, status, text, target) = LIVE[app.act_next % LIVE.len()];
        app.act_next += 1;
        if app.data.workspaces[ws].state != WsState::Stopped {
            let secs = 20 * 3600 + 47 * 60 + app.act_next * 6;
            app.data.log.push(LogLine { at: format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60), ws, status, text: text.to_string(), target });
        }
    }
}

fn on_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // Ctrl-C asks like q does; a second Ctrl-C leaves.
    if key.code == KeyCode::Char('c') && ctrl && !matches!(app.modal, Some(Modal::Quit)) {
        app.mode = Mode::Normal;
        app.modal = Some(Modal::Quit);
        return;
    }
    if matches!(app.modal, Some(Modal::NewItem(_))) {
        let screen = app.rects.screen;
        let (w, h) = &mut app.new_item_size;
        match key.code {
            KeyCode::Left if ctrl => return *w = w.saturating_sub(4).max(60),
            KeyCode::Right if ctrl => return *w = (*w + 4).min(screen.width.max(60)),
            KeyCode::Up if ctrl => return *h = h.saturating_sub(2).max(16),
            KeyCode::Down if ctrl => return *h = (*h + 2).min(screen.height.max(16)),
            KeyCode::F(11) => {
                let full = (screen.width.saturating_sub(2), screen.height.saturating_sub(2));
                app.new_item_size = if app.new_item_size == full { (96, 28) } else { full };
                return;
            }
            _ => {}
        }
    }
    if let Some(modal) = &mut app.modal {
        let outcome = modal::on_key(modal, key, &app.data);
        return on_outcome(app, outcome);
    }
    match app.mode {
        Mode::Filter => return filter_key(app, key),
        Mode::Compose => return compose_key(app, key),
        Mode::Edit(_) => return edit_key(app, key),
        Mode::Normal => {}
    }
    if app.help {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
            app.help = false;
        }
        return;
    }

    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        // Moving between panes takes a modifier; plain arrows stay inside the pane.
        KeyCode::Left | KeyCode::Char('h') if ctrl => app.cycle_focus(-1),
        KeyCode::Right | KeyCode::Char('l') if ctrl => app.cycle_focus(1),
        // Sub-tabs: Shift+arrows (no AltGr needed), or [ ].
        KeyCode::Left if shift => step_subtab(app, -1),
        KeyCode::Right if shift => step_subtab(app, 1),
        KeyCode::Char('q') => app.modal = Some(Modal::Quit),
        KeyCode::Backspace if !app.back.is_empty() => app.go_back(),
        KeyCode::Left if alt && !app.back.is_empty() => app.go_back(),
        KeyCode::Char('?') => app.help = true,
        KeyCode::F(1) => {
            app.about = !app.about;
            app.flash(if app.about { "about panel on" } else { "about panel off · F1 brings it back" });
        }
        KeyCode::Char('1') if alt => toggle_list(app),
        KeyCode::Char('2') if alt => toggle_activity(app),
        KeyCode::F(2) => toggle_list(app),
        KeyCode::F(3) => toggle_activity(app),
        KeyCode::Char(c @ '1'..='6') => switch_tab(app, c as usize - '1' as usize),
        KeyCode::Char('w') => go_to_workspaces(app),
        KeyCode::Tab => app.cycle_focus(1),
        KeyCode::BackTab => app.cycle_focus(-1),
        KeyCode::Char('p') => step_preset(app, 1),
        KeyCode::Char('P') => step_preset(app, -1),
        KeyCode::Char('<') => resize_list(app, -3),
        KeyCode::Char('>') => resize_list(app, 3),
        KeyCode::Char('{') => resize_activity(app, -3),
        KeyCode::Char('}') => resize_activity(app, 3),
        KeyCode::Char('[') => step_subtab(app, -1),
        KeyCode::Char(']') => step_subtab(app, 1),
        KeyCode::Char('z') => {
            let t = app.cur_mut();
            t.zoom = !t.zoom;
            let msg = if t.zoom { "zoomed · z to restore" } else { "layout restored" };
            app.flash(msg);
        }
        KeyCode::Char('c') => {
            app.scheme = app.scheme.next();
            app.flash(format!("colours: {}", app.scheme.name()));
        }
        KeyCode::Char('/') => {
            app.cur_mut().layout.list_on = true;
            app.cur_mut().focus = Pane::List;
            app.mode = Mode::Filter;
        }
        KeyCode::Char('N' | 'n') => new_thing(app),
        KeyCode::Char('i') if app.tab == CHAT => start_compose(app),
        KeyCode::Char(c @ ('a' | 'A' | 'r')) if app.tab == CHAT && answer_permission(app, c) => {}
        KeyCode::Char('o') if app.tab == WORKSPACES => {
            if let Some(ws) = selected_ws(app) {
                app.open_workspace(ws);
            }
        }
        KeyCode::Char('s') if app.tab == WORKSPACES => stop_workspace(app),
        _ => pane_key(app, key),
    }
}

fn pane_key(app: &mut App, key: KeyEvent) {
    let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
    let up = matches!(key.code, KeyCode::Up | KeyCode::Char('k'));
    let left = matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
    let right = matches!(key.code, KeyCode::Right | KeyCode::Char('l'));
    match app.cur().focus {
        Pane::List => list_key(app, key, up, down, left, right),
        Pane::Detail => detail_key(app, key, up, down, left, right),
        Pane::Activity => activity_key(app, key, up, down),
        Pane::Composer => {
            if key.code == KeyCode::Enter {
                start_compose(app);
            }
        }
    }
}

fn list_key(app: &mut App, key: KeyEvent, up: bool, down: bool, left: bool, right: bool) {
    let tree = &mut app.cur_mut().tree;
    let before = tree.cursor;
    match key.code {
        _ if down => tree.move_by(1),
        _ if up => tree.move_by(-1),
        KeyCode::PageDown => tree.move_by(10),
        KeyCode::PageUp => tree.move_by(-10),
        KeyCode::Char('g') | KeyCode::Home => tree.move_by(-10_000),
        KeyCode::Char('G') | KeyCode::End => tree.move_by(10_000),
        KeyCode::Esc => {
            tree.filter.clear();
            tree.clamp();
        }
        // Folding stays in the list; it never moves focus.
        _ if left => tree.set_open(false),
        _ if right => tree.set_open(true),
        KeyCode::Enter => match tree.current() {
            Some(RowRef::Group(gi)) => {
                let open = !tree.groups[gi].open;
                tree.set_open(open);
            }
            _ => {
                if app.cur().tree.selected().is_some() {
                    app.cur_mut().focus = Pane::Detail;
                }
            }
        },
        _ => {}
    }
    if app.cur().tree.cursor != before {
        let t = app.cur_mut();
        t.detail_cursor = 0;
        t.detail_scroll = 0;
    }
}


fn detail_key(app: &mut App, key: KeyEvent, up: bool, down: bool, left: bool, right: bool) {
    let tab = app.tab;
    let Some(idx) = app.selected_entry() else { return };
    if key.code == KeyCode::Esc {
        app.cur_mut().focus = Pane::List;
        return;
    }
    if tab == BACKLOG && key.code == KeyCode::Char('v') && app.cur().subtab == 1 {
        let t = app.cur_mut();
        t.runs_table = !t.runs_table;
        return;
    }
    if tab == WORKSPACES && key.code == KeyCode::Char('v') && app.cur().subtab == 2 {
        let t = app.cur_mut();
        t.req_graph = !t.req_graph;
        t.detail_scroll = 0;
        return;
    }
    if let Some(flow_key) = views::flow_key(app) {
        graph_key(app, key, &flow_key);
        return;
    }
    let scroll = |app: &mut App, by: i32| {
        let max = app.rects.detail_max_scroll as i32;
        let t = app.cur_mut();
        t.detail_scroll = (t.detail_scroll as i32 + by).clamp(0, max) as u16;
    };
    match tab {
        SETTINGS => {
            let len = views::settings_rows(app, idx);
            let t = app.cur_mut();
            if down {
                t.detail_cursor = (t.detail_cursor + 1).min(len.saturating_sub(1));
            } else if up {
                t.detail_cursor = t.detail_cursor.saturating_sub(1);
            } else if key.code == KeyCode::Enter {
                start_edit(app, idx);
            }
        }
        LOGS => logs_key(app, key, idx, up, down),
        WORKSPACES if app.cur().subtab == 2 => {
            let (ws, project) = app.ws_nodes[idx];
            let (_, sel) = wsview::requirements(app, ws, project, None, theme::Palette::new(app.scheme, WORKSPACES));
            let t = app.cur_mut();
            if down {
                t.detail_cursor = (t.detail_cursor + 1).min(sel.len().saturating_sub(1));
            } else if up {
                t.detail_cursor = t.detail_cursor.saturating_sub(1);
            } else if left || right {
                step_subtab(app, if left { -1 } else { 1 });
            } else if key.code == KeyCode::Enter {
                match sel.get(t.detail_cursor).map(|(_, s)| *s) {
                    Some(ReqSel::Item(key)) => jump(app, |app| go_to_item(app, key)),
                    Some(ReqSel::Req(id)) => app.flash(format!("{id}: e edits it · L links an item (mock)")),
                    None => {}
                }
            }
        }
        WORKSPACES if app.cur().subtab == 1 => {
            let (ws, project) = app.ws_nodes[idx];
            let comps: Vec<usize> = (0..app.data.components.len()).filter(|i| app.data.components[*i].ws == ws).collect();
            let t = app.cur_mut();
            if down {
                t.detail_cursor = (t.detail_cursor + 1).min(comps.len().saturating_sub(1));
            } else if up {
                t.detail_cursor = t.detail_cursor.saturating_sub(1);
            } else if left || right {
                step_subtab(app, if left { -1 } else { 1 });
            } else if let (KeyCode::Enter, Some(p), Some(ci)) = (key.code, project, comps.get(t.detail_cursor)) {
                cycle_usage(app, *ci, p);
            } else if let (KeyCode::Char('d'), None, Some(ci)) = (key.code, project, comps.get(t.detail_cursor)) {
                let c = app.data.components.remove(*ci);
                app.cur_mut().detail_cursor = 0;
                app.flash(format!("removed {} · docker-compose.yml updated", c.name));
            }
        }
        _ => {
            if down {
                scroll(app, 1);
            } else if up {
                scroll(app, -1);
            } else if key.code == KeyCode::PageDown {
                scroll(app, 10);
            } else if key.code == KeyCode::PageUp {
                scroll(app, -10);
            } else if left || right {
                step_subtab(app, if left { -1 } else { 1 });
            }
        }
    }
}

/// A graph takes the arrows (pick a node), h j k l (pan), + - 0 (zoom), f (fit), Enter (open),
/// and the view options: L (how links are labelled), m (minimap), d (direction).
fn graph_key(app: &mut App, key: KeyEvent, flow_key: &str) {
    match key.code {
        KeyCode::Char('L') => {
            app.graph.labels = app.graph.labels.next();
            app.flash(format!("links: {}", app.graph.labels.name()));
            return;
        }
        KeyCode::Char('m') => {
            app.graph.minimap = !app.graph.minimap;
            app.flash(if app.graph.minimap { "minimap on" } else { "minimap off" });
            return;
        }
        KeyCode::Char('d') => {
            app.graph.horizontal = !app.graph.horizontal;
            app.flash(if app.graph.horizontal { "graphs flow left to right" } else { "graphs flow top to bottom" });
            return;
        }
        KeyCode::Enter => return graph_enter(app, flow_key),
        _ => {}
    }
    let mut flows = app.flows.borrow_mut();
    let Some((graph, _)) = flows.get_mut(flow_key) else { return };
    match key.code {
        KeyCode::Char('f') => flow::fit_all(graph),
        KeyCode::Char('+' | '=') => graph.zoom_in(),
        KeyCode::Char('-') => graph.zoom_out(),
        KeyCode::Char('0') => graph.reset_zoom(),
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'j' | 'k' | 'l') => {
            let _ = graph.handle_key_event(key);
        }
        _ => {}
    }
}

/// Enter on a graph node: another item opens that item, the item itself or a run step opens its
/// chat, a requirement opens it in Workspaces.
/// Runs a navigation and remembers where it came from, so Backspace can return.
fn jump(app: &mut App, go: impl FnOnce(&mut App)) {
    let from = app.here();
    go(app);
    let to = app.here();
    if from.same(&to) {
        return;
    }
    if app.back.last().is_some_and(|(_, dest)| !dest.same(&from)) {
        app.back.clear();
    }
    app.back.push((from, to));
}

fn graph_enter(app: &mut App, flow_key: &str) {
    let node = app.flows.borrow().get(flow_key).and_then(|(g, _)| g.first_selected_node_id());
    let Some(node) = node else { return };
    let current = if app.tab == BACKLOG { app.cur().tree.selected().map(|i| app.data.items[i].key) } else { None };
    if flow_key.starts_with("runs:") {
        let Some(idx) = app.cur().tree.selected() else { return };
        let item = &app.data.items[idx];
        let Some(step) = flow::run_steps(item).into_iter().find(|s| s.id == node) else { return };
        let key = item.key;
        match step.status {
            // Not started: nothing to show yet.
            Status::Queued => app.flash(format!("{} has not started yet", step.name)),
            Status::Done | Status::Closed => jump(app, |app| open_archived(app, key, step.id, step.name, step.agent)),
            _ => jump(app, |app| open_item_chat(app, key, Some(step.name))),
        }
    } else if let Some(id) = node.strip_prefix("req:") {
        let id = id.to_string();
        jump(app, |app| go_to_requirement(app, &id));
    } else if Some(node.as_str()) == current {
        jump(app, |app| open_item_chat(app, current.unwrap_or_default(), None));
    } else {
        jump(app, |app| go_to_item(app, &node));
    }
}

/// A finished step's transcript, read only, in the Chat tab's "archived" group.
fn open_archived(app: &mut App, key: &'static str, step: &'static str, name: &'static str, agent: &'static str) {
    let existing = app.data.sessions.iter().position(|s| s.kind == SessionKind::Archived(key, step));
    let idx = match existing {
        Some(i) => i,
        None => {
            let (a, model) = agent.split_once(" · ").unwrap_or((agent, ""));
            let project = app.data.items.iter().find(|i| i.key == key).map_or("", |i| i.project);
            app.data.sessions.push(data::Session {
                ws: app.workspace,
                kind: SessionKind::Archived(key, step),
                name: Box::leak(format!("{key} · {name}").into_boxed_str()),
                agent: a,
                model,
                project,
                state: Status::Done,
                msgs: vec![
                    Msg::Agent(format!("Starting {name} for {key}.")),
                    Msg::Tool { name: "read", arg: "the item body and its decisions", ok: true, took: "0.2s" },
                    Msg::Tool { name: "search", arg: "related code and prior runs", ok: true, took: "0.4s" },
                    Msg::Agent(format!("{name} finished. Summary and artifacts are attached to the run.")),
                ],
            });
            app.data.sessions.len() - 1
        }
    };
    app.tabs[CHAT].tree = app::chat_tree(&app.data, app.workspace);
    for g in app.tabs[CHAT].tree.groups.iter_mut().filter(|g| g.name == "archived") {
        g.open = true;
    }
    switch_tab(app, CHAT);
    app.tabs[CHAT].tree.select(idx);
    app.tabs[CHAT].focus = Pane::Detail;
}

fn open_item_chat(app: &mut App, key: &str, step: Option<&str>) {
    let found = app.data.sessions.iter().position(|s| s.kind == SessionKind::Item(app_key(app, key)) && s.ws == app.workspace);
    match found {
        Some(idx) => {
            switch_tab(app, CHAT);
            app.tabs[CHAT].tree.select(idx);
            app.tabs[CHAT].focus = Pane::Detail;
            if let Some(step) = step {
                app.flash(format!("{key} · {step}: live"));
            }
        }
        None => app.flash(format!("{key} has no live chat{}: its transcript would open here", step.map_or(String::new(), |s| format!(" for {s}")))),
    }
}

/// The `&'static str` key of an item, for comparing with a session's.
fn app_key(app: &App, key: &str) -> &'static str {
    app.data.items.iter().find(|i| i.key == key).map_or("", |i| i.key)
}

fn go_to_item(app: &mut App, key: &str) {
    let Some(idx) = app.data.items.iter().position(|i| i.key == key) else {
        app.flash(format!("{key} is in another workspace"));
        return;
    };
    if !app.workspace_projects().contains(&app.data.items[idx].project) {
        app.flash(format!("{key} is in another workspace: open it first (w)"));
        return;
    }
    switch_tab(app, BACKLOG);
    app.tabs[BACKLOG].tree.select(idx);
    app.tabs[BACKLOG].focus = Pane::Detail;
    app.flash(format!("{key} · {}", app.data.items[idx].title));
}

fn go_to_requirement(app: &mut App, id: &str) {
    let Some(r) = app.data.reqs.iter().find(|r| r.id == id && r.ws == app.workspace) else {
        app.flash(format!("{id} is in another workspace"));
        return;
    };
    let (ws, project, rid, title) = (r.ws, r.project, r.id, r.title);
    switch_tab(app, WORKSPACES);
    if let Some(i) = app.ws_nodes.iter().position(|n| *n == (ws, project)) {
        app.tabs[WORKSPACES].tree.select(i);
    }
    let (_, sel) = wsview::requirements(app, ws, project, None, theme::Palette::new(app.scheme, WORKSPACES));
    let row = sel.iter().position(|(_, s)| *s == ReqSel::Req(rid)).unwrap_or(0);
    let t = &mut app.tabs[WORKSPACES];
    t.subtab = 2;
    t.req_graph = false;
    t.detail_cursor = row;
    t.focus = Pane::Detail;
    app.flash(format!("{rid} · {title}"));
}

fn logs_key(app: &mut App, key: KeyEvent, idx: usize, up: bool, down: bool) {
    let total = wsview::log_lines(app, idx).len();
    let height = app.rects.detail.map_or(10, |r| r.height.saturating_sub(2) as usize);
    let max_top = total.saturating_sub(height);
    let l = &mut app.logs;
    let by: isize = match key.code {
        _ if up => -1,
        _ if down => 1,
        KeyCode::PageUp => -10,
        KeyCode::PageDown => 10,
        _ => 0,
    };
    if by != 0 {
        if l.follow {
            l.follow = false;
            l.top = max_top;
        }
        l.top = (l.top as isize + by).clamp(0, max_top as isize) as usize;
        if by > 0 && l.top == max_top {
            l.follow = true;
        }
        return;
    }
    match key.code {
        KeyCode::Char('G') | KeyCode::End => l.follow = true,
        KeyCode::Char('g') | KeyCode::Home => {
            l.follow = false;
            l.top = 0;
        }
        KeyCode::Char('F') => {
            l.follow = !l.follow;
            l.top = max_top;
        }
        KeyCode::Char('v') => {
            let pos = Level::ALL.iter().position(|x| *x == l.min_level).unwrap_or(0);
            l.min_level = Level::ALL[(pos + 1) % Level::ALL.len()];
            let name = l.min_level.name();
            app.flash(format!("showing {name} and above"));
        }
        _ => {}
    }
}

fn activity_key(app: &mut App, key: KeyEvent, up: bool, down: bool) {
    let events = views::activity_events(app);
    match key.code {
        _ if down => app.act_cursor = (app.act_cursor + 1).min(events.len().saturating_sub(1)),
        _ if up => app.act_cursor = app.act_cursor.saturating_sub(1),
        KeyCode::Char('f') => {
            app.act_filter = app.act_filter.next();
            app.act_cursor = 0;
        }
        KeyCode::Enter => {
            if let Some(i) = events.get(app.act_cursor).copied() {
                jump(app, |app| jump_to(app, i));
            }
        }
        _ => {}
    }
}

/// Opens what an activity event is about: its item in the Backlog, or its chat.
fn jump_to(app: &mut App, event: usize) {
    let Some(target) = app.data.log[event].target else {
        app.flash("this event has nothing to open");
        return;
    };
    let ws_name = app.data.workspaces[app.data.log[event].ws].name;
    if let Some(idx) = app.data.items.iter().position(|i| i.key == target) {
        if !app.workspace_projects().contains(&app.data.items[idx].project) {
            app.flash(format!("{target} is in {ws_name}: open that workspace first (w)"));
            return;
        }
        switch_tab(app, BACKLOG);
        app.tabs[BACKLOG].tree.select(idx);
        app.tabs[BACKLOG].focus = Pane::Detail;
    } else if let Some(idx) = app.data.sessions.iter().position(|s| s.name == target) {
        if app.data.sessions[idx].ws != app.workspace {
            app.flash(format!("that chat is in {ws_name}: open that workspace first (w)"));
            return;
        }
        switch_tab(app, CHAT);
        app.tabs[CHAT].tree.select(idx);
        app.tabs[CHAT].focus = Pane::Detail;
    } else {
        app.flash(format!("{target} is not in this workspace"));
    }
}

fn selected_ws(app: &App) -> Option<usize> {
    let t = &app.tabs[WORKSPACES];
    match t.tree.current()? {
        RowRef::Group(gi) => Some(gi),
        RowRef::Entry(i) => Some(app.ws_nodes[i].0),
    }
}

fn go_to_workspaces(app: &mut App) {
    switch_tab(app, WORKSPACES);
    let overview = app.ws_nodes.iter().position(|(w, p)| *w == app.workspace && p.is_none());
    if let Some(i) = overview {
        app.tabs[WORKSPACES].tree.select(i);
    }
}

fn stop_workspace(app: &mut App) {
    let Some(ws) = selected_ws(app) else { return };
    let name = app.data.workspaces[ws].name;
    let running = app.running(ws);
    match app.data.workspaces[ws].state {
        WsState::Open => app.flash(format!("{name} is open: open another one first, or stop single runs in the Backlog")),
        WsState::Background => {
            app.data.workspaces[ws].state = WsState::Stopping;
            app.stopping.insert(ws, Instant::now());
            app.flash(format!("stopping {name}: {running} run(s) finish their step · s again forces"));
        }
        WsState::Stopping => {
            app.stopping.remove(&ws);
            app.data.workspaces[ws].state = WsState::Stopped;
            app.flash(format!("forced: {name} stopped, {running} run(s) killed"));
        }
        WsState::Stopped => app.flash(format!("{name} is already stopped · o opens it")),
    }
}

fn cycle_usage(app: &mut App, ci: usize, project: &'static str) {
    let used_ports: Vec<u16> = app
        .data
        .components
        .iter()
        .filter(|c| c.ws == app.data.components[ci].ws)
        .flat_map(|c| std::iter::once(c.port).chain(c.usage.iter().filter_map(|(_, u)| if let Usage::Own(p) = u { Some(*p) } else { None })))
        .collect();
    let c = &mut app.data.components[ci];
    let mut free = c.port + 1;
    while used_ports.contains(&free) {
        free += 1;
    }
    let Some((_, usage)) = c.usage.iter_mut().find(|(p, _)| *p == project) else { return };
    *usage = match *usage {
        Usage::Shared => Usage::Own(free),
        Usage::Own(_) => Usage::Unused,
        Usage::Unused => Usage::Shared,
    };
    let text = format!("{project} → {} {} · compose updated", c.name, usage.label());
    app.flash(text);
}

fn new_thing(app: &mut App) {
    match app.tab {
        BACKLOG => {
            let projects = app.workspace_projects();
            if projects.is_empty() {
                app.flash("this workspace has no projects yet: add one in workspaces (w)");
            } else {
                // The project of the item on screen, or of the group under the cursor.
                let here = match app.tabs[BACKLOG].tree.current() {
                    Some(RowRef::Entry(i)) => Some(app.data.items[i].project),
                    Some(RowRef::Group(gi)) => projects.iter().copied().find(|p| *p == app.tabs[BACKLOG].tree.groups[gi].name),
                    None => None,
                };
                let mut n = modal::NewItem::new(projects.clone());
                n.project = here.and_then(|p| projects.iter().position(|x| *x == p)).unwrap_or(0);
                app.modal = Some(Modal::NewItem(n));
            }
        }
        WORKSPACES => {
            let (ws, project) = app.selected_entry().map_or((app.workspace, None), |i| app.ws_nodes[i]);
            app.modal = Some(match app.cur().subtab {
                1 => Modal::AddComponent(modal::Wizard::new(ws, project)),
                2 => requirement_form(app, ws, project),
                _ => Modal::Chooser(forms::Chooser {
                    options: vec![
                        ("workspace", "a new workspace, in its own folder"),
                        ("project", "a repo in this workspace"),
                        ("component", "a database, cache, queue… (docker compose)"),
                        ("requirement", "something this must do; can refine another"),
                    ],
                    cursor: if project.is_some() { 3 } else { 0 },
                }),
            });
        }
        _ => app.flash("mock: in htui this opens a modal like Backlog's N"),
    }
}

fn requirement_form(app: &App, ws: usize, project: Option<&'static str>) -> Modal {
    let parents = app.data.reqs.iter().filter(|r| r.ws == ws && (project.is_none() || r.project == project)).map(|r| r.id.to_string()).collect();
    Modal::Form(forms::Form::requirement(ws, project, parents))
}

fn on_outcome(app: &mut App, outcome: Outcome) {
    match outcome {
        Outcome::Leave { stop_all } => {
            let running: usize = (0..app.data.workspaces.len()).map(|w| app.running(w)).sum();
            app.goodbye = Some(if stop_all {
                format!("htui: stopping {running} run(s) gracefully in the background; run htui to watch.")
            } else {
                format!("htui keeps running in the background: {running} run(s). Run htui to pick up where you left.")
            });
            app.quit = true;
        }
        Outcome::Choose(what) => {
            let (ws, project) = app.selected_entry().map_or((app.workspace, None), |i| app.ws_nodes[i]);
            app.modal = Some(match what {
                "workspace" => Modal::Form(forms::Form::workspace()),
                "project" => Modal::Form(forms::Form::project(ws)),
                "component" => Modal::AddComponent(modal::Wizard::new(ws, project)),
                _ => requirement_form(app, ws, project),
            });
        }
        Outcome::SubmitForm => {
            if let Some(Modal::Form(form)) = app.modal.take() {
                submit_form(app, form);
            }
        }
        Outcome::Stay => {}
        Outcome::Close => app.modal = None,
        Outcome::Create { project, draft } => {
            app.modal = None;
            let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
            let key = leak(draft.key);
            app.data.items.push(Item {
                key,
                title: leak(draft.title),
                project,
                status: Status::Open,
                priority: "medium",
                deps: &[],
                rels: &[],
                body: leak(draft.body),
            });
            let idx = app.data.items.len() - 1;
            app.rebuild_for_workspace(Some(idx));
            app.flash(format!("created {key} in {project}"));
        }
        Outcome::AddComponent(c) => {
            app.modal = None;
            let text = format!("{} added · docker-compose.yml updated · localhost:{}", c.name, c.port);
            let ws = c.ws;
            app.data.components.push(c);
            let pos = app.data.components.iter().filter(|x| x.ws == ws).count() - 1;
            let t = &mut app.tabs[WORKSPACES];
            t.subtab = 1;
            t.detail_cursor = pos;
            t.focus = Pane::Detail;
            app.flash(text);
        }
    }
}

fn submit_form(app: &mut App, form: forms::Form) {
    let leak = |s: &str| -> &'static str { Box::leak(s.trim().to_string().into_boxed_str()) };
    match form.kind {
        forms::FormKind::Workspace => {
            let name = leak(form.value("name"));
            let path = leak(form.value("folder"));
            app.data.workspaces.push(data::Workspace {
                name,
                projects: vec![],
                opened: "never",
                path,
                description: leak(form.value("description")),
                goals: &[],
                state: WsState::Stopped,
            });
            let ws = app.data.workspaces.len() - 1;
            app.data.sessions.push(data::Session {
                ws,
                kind: SessionKind::Workspace,
                name: leak(&format!("{name} coordinator")),
                agent: "claude",
                model: "sonnet",
                project: name,
                state: Status::Open,
                msgs: vec![],
            });
            app.rebuild_ws_tree(Some((ws, None)));
            app.flash(format!("created {name} in {path} · o opens it"));
        }
        forms::FormKind::Project => {
            let name = leak(form.value("name"));
            let repo = leak(form.value("location"));
            let ws = form.ws;
            app.data.projects.push(data::Project { name, repo, description: "" });
            app.data.workspaces[ws].projects.push(name);
            for c in app.data.components.iter_mut().filter(|c| c.ws == ws) {
                c.usage.push((name, Usage::Shared));
            }
            app.data.sessions.push(data::Session {
                ws,
                kind: SessionKind::Project,
                name: leak(&format!("{name} coordinator")),
                agent: "claude",
                model: "sonnet",
                project: name,
                state: Status::Open,
                msgs: vec![],
            });
            app.rebuild_ws_tree(Some((ws, Some(name))));
            if ws == app.workspace {
                app.rebuild_for_workspace(None);
            }
            app.flash(format!("added {name} to {} · its coordinator is in Chat", app.data.workspaces[ws].name));
        }
        forms::FormKind::Requirement => {
            let parent = form.value("refines");
            let parent = (!parent.starts_with("none")).then(|| leak(parent));
            let siblings = app.data.reqs.iter().filter(|r| r.ws == form.ws && r.parent == parent).count();
            let top = app.data.reqs.iter().filter(|r| r.ws == form.ws).count();
            let id = leak(&match parent {
                Some(p) => format!("{p}.{}", siblings + 1),
                None => format!("R-{}", top + 40),
            });
            app.data.reqs.push(data::Requirement {
                parent,
                ws: form.ws,
                project: form.project,
                id,
                area: form.project.unwrap_or("Workspace"),
                title: leak(form.value("title")),
                text: leak(form.value("text")),
                status: Status::Open,
                items: &[],
            });
            app.highlight_req = Some(id);
            app.flows.borrow_mut().retain(|k, _| !k.starts_with("reqs:"));
            app.flash(format!("added {id}"));
        }
    }
}

fn start_edit(app: &mut App, idx: usize) {
    let cursor = app.cur().detail_cursor;
    let section = &mut app.data.sections[idx];
    if section.name == "Agents" {
        app.flash("mock: probe / authenticate the selected agent");
        return;
    }
    let Some(field) = section.fields.get_mut(cursor) else { return };
    match field.kind {
        FieldKind::Toggle => field.value = if field.value == "on" { "off".into() } else { "on".into() },
        FieldKind::Choice(options) => {
            let pos = options.iter().position(|o| *o == field.value).unwrap_or(0);
            field.value = options[(pos + 1) % options.len()].to_string();
        }
        FieldKind::ReadOnly => app.flash("read-only: htui measures this"),
        FieldKind::Action(done) => {
            if section.name == "Catalogue" {
                app.data.catalogue.updated = "just now".into();
                if let Some(f) = app.data.sections[idx].fields.iter_mut().find(|f| f.label == "last updated") {
                    f.value = "just now".into();
                }
            }
            app.flash(done);
        }
        FieldKind::Secret => app.mode = Mode::Edit(String::new()),
        FieldKind::Text => app.mode = Mode::Edit(field.value.clone()),
    }
}

fn edit_key(app: &mut App, key: KeyEvent) {
    let Mode::Edit(buf) = &mut app.mode else { return };
    match key.code {
        KeyCode::Esc => app.mode = Mode::Normal,
        KeyCode::Enter => {
            let value = std::mem::take(buf);
            app.mode = Mode::Normal;
            let cursor = app.cur().detail_cursor;
            if let Some(idx) = app.cur().tree.selected()
                && let Some(field) = app.data.sections[idx].fields.get_mut(cursor)
            {
                field.value = value;
                let label = field.label;
                app.flash(format!("saved {label}"));
            }
        }
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Char(c) => buf.push(c),
        _ => {}
    }
}

fn filter_key(app: &mut App, key: KeyEvent) {
    let tree = &mut app.cur_mut().tree;
    match key.code {
        KeyCode::Esc => {
            tree.filter.clear();
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => app.mode = Mode::Normal,
        KeyCode::Backspace => {
            tree.filter.pop();
        }
        KeyCode::Down => tree.move_by(1),
        KeyCode::Up => tree.move_by(-1),
        KeyCode::Char(c) => {
            tree.filter.push(c);
            // Land on the first hit, not on a group header.
            tree.cursor = tree.rows().iter().position(|r| matches!(r, RowRef::Entry(_))).unwrap_or(0);
        }
        _ => {}
    }
    app.cur_mut().tree.clamp();
}

fn start_compose(app: &mut App) {
    let Some(idx) = app.cur().tree.selected() else { return };
    let s = &app.data.sessions[idx];
    if views::chat_writable(s) {
        app.cur_mut().focus = Pane::Composer;
        app.mode = Mode::Compose;
    } else {
        let text = format!("view only: {} is working on its own and unlocks when it asks you", s.agent);
        app.flash(text);
    }
}

fn compose_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Normal,
        KeyCode::Enter => {
            let text = std::mem::take(&mut app.compose);
            if text.trim().is_empty() {
                return;
            }
            if let Some(idx) = app.cur().tree.selected() {
                let s = &mut app.data.sessions[idx];
                s.msgs.push(Msg::You(text));
                if matches!(s.kind, SessionKind::Item(_)) {
                    // Answered: the run goes back to working on its own, and the chat locks again.
                    s.state = Status::InProgress;
                    s.msgs.push(Msg::Agent("Thanks, going with that.".into()));
                    app.mode = Mode::Normal;
                } else {
                    s.msgs.push(Msg::Agent("(mock reply) On it. I'll report back here.".into()));
                }
            }
        }
        KeyCode::Backspace => {
            app.compose.pop();
        }
        KeyCode::Char(c) => app.compose.push(c),
        _ => {}
    }
}

/// Answers the selected chat's pending permission; false if there is none.
fn answer_permission(app: &mut App, c: char) -> bool {
    let Some(idx) = app.cur().tree.selected() else { return false };
    let s = &mut app.data.sessions[idx];
    for msg in &mut s.msgs {
        if let Msg::Permission { answer: answer @ None, .. } = msg {
            *answer = Some(match c {
                'a' => "allowed once",
                'A' => "always allowed",
                _ => "rejected",
            });
            s.state = Status::InProgress;
            app.flash(format!("permission {}", if c == 'r' { "rejected" } else { "allowed" }));
            return true;
        }
    }
    false
}

fn switch_tab(app: &mut App, tab: usize) {
    if tab < TABS.len() {
        app.tab = tab;
        app.fix_focus();
    }
}

fn step_subtab(app: &mut App, by: isize) {
    let n = app::subtabs(app.tab).len();
    if n == 0 {
        return;
    }
    let t = app.cur_mut();
    t.subtab = ((t.subtab as isize + by).rem_euclid(n as isize)) as usize;
    t.detail_scroll = 0;
    t.detail_cursor = 0;
}

fn step_preset(app: &mut App, by: isize) {
    let t = app.cur_mut();
    let next = t.layout.preset.step(by);
    t.layout.apply(next);
    t.zoom = false;
    app.fix_focus();
    app.flash(format!("layout: {} (p / P to change)", next.name()));
}

fn toggle_list(app: &mut App) {
    let t = app.cur_mut();
    t.layout.list_on = !t.layout.list_on;
    app.fix_focus();
}

fn toggle_activity(app: &mut App) {
    let t = app.cur_mut();
    t.layout.act_on = !t.layout.act_on;
    app.fix_focus();
}

fn resize_list(app: &mut App, by: i16) {
    let l = &mut app.cur_mut().layout;
    l.list_pct = (l.list_pct as i16 + by).clamp(12, 70) as u16;
    let pct = l.list_pct;
    app.flash(format!("list {pct}%"));
}

fn resize_activity(app: &mut App, by: i16) {
    let l = &mut app.cur_mut().layout;
    l.act_pct = (l.act_pct as i16 + by).clamp(10, 70) as u16;
    let pct = l.act_pct;
    app.flash(format!("activity {pct}%"));
}

fn contains(rect: Option<Rect>, x: u16, y: u16) -> bool {
    rect.is_some_and(|r| r.contains(Position::new(x, y)))
}

fn forward_to_flow(app: &mut App, m: MouseEvent) {
    if let Some(key) = views::flow_key(app)
        && let Some((graph, _)) = app.flows.borrow_mut().get_mut(&key)
    {
        let _ = graph.handle_mouse_event(m);
    }
}

fn on_mouse(app: &mut App, m: MouseEvent) {
    if matches!(app.modal, Some(Modal::NewItem(_))) {
        return modal_mouse(app, m);
    }
    if app.modal.is_some() || app.help {
        return;
    }
    let (x, y) = (m.column, m.row);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if y == 0 {
                let r = &app.rects;
                if (r.workspace.0..r.workspace.1).contains(&x) {
                    go_to_workspaces(app);
                } else if let Some(&(_, _, tab)) = r.tabs.iter().find(|(x0, x1, _)| (*x0..*x1).contains(&x)) {
                    switch_tab(app, tab);
                }
                return;
            }
            if let Some(&(.., i)) = app.rects.subtabs.iter().find(|(x0, x1, row, _)| *row == y && (*x0..*x1).contains(&x)) {
                let t = app.cur_mut();
                t.subtab = i;
                t.detail_scroll = 0;
                t.detail_cursor = 0;
                return;
            }
            // Grabbing the border between two panes starts a resize.
            let r = &app.rects;
            if let Some(list) = r.list
                && !app.cur().zoom
                && (x + 1 == list.right() || x == list.right())
                && (list.y..list.bottom()).contains(&y)
            {
                app.drag = Some(Drag::List);
                return;
            }
            if let Some(act) = r.activity
                && !app.cur().zoom
            {
                let side = app.cur().layout.act_side;
                let on_edge = if side { x == act.x || x + 1 == act.x } else { y == act.y || y + 1 == act.y };
                if on_edge {
                    app.drag = Some(Drag::Activity);
                    return;
                }
            }
            if contains(app.rects.flow, x, y) {
                app.cur_mut().focus = Pane::Detail;
                app.drag = Some(Drag::Flow);
                forward_to_flow(app, m);
                return;
            }
            click(app, x, y);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            let body = app.rects.body;
            match app.drag {
                Some(Drag::List) => {
                    let pct = ((x.saturating_sub(body.x) as u32 + 1) * 100 / body.width.max(1) as u32) as u16;
                    app.cur_mut().layout.list_pct = pct.clamp(12, 70);
                }
                Some(Drag::Activity) => {
                    let side = app.cur().layout.act_side;
                    let pct = if side {
                        (body.right().saturating_sub(x) as u32 * 100 / body.width.max(1) as u32) as u16
                    } else {
                        (body.bottom().saturating_sub(y) as u32 * 100 / body.height.max(1) as u32) as u16
                    };
                    app.cur_mut().layout.act_pct = pct.clamp(10, 70);
                }
                Some(Drag::Flow) => forward_to_flow(app, m),
                Some(Drag::Modal) | None => {}
            }
        }
        MouseEventKind::Up(_) => {
            if app.drag == Some(Drag::Flow) {
                forward_to_flow(app, m);
            }
            app.drag = None;
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let by = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
            if contains(app.rects.flow, x, y) {
                forward_to_flow(app, m);
            } else if contains(app.rects.list, x, y) {
                app.cur_mut().tree.move_by(by);
            } else if contains(app.rects.activity, x, y) {
                let n = views::activity_events(app).len();
                app.act_cursor = (app.act_cursor as isize + by).clamp(0, n.saturating_sub(1) as isize) as usize;
            } else if contains(app.rects.detail, x, y) {
                if app.tab == LOGS {
                    let code = if by > 0 { KeyCode::Down } else { KeyCode::Up };
                    let idx = app.cur().tree.selected().unwrap_or(0);
                    logs_key(app, KeyEvent::new(code, KeyModifiers::NONE), idx, by < 0, by > 0);
                } else {
                    let max = app.rects.detail_max_scroll as isize;
                    let t = app.cur_mut();
                    t.detail_scroll = (t.detail_scroll as isize + by).clamp(0, max) as u16;
                }
            }
        }
        _ => {}
    }
}

/// Dragging the new-item modal's border resizes it; it stays centred.
fn modal_mouse(app: &mut App, m: MouseEvent) {
    let screen = app.rects.screen;
    let rect = modal::new_item_rect(screen, app.new_item_size);
    let (x, y) = (m.column, m.row);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let on_border = (x == rect.x || x + 1 == rect.right()) && (rect.y..rect.bottom()).contains(&y)
                || (y == rect.y || y + 1 == rect.bottom()) && (rect.x..rect.right()).contains(&x);
            if on_border {
                app.drag = Some(Drag::Modal);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) if app.drag == Some(Drag::Modal) => {
            let cx = screen.x + screen.width / 2;
            let cy = screen.y + screen.height / 2;
            let w = (x.abs_diff(cx) * 2 + 1).clamp(60, screen.width);
            let h = (y.abs_diff(cy) * 2 + 1).clamp(16, screen.height);
            app.new_item_size = (w, h);
        }
        MouseEventKind::Up(_) => app.drag = None,
        _ => {}
    }
}

fn click(app: &mut App, x: u16, y: u16) {
    let r = &app.rects;
    if let Some(list) = r.list.filter(|l| l.contains(Position::new(x, y))) {
        let row = r.list_offset + (y.saturating_sub(list.y + 1)) as usize;
        let t = app.cur_mut();
        t.focus = Pane::List;
        if y > list.y && row < t.tree.rows().len() {
            if t.tree.cursor == row
                && let Some(RowRef::Group(gi)) = t.tree.current()
            {
                let open = !t.tree.groups[gi].open;
                t.tree.set_open(open);
            }
            t.tree.cursor = row;
            t.detail_cursor = 0;
            t.detail_scroll = 0;
        }
    } else if contains(r.detail, x, y) {
        if let Some(&(_, i)) = r.field_rows.iter().find(|(row, _)| *row == y) {
            app.cur_mut().detail_cursor = i;
        }
        app.cur_mut().focus = Pane::Detail;
    } else if contains(r.composer, x, y) {
        start_compose(app);
    } else if contains(r.activity, x, y) {
        if let Some(&(_, pos)) = r.act_rows.iter().find(|(row, _)| *row == y) {
            app.act_cursor = pos;
        }
        app.cur_mut().focus = Pane::Activity;
    }
}

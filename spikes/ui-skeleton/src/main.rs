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
            let before = app.here();
            on_key(&mut app, KeyEvent::new(code, KeyModifiers::NONE));
            app.track(before);
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
            let before = app.here();
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => on_key(app, key),
                Event::Mouse(mouse) => on_mouse(app, mouse),
                _ => {}
            }
            app.track(before);
            if app.quit || !event::poll(Duration::ZERO)? {
                break;
            }
        }
        app.save();
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
    // A graceful stop only waits for the current agent turns to end: seconds, not minutes.
    let done: Vec<usize> = app.stopping.iter().filter(|(_, at)| at.elapsed() > Duration::from_millis(1800)).map(|(w, _)| *w).collect();
    for w in done {
        app.stopping.remove(&w);
        park_runs(app, w, false);
        let name = app.data.workspaces[w].name;
        let n = app.data.workspaces[w].parked.len();
        app.flash(format!("{name}: {n} run(s) stopped, conversations saved · r resumes"));
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
    if matches!(app.modal, Some(Modal::Links(_))) {
        return links_key(app, key);
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
    // Moving a graph node takes every key until it is placed.
    if app.moving {
        match views::flow_key(app) {
            Some(k) if app.cur().focus == Pane::Detail => return move_key(app, key, &k),
            _ => app.moving = false,
        }
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
        KeyCode::Backspace if app.back_label().is_some() => app.go_history(-1),
        KeyCode::Left if alt => app.go_history(-1),
        KeyCode::Right if alt => app.go_history(1),
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
        KeyCode::Char('r') if app.tab == WORKSPACES && app.cur().subtab == 0 => resume_workspace(app),
        KeyCode::Char('T') if app.tab == WORKSPACES && app.cur().subtab == 0 => cycle_scope_target(app),
        // Backlog list options (a focused graph keeps h for panning).
        KeyCode::Char('h') if app.tab == BACKLOG && !graph_focused(app) => {
            app.show_done = !app.show_done;
            let keep = app.cur().tree.selected();
            app.rebuild_for_workspace(keep);
            let n = app::finished_count(&app.data, app.workspace);
            app.flash(if app.show_done { format!("showing {n} done / closed") } else { format!("{n} done / closed hidden · h shows them") });
        }
        KeyCode::Char('t') if app.tab == BACKLOG => {
            if let Some(idx) = app.cur().tree.selected() {
                let item = &app.data.items[idx];
                let target = app.item_target.get(item.key).map_or(0, |t| t + 1);
                let workflow = app.item_workflow.get(item.key).map_or(0, |w| w + 1);
                app.modal = Some(Modal::RunSetup(modal::RunSetup { item: idx, section: 0, target, workflow }));
            }
        }
        KeyCode::Char('g') if app.tab == BACKLOG && !graph_focused(app) => {
            app.group_by = app.group_by.next();
            let keep = app.cur().tree.selected();
            app.rebuild_for_workspace(keep);
            app.flash(format!("grouped by {} · g changes", app.group_by.name()));
        }
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
                    Some(ReqSel::Link(id)) => jump(app, |app| go_to_requirement(app, id)),
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
            } else if let (KeyCode::Char(' '), None, Some(ci)) = (key.code, project, comps.get(t.detail_cursor)) {
                let c = &mut app.data.components[*ci];
                let on = c.status != Status::InProgress;
                c.status = if on { Status::InProgress } else { Status::Queued };
                let text = if on { format!("{} started · localhost:{}", c.name, c.port) } else { format!("{} stopped · its data stays in its volume", c.name) };
                app.flash(text);
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
    if app.moving {
        return move_key(app, key, flow_key);
    }
    if flow_key.starts_with("wf:") && wf_key(app, key, flow_key) {
        return;
    }
    match key.code {
        KeyCode::Char('M') => {
            let node = app.flows.borrow().get(flow_key).and_then(|(g, _)| g.first_selected_node_id());
            match node {
                Some(id) => {
                    app.moving = true;
                    app.flash(format!("moving {} · arrows move it · Enter / Esc done", id.trim_start_matches("req:")));
                }
                None => app.flash("pick a node first (arrows), then M"),
            }
            return;
        }
        KeyCode::Char('r') => {
            app.flows.borrow_mut().remove(flow_key);
            app.flash("graph laid out again");
            return;
        }
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
        KeyCode::Char('+' | '=') => {
            graph.zoom_in();
            drop(flows);
            app.wheel_hint_off = true;
        }
        KeyCode::Char('-') => {
            graph.zoom_out();
            drop(flows);
            app.wheel_hint_off = true;
        }
        KeyCode::Char('0') => graph.reset_zoom(),
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'j' | 'k' | 'l') => {
            let _ = graph.handle_key_event(key);
        }
        _ => {}
    }
}

/// The workflow editor's keys on a template graph. Returns false for keys the graph handles itself.
fn wf_key(app: &mut App, key: KeyEvent, flow_key: &str) -> bool {
    let Some(wf) = flow_key.split(':').nth(1).and_then(|v| v.parse::<usize>().ok()) else { return false };
    let sel = app.flows.borrow().get(flow_key).and_then(|(g, _)| g.first_selected_node_id());
    if let Some((w, from)) = app.connecting.clone() {
        match key.code {
            KeyCode::Enter => {
                if let Some(to) = sel {
                    app.connecting = None;
                    app.pending_link = Some((w, from, to));
                    app.modal = Some(Modal::Chooser(forms::Chooser { title: "link kind", options: data::LINK_KINDS.to_vec(), cursor: 0 }));
                }
            }
            KeyCode::Esc => {
                app.connecting = None;
                app.flash("not connected");
            }
            _ => return false,
        }
        return true;
    }
    let steps = &app.data.workflows[wf].steps;
    let pos = sel.as_deref().and_then(|id| steps.iter().position(|s| s.id == id));
    match key.code {
        KeyCode::Char('a') => {
            let after = pos.unwrap_or(steps.len().saturating_sub(1));
            app.modal = Some(Modal::Form(forms::Form::step(wf, None, after, "", "claude · sonnet", "")));
        }
        KeyCode::Char('e') | KeyCode::Enter => {
            let Some(p) = pos else { return true };
            let s = &steps[p];
            let parts: Vec<&str> = s.parts.iter().map(|(n, _)| n.as_str()).collect();
            app.modal = Some(Modal::Form(forms::Form::step(wf, Some(p), p, &s.name, &s.agent, &parts.join(", "))));
        }
        KeyCode::Char('x') => {
            let Some(p) = pos else { return true };
            let name = steps[p].name.clone();
            let lines = vec![
                format!("{name} goes, with its links; what led into it now leads to what came after."),
                "Runs already going keep the version they started with.".to_string(),
            ];
            app.modal = Some(Modal::Confirm(modal::Confirm { title: format!("delete step {name}?"), lines, yes: "delete", action: modal::Action::DeleteStep(wf, p) }));
        }
        KeyCode::Char('c') => {
            let Some(id) = sel else { return true };
            let name = pos.map_or(id.clone(), |p| steps[p].name.clone());
            app.connecting = Some((wf, id));
            app.flash(format!("connect from {name}: arrows pick the target, Enter chooses the link (a loop can point back)"));
        }
        KeyCode::Char('i') => {
            let Some(node) = sel else { return true };
            app.modal = Some(Modal::Links(modal::LinksEdit { wf, node, cursor: 0 }));
        }
        _ => return false,
    }
    true
}

/// The links modal: ↑↓ picks a link, ← → changes its kind, d deletes it.
fn links_key(app: &mut App, key: KeyEvent) {
    let Some(Modal::Links(l)) = &mut app.modal else { return };
    let wf = &mut app.data.workflows[l.wf];
    let ids = modal::step_links(wf, &l.node);
    let mut close = false;
    match key.code {
        KeyCode::Esc | KeyCode::Enter => close = true,
        KeyCode::Up => l.cursor = l.cursor.saturating_sub(1),
        KeyCode::Down => l.cursor = (l.cursor + 1).min(ids.len().saturating_sub(1)),
        KeyCode::Left | KeyCode::Right => {
            if let Some(i) = ids.get(l.cursor) {
                let kinds = data::LINK_KINDS;
                let at = kinds.iter().position(|(k, _)| *k == wf.links[*i].kind).unwrap_or(0) as isize;
                let by = if key.code == KeyCode::Left { -1 } else { 1 };
                wf.links[*i].kind = kinds[(at + by).rem_euclid(kinds.len() as isize) as usize].0;
                wf.version += 1;
            }
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if let Some(i) = ids.get(l.cursor) {
                wf.links.remove(*i);
                wf.version += 1;
                l.cursor = l.cursor.min(ids.len().saturating_sub(2));
            }
        }
        _ => {}
    }
    let node = l.node.clone();
    if close {
        app.modal = None;
    }
    app.wf_select = Some(node);
}

/// Adds or edits a workflow step from the step form.
fn save_step(app: &mut App, wf: usize, edit: Option<usize>, after: usize, form: &forms::Form) {
    let name = form.value("name").trim().to_string();
    let agent = form.value("agent").to_string();
    let parts: Vec<(String, String)> = form.value("parts").split(',').map(str::trim).filter(|p| !p.is_empty()).map(|p| (p.to_string(), agent.clone())).collect();
    let w = &mut app.data.workflows[wf];
    let id = match edit {
        Some(p) => {
            let s = &mut w.steps[p];
            s.name = name.clone();
            s.agent = agent;
            s.parts = parts;
            s.id.clone()
        }
        None => {
            let base: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
            let mut id = base.clone();
            let mut n = 2;
            while w.steps.iter().any(|s| s.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            let prev = w.steps.get(after).map(|s| s.id.clone());
            // The new step takes over the previous one's forward links, and follows it.
            if let Some(prev) = &prev {
                for l in w.links.iter_mut().filter(|l| l.from == *prev && !data::is_loop(l.kind)) {
                    l.from = id.clone();
                }
                w.links.push(data::WfLink { from: prev.clone(), to: id.clone(), kind: "next" });
            }
            let at = if prev.is_some() { after + 1 } else { w.steps.len() };
            w.steps.insert(at, data::WfStep { id: id.clone(), name: name.clone(), agent, parts });
            id
        }
    };
    w.version += 1;
    app.wf_select = Some(id);
    app.flash(format!("{} {name} · the template is saved", if edit.is_some() { "edited" } else { "added" }));
}

/// Deletes a workflow step; what led into it now leads to what came after it.
fn delete_step(app: &mut App, wf: usize, p: usize) {
    let w = &mut app.data.workflows[wf];
    let id = w.steps[p].id.clone();
    let preds: Vec<String> = w.links.iter().filter(|l| l.to == id && l.from != id && !data::is_loop(l.kind)).map(|l| l.from.clone()).collect();
    let succs: Vec<(String, &'static str)> = w.links.iter().filter(|l| l.from == id && l.to != id && !data::is_loop(l.kind)).map(|l| (l.to.clone(), l.kind)).collect();
    w.links.retain(|l| l.from != id && l.to != id);
    for a in &preds {
        for (b, kind) in &succs {
            if !w.links.iter().any(|l| l.from == *a && l.to == *b) {
                w.links.push(data::WfLink { from: a.clone(), to: b.clone(), kind });
            }
        }
    }
    let name = w.steps.remove(p).name;
    w.version += 1;
    app.wf_select = w.steps.get(p.saturating_sub(1)).map(|s| s.id.clone());
    app.flash(format!("deleted {name}"));
}

/// `M` on a graph: the arrows (or h j k l) move the selected node; its links follow, and nothing
/// about what it links to changes.
fn move_key(app: &mut App, key: KeyEvent, flow_key: &str) {
    let (dx, dy) = match key.code {
        KeyCode::Left | KeyCode::Char('h') => (-2.0, 0.0),
        KeyCode::Right | KeyCode::Char('l') => (2.0, 0.0),
        KeyCode::Up | KeyCode::Char('k') => (0.0, -1.0),
        KeyCode::Down | KeyCode::Char('j') => (0.0, 1.0),
        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('M') => {
            app.moving = false;
            app.flash("placed · r lays the graph out again");
            return;
        }
        _ => return,
    };
    let mut flows = app.flows.borrow_mut();
    if let Some((graph, _)) = flows.get_mut(flow_key)
        && let Some(id) = graph.first_selected_node_id()
    {
        graph.move_node(&id, (dx, dy));
    }
}

fn graph_focused(app: &App) -> bool {
    app.cur().focus == Pane::Detail && views::flow_key(app).is_some()
}

/// Runs a navigation and records it in the history, so Backspace / Alt ← can return.
fn jump(app: &mut App, go: impl FnOnce(&mut App)) {
    let from = app.here();
    go(app);
    let to = app.here();
    if !from.same(&to) {
        app.record_jump(from, to);
    }
}

/// Enter on a graph node: another item opens that item, the item itself or a run step opens its
/// chat, a requirement opens it in Workspaces.
fn graph_enter(app: &mut App, flow_key: &str) {
    let node = app.flows.borrow().get(flow_key).and_then(|(g, _)| g.first_selected_node_id());
    let Some(node) = node else { return };
    let current = if app.tab == BACKLOG { app.cur().tree.selected().map(|i| app.data.items[i].key) } else { None };
    if flow_key.starts_with("runs:") {
        let Some(idx) = app.cur().tree.selected() else { return };
        let item = &app.data.items[idx];
        let Some(step) = app.steps_of(item).into_iter().find(|s| s.id == node) else { return };
        let key = item.key;
        match (step.id.as_str(), step.status) {
            // htui's own stages have no chat: say what they did.
            ("prepare", Status::Queued) => app.flash(format!("{} has not started yet", step.name)),
            ("prepare", _) => {
                let t = &app.data.targets[app.target_of(key)];
                app.flash(format!("{}: {}", step.name, t.how.first().copied().unwrap_or("").replace("<item>", &key.to_lowercase())));
            }
            ("pick", Status::Queued) => app.flash("the fast pick runs when the run starts"),
            ("collect", Status::Queued) => app.flash("when the run is done, collect brings the branch back here (like scripts/hr collect)"),
            ("collect", _) => app.flash(format!("collect: fetch branch {} from the sandbox and merge it here", key.to_lowercase())),
            ("pick", _) => {
                let (wf, why) = app.workflow_of(item);
                let name = app.data.workflows[wf].name;
                app.flash(match why {
                    Some(why) => format!("fast pick chose {name}: {why} · t changes it"),
                    None => format!("{name}, pinned by you · t changes it"),
                });
            }
            // Not started: nothing to show yet.
            (_, Status::Queued) => app.flash(format!("{} has not started yet", step.name)),
            (_, Status::Done | Status::Closed) => {
                let id: &'static str = Box::leak(step.id.clone().into_boxed_str());
                let agent: &'static str = Box::leak(step.agent.clone().into_boxed_str());
                jump(app, |app| open_archived(app, key, id, &step.name, agent))
            }
            _ => jump(app, |app| open_item_chat(app, key, Some(&step.name))),
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
fn open_archived(app: &mut App, key: &'static str, step: &'static str, name: &str, agent: &'static str) {
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
    select_item(app, idx);
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
        select_item(app, idx);
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
    match app.data.workspaces[ws].state {
        WsState::Open | WsState::Background => {
            let runs = app.running_items(ws);
            if runs.is_empty() && ws == app.workspace {
                return app.flash(format!("nothing is running in {name}"));
            }
            let mut lines = if runs.is_empty() {
                vec!["Nothing is running there; it stops right away.".to_string()]
            } else {
                vec![format!("{} run(s) stop:", runs.len())]
            };
            lines.extend(runs.iter().map(|(k, s, a)| format!("   {k:<9} {s} · {a}: {}", app::graceful_note(a))));
            lines.push(String::new());
            lines.push("Nothing new starts. r resumes them; s again stops them at once.".into());
            app.modal = Some(Modal::Confirm(modal::Confirm { title: format!("stop the runs in {name}?"), lines, yes: "stop", action: modal::Action::Stop(ws) }));
        }
        WsState::Stopping => {
            let runs = app.running_items(ws);
            let mut lines = vec![format!("{} run(s) stop now, mid-turn:", runs.len())];
            lines.extend(runs.iter().map(|(k, s, a)| format!("   {k:<9} {s} · {a}")));
            lines.push(String::new());
            lines.push("Conversations are saved; on resume the interrupted turn runs again.".into());
            app.modal = Some(Modal::Confirm(modal::Confirm { title: format!("force-stop {name}?"), lines, yes: "force stop", action: modal::Action::ForceStop(ws) }));
        }
        WsState::Stopped => app.flash(format!("{name} is already stopped · r resumes its runs · o opens it")),
    }
}

/// Stops workspace `ws`, setting its running items aside. Graceful: each finished its current step
/// and resumes at the next one. Forced: each was cut short and redoes that step on resume.
fn park_runs(app: &mut App, ws: usize, interrupted: bool) {
    let parked: Vec<data::Parked> =
        app.running_items(ws).into_iter().map(|(key, step, agent)| data::Parked { key, step, interrupted, resumable: app::resumable(&agent) }).collect();
    let open = ws == app.workspace;
    let w = &mut app.data.workspaces[ws];
    // The workspace on screen stays open; its runs are what stopped.
    w.state = if open { WsState::Open } else { WsState::Stopped };
    w.parked.extend(parked);
}

/// `r`: a stopped workspace's parked runs start again, in the background (or on screen if it is the
/// open one).
fn resume_workspace(app: &mut App) {
    let Some(ws) = selected_ws(app) else { return };
    let w = &mut app.data.workspaces[ws];
    let name = w.name;
    if w.state == WsState::Stopping {
        return app.flash(format!("{name} is still stopping · s forces it"));
    }
    if w.state != WsState::Stopped && w.parked.is_empty() {
        return app.flash(format!("{name} is running; nothing to resume"));
    }
    let parked = std::mem::take(&mut w.parked);
    let redo = parked.iter().filter(|p| p.interrupted).count();
    if w.state == WsState::Stopped {
        w.state = if ws == app.workspace { WsState::Open } else { WsState::Background };
    }
    let how = if redo > 0 { format!(", {redo} redo the step they were cut in") } else { String::new() };
    app.flash(format!("{name} resumed: {} run(s) pick up where they parked{how}", parked.len()));
}

/// `T` on a workspace or project overview: where its items run, by default. Cycles "inherit" and
/// every selectable target.
fn cycle_scope_target(app: &mut App) {
    let Some(i) = app.selected_entry() else { return };
    let (ws, project) = app.ws_nodes[i];
    let choices: Vec<usize> = (0..app.data.targets.len()).filter(|t| app.data.targets[*t].planned.is_none()).collect();
    let current = match project {
        Some(p) => app.project_target.get(p).copied(),
        None => app.ws_target.get(&ws).copied(),
    };
    // None (inherit) → each target → None.
    let next = match current.and_then(|c| choices.iter().position(|t| *t == c)) {
        None => choices.first().copied(),
        Some(k) => choices.get(k + 1).copied(),
    };
    match (project, next) {
        (Some(p), Some(t)) => _ = app.project_target.insert(p, t),
        (Some(p), None) => _ = app.project_target.remove(p),
        (None, Some(t)) => _ = app.ws_target.insert(ws, t),
        (None, None) => _ = app.ws_target.remove(&ws),
    }
    let (t, from) = match project {
        Some(p) => app.inherited_target(ws, Some(p)),
        None => app.inherited_target(ws, None),
    };
    let what = project.unwrap_or(app.data.workspaces[ws].name);
    let how = if next.is_some() { "set here".to_string() } else { format!("inherited from {from}") };
    app.flash(format!("{what}: items run on {} ({how}) unless an item says otherwise", app.data.targets[t].name));
}

/// Selects item `idx` in the Backlog, showing done items first if it is one of them.
fn select_item(app: &mut App, idx: usize) {
    if !app.tabs[BACKLOG].tree.select(idx) && app::is_finished(app.data.items[idx].status) && !app.show_done {
        app.show_done = true;
        app.rebuild_for_workspace(Some(idx));
        app.flash("showing done items (h hides them)");
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
                2 => Modal::NewItem(modal::NewItem::requirement(ws, project, &app.data)),
                _ => Modal::Chooser(forms::Chooser {
                    title: "new",
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
        Outcome::Choose(kind) if app.pending_link.is_some() => {
            app.modal = None;
            let Some((wf, from, to)) = app.pending_link.take() else { return };
            let w = &mut app.data.workflows[wf];
            let name = |id: &str| w.steps.iter().find(|s| s.id == id).map_or(id.to_string(), |s| s.name.clone());
            let text = format!("{} → {}: {kind}", name(&from), name(&to));
            w.links.retain(|l| !(l.from == from && l.to == to));
            w.links.push(data::WfLink { from: from.clone(), to, kind });
            w.version += 1;
            app.wf_select = Some(from);
            app.flash(text);
        }
        Outcome::Choose(what) => {
            let (ws, project) = app.selected_entry().map_or((app.workspace, None), |i| app.ws_nodes[i]);
            app.modal = Some(match what {
                "workspace" => Modal::Form(forms::Form::workspace()),
                "project" => Modal::Form(forms::Form::project(ws)),
                "component" => Modal::AddComponent(modal::Wizard::new(ws, project)),
                _ => Modal::NewItem(modal::NewItem::requirement(ws, project, &app.data)),
            });
        }
        Outcome::SubmitForm => {
            if let Some(Modal::Form(form)) = app.modal.take() {
                if let forms::FormKind::Step { wf, edit, after } = form.kind {
                    save_step(app, wf, edit, after, &form);
                } else {
                    submit_form(app, form);
                }
            }
        }
        Outcome::Stay => {}
        Outcome::Close => app.modal = None,
        Outcome::Confirmed(action) => {
            app.modal = None;
            match action {
                modal::Action::Stop(ws) => {
                    let name = app.data.workspaces[ws].name;
                    let n = app.running_items(ws).len();
                    app.data.workspaces[ws].state = WsState::Stopping;
                    app.stopping.insert(ws, Instant::now());
                    app.flash(format!("stopping {name}: {n} run(s) stop before their next turn · s again: now"));
                }
                modal::Action::DeleteStep(wf, p) => delete_step(app, wf, p),
                modal::Action::ForceStop(ws) => {
                    let name = app.data.workspaces[ws].name;
                    let n = app.running_items(ws).len();
                    app.stopping.remove(&ws);
                    park_runs(app, ws, true);
                    app.flash(format!("{name}: {n} run(s) stopped mid-turn, conversations saved · r resumes"));
                }
            }
        }
        Outcome::SetRun { item, target, workflow } => {
            app.modal = None;
            let key = app.data.items[item].key;
            match target {
                Some(t) => app.item_target.insert(key, t),
                None => app.item_target.remove(key),
            };
            match workflow {
                Some(wf) => app.item_workflow.insert(key, wf),
                None => app.item_workflow.remove(key),
            };
            let (wf, why) = app.workflow_of(&app.data.items[item]);
            let how = if why.is_some() { "fast pick" } else { "pinned" };
            let (t, from) = app.target_source(key);
            app.flash(format!("{key} runs on {} (from {from}) · workflow {} ({how})", app.data.targets[t].name, app.data.workflows[wf].name));
        }
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
        Outcome::CreateReq { ws, project, draft } => {
            app.modal = None;
            create_requirement(app, ws, project, draft);
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
        // Saved by `save_step` before this is reached.
        forms::FormKind::Step { .. } => {}
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
                parked: vec![],
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
    }
}

/// Adds a requirement from the new-requirement modal, and for a workspace-wide one the project
/// requirements it was split into (each refines it). The cursor lands on it.
fn create_requirement(app: &mut App, ws: usize, project: Option<&'static str>, draft: modal::Draft) {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    // The body's first paragraph is the requirement's text.
    let text = draft.body.lines().skip(1).take_while(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    let id = leak(draft.key);
    app.data.reqs.push(data::Requirement {
        parent: draft.refines,
        ws,
        project,
        id,
        area: project.unwrap_or("Workspace"),
        title: leak(draft.title),
        text: leak(text),
        status: Status::Open,
        items: &[],
    });
    let mut made = Vec::new();
    for s in draft.splits.into_iter().filter(|s| s.on) {
        let sid = leak(s.id);
        made.push(sid);
        app.data.reqs.push(data::Requirement {
            parent: Some(id),
            ws,
            project: Some(s.project),
            id: sid,
            area: s.project,
            title: leak(s.title),
            text: leak(s.text),
            status: Status::Open,
            items: &[],
        });
    }
    app.flows.borrow_mut().retain(|k, _| !k.starts_with("reqs:"));
    // Show it where it lives, with the cursor on it.
    if let Some(i) = app.ws_nodes.iter().position(|n| *n == (ws, project)) {
        app.tabs[WORKSPACES].tree.select(i);
    }
    let (_, sel) = wsview::requirements(app, ws, project, None, theme::Palette::new(app.scheme, WORKSPACES));
    let t = &mut app.tabs[WORKSPACES];
    t.subtab = 2;
    t.req_graph = false;
    t.detail_cursor = sel.iter().position(|(_, s)| *s == ReqSel::Req(id)).unwrap_or(0);
    t.focus = Pane::Detail;
    app.highlight_req = Some(id);
    app.flash(if made.is_empty() { format!("added {id}") } else { format!("added {id} and {}", made.join(", ")) });
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
            app.wheel_seen = true;
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

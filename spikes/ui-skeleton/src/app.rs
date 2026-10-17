//! App state, the list tree, layout presets, and the tiny state file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use ratatui::layout::Rect;

use crate::data::{self, Data, EVERY_WORKSPACE, Level, SessionKind, Status, WsState};
use crate::flow::{Graph, GraphOpts, LinkLabels};
use crate::modal::Modal;
use crate::theme::Scheme;

pub const WORKSPACES: usize = 0;
pub const BACKLOG: usize = 1;
pub const CHAT: usize = 2;
pub const SKILLS: usize = 3;
pub const LOGS: usize = 4;
pub const SETTINGS: usize = 5;
pub const TABS: [&str; 6] = ["workspaces", "backlog", "chat", "toolbox", "logs", "settings"];

pub const BACKLOG_SUBTABS: [&str; 5] = ["body", "runs", "graph", "docs", "notes"];
pub const WS_SUBTABS: [&str; 3] = ["overview", "components", "requirements"];

pub fn subtabs(tab: usize) -> &'static [&'static str] {
    match tab {
        BACKLOG => &BACKLOG_SUBTABS,
        WORKSPACES => &WS_SUBTABS,
        _ => &[],
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    List,
    Detail,
    Activity,
    Composer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    /// Narrow list, wide detail.
    Ide,
    /// The IDE layout with an activity panel underneath.
    IdeActivity,
    /// List, detail and activity side by side (wide screens).
    Columns,
    /// The detail alone.
    Focus,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Ide, Preset::IdeActivity, Preset::Columns, Preset::Focus];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Ide => "IDE",
            Preset::IdeActivity => "IDE + activity",
            Preset::Columns => "three columns",
            Preset::Focus => "focus",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    pub fn step(self, by: isize) -> Self {
        let len = Self::ALL.len() as isize;
        Self::ALL[((self.index() as isize + by).rem_euclid(len)) as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub preset: Preset,
    /// Width of the list, in percent of the body.
    pub list_pct: u16,
    /// Height (bottom) or width (side) of the activity panel, in percent.
    pub act_pct: u16,
    pub list_on: bool,
    pub act_on: bool,
    pub act_side: bool,
}

impl Layout {
    pub fn new(preset: Preset) -> Self {
        let mut layout = Layout { preset, list_pct: 24, act_pct: 28, list_on: true, act_on: false, act_side: false };
        layout.apply(preset);
        layout
    }

    pub fn apply(&mut self, preset: Preset) {
        self.preset = preset;
        (self.list_on, self.act_on, self.act_side) = match preset {
            Preset::Ide => (true, false, false),
            Preset::IdeActivity => (true, true, false),
            Preset::Columns => (true, true, true),
            Preset::Focus => (false, false, false),
        };
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowRef {
    Group(usize),
    Entry(usize),
}

pub struct Group {
    pub name: String,
    pub entries: Vec<usize>,
    pub open: bool,
}

/// A grouped, collapsible, filterable list: the left column of every tab.
pub struct Tree {
    pub groups: Vec<Group>,
    /// Search text per data index.
    pub labels: HashMap<usize, String>,
    pub filter: String,
    pub cursor: usize,
    pub offset: usize,
}

impl Tree {
    fn new<'a>(entries: impl Iterator<Item = (usize, &'a str, String)>) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        let mut labels = HashMap::new();
        for (idx, group, label) in entries {
            labels.insert(idx, label.to_lowercase());
            match groups.iter_mut().find(|g| g.name == group) {
                Some(g) => g.entries.push(idx),
                None => groups.push(Group { name: group.to_string(), entries: vec![idx], open: true }),
            }
        }
        let mut tree = Tree { groups, labels, filter: String::new(), cursor: 0, offset: 0 };
        tree.cursor = 1.min(tree.rows().len().saturating_sub(1));
        tree
    }

    pub fn matches(&self, idx: usize) -> bool {
        self.filter.is_empty() || self.labels.get(&idx).is_some_and(|l| l.contains(&self.filter.to_lowercase()))
    }

    pub fn rows(&self) -> Vec<RowRef> {
        let mut rows = Vec::new();
        for (gi, group) in self.groups.iter().enumerate() {
            let hits: Vec<usize> = group.entries.iter().copied().filter(|i| self.matches(*i)).collect();
            if hits.is_empty() {
                continue;
            }
            rows.push(RowRef::Group(gi));
            // A filter opens every group, so a hit is never hidden.
            if group.open || !self.filter.is_empty() {
                rows.extend(hits.into_iter().map(RowRef::Entry));
            }
        }
        rows
    }

    pub fn current(&self) -> Option<RowRef> {
        self.rows().get(self.cursor).copied()
    }

    pub fn selected(&self) -> Option<usize> {
        match self.current() {
            Some(RowRef::Entry(i)) => Some(i),
            _ => None,
        }
    }

    pub fn select(&mut self, idx: usize) -> bool {
        match self.rows().iter().position(|r| *r == RowRef::Entry(idx)) {
            Some(pos) => {
                self.cursor = pos;
                true
            }
            None => false,
        }
    }

    pub fn count(&self, gi: usize) -> usize {
        self.groups[gi].entries.iter().filter(|i| self.matches(**i)).count()
    }

    pub fn move_by(&mut self, by: isize) {
        let len = self.rows().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as isize + by).clamp(0, len as isize - 1) as usize;
    }

    pub fn clamp(&mut self) {
        self.move_by(0);
    }

    /// Opens or closes the group under the cursor (or the cursor's group).
    pub fn set_open(&mut self, open: bool) {
        let rows = self.rows();
        let gi = match rows.get(self.cursor) {
            Some(RowRef::Group(gi)) => *gi,
            Some(RowRef::Entry(i)) => match self.groups.iter().position(|g| g.entries.contains(i)) {
                Some(gi) => gi,
                None => return,
            },
            None => return,
        };
        self.groups[gi].open = open;
        if !open && let Some(pos) = self.rows().iter().position(|r| *r == RowRef::Group(gi)) {
            self.cursor = pos;
        }
    }
}

pub struct TabState {
    pub tree: Tree,
    pub layout: Layout,
    pub focus: Pane,
    pub zoom: bool,
    pub subtab: usize,
    pub detail_cursor: usize,
    pub detail_scroll: u16,
    /// Runs sub-tab: the step table instead of the flow graph.
    pub runs_table: bool,
    /// Workspaces requirements sub-tab: the graph instead of the list.
    pub req_graph: bool,
}

pub enum Mode {
    Normal,
    Filter,
    Compose,
    Edit(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    List,
    Activity,
    /// Resizing the new-item modal from its border.
    Modal,
    /// A press that started on a rataflow graph: the rest of the gesture goes to it.
    Flow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActFilter {
    All,
    NeedsYou,
    Failures,
    ThisWorkspace,
}

impl ActFilter {
    pub fn next(self) -> Self {
        match self {
            ActFilter::All => ActFilter::NeedsYou,
            ActFilter::NeedsYou => ActFilter::Failures,
            ActFilter::Failures => ActFilter::ThisWorkspace,
            ActFilter::ThisWorkspace => ActFilter::All,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ActFilter::All => "all workspaces",
            ActFilter::NeedsYou => "needs you",
            ActFilter::Failures => "failures",
            ActFilter::ThisWorkspace => "this workspace",
        }
    }
}

pub struct LogsView {
    pub min_level: Level,
    pub follow: bool,
    /// When not following: the first line shown.
    pub top: usize,
    pub next: usize,
    pub last: Instant,
}

/// Where things were drawn last frame, for the mouse.
#[derive(Default)]
pub struct Rects {
    pub tabs: Vec<(u16, u16, usize)>,
    pub workspace: (u16, u16),
    pub body: Rect,
    pub list: Option<Rect>,
    pub detail: Option<Rect>,
    pub activity: Option<Rect>,
    pub composer: Option<Rect>,
    /// First list row index drawn, for click-to-select.
    pub list_offset: usize,
    /// Screen row of each selectable detail row drawn, for click-to-select.
    pub field_rows: Vec<(u16, usize)>,
    /// Screen row of each activity event drawn.
    pub act_rows: Vec<(u16, usize)>,
    /// Where a rataflow graph was drawn: the mouse there goes to the graph.
    pub flow: Option<Rect>,
    /// How far the detail can scroll before its end leaves the pane.
    pub detail_max_scroll: u16,
    /// Sub-tab chips: (x0, x1, y, index).
    pub subtabs: Vec<(u16, u16, u16, usize)>,
    /// The whole terminal, for centring modals.
    pub screen: Rect,
}

/// A place in the UI, for going back after a jump.
#[derive(Clone, Debug)]
pub struct Loc {
    pub tab: usize,
    pub cursor: usize,
    pub subtab: usize,
    pub detail_cursor: usize,
    pub focus: Pane,
    pub label: String,
}

impl Loc {
    /// The same place, whatever pane has focus.
    pub fn same(&self, other: &Loc) -> bool {
        (self.tab, self.cursor, self.subtab) == (other.tab, other.cursor, other.subtab)
    }
}

pub struct App {
    pub tab: usize,
    pub tabs: Vec<TabState>,
    pub scheme: Scheme,
    pub about: bool,
    pub help: bool,
    pub mode: Mode,
    pub compose: String,
    pub data: Data,
    pub rects: Rects,
    pub drag: Option<Drag>,
    pub flash: Option<(String, Instant)>,
    pub quit: bool,
    pub workspace: usize,
    pub modal: Option<Modal>,
    /// rataflow graphs and the link kinds each uses, built on first view and kept so pan and
    /// zoom survive redraws.
    pub flows: RefCell<HashMap<String, (Graph, Vec<&'static str>)>>,
    pub graph: GraphOpts,
    /// A requirement reached from a graph, highlighted in the requirements list.
    pub highlight_req: Option<&'static str>,
    /// Printed after the terminal is restored.
    pub goodbye: Option<String>,
    /// Places reached by jumps, browser-style: Backspace / Alt ← go back, Alt → forward.
    pub history: Vec<Loc>,
    /// Where in `history` the user is.
    pub hist_pos: usize,
    /// Backlog: how the list is grouped, and whether done and closed items show.
    pub group_by: GroupBy,
    pub show_done: bool,
    /// A graph node being moved with the keyboard (`M`).
    pub moving: bool,
    /// What the state file holds, so it is only written when something changed.
    saved: String,
    /// Running inside tmux, whose config may keep the mouse wheel from reaching htui.
    pub tmux: bool,
    /// A wheel event has reached htui once: the tmux hint is never needed again.
    pub wheel_seen: bool,
    /// The user zoomed with the keyboard: they know the way round it; stop hinting.
    pub wheel_hint_off: bool,
    /// Set when the last event already moved through the history (a jump, back, forward).
    nav_done: bool,
    /// Where each item runs (an index into `data.targets`); missing: the default, this box.
    pub item_target: HashMap<&'static str, usize>,
    /// The workflow chosen for an item; missing: the fast pick decides.
    pub item_workflow: HashMap<&'static str, usize>,
    /// Targets set on a workspace or a project; items inherit them (item > project > workspace >
    /// Settings).
    pub ws_target: HashMap<usize, usize>,
    pub project_target: HashMap<&'static str, usize>,
    /// Workflow editor: connecting from (workflow, step) until a target is picked.
    pub connecting: Option<(usize, String)>,
    /// A link waiting for its kind from the chooser: (workflow, from, to).
    pub pending_link: Option<(usize, String, String)>,
    /// The step to select when an edited template is drawn again.
    pub wf_select: Option<String>,
    /// The new-item modal's size, kept between uses.
    pub new_item_size: (u16, u16),
    /// The Workspaces list: (workspace, project) per entry; project None is the overview.
    pub ws_nodes: Vec<(usize, Option<&'static str>)>,
    /// When each stopping workspace was asked to stop.
    pub stopping: HashMap<usize, Instant>,
    pub act_filter: ActFilter,
    pub act_cursor: usize,
    pub act_last: Instant,
    pub act_next: usize,
    pub logs: LogsView,
}

impl App {
    pub fn new() -> Self {
        let data = data::mock();
        let ws_nodes = ws_nodes(&data);
        let ws_tree = ws_tree(&data, &ws_nodes);
        let logs_tree = Tree::new(
            std::iter::once((0, "sources", "all".to_string()))
                .chain(data::LOG_TARGETS.iter().enumerate().map(|(i, (_, name))| (i + 1, "sources", name.to_string()))),
        );
        let trees = [
            ws_tree,
            backlog_tree(&data, 0, GroupBy::Project, false),
            chat_tree(&data, 0),
            toolbox_tree(&data),
            logs_tree,
            Tree::new(data.sections.iter().enumerate().map(|(i, s)| (i, s.group, s.name.to_string()))),
        ];
        let tabs = trees
            .into_iter()
            .enumerate()
            .map(|(i, tree)| TabState {
                tree,
                // Backlog starts with the activity panel on, so it is seen; the rest plain IDE.
                layout: Layout::new(if i == BACKLOG { Preset::IdeActivity } else { Preset::Ide }),
                focus: Pane::List,
                zoom: false,
                subtab: 0,
                detail_cursor: 0,
                detail_scroll: 0,
                runs_table: false,
                req_graph: false,
            })
            .collect();
        let mut app = App {
            tab: BACKLOG,
            tabs,
            scheme: Scheme::Both,
            about: true,
            help: false,
            mode: Mode::Normal,
            compose: String::new(),
            data,
            rects: Rects::default(),
            drag: None,
            flash: None,
            quit: false,
            workspace: 0,
            modal: None,
            flows: RefCell::new(HashMap::new()),
            graph: GraphOpts { labels: LinkLabels::Legend, minimap: false, horizontal: false },
            highlight_req: None,
            goodbye: None,
            history: Vec::new(),
            hist_pos: 0,
            group_by: GroupBy::Project,
            show_done: false,
            moving: false,
            saved: String::new(),
            tmux: std::env::var_os("TMUX").is_some(),
            wheel_seen: false,
            wheel_hint_off: false,
            nav_done: false,
            // A few items already set up elsewhere, so the picture is not all "this box".
            item_target: HashMap::from([("MOD-67", 1), ("TOOL-8", 2), ("FEAT-3", 3)]),
            item_workflow: HashMap::new(),
            connecting: None,
            ws_target: HashMap::new(),
            project_target: HashMap::from([("agy", 1)]),
            pending_link: None,
            wf_select: None,
            new_item_size: (96, 28),
            ws_nodes,
            stopping: HashMap::new(),
            act_filter: ActFilter::All,
            act_cursor: 0,
            act_last: Instant::now(),
            act_next: 0,
            logs: LogsView { min_level: Level::Debug, follow: true, top: 0, next: 60, last: Instant::now() },
        };
        app.load();
        app.rebuild_for_workspace(None);
        app.saved = app.state_text();
        app
    }

    /// Rebuilds the Workspaces list after a workspace or project was added, keeping `select`.
    pub fn rebuild_ws_tree(&mut self, select: Option<(usize, Option<&'static str>)>) {
        self.ws_nodes = ws_nodes(&self.data);
        let mut tree = ws_tree(&self.data, &self.ws_nodes);
        if let Some(target) = select
            && let Some(i) = self.ws_nodes.iter().position(|n| *n == target)
        {
            tree.select(i);
        }
        self.tabs[WORKSPACES].tree = tree;
    }

    pub fn workspace_projects(&self) -> Vec<&'static str> {
        self.data.workspaces[self.workspace].projects.clone()
    }

    /// Items running in workspace `ws` (none once it is stopped).
    pub fn running(&self, ws: usize) -> usize {
        let w = &self.data.workspaces[ws];
        if w.state == WsState::Stopped {
            return 0;
        }
        let parked = |key: &str| w.parked.iter().any(|p| p.key == key);
        self.data.items.iter().filter(|i| w.projects.contains(&i.project) && i.status == Status::InProgress && !parked(i.key)).count()
    }

    pub fn needs_you(&self, ws: usize) -> usize {
        let w = &self.data.workspaces[ws];
        self.data.items.iter().filter(|i| w.projects.contains(&i.project) && i.status == Status::AwaitingApproval).count()
    }

    /// Opens workspace `ws`: the one on screen goes to the background and keeps running.
    pub fn open_workspace(&mut self, ws: usize) {
        if ws == self.workspace {
            self.data.workspaces[ws].state = WsState::Open;
            return;
        }
        let old = self.workspace;
        self.data.workspaces[old].state = WsState::Background;
        self.data.workspaces[ws].state = WsState::Open;
        self.stopping.remove(&ws);
        self.workspace = ws;
        self.rebuild_for_workspace(None);
        let running = self.running(old);
        let (old_name, new_name) = (self.data.workspaces[old].name, self.data.workspaces[ws].name);
        self.flash(format!("opened {new_name} · {old_name} in the background ({running} running)"));
    }

    /// Rebuilds the Backlog and Chat lists for the open workspace.
    pub fn rebuild_for_workspace(&mut self, select_item: Option<usize>) {
        let mut tree = backlog_tree(&self.data, self.workspace, self.group_by, self.show_done);
        if let Some(idx) = select_item {
            tree.select(idx);
        }
        self.tabs[BACKLOG].tree = tree;
        self.tabs[BACKLOG].detail_scroll = 0;
        self.tabs[CHAT].tree = chat_tree(&self.data, self.workspace);
    }

    /// The entry the detail shows. In Workspaces a workspace's own row shows its overview.
    pub fn selected_entry(&self) -> Option<usize> {
        let t = self.cur();
        match (self.tab, t.tree.current()?) {
            (_, RowRef::Entry(i)) => Some(i),
            (WORKSPACES, RowRef::Group(gi)) => self.ws_nodes.iter().position(|(w, p)| *w == gi && p.is_none()),
            _ => None,
        }
    }

    /// Where the user is now.
    pub fn here(&self) -> Loc {
        let t = self.cur();
        let entry = match (self.tab, self.selected_entry()) {
            (BACKLOG, Some(i)) => self.data.items[i].key.to_string(),
            (CHAT, Some(i)) => self.data.sessions[i].name.to_string(),
            (WORKSPACES, Some(i)) => {
                let (w, p) = self.ws_nodes[i];
                p.unwrap_or(self.data.workspaces[w].name).to_string()
            }
            _ => TABS[self.tab].to_string(),
        };
        let sub = subtabs(self.tab).get(t.subtab).map_or(String::new(), |s| format!(" · {s}"));
        let pane = match t.focus {
            Pane::List => " · list",
            Pane::Activity => " · activity",
            Pane::Composer => " · composer",
            Pane::Detail => "",
        };
        Loc { tab: self.tab, cursor: t.tree.cursor, subtab: t.subtab, detail_cursor: t.detail_cursor, focus: t.focus, label: format!("{entry}{sub}{pane}") }
    }

    /// After an event: a change of tab, sub-tab or pane goes into the history too (jumps record
    /// themselves). Moving inside a pane does not.
    pub fn track(&mut self, before: Loc) {
        if std::mem::take(&mut self.nav_done) || self.modal.is_some() {
            return;
        }
        let now = self.here();
        if (before.tab, before.subtab, before.focus) != (now.tab, now.subtab, now.focus) {
            self.record_jump(before, now);
        }
    }

    /// A jump from `from` to `to`: like a browser, anything forward of here is dropped.
    pub fn record_jump(&mut self, from: Loc, to: Loc) {
        self.nav_done = true;
        if self.history.is_empty() {
            self.history.push(from);
        } else {
            // Where the user is now may not be where they landed last time: keep the real place.
            self.history[self.hist_pos] = from;
            self.history.truncate(self.hist_pos + 1);
        }
        self.history.push(to);
        if self.history.len() > 50 {
            self.history.remove(0);
        }
        self.hist_pos = self.history.len() - 1;
    }

    pub fn back_label(&self) -> Option<&str> {
        (self.hist_pos > 0).then(|| self.history[self.hist_pos - 1].label.as_str())
    }

    pub fn can_forward(&self) -> bool {
        self.hist_pos + 1 < self.history.len()
    }

    /// Back (-1) or forward (+1) through the history.
    pub fn go_history(&mut self, by: isize) {
        let next = self.hist_pos as isize + by;
        if next < 0 || next as usize >= self.history.len() {
            return;
        }
        self.nav_done = true;
        self.history[self.hist_pos] = self.here();
        self.hist_pos = next as usize;
        let to = self.history[self.hist_pos].clone();
        self.tab = to.tab;
        let t = &mut self.tabs[to.tab];
        t.tree.cursor = to.cursor;
        t.subtab = to.subtab;
        t.detail_cursor = to.detail_cursor;
        t.focus = to.focus;
        self.moving = false;
        self.fix_focus();
        self.flash(format!("{} {}", if by < 0 { "↩" } else { "↪" }, to.label));
    }

    /// The steps of item `item`'s run, from its workflow and target.
    pub fn steps_of(&self, item: &data::Item) -> Vec<crate::flow::Step> {
        let (wf, _) = self.workflow_of(item);
        crate::flow::run_steps(item, &self.data.workflows[wf], &self.data.targets[self.target_of(item.key)])
    }

    /// The runs going on in workspace `ws`: (item, the step it is in, the agent running it).
    pub fn running_items(&self, ws: usize) -> Vec<(&'static str, String, String)> {
        let w = &self.data.workspaces[ws];
        self.data
            .items
            .iter()
            .filter(|i| w.projects.contains(&i.project) && matches!(i.status, Status::InProgress | Status::AwaitingApproval))
            .filter(|i| !w.parked.iter().any(|p| p.key == i.key))
            .filter_map(|i| self.steps_of(i).into_iter().find(|s| s.status != Status::Done).map(|s| (i.key, s.name, s.agent)))
            .collect()
    }

    /// A Settings field's value, e.g. ("Workflows", "fast pick model").
    pub fn setting(&self, section: &str, label: &str) -> String {
        self.data.sections.iter().find(|s| s.name == section).and_then(|s| s.fields.iter().find(|f| f.label == label)).map_or_else(String::new, |f| f.value.clone())
    }

    /// Where item `key` runs: its own choice, else its project's, its workspace's, then Settings'.
    pub fn target_of(&self, key: &str) -> usize {
        self.target_source(key).0
    }

    /// The target for item `key` and where it comes from.
    pub fn target_source(&self, key: &str) -> (usize, String) {
        if let Some(t) = self.item_target.get(key) {
            return (*t, "this item".into());
        }
        let project = self.data.items.iter().find(|i| i.key == key).map_or("", |i| i.project);
        self.inherited_target(self.workspace_of(project), Some(project))
    }

    /// What a project (or, with None, a workspace) inherits when it sets nothing itself.
    pub fn inherited_target(&self, ws: usize, project: Option<&str>) -> (usize, String) {
        if let Some(p) = project
            && let Some(t) = self.project_target.get(p)
        {
            return (*t, format!("project {p}"));
        }
        if let Some(t) = self.ws_target.get(&ws) {
            return (*t, format!("workspace {}", self.data.workspaces[ws].name));
        }
        (self.settings_target(), "Settings".into())
    }

    /// Settings → Targets → default.
    pub fn settings_target(&self) -> usize {
        let value = self.data.sections.iter().find(|s| s.name == "Targets").and_then(|s| s.fields.iter().find(|f| f.label == "default")).map(|f| f.value.clone());
        value.and_then(|v| self.data.targets.iter().position(|t| t.name == v)).unwrap_or(0)
    }

    pub fn workspace_of(&self, project: &str) -> usize {
        self.data.workspaces.iter().position(|w| w.projects.contains(&project)).unwrap_or(self.workspace)
    }

    /// The item's workflow, and whether the fast pick chose it (with its reason).
    pub fn workflow_of(&self, item: &data::Item) -> (usize, Option<&'static str>) {
        match self.item_workflow.get(item.key) {
            Some(wf) => (*wf, None),
            None => {
                let (wf, why) = auto_pick(item);
                (wf, Some(why))
            }
        }
    }

    pub fn cur(&self) -> &TabState {
        &self.tabs[self.tab]
    }

    pub fn cur_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.tab]
    }

    pub fn flash(&mut self, text: impl Into<String>) {
        self.flash = Some((text.into(), Instant::now()));
    }

    /// The panes on screen in the current tab, in focus order.
    pub fn visible_panes(&self) -> Vec<Pane> {
        let t = self.cur();
        if t.zoom {
            return vec![t.focus];
        }
        let mut panes = Vec::new();
        if t.layout.list_on {
            panes.push(Pane::List);
        }
        panes.push(Pane::Detail);
        if self.tab == CHAT {
            panes.push(Pane::Composer);
        }
        if t.layout.act_on {
            panes.push(Pane::Activity);
        }
        panes
    }

    pub fn cycle_focus(&mut self, by: isize) {
        let panes = self.visible_panes();
        let pos = panes.iter().position(|p| *p == self.cur().focus).unwrap_or(0) as isize;
        let next = panes[((pos + by).rem_euclid(panes.len() as isize)) as usize];
        self.cur_mut().focus = next;
    }

    /// Keeps focus on a pane that is still on screen after a layout change.
    pub fn fix_focus(&mut self) {
        if !self.visible_panes().contains(&self.cur().focus) {
            self.cur_mut().focus = Pane::Detail;
        }
    }

    fn state_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".ui-skeleton.state")
    }

    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(Self::state_path()) else { return };
        let kv: HashMap<&str, &str> = text.lines().filter_map(|l| l.split_once('=')).collect();
        if let Some(g) = kv.get("backlog.group").and_then(|v| GroupBy::ALL.iter().find(|g| g.name() == *v)) {
            self.group_by = *g;
        }
        self.wheel_seen = kv.get("wheel_seen").is_some_and(|v| *v == "1");
        self.wheel_hint_off = kv.get("wheel_hint_off").is_some_and(|v| *v == "1");
        if let Some(v) = kv.get("backlog.show_done") {
            self.show_done = *v == "1";
        }
        if let Some(v) = kv.get("about") {
            self.about = *v == "1";
        }
        if let Some((w, h)) = kv.get("new_item_size").and_then(|v| v.split_once('x')) {
            self.new_item_size = (w.parse().unwrap_or(96), h.parse().unwrap_or(28));
        }
        if let Some(v) = kv.get("tab").and_then(|v| v.parse::<usize>().ok()) {
            self.tab = v.min(TABS.len() - 1);
        }
        for (i, name) in TABS.iter().enumerate() {
            let get = |k: &str| kv.get(format!("{name}.{k}").as_str()).copied();
            let layout = &mut self.tabs[i].layout;
            if let Some(p) = get("preset").and_then(|v| v.parse::<usize>().ok()).and_then(|i| Preset::ALL.get(i)) {
                layout.apply(*p);
            }
            if let Some(v) = get("list_pct").and_then(|v| v.parse().ok()) {
                layout.list_pct = v;
            }
            if let Some(v) = get("act_pct").and_then(|v| v.parse().ok()) {
                layout.act_pct = v;
            }
            if let Some(v) = get("list_on") {
                layout.list_on = v == "1";
            }
            if let Some(v) = get("act_on") {
                layout.act_on = v == "1";
            }
        }
        self.fix_focus();
    }

    fn state_text(&self) -> String {
        let mut out = format!(
            "about={}\ntab={}\nnew_item_size={}x{}\nbacklog.group={}\nbacklog.show_done={}\nwheel_seen={}\nwheel_hint_off={}\n",
            u8::from(self.about),
            self.tab,
            self.new_item_size.0,
            self.new_item_size.1,
            self.group_by.name(),
            u8::from(self.show_done),
            u8::from(self.wheel_seen),
            u8::from(self.wheel_hint_off)
        );
        for (i, name) in TABS.iter().enumerate() {
            let l = &self.tabs[i].layout;
            out += &format!(
                "{name}.preset={}\n{name}.list_pct={}\n{name}.act_pct={}\n{name}.list_on={}\n{name}.act_on={}\n",
                l.preset.index(),
                l.list_pct,
                l.act_pct,
                u8::from(l.list_on),
                u8::from(l.act_on)
            );
        }
        out
    }

    /// Writes the state file as soon as anything in it changed, so closing the terminal (or
    /// killing the tmux pane) loses nothing.
    pub fn save(&mut self) {
        let text = self.state_text();
        if text != self.saved && std::fs::write(Self::state_path(), &text).is_ok() {
            self.saved = text;
        }
    }
}

/// Whether an agent can stop between turns and later reopen its saved conversation (ACP session
/// load). Checks and htui's own steps cannot: they stop at once and run again.
pub fn resumable(agent: &str) -> bool {
    ["claude", "codex", "gemini", "rust-reviewer"].iter().any(|a| agent.starts_with(a))
}

/// What happens to one run when a graceful stop reaches it.
pub fn graceful_note(agent: &str) -> &'static str {
    if resumable(agent) { "stops before its next turn; the conversation is saved and resumes" } else { "cannot pause between turns: stops now; the step runs again on resume" }
}

/// The mock fast pick: what a small model would answer from the item's kind and shape, in a second.
pub fn auto_pick(item: &data::Item) -> (usize, &'static str) {
    match item.key.split('-').next().unwrap_or("") {
        "ANA" => (3, "a question to answer, no code"),
        "FIX" => (2, "a bug with a known cause"),
        "CLEAN" => (2, "a small, contained change"),
        _ if item.deps.len() > 1 || item.title.to_lowercase().contains("keymap") => (1, "touches several areas; needs prior art"),
        _ => (0, "one area, a clear goal"),
    }
}

/// How the Backlog list is grouped (`g` cycles).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupBy {
    Project,
    Kind,
    Status,
}

impl GroupBy {
    pub const ALL: [GroupBy; 3] = [GroupBy::Project, GroupBy::Kind, GroupBy::Status];

    pub fn name(self) -> &'static str {
        match self {
            GroupBy::Project => "project",
            GroupBy::Kind => "kind",
            GroupBy::Status => "status",
        }
    }

    pub fn next(self) -> Self {
        let pos = Self::ALL.iter().position(|g| *g == self).unwrap_or(0);
        Self::ALL[(pos + 1) % Self::ALL.len()]
    }
}

pub fn is_finished(status: Status) -> bool {
    matches!(status, Status::Done | Status::Closed)
}

/// Done and closed items in the open workspace, which `h` shows or hides.
pub fn finished_count(data: &Data, workspace: usize) -> usize {
    let projects = &data.workspaces[workspace].projects;
    data.items.iter().filter(|i| projects.contains(&i.project) && is_finished(i.status)).count()
}

fn backlog_tree(data: &Data, workspace: usize, group_by: GroupBy, show_done: bool) -> Tree {
    // What needs attention first.
    const STATUS_ORDER: [Status; 8] = [
        Status::AwaitingApproval,
        Status::Failed,
        Status::InProgress,
        Status::Blocked,
        Status::Queued,
        Status::Open,
        Status::Done,
        Status::Closed,
    ];
    let projects = &data.workspaces[workspace].projects;
    let mut entries: Vec<(usize, &str, String)> = data
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| projects.contains(&i.project) && (show_done || !is_finished(i.status)))
        .map(|(idx, i)| {
            let group = match group_by {
                GroupBy::Project => i.project,
                GroupBy::Kind => i.key.split('-').next().unwrap_or(i.key),
                GroupBy::Status => i.status.label(),
            };
            (idx, group, format!("{} {}", i.key, i.title))
        })
        .collect();
    match group_by {
        GroupBy::Project => {}
        GroupBy::Kind => entries.sort_by(|a, b| a.1.cmp(b.1)),
        GroupBy::Status => entries.sort_by_key(|(idx, ..)| STATUS_ORDER.iter().position(|s| *s == data.items[*idx].status)),
    }
    Tree::new(entries.into_iter())
}

/// htui's own agent first, then the workspace's coordinator, the projects' coordinators, every
/// chat running for an item, and finished steps' transcripts (folded).
pub fn chat_tree(data: &Data, workspace: usize) -> Tree {
    const GROUPS: [&str; 5] = ["htui agent", "workspace", "projects", "item runs", "archived"];
    let mut entries: Vec<(usize, &str, String)> = data
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| s.ws == workspace || s.ws == EVERY_WORKSPACE)
        .map(|(i, s)| {
            let group = match s.kind {
                SessionKind::Global => GROUPS[0],
                SessionKind::Workspace => GROUPS[1],
                SessionKind::Project => GROUPS[2],
                SessionKind::Item(_) => GROUPS[3],
                SessionKind::Archived(..) => GROUPS[4],
            };
            (i, group, s.name.to_string())
        })
        .collect();
    entries.sort_by_key(|(_, g, _)| GROUPS.iter().position(|x| x == g));
    let mut tree = Tree::new(entries.into_iter());
    for g in tree.groups.iter_mut().filter(|g| g.name == "archived") {
        g.open = false;
    }
    tree
}

/// The Toolbox list: workflows first (entries 0..n), then skills, personas, templates, mods and MCP
/// servers (entry n + i is `skills[i]`).
fn toolbox_tree(data: &Data) -> Tree {
    let n = data.workflows.len();
    Tree::new(
        data.workflows
            .iter()
            .enumerate()
            .map(|(i, w)| (i, "workflows", w.name.to_string()))
            .chain(data.skills.iter().enumerate().map(move |(i, s)| (n + i, s.group, s.name.to_string()))),
    )
}

fn ws_nodes(data: &Data) -> Vec<(usize, Option<&'static str>)> {
    data.workspaces
        .iter()
        .enumerate()
        .flat_map(|(w, ws)| std::iter::once((w, None)).chain(ws.projects.iter().map(move |p| (w, Some(*p)))))
        .collect()
}

fn ws_tree(data: &Data, nodes: &[(usize, Option<&'static str>)]) -> Tree {
    Tree::new(nodes.iter().enumerate().map(|(i, (w, p))| (i, data.workspaces[*w].name, p.map_or("overview".to_string(), |p| p.to_string()))))
}

//! Centred modals: "new item" (describe → AI rewrite → review, or talk it through with an agent)
//! and "add component" (type → kind → name and port → the generated compose file).

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::App;
use crate::data::{CatKind, Component, Data, Status, Usage};
use crate::theme::{self, INK, Palette, SEL_BG};

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

pub enum Modal {
    NewItem(NewItem),
    /// Asks before something hard to undo; Enter does it.
    Confirm(Confirm),
    /// Where an item runs, and with which workflow.
    RunSetup(RunSetup),
    /// The links of one workflow step: change their kind or delete them.
    Links(LinksEdit),
    AddComponent(Wizard),
    Quit,
    Chooser(crate::forms::Chooser),
    Form(crate::forms::Form),
}

#[derive(Clone)]
pub struct Draft {
    pub key: String,
    pub title: String,
    pub body: String,
    /// Requirements: the one it refines, if any.
    pub refines: Option<&'static str>,
    /// Workspace requirements: the project requirements the agent split it into.
    pub splits: Vec<Split>,
}

/// One project's share of a workspace requirement.
#[derive(Clone)]
pub struct Split {
    pub project: &'static str,
    pub id: String,
    pub title: String,
    pub text: String,
    /// Unticked splits are not created.
    pub on: bool,
}

/// What the modal creates.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum What {
    Item,
    /// A requirement in workspace `ws`; the scope picker's first entry is the workspace itself.
    Requirement { ws: usize },
}

#[derive(PartialEq, Eq)]
pub enum Stage {
    /// Typing what you want.
    Describe,
    /// The model is rewriting it.
    Refining,
    /// Looking at the rewrite.
    Review,
    /// Talking it through with an agent; the draft updates beside the chat.
    Chat,
}

pub struct NewItem {
    pub what: What,
    pub stage: Stage,
    pub projects: Vec<&'static str>,
    /// Items: an index into `projects`. Requirements: 0 is workspace-wide, n is `projects[n - 1]`.
    pub project: usize,
    /// Requirements it may refine (cycled with `p` in review).
    pub parents: Vec<&'static str>,
    /// Review: the split under the cursor.
    pub split_cursor: usize,
    pub input: String,
    pub draft: Option<Draft>,
    pub variant: usize,
    pub chat: Vec<(bool, String)>,
    pub chat_input: String,
    pub turn: usize,
    /// When the mock model started "thinking".
    pub busy: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WStep {
    Type,
    Kind,
    Version,
    /// Only for kinds that emulate services (cloud emulators).
    Services,
    Name,
    Compose,
}

pub struct Wizard {
    pub ws: usize,
    /// Some: a component only this project uses.
    pub project: Option<&'static str>,
    pub step: WStep,
    pub type_idx: usize,
    pub kind_idx: usize,
    pub version_idx: usize,
    pub services: Vec<&'static str>,
    pub service_cursor: usize,
    pub name: String,
    pub port: String,
    /// Name step: 0 edits the name, 1 the port.
    pub field: usize,
    /// When a catalogue update was asked for.
    pub updating: Option<Instant>,
}

pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    /// The Enter key's label, e.g. "stop".
    pub yes: &'static str,
    pub action: Action,
}

#[derive(Clone, Copy)]
pub enum Action {
    Stop(usize),
    ForceStop(usize),
    /// Delete step `.1` of workflow `.0`.
    DeleteStep(usize, usize),
}

pub struct LinksEdit {
    pub wf: usize,
    pub node: String,
    pub cursor: usize,
}

pub struct RunSetup {
    pub item: usize,
    /// 0: where it runs, 1: the workflow.
    pub section: usize,
    /// 0 is "inherit"; n is `targets[n - 1]`.
    pub target: usize,
    /// 0 is "let the fast pick choose"; n is `workflows[n - 1]`.
    pub workflow: usize,
}

/// What a modal asks the app to do.
pub enum Outcome {
    Confirmed(Action),
    /// Run setup chosen: target, and workflow (None: the fast pick).
    SetRun { item: usize, target: Option<usize>, workflow: Option<usize> },
    Stay,
    Close,
    Create { project: &'static str, draft: Draft },
    /// A requirement: workspace-wide (project None) with its project splits, or a project's.
    CreateReq { ws: usize, project: Option<&'static str>, draft: Draft },
    AddComponent(Component),
    /// Close the window; the work goes on (or is stopped first).
    Leave { stop_all: bool },
    /// The chooser picked what to create.
    Choose(&'static str),
    /// The form is filled in: the app reads it out of the modal.
    SubmitForm,
}

impl NewItem {
    pub fn new(projects: Vec<&'static str>) -> Self {
        NewItem {
            what: What::Item,
            stage: Stage::Describe,
            projects,
            project: 0,
            parents: Vec::new(),
            split_cursor: 0,
            input: String::new(),
            draft: None,
            variant: 0,
            chat: Vec::new(),
            chat_input: String::new(),
            turn: 0,
            busy: None,
        }
    }

    /// A requirement for workspace `ws`, scoped to `project` (None: workspace-wide).
    pub fn requirement(ws: usize, project: Option<&'static str>, data: &Data) -> Self {
        let projects = data.workspaces[ws].projects.clone();
        let mut n = NewItem::new(projects.clone());
        n.what = What::Requirement { ws };
        n.project = project.and_then(|p| projects.iter().position(|x| *x == p)).map_or(0, |i| i + 1);
        n
    }

    pub fn is_req(&self) -> bool {
        matches!(self.what, What::Requirement { .. })
    }

    /// The scope picked: None is the workspace, or the item's / requirement's project.
    pub fn scope(&self) -> Option<&'static str> {
        match self.what {
            What::Item => self.projects.get(self.project).copied(),
            What::Requirement { .. } => self.project.checked_sub(1).and_then(|i| self.projects.get(i)).copied(),
        }
    }

    fn scope_len(&self) -> usize {
        self.projects.len() + usize::from(self.is_req())
    }

    fn redraft(&self, input: &str, variant: usize, data: &Data) -> Draft {
        match self.what {
            What::Item => rewrite(input, variant, data),
            What::Requirement { ws } => rewrite_req(input, variant, data, ws, self.scope()),
        }
    }

    /// Finishes whatever the mock model was doing, once it has "thought" long enough.
    pub fn tick(&mut self, data: &Data) {
        let Some(started) = self.busy else { return };
        if started.elapsed() < Duration::from_millis(1100) {
            return;
        }
        self.busy = None;
        match self.stage {
            Stage::Refining => {
                self.draft = Some(self.redraft(&self.input, self.variant, data));
                self.split_cursor = 0;
                self.parents = match self.what {
                    What::Item => Vec::new(),
                    What::Requirement { ws } => data.reqs.iter().filter(|r| r.ws == ws && may_refine(r, self.scope())).map(|r| r.id).collect(),
                };
                self.stage = Stage::Review;
            }
            Stage::Chat => {
                let last = self.chat.iter().rev().find(|(you, _)| *you).map(|(_, t)| t.clone()).unwrap_or_default();
                let reply = self.agent_reply(&last, data);
                self.chat.push((false, reply));
                self.turn += 1;
            }
            _ => {}
        }
    }

    fn agent_reply(&mut self, said: &str, data: &Data) -> String {
        if self.draft.is_none() {
            let seed = if self.input.trim().is_empty() { said.to_string() } else { self.input.clone() };
            self.draft = Some(self.redraft(&seed, 0, data));
        }
        let req = self.is_req();
        let workspace_wide = req && self.scope().is_none();
        let draft = self.draft.as_mut().expect("set above");
        let checks = if req { "# How to check\n" } else { "# Acceptance\n" };
        match self.turn {
            0 if workspace_wide => "Got it. Does it touch every project, or only some? I split it into one requirement per project; tell me which to leave out. And how would you check it is true?".into(),
            0 if req => "Got it. How would you check it is true? One observable thing is enough.".into(),
            0 => "Got it. Who is it for: just you, or the whole team? And what is the one thing it must do on day one?".into(),
            1 => {
                draft.body = draft.body.replace(checks, &format!("{checks}- {}\n", said.trim()));
                if req { "Added that as the first check. Anything it must not do?".into() } else { "Added that as the first acceptance criterion. Anything explicitly out of scope?".into() }
            }
            2 => {
                draft.body.push_str(&format!("\n\n# Out of scope\n- {}", said.trim()));
                "Noted as out of scope. I think the draft is ready: Ctrl-D to review it, or keep talking.".into()
            }
            _ => {
                draft.body.push_str(&format!("\n- {}", said.trim()));
                "Folded that into the draft.".into()
            }
        }
    }
}

impl Wizard {
    pub fn new(ws: usize, project: Option<&'static str>) -> Self {
        Wizard {
            ws,
            project,
            step: WStep::Type,
            type_idx: 0,
            kind_idx: 0,
            version_idx: 0,
            services: Vec::new(),
            service_cursor: 0,
            name: String::new(),
            port: String::new(),
            field: 0,
            updating: None,
        }
    }

    pub fn kind<'a>(&self, data: &'a Data) -> &'a CatKind {
        let kinds = &data.catalogue.types[self.type_idx].kinds;
        &kinds[self.kind_idx.min(kinds.len() - 1)]
    }

    fn steps(&self, data: &Data) -> Vec<WStep> {
        let mut steps = vec![WStep::Type, WStep::Kind, WStep::Version];
        let k = self.kind(data);
        if !k.services.is_empty() && !all_services(k) {
            steps.push(WStep::Services);
        }
        steps.extend([WStep::Name, WStep::Compose]);
        steps
    }

    fn step_by(&mut self, data: &Data, by: isize) {
        let steps = self.steps(data);
        let pos = steps.iter().position(|s| *s == self.step).unwrap_or(0) as isize;
        self.step = steps[(pos + by).clamp(0, steps.len() as isize - 1) as usize];
    }

    /// A free name and port for the chosen kind.
    fn defaults(&mut self, data: &Data) {
        let taken: Vec<&Component> = data.components.iter().filter(|c| c.ws == self.ws).collect();
        let base = match data.catalogue.types[self.type_idx].name {
            "database" => "db",
            "cache" => "cache",
            "queue" => "queue",
            "cloud emulator" => "cloud",
            "object storage" => "storage",
            "search" => "search",
            _ => "service",
        };
        let base = match self.project {
            Some(p) => format!("{p}-{base}"),
            None => base.to_string(),
        };
        let mut name = base.clone();
        let mut n = 2;
        while taken.iter().any(|c| c.name == name) {
            name = format!("{base}{n}");
            n += 1;
        }
        let mut port = self.kind(data).port;
        while taken.iter().any(|c| c.port == port || c.usage.iter().any(|(_, u)| *u == Usage::Own(port))) {
            port += 1;
        }
        self.name = name;
        self.port = port.to_string();
    }

    fn component(&self, data: &Data) -> Component {
        let k = self.kind(data);
        let version = k.versions[self.version_idx.min(k.versions.len() - 1)];
        let projects = &data.workspaces[self.ws].projects;
        let usage = projects
            .iter()
            .map(|p| match self.project {
                None => (*p, Usage::Shared),
                Some(only) if only == *p => (*p, Usage::Shared),
                Some(_) => (*p, Usage::Unused),
            })
            .collect();
        Component {
            ws: self.ws,
            name: self.name.trim().to_string(),
            ctype: data.catalogue.types[self.type_idx].name,
            kind: k.name,
            image: format!("{}:{version}", k.image),
            port: self.port.parse().unwrap_or(k.port),
            status: Status::Queued,
            services: self.services.clone(),
            usage,
        }
    }
}

/// The mock "LLM rewrite": a kind guessed from the words, a title, and a structured body.
fn rewrite(input: &str, variant: usize, data: &Data) -> Draft {
    let text = input.trim();
    let lower = text.to_lowercase();
    let kind = if ["fix", "bug", "crash", "broken", "leak"].iter().any(|w| lower.contains(w)) {
        "FIX"
    } else if ["why", "investigate", "analy", "compare", "should we"].iter().any(|w| lower.contains(w)) {
        "ANA"
    } else if ["clean", "remove", "refactor", "split", "drop"].iter().any(|w| lower.contains(w)) {
        "CLEAN"
    } else {
        "MOD"
    };
    let next = data
        .items
        .iter()
        .filter_map(|i| i.key.strip_prefix(kind).and_then(|n| n.strip_prefix('-')).and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0)
        + 1;
    let first = text.split(['.', '\n']).next().unwrap_or(text).trim();
    let mut title: String = first.chars().take(56).collect();
    if let Some(c) = title.get(..1) {
        title = c.to_uppercase() + &title[1..];
    }
    if title.is_empty() {
        title = "Untitled".into();
    }
    let goal = if variant.is_multiple_of(2) {
        format!("{title}, so that it works end to end from the UI.")
    } else {
        format!("Make it possible to: {}.", first.to_lowercase())
    };
    Draft {
        key: format!("{kind}-{next}"),
        title,
        body: format!(
            "# Goal\n{goal}\n\n# Acceptance\n- the main path is covered by an integration test\n- failures show a next step, not an error code\n\n# Open questions\n- does this need a setting, or is one behaviour enough?"
        ),
        refines: None,
        splits: Vec::new(),
    }
}

/// What a new requirement in `scope` may refine: a workspace one only another workspace one; a
/// project one its own project's, or a workspace one (as a share of it).
fn may_refine(r: &crate::data::Requirement, scope: Option<&str>) -> bool {
    r.project.is_none() || (scope.is_some() && r.project == scope)
}

/// A title from the first sentence: capitalised, cut at a word boundary.
fn title_of(first: &str) -> String {
    let mut title = String::new();
    for word in first.split_whitespace() {
        if title.chars().count() + word.chars().count() + 1 > 72 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    if let Some(c) = title.get(..1) {
        title = c.to_uppercase() + &title[1..];
    }
    if title.is_empty() { "Untitled".into() } else { title }
}

/// The mock "LLM rewrite" of a requirement: one line that must be true, how to check it, and, for a
/// workspace-wide one, one requirement per project it touches.
fn rewrite_req(input: &str, variant: usize, data: &Data, ws: usize, project: Option<&'static str>) -> Draft {
    let text = input.trim();
    let first = text.split(['.', '\n']).next().unwrap_or(text).trim();
    let title = title_of(first);
    // Workspace requirements are W-n; project ones share the workspace's R-n.
    let next = |prefix: &str, skip: usize| {
        data.reqs
            .iter()
            .filter(|r| r.ws == ws)
            .filter_map(|r| r.id.strip_prefix(prefix).and_then(|n| n.split('.').next()).and_then(|n| n.parse::<u32>().ok()))
            .max()
            .unwrap_or(0)
            + 1
            + skip as u32
    };
    let key = match project {
        None => format!("W-{}", next("W-", 0)),
        Some(_) => format!("R-{}", next("R-", 0)),
    };
    let must = if variant.is_multiple_of(2) { format!("{title}.") } else { format!("It must always be true that: {}.", first.to_lowercase()) };
    // The words that matter, to guess what it refines.
    const COMMON: [&str; 6] = ["every", "project", "projects", "should", "always", "there"];
    let words: Vec<String> =
        text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 4 && !COMMON.contains(w)).map(String::from).collect();
    let refines = data
        .reqs
        .iter()
        .filter(|r| r.ws == ws && may_refine(r, project) && r.parent.is_none())
        .find(|r| words.iter().any(|w| r.title.to_lowercase().contains(w.as_str())))
        .map(|r| r.id);
    let splits = match project {
        Some(_) => Vec::new(),
        None => data.workspaces[ws]
            .projects
            .iter()
            .enumerate()
            .map(|(i, p)| Split {
                project: p,
                id: format!("R-{}", next("R-", i)),
                title: format!("{p}: {}", title.to_lowercase()),
                text: format!("What {key} means for {p}, checked in {p}'s own tests."),
                on: true,
            })
            .collect(),
    };
    Draft {
        key,
        title,
        body: format!("# Must\n{must}\n\n# How to check\n- a test or a check htui can run says yes or no\n\n# Open questions\n- is it for every case, or only the common one?"),
        refines,
        splits,
    }
}

pub fn on_key(modal: &mut Modal, key: KeyEvent, data: &Data) -> Outcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    use crate::forms::{self, FormOutcome};
    let small = match modal {
        Modal::NewItem(n) => return new_item_key(n, key, ctrl),
        Modal::AddComponent(w) => return wizard_key(w, key, ctrl, data),
        Modal::Confirm(c) => {
            return match key.code {
                KeyCode::Enter | KeyCode::Char('y') => Outcome::Confirmed(c.action),
                KeyCode::Esc | KeyCode::Char('n') => Outcome::Close,
                _ => Outcome::Stay,
            };
        }
        Modal::RunSetup(s) => return run_setup_key(s, key, data),
        // Handled by the app, which owns the workflows.
        Modal::Links(_) => return Outcome::Stay,
        Modal::Quit => forms::quit_key(key),
        Modal::Chooser(c) => forms::chooser_key(c, key),
        Modal::Form(f) => forms::form_key(f, key),
    };
    match small {
        FormOutcome::Stay => Outcome::Stay,
        FormOutcome::Close => Outcome::Close,
        FormOutcome::Submit => Outcome::SubmitForm,
        FormOutcome::Choose(what) => Outcome::Choose(what),
        FormOutcome::Leave { stop_all } => Outcome::Leave { stop_all },
    }
}

fn new_item_key(n: &mut NewItem, key: KeyEvent, ctrl: bool) -> Outcome {
    match n.stage {
        Stage::Describe => match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Up => n.project = (n.project + n.scope_len().max(1) - 1) % n.scope_len().max(1),
            KeyCode::Down => n.project = (n.project + 1) % n.scope_len().max(1),
            KeyCode::Tab => {
                n.stage = Stage::Chat;
                if !n.input.trim().is_empty() && n.chat.is_empty() {
                    n.chat.push((true, n.input.clone()));
                    n.busy = Some(Instant::now());
                }
            }
            KeyCode::Enter if !n.input.trim().is_empty() => {
                n.stage = Stage::Refining;
                n.busy = Some(Instant::now());
            }
            KeyCode::Backspace => {
                n.input.pop();
            }
            KeyCode::Char(c) if !ctrl => n.input.push(c),
            _ => {}
        },
        Stage::Refining => {
            if key.code == KeyCode::Esc {
                n.busy = None;
                n.stage = Stage::Describe;
            }
        }
        Stage::Review => match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => match (n.what, &n.draft) {
                (What::Requirement { ws }, Some(d)) => return Outcome::CreateReq { ws, project: n.scope(), draft: d.clone() },
                (What::Item, Some(d)) => {
                    if let Some(p) = n.scope() {
                        return Outcome::Create { project: p, draft: d.clone() };
                    }
                }
                _ => {}
            },
            KeyCode::Up => n.split_cursor = n.split_cursor.saturating_sub(1),
            KeyCode::Down => {
                let len = n.draft.as_ref().map_or(0, |d| d.splits.len());
                n.split_cursor = (n.split_cursor + 1).min(len.saturating_sub(1));
            }
            KeyCode::Char(' ') => {
                if let Some(s) = n.draft.as_mut().and_then(|d| d.splits.get_mut(n.split_cursor)) {
                    s.on = !s.on;
                }
            }
            KeyCode::Char('p') if n.is_req() => {
                if let Some(d) = n.draft.as_mut() {
                    let pos = d.refines.and_then(|r| n.parents.iter().position(|p| *p == r)).map_or(0, |i| i + 1);
                    d.refines = n.parents.get(pos).copied();
                }
            }
            KeyCode::Char('e') => n.stage = Stage::Describe,
            KeyCode::Char('r') => {
                n.variant += 1;
                n.stage = Stage::Refining;
                n.busy = Some(Instant::now());
            }
            KeyCode::Char('t') | KeyCode::Tab => n.stage = Stage::Chat,
            _ => {}
        },
        Stage::Chat => match key.code {
            KeyCode::Esc => n.stage = if n.draft.is_some() { Stage::Review } else { Stage::Describe },
            KeyCode::Char('d') if ctrl => {
                if n.draft.is_some() {
                    n.stage = Stage::Review;
                }
            }
            KeyCode::Enter if !n.chat_input.trim().is_empty() && n.busy.is_none() => {
                let text = std::mem::take(&mut n.chat_input);
                n.chat.push((true, text));
                n.busy = Some(Instant::now());
            }
            KeyCode::Backspace => {
                n.chat_input.pop();
            }
            KeyCode::Char(c) if !ctrl => n.chat_input.push(c),
            _ => {}
        },
    }
    Outcome::Stay
}

fn run_setup_key(s: &mut RunSetup, key: KeyEvent, data: &Data) -> Outcome {
    match key.code {
        KeyCode::Esc => return Outcome::Close,
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => s.section = 1 - s.section,
        KeyCode::Up if s.section == 0 => s.target = s.target.saturating_sub(1),
        KeyCode::Down if s.section == 0 => s.target = (s.target + 1).min(data.targets.len()),
        KeyCode::Up => s.workflow = s.workflow.saturating_sub(1),
        KeyCode::Down => s.workflow = (s.workflow + 1).min(data.workflows.len()),
        KeyCode::Enter => {
            let target = s.target.checked_sub(1);
            if target.is_some_and(|t| data.targets[t].planned.is_some()) {
                return Outcome::Stay;
            }
            return Outcome::SetRun { item: s.item, target, workflow: s.workflow.checked_sub(1) };
        }
        _ => {}
    }
    Outcome::Stay
}

fn wizard_key(w: &mut Wizard, key: KeyEvent, ctrl: bool, data: &Data) -> Outcome {
    if key.code == KeyCode::Char('U') && w.step != WStep::Name {
        w.updating = Some(Instant::now());
        return Outcome::Stay;
    }
    let types = &data.catalogue.types;
    match (w.step, key.code) {
        (WStep::Type, KeyCode::Esc) => return Outcome::Close,
        (_, KeyCode::Esc) => w.step_by(data, -1),
        (WStep::Type, KeyCode::Up) => w.type_idx = w.type_idx.saturating_sub(1),
        (WStep::Type, KeyCode::Down) => w.type_idx = (w.type_idx + 1).min(types.len() - 1),
        (WStep::Kind, KeyCode::Up) => w.kind_idx = w.kind_idx.saturating_sub(1),
        (WStep::Kind, KeyCode::Down) => w.kind_idx = (w.kind_idx + 1).min(types[w.type_idx].kinds.len() - 1),
        (WStep::Version, KeyCode::Up) => w.version_idx = w.version_idx.saturating_sub(1),
        (WStep::Version, KeyCode::Down) => w.version_idx = (w.version_idx + 1).min(w.kind(data).versions.len() - 1),
        (WStep::Services, KeyCode::Up) => w.service_cursor = w.service_cursor.saturating_sub(1),
        (WStep::Services, KeyCode::Down) => w.service_cursor = (w.service_cursor + 1).min(w.kind(data).services.len() - 1),
        (WStep::Services, KeyCode::Char(' ')) => {
            let s = w.kind(data).services[w.service_cursor];
            match w.services.iter().position(|x| *x == s) {
                Some(i) => {
                    w.services.remove(i);
                }
                None => w.services.push(s),
            }
        }
        (WStep::Services, KeyCode::Enter) if w.services.is_empty() => {}
        (WStep::Name, KeyCode::Tab | KeyCode::Up | KeyCode::Down) => w.field = 1 - w.field,
        (WStep::Name, KeyCode::Backspace) => {
            if w.field == 0 { w.name.pop() } else { w.port.pop() };
        }
        (WStep::Name, KeyCode::Char(c)) if !ctrl => {
            if w.field == 0 && (c.is_ascii_alphanumeric() || c == '-') {
                w.name.push(c);
            } else if w.field == 1 && c.is_ascii_digit() && w.port.len() < 5 {
                w.port.push(c);
            }
        }
        (WStep::Name, KeyCode::Enter) if w.name.trim().is_empty() || w.port.parse::<u16>().is_err() => {}
        (WStep::Compose, KeyCode::Enter) => return Outcome::AddComponent(w.component(data)),
        (step, KeyCode::Enter) => {
            if step == WStep::Type {
                w.kind_idx = 0;
            }
            if step == WStep::Kind {
                w.version_idx = 0;
                // floci always emulates everything; other emulators start with two services on.
                let k = w.kind(data);
                w.services = if all_services(k) { k.services.clone() } else { k.services.iter().take(2).copied().collect() };
                w.service_cursor = 0;
            }
            w.step_by(data, 1);
            if w.step == WStep::Name {
                w.defaults(data);
                w.field = 0;
            }
        }
        _ => {}
    }
    Outcome::Stay
}

/// Emulators that always run every service they emulate: no choice to make.
pub fn all_services(k: &CatKind) -> bool {
    k.name == "floci"
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn frame<'a>(title: &str, hints: Line<'a>, pal: Palette) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Line::from(Span::styled(format!(" {title} "), Style::new().fg(INK).bg(pal.accent).add_modifier(Modifier::BOLD))))
        .title_bottom(hints.right_aligned())
}

fn spinner() -> &'static str {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    SPINNER[(ms / 100) as usize % SPINNER.len()]
}

/// "a  ›  b  ›  c" with the current step highlighted and finished ones ticked.
fn crumbs(steps: &[&str], at: usize, pal: Palette) -> Vec<Span<'static>> {
    let mut out = vec![];
    for (i, s) in steps.iter().enumerate() {
        if i > 0 {
            out.push(Span::styled("  ›  ", theme::dim()));
        }
        let style = if i == at { pal.accent().add_modifier(Modifier::BOLD) } else if i < at { theme::text() } else { theme::dim() };
        out.push(Span::styled(format!("{}{s}", if i < at { "✓ " } else { "" }), style));
    }
    out
}

pub fn draw(f: &mut Frame, app: &App, pal: Palette) {
    match &app.modal {
        Some(Modal::NewItem(n)) => draw_new_item(f, n, &app.data, app.new_item_size, pal),
        Some(Modal::AddComponent(w)) => draw_wizard(f, w, app, pal),
        Some(Modal::Confirm(c)) => draw_confirm(f, c, pal),
        Some(Modal::RunSetup(s)) => draw_run_setup(f, s, app, pal),
        Some(Modal::Links(l)) => draw_links(f, l, app, pal),
        Some(Modal::Quit) => crate::forms::draw_quit(f, app, pal),
        Some(Modal::Chooser(c)) => crate::forms::draw_chooser(f, c, pal),
        Some(Modal::Form(form)) => crate::forms::draw_form(f, form, app, pal),
        None => {}
    }
}

/// The links touching a workflow step, in the order the editor walks them.
pub fn step_links(wf: &crate::data::Workflow, node: &str) -> Vec<usize> {
    (0..wf.links.len()).filter(|i| wf.links[*i].from == node || wf.links[*i].to == node).collect()
}

fn draw_links(f: &mut Frame, l: &LinksEdit, app: &App, pal: Palette) {
    let wf = &app.data.workflows[l.wf];
    let ids = step_links(wf, &l.node);
    let name = |id: &str| wf.steps.iter().find(|s| s.id == id).map_or(id.to_string(), |s| s.name.clone());
    let rect = centered(f.area(), 84, ids.len() as u16 + 8);
    f.render_widget(Clear, rect);
    let hints: &[(&str, &str)] = &[("↑↓", "link"), ("← →", "kind"), ("d", "delete"), ("Esc", "done")];
    let block = frame(&format!("links of {}", name(&l.node)), crate::ui::hint_line(hints, pal), pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines = vec![Line::raw("")];
    if ids.is_empty() {
        lines.push(Line::from(Span::styled(" No links. c on the graph connects this step to another.", theme::dim())));
    }
    for (row, i) in ids.iter().enumerate() {
        let link = &wf.links[*i];
        let (color, dotted, _) = crate::flow::rel_style(link.kind);
        let meaning = crate::data::LINK_KINDS.iter().find(|(k, _)| *k == link.kind).map_or("", |(_, m)| m);
        let mut line = Line::from(vec![
            Span::styled(if row == l.cursor { " ▌ " } else { "   " }, pal.accent()),
            Span::styled(format!("{:<16} {} ", name(&link.from), if dotted { "┄┄▶" } else { "──▶" }), theme::text()),
            Span::styled(format!("{:<16}", name(&link.to)), theme::text()),
            Span::styled("‹ ", pal.accent()),
            Span::styled(format!("{:<17}", link.kind), Style::new().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(" › ", pal.accent()),
            Span::styled(meaning, theme::dim()),
        ]);
        if row == l.cursor {
            line = line.style(Style::new().bg(SEL_BG));
        }
        lines.push(crate::ui::fit(line, inner.width));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(" Loops (changes requested, fails, retries) point back to an earlier step; a run unrolls them.", theme::dim())));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_confirm(f: &mut Frame, c: &Confirm, pal: Palette) {
    let w = c.lines.iter().map(|l| l.chars().count()).max().unwrap_or(0).max(c.title.len() + 8) as u16 + 6;
    let rect = centered(f.area(), w.max(56), c.lines.len() as u16 + 4);
    f.render_widget(Clear, rect);
    let block = frame(&c.title, crate::ui::hint_line(&[("Enter", c.yes), ("Esc", "cancel")], pal), pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines = vec![Line::raw("")];
    lines.extend(c.lines.iter().map(|l| Line::from(Span::styled(format!(" {l}"), theme::text()))));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Where an item runs (left) and its workflow (right), each with what it means underneath.
fn draw_run_setup(f: &mut Frame, s: &RunSetup, app: &App, pal: Palette) {
    let data = &app.data;
    let item = &data.items[s.item];
    let rect = centered(f.area(), 124, 30);
    f.render_widget(Clear, rect);
    let hints: &[(&str, &str)] = &[("↑↓", "choose"), ("Tab", "where / workflow"), ("Enter", "save"), ("Esc", "cancel")];
    let block = frame(&format!("how {} runs", item.key), crate::ui::hint_line(hints, pal), pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let [left, _, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
    let head = |text: &str, on: bool| {
        Line::from(Span::styled(text.to_string(), if on { pal.accent().add_modifier(Modifier::BOLD) } else { theme::dim().add_modifier(Modifier::BOLD) }))
    };
    let row = |label: String, note: String, selected: bool, focused: bool, disabled: bool| {
        let mut line = Line::from(vec![
            Span::styled(if selected { "▌ " } else { "  " }, if focused { pal.accent() } else { theme::dim() }),
            Span::styled(format!("{label:<20}"), if disabled { theme::dim() } else if selected { theme::bold() } else { theme::text() }),
            Span::styled(note, theme::dim()),
        ]);
        if selected {
            line = line.style(Style::new().bg(SEL_BG));
        }
        line
    };

    let mut l = vec![head("Where it runs", s.section == 0), Line::raw("")];
    let (inherited, from) = app.inherited_target(app.workspace_of(item.project), Some(item.project));
    l.push(crate::ui::fit(row("inherit".into(), format!("{} (from {from})", data.targets[inherited].name), s.target == 0, s.section == 0, false), left.width));
    for (i, t) in data.targets.iter().enumerate() {
        let note = if t.planned.is_some() { "planned".to_string() } else { format!("{} {}", theme::status_glyph(t.status), t.kind) };
        l.push(crate::ui::fit(row(t.name.to_string(), note, i + 1 == s.target, s.section == 0, t.planned.is_some()), left.width));
    }
    let t = &data.targets[s.target.checked_sub(1).unwrap_or(inherited)];
    l.push(Line::raw(""));
    l.push(Line::from(vec![Span::styled(t.address.to_string(), theme::dim())]));
    l.push(Line::from(Span::styled("htui prepares it there:", theme::bold())));
    let branch = item.key.to_lowercase();
    for step in t.how {
        l.push(Line::from(vec![Span::styled("  • ", pal.accent()), Span::styled(step.replace("<item>", &branch), theme::text())]));
    }
    if let Some(why) = t.planned {
        l.push(Line::from(Span::styled(format!("  {why}"), Style::new().fg(theme::status_color(Status::AwaitingApproval)))));
    }
    let comps: Vec<&str> = data.components.iter().filter(|c| c.usage.iter().any(|(p, u)| *p == item.project && *u != Usage::Unused)).map(|c| c.name.as_str()).collect();
    if !comps.is_empty() {
        l.push(Line::from(Span::styled(format!("  components started there: {}", comps.join(", ")), theme::dim())));
    }
    f.render_widget(Paragraph::new(l).wrap(Wrap { trim: false }), left);

    let (auto, why) = crate::app::auto_pick(item);
    let mut r = vec![head("Workflow", s.section == 1), Line::raw("")];
    let width = right.width;
    r.push(crate::ui::fit(row("auto".into(), format!("fast pick → {}", data.workflows[auto].name), s.workflow == 0, s.section == 1, false), width));
    for (i, w) in data.workflows.iter().enumerate() {
        r.push(crate::ui::fit(row(w.name.to_string(), w.when.to_string(), s.workflow == i + 1, s.section == 1, false), width));
    }
    let wf = &data.workflows[s.workflow.checked_sub(1).unwrap_or(auto)];
    r.push(Line::raw(""));
    if s.workflow == 0 {
        r.push(Line::from(Span::styled(format!("Would pick {} now: {why}.", wf.name), theme::bold())));
    }
    r.push(Line::from(Span::styled(wf.desc.to_string(), theme::text())));
    r.push(Line::from(Span::styled(workflow_chain(wf), theme::dim())));
    r.push(Line::raw(""));
    r.push(Line::from(Span::styled(
        if s.workflow == 0 {
            format!("When the run starts, {} reads the item and picks the workflow (Settings → Workflows).", app.setting("Workflows", "fast pick model"))
        } else {
            "Pinned: the fast pick is skipped.".to_string()
        },
        theme::dim(),
    )));
    let changes = if app.setting("Workflows", "changes at run time") == "automatic" { "on its own" } else { "after asking you" };
    r.push(Line::from(Span::styled(format!("Either way the agent may change it at run time, {changes} (Settings → Workflows)."), theme::dim())));
    f.render_widget(Paragraph::new(r).wrap(Wrap { trim: false }), right);
}

/// "plan → research ×3 → merge → code ⇄ review → gate": a template in one line; ⇄ marks a loop
/// between neighbours, ↺ a loop further back.
pub fn workflow_chain(wf: &crate::data::Workflow) -> String {
    let mut out = String::new();
    for (i, s) in wf.steps.iter().enumerate() {
        if i > 0 {
            let back = wf.links.iter().any(|l| crate::data::is_loop(l.kind) && l.from == s.id && l.to == wf.steps[i - 1].id);
            out.push_str(if back { " ⇄ " } else { " → " });
        }
        out.push_str(&s.name);
        if !s.parts.is_empty() {
            out.push_str(&format!(" ×{}", s.parts.len()));
        }
        let far = wf.links.iter().filter(|l| crate::data::is_loop(l.kind) && l.from == s.id).any(|l| i == 0 || l.to != wf.steps[i - 1].id);
        if far {
            out.push_str(" ↺");
        }
    }
    out
}

/// Where the new-item modal sits for a given size.
pub fn new_item_rect(area: Rect, size: (u16, u16)) -> Rect {
    centered(area, size.0, size.1)
}

fn draw_new_item(f: &mut Frame, n: &NewItem, data: &Data, size: (u16, u16), pal: Palette) {
    let rect = new_item_rect(f.area(), size);
    f.render_widget(Clear, rect);
    let splits = n.draft.as_ref().is_some_and(|d| !d.splits.is_empty());
    let hints: &[(&str, &str)] = match n.stage {
        Stage::Describe if n.is_req() => &[("Enter", "rewrite with AI"), ("Tab", "talk it through"), ("↑↓", "scope"), ("Ctrl ←→↑↓", "resize"), ("Esc", "cancel")],
        Stage::Describe => &[("Enter", "rewrite with AI"), ("Tab", "talk it through"), ("↑↓", "project"), ("Ctrl ←→↑↓", "resize"), ("Esc", "cancel")],
        Stage::Refining => &[("Esc", "stop")],
        Stage::Review if splits => &[("Enter", "create all"), ("↑↓ Space", "keep / drop a split"), ("p", "refines"), ("e", "edit"), ("r", "rewrite"), ("t", "talk"), ("Esc", "cancel")],
        Stage::Review if n.is_req() => &[("Enter", "create"), ("p", "refines"), ("e", "edit"), ("r", "rewrite again"), ("t", "talk it through"), ("Esc", "cancel")],
        Stage::Review => &[("Enter", "create"), ("e", "edit"), ("r", "rewrite again"), ("t", "talk it through"), ("Esc", "cancel")],
        Stage::Chat => &[("Enter", "send"), ("Ctrl-D", "review draft"), ("Esc", "back")],
    };
    let title = match n.what {
        What::Item => "new item".to_string(),
        What::Requirement { ws } => format!("new requirement · {}", data.workspaces[ws].name),
    };
    let block = frame(&title, crate::ui::hint_line(hints, pal), pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };

    let at = match n.stage {
        Stage::Describe | Stage::Chat => 0,
        Stage::Refining => 1,
        Stage::Review => 2,
    };
    let mut head_spans = if n.stage == Stage::Chat {
        vec![Span::styled("talking it through with ", theme::dim()), Span::styled("claude · sonnet", pal.accent().add_modifier(Modifier::BOLD))]
    } else {
        crumbs(&["describe", "rewrite", "review"], at, pal)
    };
    let project = match (n.is_req(), n.scope()) {
        (_, Some(p)) => p.to_string(),
        (true, None) => "workspace-wide".to_string(),
        (false, None) => "no project".to_string(),
    };
    head_spans.push(Span::styled(if n.is_req() { "      for " } else { "      project " }, theme::dim()));
    head_spans.push(Span::styled(format!("‹ {project} ›"), theme::bold()));
    let [head, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Line::from(head_spans), head);

    match n.stage {
        Stage::Describe => {
            let lines = match (n.is_req(), n.scope()) {
                (false, _) => vec![
                    Line::from(Span::styled("What do you want to build or fix?", theme::bold())),
                    Line::from(Span::styled("Write it however it comes; an agent turns it into a proper item for you to check.", theme::dim())),
                    Line::raw(""),
                ],
                (true, None) => vec![
                    Line::from(Span::styled("What must be true across the workspace?", theme::bold())),
                    Line::from(Span::styled("An agent words it as a requirement and splits it into one per project it touches, for you to check.", theme::dim())),
                    Line::raw(""),
                ],
                (true, Some(_)) => vec![
                    Line::from(Span::styled("What must this project do?", theme::bold())),
                    Line::from(Span::styled("Write it however it comes; an agent words it as a requirement you can check.", theme::dim())),
                    Line::raw(""),
                ],
            };
            let [top, field] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(rest);
            f.render_widget(Paragraph::new(lines), top);
            let field_block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(theme::FAINT));
            let example = if n.is_req() { "e.g. every project reports its errors to the same tracker" } else { "e.g. the backlog list should remember which groups I folded" };
            let text = if n.input.is_empty() {
                Line::from(vec![Span::styled("▏", pal.accent()), Span::styled(example, theme::dim())])
            } else {
                Line::from(vec![Span::styled(n.input.clone(), theme::text()), Span::styled("▏", pal.accent())])
            };
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(field_block), field);
        }
        Stage::Refining => {
            let lines = vec![
                Line::raw(""),
                Line::from(vec![Span::styled(format!("{} ", spinner()), pal.accent()), Span::styled("claude · sonnet is rewriting your description…", theme::text())]),
                Line::raw(""),
                Line::from(Span::styled(format!("“{}”", n.input.trim()), theme::dim())),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rest);
        }
        Stage::Review => {
            if let Some(d) = &n.draft {
                let mut lines = draft_lines(d, n.scope(), pal, "(mock rewrite — no model was called)");
                if n.is_req() {
                    lines.extend(req_extra_lines(d, data, n.split_cursor, pal));
                }
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rest);
            }
        }
        Stage::Chat => {
            let [left, right] = Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(rest);
            let draft_block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(theme::FAINT)).title(Span::styled(" draft ", theme::bold()));
            let lines = match &n.draft {
                Some(d) => {
                    let mut lines = draft_lines(d, n.scope(), pal, "updates as you talk");
                    if n.is_req() {
                        lines.extend(req_extra_lines(d, data, usize::MAX, pal));
                    }
                    lines
                }
                None => vec![Line::from(Span::styled("The draft appears once the agent has something to go on.", theme::dim()))],
            };
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(draft_block), left);

            let [chat, input] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(right);
            let mut lines = Vec::new();
            if n.chat.is_empty() {
                lines.push(Line::from(Span::styled(" Tell the agent what you have in mind. It asks questions and fills in the draft.", theme::dim())));
            }
            for (you, text) in &n.chat {
                let (who, color) = if *you { ("you", pal.accent) } else { ("claude", theme::kind_color("MOD")) };
                lines.push(Line::from(vec![Span::styled(" ▍", Style::new().fg(color)), Span::styled(who, Style::new().fg(color).add_modifier(Modifier::BOLD))]));
                lines.push(Line::from(vec![Span::styled(" ▍", Style::new().fg(color)), Span::styled(text.clone(), theme::text())]));
                lines.push(Line::raw(""));
            }
            if n.busy.is_some() {
                lines.push(Line::from(vec![Span::styled(format!(" {} ", spinner()), pal.accent()), Span::styled("thinking…", theme::dim())]));
            }
            let width = chat.width.max(1) as usize;
            let total: usize = lines.iter().map(|l| l.width().max(1).div_ceil(width)).sum();
            let scroll = total.saturating_sub(chat.height as usize) as u16;
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)), chat);
            let input_block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(pal.accent));
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled("› ", pal.accent()), Span::styled(n.chat_input.clone(), theme::text()), Span::styled("▏", pal.accent())]))
                    .block(input_block),
                input,
            );
        }
    }
}

fn draft_lines(d: &Draft, scope: Option<&str>, pal: Palette, note: &str) -> Vec<Line<'static>> {
    let req = d.key.starts_with("W-") || d.key.starts_with("R-");
    let head = if req {
        vec![
            Span::styled("◇ ", Style::new().fg(theme::status_color(Status::AwaitingApproval))),
            Span::styled(d.key.clone(), Style::new().fg(theme::status_color(Status::AwaitingApproval)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", d.title), theme::bold()),
            Span::styled(format!("   {}", scope.unwrap_or("workspace-wide")), theme::dim()),
        ]
    } else {
        vec![theme::glyph(Status::Open), Span::raw(" "), Span::styled(d.key.clone(), pal.key(&d.key)), Span::styled(format!("  {}", d.title), theme::bold())]
    };
    let mut lines = vec![
        Line::from(head),
        Line::from(Span::styled(note.to_string(), theme::dim())),
        Line::raw(""),
    ];
    for l in d.body.lines() {
        lines.push(match l.strip_prefix("# ") {
            Some(h) => Line::from(Span::styled(h.to_string(), pal.accent().add_modifier(Modifier::BOLD))),
            None => match l.strip_prefix("- ") {
                Some(b) => Line::from(vec![Span::styled("  • ", pal.accent()), Span::styled(b.to_string(), theme::text())]),
                None => Line::from(Span::styled(l.to_string(), theme::text())),
            },
        });
    }
    lines
}

/// What only a requirement draft has: what it refines, and the project requirements it splits into.
fn req_extra_lines(d: &Draft, data: &Data, cursor: usize, pal: Palette) -> Vec<Line<'static>> {
    let req_style = Style::new().fg(theme::status_color(Status::AwaitingApproval)).add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::raw(""), Line::from(Span::styled("Refines", pal.accent().add_modifier(Modifier::BOLD)))];
    lines.push(match d.refines.and_then(|id| data.reqs.iter().find(|r| r.id == id)) {
        Some(r) => Line::from(vec![Span::styled("  ↑ ", theme::dim()), Span::styled(r.id, req_style), Span::styled(format!("  {}", r.title), theme::text()), Span::styled("   p changes", theme::dim())]),
        None => Line::from(Span::styled("  nothing: a top-level requirement · p picks a parent", theme::dim())),
    });
    if d.splits.is_empty() {
        return lines;
    }
    let kept = d.splits.iter().filter(|s| s.on).count();
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("Splits into project requirements", pal.accent().add_modifier(Modifier::BOLD)),
        Span::styled(format!("   {kept} of {} kept · each refines {}", d.splits.len(), d.key), theme::dim()),
    ]));
    for (i, s) in d.splits.iter().enumerate() {
        let mut line = Line::from(vec![
            Span::styled(if i == cursor { "▌ " } else { "  " }, pal.accent()),
            Span::styled(if s.on { "[x] " } else { "[ ] " }, if s.on { Style::new().fg(theme::status_color(Status::Done)) } else { theme::dim() }),
            Span::styled("◇ ", if s.on { req_style } else { theme::dim() }),
            Span::styled(format!("{:<7}", s.id), if s.on { req_style } else { theme::dim() }),
            Span::styled(format!("{:<6}", s.project), theme::bold()),
            Span::styled(s.title.clone(), if s.on { theme::text() } else { theme::dim() }),
        ]);
        if i == cursor {
            line = line.style(Style::new().bg(SEL_BG));
        }
        lines.push(line);
    }
    lines
}

fn draw_wizard(f: &mut Frame, w: &Wizard, app: &App, pal: Palette) {
    let data = &app.data;
    let rect = centered(f.area(), 96, 30);
    f.render_widget(Clear, rect);
    let hints: &[(&str, &str)] = match w.step {
        WStep::Type | WStep::Kind | WStep::Version => &[("↑↓", "choose"), ("Enter", "next"), ("U", "update catalogue"), ("Esc", "back")],
        WStep::Services => &[("↑↓", "move"), ("Space", "on / off"), ("Enter", "next"), ("Esc", "back")],
        WStep::Name => &[("Tab", "name / port"), ("Enter", "next"), ("Esc", "back")],
        WStep::Compose => &[("Enter", "add and write compose"), ("Esc", "back")],
    };
    let scope = match w.project {
        Some(p) => format!("add component · only {p}"),
        None => format!("add component · {}", data.workspaces[w.ws].name),
    };
    let block = frame(&scope, crate::ui::hint_line(hints, pal), pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let [head, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let steps = w.steps(data);
    let names: Vec<&str> = steps
        .iter()
        .map(|s| match s {
            WStep::Type => "type",
            WStep::Kind => "kind",
            WStep::Version => "version",
            WStep::Services => "services",
            WStep::Name => "name & port",
            WStep::Compose => "compose",
        })
        .collect();
    let at = steps.iter().position(|s| *s == w.step).unwrap_or(0);
    let mut head_spans = crumbs(&names, at, pal);
    head_spans.push(Span::styled("     catalogue ", theme::dim()));
    head_spans.push(match w.updating {
        Some(_) => Span::styled(format!("{} updating…", spinner()), pal.accent()),
        None => Span::styled(format!("updated {}", data.catalogue.updated), theme::dim()),
    });
    f.render_widget(Line::from(head_spans), head);

    let choice = |label: String, note: String, selected: bool| {
        let mut line = Line::from(vec![
            Span::styled(if selected { "▌ " } else { "  " }, pal.accent()),
            Span::styled(format!("{label:<18}"), if selected { theme::bold() } else { theme::text() }),
            Span::styled(note, theme::dim()),
        ]);
        if selected {
            line = line.style(Style::new().bg(SEL_BG));
        }
        line
    };
    let ty = &data.catalogue.types[w.type_idx];
    let k = w.kind(data);
    match w.step {
        WStep::Type => {
            let mut lines = vec![Line::from(Span::styled("What kind of service?", theme::bold())), Line::raw("")];
            for (i, t) in data.catalogue.types.iter().enumerate() {
                let names: Vec<&str> = t.kinds.iter().map(|k| k.name).collect();
                lines.push(choice(t.name.to_string(), names.join(", "), i == w.type_idx));
            }
            f.render_widget(Paragraph::new(lines), rest);
        }
        WStep::Kind => {
            let mut lines = vec![Line::from(Span::styled(format!("Which {}?", ty.name), theme::bold())), Line::raw("")];
            for (i, k) in ty.kinds.iter().enumerate() {
                let extra = match (k.services.is_empty(), all_services(k)) {
                    (true, _) => String::new(),
                    (false, true) => " · AWS, Azure, GCP, Oracle: all services".to_string(),
                    (false, false) => format!(" · emulates {}", k.services.join(", ")),
                };
                let ports = if all_services(k) { "ports 4566–4599".to_string() } else { format!("port {}", k.port) };
                lines.push(choice(k.name.to_string(), format!("{} · {ports}{extra}", k.image), i == w.kind_idx));
            }
            f.render_widget(Paragraph::new(lines), rest);
        }
        WStep::Version => {
            let mut lines = vec![Line::from(Span::styled(format!("Which {} version?", k.name), theme::bold())), Line::raw("")];
            for (i, v) in k.versions.iter().enumerate() {
                let note = if i == 0 { "newest in the catalogue" } else { "" };
                lines.push(choice(format!("{}:{v}", k.image.rsplit('/').next().unwrap_or(k.image)), note.into(), i == w.version_idx));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("Versions come from the catalogue in the database; U fetches new ones.", theme::dim())));
            f.render_widget(Paragraph::new(lines), rest);
        }
        WStep::Services => {
            let mut lines = vec![Line::from(Span::styled(format!("Which services should {} emulate?", k.name), theme::bold())), Line::raw("")];
            for (i, s) in k.services.iter().enumerate() {
                let on = w.services.contains(s);
                let mut line = Line::from(vec![
                    Span::styled(if i == w.service_cursor { "▌ " } else { "  " }, pal.accent()),
                    Span::styled(if on { "[x] " } else { "[ ] " }, if on { Style::new().fg(theme::status_color(Status::Done)) } else { theme::dim() }),
                    Span::styled(s.to_string(), if on { theme::bold() } else { theme::text() }),
                ]);
                if i == w.service_cursor {
                    line = line.style(Style::new().bg(SEL_BG));
                }
                lines.push(line);
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("Projects get one endpoint for all of them, e.g. AWS_ENDPOINT_URL=http://localhost:4566", theme::dim())));
            f.render_widget(Paragraph::new(lines), rest);
        }
        WStep::Name => {
            let field = |label: &str, value: &str, active: bool, help: &str| {
                Line::from(vec![
                    Span::styled(if active { "▌ " } else { "  " }, pal.accent()),
                    Span::styled(format!("{label:<8}"), if active { pal.accent().add_modifier(Modifier::BOLD) } else { theme::dim() }),
                    Span::styled(format!(" {value}"), Style::new().fg(theme::TEXT).bg(SEL_BG)),
                    Span::styled(if active { "▏" } else { " " }, pal.accent()),
                    Span::styled(format!("   {help}"), theme::dim()),
                ])
            };
            let c = w.component(data);
            let lines = vec![
                Line::from(vec![Span::styled(k.name.to_string(), theme::bold()), Span::styled(format!("  {}", c.image), theme::dim())]),
                Line::raw(""),
                field("name", &w.name, w.field == 0, "the service name in compose, and the prefix of its env vars"),
                Line::raw(""),
                field("port", &w.port, w.field == 1, "exposed on localhost; picked to not clash with the others"),
            ];
            f.render_widget(Paragraph::new(lines), rest);
        }
        WStep::Compose => {
            let c = w.component(data);
            let comment = Some(match w.project {
                Some(p) => format!("only {p}"),
                None => "shared by every project".into(),
            });
            let new = crate::wsview::expand(&c.name, &c.image, c.port, k.port, c.kind, &c.services, comment, true);
            let mut lines = vec![Line::from(vec![
                Span::styled("docker-compose.yml", pal.accent().add_modifier(Modifier::BOLD)),
                Span::styled("  after adding · ", theme::dim()),
                Span::styled("+", Style::new().fg(theme::status_color(Status::Done))),
                Span::styled(" marks what is new", theme::dim()),
            ])];
            lines.extend(crate::wsview::compose_lines(&crate::wsview::services(app, w.ws, new), pal));
            f.render_widget(Paragraph::new(lines), rest);
        }
    }
}

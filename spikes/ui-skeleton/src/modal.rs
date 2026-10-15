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
    pub stage: Stage,
    pub projects: Vec<&'static str>,
    pub project: usize,
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

/// What a modal asks the app to do.
pub enum Outcome {
    Stay,
    Close,
    Create { project: &'static str, draft: Draft },
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
            stage: Stage::Describe,
            projects,
            project: 0,
            input: String::new(),
            draft: None,
            variant: 0,
            chat: Vec::new(),
            chat_input: String::new(),
            turn: 0,
            busy: None,
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
                self.draft = Some(rewrite(&self.input, self.variant, data));
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
            self.draft = Some(rewrite(&seed, 0, data));
        }
        let draft = self.draft.as_mut().expect("set above");
        match self.turn {
            0 => "Got it. Who is it for: just you, or the whole team? And what is the one thing it must do on day one?".into(),
            1 => {
                draft.body = draft.body.replace("# Acceptance\n", &format!("# Acceptance\n- {}\n", said.trim()));
                "Added that as the first acceptance criterion. Anything explicitly out of scope?".into()
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
        if !self.kind(data).services.is_empty() {
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
    }
}

pub fn on_key(modal: &mut Modal, key: KeyEvent, data: &Data) -> Outcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    use crate::forms::{self, FormOutcome};
    let small = match modal {
        Modal::NewItem(n) => return new_item_key(n, key, ctrl),
        Modal::AddComponent(w) => return wizard_key(w, key, ctrl, data),
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
            KeyCode::Up => n.project = (n.project + n.projects.len().max(1) - 1) % n.projects.len().max(1),
            KeyCode::Down => n.project = (n.project + 1) % n.projects.len().max(1),
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
            KeyCode::Enter => {
                if let (Some(d), Some(p)) = (&n.draft, n.projects.get(n.project)) {
                    return Outcome::Create { project: p, draft: d.clone() };
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
                // Emulators start with their first two services on.
                w.services = w.kind(data).services.iter().take(2).copied().collect();
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
        Some(Modal::NewItem(n)) => draw_new_item(f, n, app.new_item_size, pal),
        Some(Modal::AddComponent(w)) => draw_wizard(f, w, app, pal),
        Some(Modal::Quit) => crate::forms::draw_quit(f, app, pal),
        Some(Modal::Chooser(c)) => crate::forms::draw_chooser(f, c, pal),
        Some(Modal::Form(form)) => crate::forms::draw_form(f, form, app, pal),
        None => {}
    }
}

/// Where the new-item modal sits for a given size.
pub fn new_item_rect(area: Rect, size: (u16, u16)) -> Rect {
    centered(area, size.0, size.1)
}

fn draw_new_item(f: &mut Frame, n: &NewItem, size: (u16, u16), pal: Palette) {
    let rect = new_item_rect(f.area(), size);
    f.render_widget(Clear, rect);
    let hints: &[(&str, &str)] = match n.stage {
        Stage::Describe => &[("Enter", "rewrite with AI"), ("Tab", "talk it through"), ("↑↓", "project"), ("Ctrl ←→↑↓", "resize"), ("Esc", "cancel")],
        Stage::Refining => &[("Esc", "stop")],
        Stage::Review => &[("Enter", "create"), ("e", "edit"), ("r", "rewrite again"), ("t", "talk it through"), ("Esc", "cancel")],
        Stage::Chat => &[("Enter", "send"), ("Ctrl-D", "review draft"), ("Esc", "back")],
    };
    let block = frame("new item", crate::ui::hint_line(hints, pal), pal);
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
    let project = n.projects.get(n.project).copied().unwrap_or("no project");
    head_spans.push(Span::styled("      project ", theme::dim()));
    head_spans.push(Span::styled(format!("‹ {project} ›"), theme::bold()));
    let [head, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Line::from(head_spans), head);

    match n.stage {
        Stage::Describe => {
            let lines = vec![
                Line::from(Span::styled("What do you want to build or fix?", theme::bold())),
                Line::from(Span::styled("Write it however it comes; an agent turns it into a proper item for you to check.", theme::dim())),
                Line::raw(""),
            ];
            let [top, field] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(rest);
            f.render_widget(Paragraph::new(lines), top);
            let field_block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(theme::FAINT));
            let text = if n.input.is_empty() {
                Line::from(vec![Span::styled("▏", pal.accent()), Span::styled("e.g. the backlog list should remember which groups I folded", theme::dim())])
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
                f.render_widget(Paragraph::new(draft_lines(d, pal, "(mock rewrite — no model was called)")).wrap(Wrap { trim: false }), rest);
            }
        }
        Stage::Chat => {
            let [left, right] = Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(rest);
            let draft_block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(theme::FAINT)).title(Span::styled(" draft ", theme::bold()));
            let lines = match &n.draft {
                Some(d) => draft_lines(d, pal, "updates as you talk"),
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

fn draft_lines(d: &Draft, pal: Palette, note: &str) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            theme::glyph(Status::Open),
            Span::raw(" "),
            Span::styled(d.key.clone(), pal.key(&d.key)),
            Span::styled(format!("  {}", d.title), theme::bold()),
        ]),
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
                let extra = if k.services.is_empty() { String::new() } else { format!(" · emulates {}", k.services.join(", ")) };
                lines.push(choice(k.name.to_string(), format!("{} · port {}{extra}", k.image, k.port), i == w.kind_idx));
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
            let new = crate::wsview::Service {
                name: c.name.clone(),
                image: c.image.clone(),
                host_port: c.port,
                port: k.port,
                kind: c.kind.to_string(),
                services: c.services.clone(),
                comment: Some(match w.project {
                    Some(p) => format!("only {p}"),
                    None => "shared by every project".into(),
                }),
                new: true,
            };
            let mut lines = vec![Line::from(vec![
                Span::styled("docker-compose.yml", pal.accent().add_modifier(Modifier::BOLD)),
                Span::styled("  after adding · ", theme::dim()),
                Span::styled("+", Style::new().fg(theme::status_color(Status::Done))),
                Span::styled(" marks what is new", theme::dim()),
            ])];
            lines.extend(crate::wsview::compose_lines(&crate::wsview::services(app, w.ws, Some(new)), pal));
            f.render_widget(Paragraph::new(lines), rest);
        }
    }
}

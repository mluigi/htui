//! Small modals: leaving htui, "new what?", and the forms for a workspace or project.
//! Requirements use the new-item modal (describe → AI rewrite → review).

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::App;
use crate::data::{Status, WsState};
use crate::theme::{self, INK, Palette, SEL_BG};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormKind {
    Workspace,
    Project,
    /// A workflow step: edit step `edit`, or add one after step `after`.
    Step { wf: usize, edit: Option<usize>, after: usize },
}

pub struct Field {
    pub label: &'static str,
    pub value: String,
    /// Some: the value is one of these, cycled with ← →.
    pub choices: Option<Vec<String>>,
    pub help: &'static str,
}

pub struct Form {
    pub kind: FormKind,
    pub ws: usize,
    pub fields: Vec<Field>,
    pub cursor: usize,
}

/// "New what?" in the Workspaces tab; also the link-kind picker.
pub struct Chooser {
    pub title: &'static str,
    pub options: Vec<(&'static str, &'static str)>,
    pub cursor: usize,
}

pub enum FormOutcome {
    Stay,
    Close,
    Submit,
    Choose(&'static str),
    Leave { stop_all: bool },
}

impl Form {
    pub fn workspace() -> Self {
        Form {
            kind: FormKind::Workspace,
            ws: 0,
            cursor: 0,
            fields: vec![
                Field { label: "name", value: String::new(), choices: None, help: "what you call it, e.g. Mobile" },
                Field { label: "folder", value: "~/workspaces/".into(), choices: None, help: "where htui keeps its compose file, env and cloned projects" },
                Field { label: "description", value: String::new(), choices: None, help: "one line: what lives here" },
            ],
        }
    }

    pub fn project(ws: usize) -> Self {
        Form {
            kind: FormKind::Project,
            ws,
            cursor: 0,
            fields: vec![
                Field { label: "name", value: String::new(), choices: None, help: "short, used in keys and env names" },
                Field {
                    label: "source",
                    value: "existing folder".into(),
                    choices: Some(vec!["existing folder".into(), "git URL".into(), "new empty repo".into()]),
                    help: "← → to change",
                },
                Field { label: "location", value: "~/projects/".into(), choices: None, help: "the folder, or the URL to clone" },
            ],
        }
    }

    pub fn step(wf: usize, edit: Option<usize>, after: usize, name: &str, persona: &str, agent: &str, parts: &str) -> Self {
        const AGENTS: [&str; 9] = ["any agent", "claude · opus", "claude · sonnet", "claude · haiku", "codex · gpt-5", "gemini · pro", "rust-reviewer", "cargo test", "you"];
        let mut choices: Vec<String> = AGENTS.iter().map(|a| a.to_string()).collect();
        if !choices.iter().any(|c| c == agent) {
            choices.insert(1, agent.to_string());
        }
        let mut personas: Vec<String> = vec!["none (a check)".into()];
        personas.extend(crate::data::PERSONAS.iter().map(|p| p.to_string()));
        let persona = if persona.is_empty() { personas[0].clone() } else { persona.to_string() };
        Form {
            kind: FormKind::Step { wf, edit, after },
            ws: 0,
            cursor: 0,
            fields: vec![
                Field { label: "name", value: name.to_string(), choices: None, help: "what the step does, e.g. security review" },
                Field { label: "persona", value: persona, choices: Some(personas), help: "← → the role the step needs (Toolbox → personas); a check has none" },
                Field { label: "agent", value: agent.to_string(), choices: Some(choices), help: "← → who plays it: any agent (the run picks), or a specific one; \"you\" waits for you" },
                Field { label: "parts", value: parts.to_string(), choices: None, help: "comma-separated for a fan-out (one agent per part), empty for one agent" },
            ],
        }
    }

    pub fn value(&self, label: &str) -> &str {
        self.fields.iter().find(|f| f.label == label).map_or("", |f| f.value.as_str())
    }

    fn valid(&self) -> bool {
        !self.fields[0].value.trim().is_empty()
    }
}

pub fn form_key(form: &mut Form, key: KeyEvent) -> FormOutcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let last = form.fields.len() - 1;
    let cursor = form.cursor;
    let is_choice = form.fields[cursor].choices.is_some();
    if key.code == KeyCode::Enter && cursor == last && form.valid() {
        return FormOutcome::Submit;
    }
    let field = &mut form.fields[cursor];
    match key.code {
        KeyCode::Esc => return FormOutcome::Close,
        KeyCode::Up | KeyCode::BackTab => form.cursor = form.cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Tab => form.cursor = (form.cursor + 1).min(last),
        KeyCode::Left | KeyCode::Right => {
            if let Some(choices) = &field.choices {
                let pos = choices.iter().position(|c| *c == field.value).unwrap_or(0) as isize;
                let by = if key.code == KeyCode::Left { -1 } else { 1 };
                field.value = choices[((pos + by).rem_euclid(choices.len() as isize)) as usize].clone();
                if field.label == "source" {
                    let hint = if field.value == "git URL" { "https://" } else { "~/projects/" };
                    form.fields[2].value = hint.into();
                }
            }
        }
        KeyCode::Enter if cursor < last => form.cursor += 1,
        KeyCode::Backspace if !is_choice => {
            field.value.pop();
        }
        KeyCode::Char(c) if !ctrl && !is_choice => {
            field.value.push(c);
            // The folder follows the name until you edit it yourself.
            if form.kind == FormKind::Workspace && form.cursor == 0 {
                let slug: String = form.fields[0].value.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
                if form.fields[1].value.starts_with("~/workspaces/") {
                    form.fields[1].value = format!("~/workspaces/{slug}");
                }
            }
        }
        _ => {}
    }
    FormOutcome::Stay
}

pub fn chooser_key(c: &mut Chooser, key: KeyEvent) -> FormOutcome {
    match key.code {
        KeyCode::Esc => FormOutcome::Close,
        KeyCode::Up => {
            c.cursor = c.cursor.saturating_sub(1);
            FormOutcome::Stay
        }
        KeyCode::Down => {
            c.cursor = (c.cursor + 1).min(c.options.len() - 1);
            FormOutcome::Stay
        }
        KeyCode::Enter => FormOutcome::Choose(c.options[c.cursor].0),
        KeyCode::Char(ch) => match c.options.iter().find(|(name, _)| name.starts_with(ch)) {
            Some((name, _)) => FormOutcome::Choose(name),
            None => FormOutcome::Stay,
        },
        _ => FormOutcome::Stay,
    }
}

pub fn quit_key(key: KeyEvent) -> FormOutcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') if !ctrl => FormOutcome::Close,
        KeyCode::Enter | KeyCode::Char('l') => FormOutcome::Leave { stop_all: false },
        KeyCode::Char('c') if ctrl => FormOutcome::Leave { stop_all: false },
        KeyCode::Char('S') => FormOutcome::Leave { stop_all: true },
        _ => FormOutcome::Stay,
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn frame<'a>(title: &str, hints: &[(&str, &str)], pal: Palette) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Line::from(Span::styled(format!(" {title} "), Style::new().fg(INK).bg(pal.accent).add_modifier(Modifier::BOLD))))
        .title_bottom(crate::ui::hint_line(hints, pal).right_aligned())
}

pub fn draw_quit(f: &mut Frame, app: &App, pal: Palette) {
    let live: Vec<usize> = (0..app.data.workspaces.len())
        .filter(|w| matches!(app.data.workspaces[*w].state, WsState::Open | WsState::Background | WsState::Stopping))
        .collect();
    let rect = centered(f.area(), 74, 7 + live.len() as u16);
    f.render_widget(Clear, rect);
    let block = frame("leave htui", &[("Enter", "leave, keep running"), ("S", "stop all, then leave"), ("Esc", "stay")], pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines = vec![Line::from(Span::styled(" Leaving closes this window only. Your work keeps going:", theme::bold())), Line::raw("")];
    for w in &live {
        let ws = &app.data.workspaces[*w];
        let place = if *w == app.workspace { "open now" } else { "background" };
        lines.push(Line::from(vec![
            Span::styled(format!("   ▣ {:<12}", ws.name), theme::bold()),
            theme::glyph(Status::InProgress),
            Span::styled(format!(" {} running   ", app.running(*w)), theme::text()),
            theme::glyph(Status::AwaitingApproval),
            Span::styled(format!(" {} waiting for you   ", app.needs_you(*w)), theme::text()),
            Span::styled(place, theme::dim()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(" Questions wait for you; run htui again to pick up where you left.", theme::dim())));
    f.render_widget(Paragraph::new(lines), inner);
}

pub fn draw_chooser(f: &mut Frame, c: &Chooser, pal: Palette) {
    let rect = centered(f.area(), 76, c.options.len() as u16 + 4);
    f.render_widget(Clear, rect);
    let block = frame(c.title, &[("↑↓", "choose"), ("Enter", "pick"), ("Esc", "cancel")], pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines: Vec<Line> = c
        .options
        .iter()
        .enumerate()
        .map(|(i, (name, what))| {
            let selected = i == c.cursor;
            let mut line = Line::from(vec![
                Span::styled(if selected { "▌ " } else { "  " }, pal.accent()),
                Span::styled(name.chars().next().unwrap_or(' ').to_string(), pal.accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
                Span::styled(format!("{:<18}", &name[1..]), if selected { theme::bold() } else { theme::text() }),
                Span::styled(what.to_string(), theme::dim()),
            ]);
            if selected {
                line = line.style(Style::new().bg(SEL_BG));
            }
            line
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner });
}

pub fn draw_form(f: &mut Frame, form: &Form, app: &App, pal: Palette) {
    let title = match form.kind {
        FormKind::Workspace => "new workspace".to_string(),
        FormKind::Project => format!("new project · {}", app.data.workspaces[form.ws].name),
        FormKind::Step { wf, edit: Some(_), .. } => format!("edit step · {}", app.data.workflows[wf].name),
        FormKind::Step { wf, .. } => format!("new step · {}", app.data.workflows[wf].name),
    };
    let rect = centered(f.area(), 86, 24);
    f.render_widget(Clear, rect);
    let block = frame(&title, &[("↑↓ Tab", "field"), ("← →", "choice"), ("Enter", "next / create"), ("Esc", "cancel")], pal);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let [fields_area, preview_area] = Layout::vertical([Constraint::Length(form.fields.len() as u16 * 2 + 1), Constraint::Fill(1)]).areas(inner);

    let mut lines = vec![Line::raw("")];
    for (i, field) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        let mut spans = vec![
            Span::styled(if active { "▌ " } else { "  " }, pal.accent()),
            Span::styled(format!("{:<12}", field.label), if active { pal.accent().add_modifier(Modifier::BOLD) } else { theme::dim() }),
        ];
        if field.choices.is_some() {
            spans.extend([Span::styled("‹ ", pal.accent()), Span::styled(field.value.clone(), theme::bold()), Span::styled(" ›", pal.accent())]);
        } else {
            spans.push(Span::styled(format!(" {}", field.value), Style::new().fg(theme::TEXT).bg(SEL_BG)));
            spans.push(Span::styled(if active { "▏" } else { " " }, pal.accent()));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(Span::styled(if active { format!("  {:<12}↳ {}", "", field.help) } else { String::new() }, theme::dim())));
    }
    f.render_widget(Paragraph::new(lines), fields_area);

    let heading = |t: &str| Line::from(Span::styled(t.to_string(), pal.accent().add_modifier(Modifier::BOLD)));
    let tree = |t: &str, note: &str| Line::from(vec![Span::styled(format!("  {t:<28}"), theme::text()), Span::styled(note.to_string(), theme::dim())]);
    let preview: Vec<Line> = match form.kind {
        FormKind::Workspace => {
            let folder = form.value("folder").trim_end_matches('/').to_string();
            vec![
                heading("htui creates"),
                tree(&format!("{folder}/"), ""),
                tree("  .htui/workspace.toml", "name, description, goals, projects"),
                tree("  .htui/docker-compose.yml", "the components, generated"),
                match crate::wsview::infisical(app) {
                    Some(url) => tree("  (.env)", &format!("ports and URLs: in Infisical ({url}), a folder per project")),
                    None => tree("  .htui/.env", "ports and URLs per project (Secrets has no Infisical)"),
                },
                tree("  projects/", "projects cloned from a URL go here"),
                Line::raw(""),
                Line::from(Span::styled("Existing repos stay where they are; the workspace only points at them.", theme::dim())),
            ]
        }
        FormKind::Step { wf, edit, after } => {
            let w = &app.data.workflows[wf];
            let parts: Vec<&str> = form.value("parts").split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
            let who = match (form.value("persona"), form.value("agent")) {
                (p, "any agent") if !p.starts_with("none") => format!("{p}, played by whichever agent the run picks"),
                (p, a) if !p.starts_with("none") => format!("{p}, played by {a}"),
                (_, a) => format!("a check run by {a}"),
            };
            let shape = if parts.is_empty() { who } else { format!("a fan-out: {} agents in parallel ({}), each a {}", parts.len(), parts.join(", "), form.value("persona")) };
            let place = match edit {
                Some(_) => "its links stay as they are".to_string(),
                None => format!("after {}, taking over its outgoing links", w.steps.get(after).map_or("the start", |s| s.name.as_str())),
            };
            vec![
                heading("it becomes"),
                Line::from(Span::styled(format!("  {}: {shape}", form.value("name")), theme::text())),
                Line::from(Span::styled(format!("  {place}"), theme::dim())),
                Line::raw(""),
                Line::from(Span::styled("The template changes; runs already going keep the version they started with.", theme::dim())),
            ]
        }
        FormKind::Project => {
            let ws = &app.data.workspaces[form.ws];
            let name = form.value("name");
            let where_ = match form.value("source") {
                "git URL" => format!("cloned into {}/projects/{name}", ws.path),
                "new empty repo" => format!("created at {}/projects/{name}", ws.path),
                _ => format!("{} stays where it is", form.value("location")),
            };
            vec![
                heading("what happens"),
                Line::from(Span::styled(format!("  {where_}"), theme::text())),
                Line::from(Span::styled(format!("  a \"{name} coordinator\" chat appears in the Chat tab"), theme::text())),
                Line::from(Span::styled("  it uses the workspace's components until you say otherwise", theme::text())),
            ]
        }
    };
    f.render_widget(Paragraph::new(preview).wrap(Wrap { trim: false }), preview_area);
}

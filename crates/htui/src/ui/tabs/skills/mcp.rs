//! The MCP servers view of the Skills tab (MOD-127 M2 D12, T8): the toolbox's servers beside the
//! skill library, the same shape as the Library's.
//!
//! A list of servers on the left, the shown server's record on the right, a form to create or edit
//! one (`n`, `e`), a question before a delete (`d`), and the attachments pane (`a`), the one the
//! Library draws over a skill, here over a [`BindingRef::McpServer`]. The view holds no store
//! handle: it renders from the last [`SkillsSnapshot`] (the tab reads it for both views) and sends
//! [`StoreRequest::CreateMcpServer`], [`StoreRequest::EditMcpServer`],
//! [`StoreRequest::DeleteMcpServer`] and, through the pane, [`StoreRequest::SetSkillBinding`].
//!
//! **Secrets are held by reference.** An env or header entry is `NAME=value` (a literal) or
//! `NAME=@SECRET` (the project secret `SECRET`, substituted only when a run launches the server).
//! A literal under a name that looks like a secret is refused with the store's own sentence, on the
//! form while typing and again when `form.save` is pressed. No value is ever written to a notice,
//! a title or a `Debug`: the record shows names.

use core::cell::Cell;

use chrono::{DateTime, Utc};
use htui_core::model::mcp::{McpServerPatch, new_mcp_server_refusal};
use htui_core::model::{
    BindingChange, BindingKind, EnvValue, McpServer, McpServerId, McpTransport, NewMcpServer,
    UserId,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::attach::{AttachOutcome, AttachPane};
use super::library::{
    NOTICE_LINES, Notice, SAVING, SCROLL_HINT, Sent, in_flight, in_scope, landed_unread,
};
use super::{Detail, DetailPane, holder, render_detail, view_chips, view_chips_if_room};
use crate::app::{Ctx, Handled};
use crate::editor::NO_CHANGES;
use crate::keys::translate::FieldEvent;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::skills::{
    MCP_REQUEST_NAMES, McpServerDraft, McpServerWrite, READ_NAME, REQUEST_NAMES, SkillWrite,
    SkillsSnapshot, StaleWhat,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells;
use crate::ui::kit::chips::key_line;
use crate::ui::kit::form::{Field, Form as FormWidget, Value};
use crate::ui::kit::label;
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::pane::{Focus, Pane as KitPane, capture_area};
use crate::ui::kit::scroll::{Scroll, Step};
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::shell::PaneId;
use crate::ui::{FieldOutcome, TextField};
use crossterm::event::KeyEvent;

/// A list row's name field, in cells: the list's 29 usable cells hold two spaces, the name, a
/// space and the longest transport word.
const NAME_WIDTH: usize = 21;

/// This view's chip among [`VIEWS`](super::VIEWS).
const VIEW: usize = 2;

/// The pane before the first reply.
const NOT_READ: &str = "MCP servers not read yet";

/// [`NOT_READ`]'s next step.
const NOT_READ_NEXT: &str = "they appear once the store answers";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "MCP servers unavailable";

/// The pane with no server (an empty toolbox).
const NO_SERVERS: &str = "no MCP servers";

/// [`NO_SERVERS`]'s next step, through `MCP_BROWSE`.
const EMPTY_HINT: HintSpec = &[Hint::One(Act::New, "creates one")];

/// [`EMPTY_HINT`] with the key unbound.
const EMPTY_NEXT: &str = "a new server shows here";

/// The server under the cursor went while a form was open on it.
const SERVER_GONE: &str = "the server is gone";

/// `delete` on a server another session deleted first.
const ALREADY_GONE: &str = "the server was already deleted elsewhere";

/// The answer to a delete question that was not a yes.
const KEPT: &str = "kept";

/// Every Browse key, the shell's activity pane's hint.
const BROWSE_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::Delete, "delete"),
    Hint::One(Act::LibraryAttach, "attach"),
];

/// The list pane's keys while browsing.
const LIST_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::New, "new"),
];

/// The record pane's keys while browsing.
const DETAIL_HINT: HintSpec = &[
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Delete, "delete"),
    Hint::One(Act::LibraryAttach, "attach"),
];

/// The form's hint row (`Esc` is the field's).
const FORM_HINT: HintSpec = &[
    Hint::One(Act::FormNextField, "field"),
    Hint::One(Act::FormSave, "save"),
    Hint::Text("Esc cancel"),
];

/// The transport row's own help: the key that cycles it.
const TRANSPORT_HINT: HintSpec = &[Hint::One(Act::ChoiceToggle, "cycles")];

/// The delete question's keys.
const CONFIRM_HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "delete"),
    Hint::Text("any other key keeps it"),
];

/// Help under the fields, shown while one is focused.
const COMMAND_HELP: &str = "the program an agent starts";
const ARGS_HELP: &str = "words, space separated; \"quote\" one that holds a space";
const URL_HELP: &str = "http:// or https://";
const PAIRS_HELP: &str = "NAME=value, or NAME=@SECRET to read the project secret SECRET";
const ALLOW_HELP: &str = "tool names, space separated: pre-approved for the agent";
const DENY_HELP: &str = "tool names, space separated: removed from the agent";

/// The MCP servers view. Holds no store handle (`R-NF-3`); renders from the last snapshot.
#[derive(Debug, Default)]
pub(super) struct McpView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused read.
    unavailable: Option<String>,
    /// The highlighted server, an index into `snapshot.mcp_servers`.
    cursor: usize,
    /// The record pane's scroll.
    scroll: Scroll,
    /// The record pane's rows at the last draw.
    pane_rows: Cell<usize>,
    /// Browsing, the form, or the delete question.
    mode: Mode,
    /// The attachments pane, while open.
    attach: Option<AttachPane>,
    /// The write in flight, by [`StoreRequest::name`]: one at a time.
    busy: Option<&'static str>,
    /// What an attachment write in flight carries, for the landing.
    sent: Option<Sent>,
    /// The name a delete in flight removes, for the landing.
    deleting: Option<String>,
    /// The last outcome.
    notice: Option<Notice>,
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the servers.
    #[default]
    Browse,
    /// `n` or `e`.
    Form(Box<ServerForm>),
    /// `d`: the question, until a key answers it.
    Confirm {
        /// Which server.
        id: McpServerId,
        /// Its name, for the notices.
        name: String,
    },
}

/// One input of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SField {
    /// `mcp_server.name`.
    Name,
    /// `mcp_server.description`.
    Description,
    /// `mcp_server.transport`: a choice.
    Transport,
    /// `mcp_server.command` (stdio).
    Command,
    /// `mcp_server.args` (stdio).
    Args,
    /// `mcp_server.url` (http).
    Url,
    /// `mcp_server.headers` (http).
    Headers,
    /// `mcp_server.env` (stdio).
    Env,
    /// `mcp_server.allow`.
    Allow,
    /// `mcp_server.deny`.
    Deny,
}

impl SField {
    /// The inputs a form over `transport` shows, in tab order: a stdio server has no url or
    /// headers, an http one no command, args or env.
    const fn visible(transport: McpTransport) -> &'static [Self] {
        match transport {
            McpTransport::Stdio => &[
                Self::Name,
                Self::Description,
                Self::Transport,
                Self::Command,
                Self::Args,
                Self::Env,
                Self::Allow,
                Self::Deny,
            ],
            McpTransport::Http => &[
                Self::Name,
                Self::Description,
                Self::Transport,
                Self::Url,
                Self::Headers,
                Self::Allow,
                Self::Deny,
            ],
        }
    }

    /// The row's label.
    const fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Description => "description",
            Self::Transport => "transport",
            Self::Command => "command",
            Self::Args => "args",
            Self::Url => "url",
            Self::Headers => "headers",
            Self::Env => "env",
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// The open form. Its `updated_at` is the compare-and-set token of an edit.
#[derive(Debug)]
struct ServerForm {
    /// `Some((id, token))` for an edit, `None` for a new server.
    edit: Option<(McpServerId, DateTime<Utc>)>,
    /// The name.
    name: TextField,
    /// The description.
    description: TextField,
    /// Stdio or http: `choice.toggle` cycles it.
    transport: McpTransport,
    /// The program (stdio).
    command: TextField,
    /// Its arguments, quoted words.
    args: TextField,
    /// The endpoint (http).
    url: TextField,
    /// Header entries, `NAME=value` or `NAME=@SECRET`.
    headers: TextField,
    /// Env entries, the same shape.
    env: TextField,
    /// Pre-approved tool names.
    allow: TextField,
    /// Removed tool names.
    deny: TextField,
    /// Where the keys go.
    focus: SField,
}

impl ServerForm {
    /// An empty form for a new stdio server.
    fn new() -> Self {
        Self {
            edit: None,
            name: TextField::new(),
            description: TextField::new(),
            transport: McpTransport::Stdio,
            command: TextField::new(),
            args: TextField::new(),
            url: TextField::new(),
            headers: TextField::new(),
            env: TextField::new(),
            allow: TextField::new(),
            deny: TextField::new(),
            focus: SField::Name,
        }
    }

    /// A form over `server`, its fields as the record says.
    fn of(server: &McpServer) -> Self {
        Self {
            edit: Some((server.id, server.updated_at)),
            name: TextField::with_text(&server.name),
            description: TextField::with_text(&server.description),
            transport: server.transport,
            command: TextField::with_text(&server.command),
            args: TextField::with_text(&words(&server.args)),
            url: TextField::with_text(server.url.as_deref().unwrap_or_default()),
            headers: TextField::with_text(&pairs(&server.headers)),
            env: TextField::with_text(&pairs(&server.env)),
            allow: TextField::with_text(&server.allow.join(" ")),
            deny: TextField::with_text(&server.deny.join(" ")),
            focus: SField::Name,
        }
    }

    /// The text field of `field`; `None` for the transport, which is no text.
    fn field_mut(&mut self, field: SField) -> Option<&mut TextField> {
        Some(match field {
            SField::Name => &mut self.name,
            SField::Description => &mut self.description,
            SField::Transport => return None,
            SField::Command => &mut self.command,
            SField::Args => &mut self.args,
            SField::Url => &mut self.url,
            SField::Headers => &mut self.headers,
            SField::Env => &mut self.env,
            SField::Allow => &mut self.allow,
            SField::Deny => &mut self.deny,
        })
    }

    /// The text field of `field`, to draw.
    fn field(&self, field: SField) -> Option<&TextField> {
        Some(match field {
            SField::Name => &self.name,
            SField::Description => &self.description,
            SField::Transport => return None,
            SField::Command => &self.command,
            SField::Args => &self.args,
            SField::Url => &self.url,
            SField::Headers => &self.headers,
            SField::Env => &self.env,
            SField::Allow => &self.allow,
            SField::Deny => &self.deny,
        })
    }

    /// The text of `field`, trimmed.
    fn text(&self, field: SField) -> &str {
        self.field(field)
            .and_then(TextField::text)
            .unwrap_or_default()
            .trim()
    }

    /// Whether anything is typed: an untouched form shows its help, not a refusal of its blanks.
    fn is_blank(&self) -> bool {
        SField::visible(self.transport)
            .iter()
            .all(|field| self.text(*field).is_empty())
    }

    /// The next (or previous) input of the form's transport, wrapping.
    fn step(&mut self, forward: bool) {
        let fields = SField::visible(self.transport);
        let at = fields
            .iter()
            .position(|field| *field == self.focus)
            .unwrap_or(0);
        let len = fields.len();
        self.focus = fields[if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        }];
    }

    /// What a save writes, or the input whose text cannot be read and why.
    fn draft(&self) -> Result<McpServerDraft, (SField, String)> {
        let name = self.text(SField::Name).to_owned();
        let description = self.text(SField::Description).to_owned();
        let allow = tool_names(self.text(SField::Allow));
        let deny = tool_names(self.text(SField::Deny));
        Ok(match self.transport {
            McpTransport::Stdio => McpServerDraft {
                name,
                description,
                transport: McpTransport::Stdio,
                command: self.text(SField::Command).to_owned(),
                args: split_words(self.text(SField::Args)).map_err(|why| (SField::Args, why))?,
                url: None,
                headers: Vec::new(),
                env: parse_pairs(self.text(SField::Env)).map_err(|why| (SField::Env, why))?,
                allow,
                deny,
            },
            McpTransport::Http => McpServerDraft {
                name,
                description,
                transport: McpTransport::Http,
                command: String::new(),
                args: Vec::new(),
                url: Some(self.text(SField::Url).to_owned()).filter(|url| !url.is_empty()),
                headers: parse_pairs(self.text(SField::Headers))
                    .map_err(|why| (SField::Headers, why))?,
                env: Vec::new(),
                allow,
                deny,
            },
        })
    }

    /// The first sentence refusing the form, and the input it is about; `None` when it would
    /// save. The store's own chain ([`new_mcp_server_refusal`]) over the draft, and a taken name.
    fn refusal(&self, snapshot: Option<&SkillsSnapshot>) -> Option<(SField, String)> {
        let draft = match self.draft() {
            Ok(draft) => draft,
            Err(refused) => return Some(refused),
        };
        if let Some(sentence) = draft_refusal(&draft) {
            return Some((blamed(&sentence, self.focus), sentence));
        }
        let taken = snapshot
            .and_then(|snapshot| snapshot.mcp_by_name(&draft.name))
            .is_some_and(|other| self.edit.is_none_or(|(id, _)| id != other.id));
        taken.then(|| (SField::Name, format!("`{}` exists", draft.name)))
    }
}

/// The store's refusal of `draft`, the sentence both stores give.
fn draft_refusal(draft: &McpServerDraft) -> Option<String> {
    new_mcp_server_refusal(&NewMcpServer {
        id: McpServerId::new(),
        name: draft.name.clone(),
        description: draft.description.clone(),
        transport: draft.transport,
        command: draft.command.clone(),
        args: draft.args.clone(),
        url: draft.url.clone(),
        headers: draft.headers.clone(),
        env: draft.env.clone(),
        allow: draft.allow.clone(),
        deny: draft.deny.clone(),
        created_by: UserId::new(),
    })
}

/// The input a store refusal is about, by the words it leads with; `fallback` when it names none.
/// Only the text before the first backtick counts: the core's sentences quote what the user typed
/// (`the header `x-command-token` looks like a secret`) between backticks, and a typed key must
/// not decide the field.
fn blamed(sentence: &str, fallback: SField) -> SField {
    let lead = sentence.split('`').next().unwrap_or(sentence);
    let table = [
        ("mcp_server.name", SField::Name),
        ("htui's own server", SField::Name),
        ("mcp_server.description", SField::Description),
        ("command", SField::Command),
        ("url", SField::Url),
        ("env", SField::Env),
        ("header", SField::Headers),
        ("mcp_server.allow", SField::Allow),
        ("mcp_server.deny", SField::Deny),
    ];
    table
        .into_iter()
        .find(|(word, _)| lead.contains(word))
        .map_or(fallback, |(_, field)| field)
}

/// The patch from `current` to `draft`: only the columns that differ.
fn patch_of(current: &McpServer, draft: &McpServerDraft) -> McpServerPatch {
    McpServerPatch {
        name: (draft.name != current.name).then(|| draft.name.clone()),
        description: (draft.description != current.description).then(|| draft.description.clone()),
        transport: (draft.transport != current.transport).then_some(draft.transport),
        command: (draft.command != current.command).then(|| draft.command.clone()),
        args: (draft.args != current.args).then(|| draft.args.clone()),
        url: (draft.url != current.url).then(|| draft.url.clone()),
        headers: (draft.headers != current.headers).then(|| draft.headers.clone()),
        env: (draft.env != current.env).then(|| draft.env.clone()),
        allow: (draft.allow != current.allow).then(|| draft.allow.clone()),
        deny: (draft.deny != current.deny).then(|| draft.deny.clone()),
    }
}

// --- the text forms of the lists ------------------------------------------------------------

/// Splits `text` into words the way a shell does, enough for an argument list: whitespace
/// separates, a `"..."` or `'...'` groups (an empty pair is an empty word), and a backslash
/// escapes the next character outside single quotes, inside double quotes only before `"` or `\`
/// (so a Windows path survives).
fn split_words(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some('"'), '\\') => match chars.peek() {
                Some(&next) if next == '"' || next == '\\' => {
                    word.push(next);
                    chars.next();
                }
                _ => word.push('\\'),
            },
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                started = true;
                match chars.peek() {
                    Some(&next) if next.is_whitespace() || "\"'\\".contains(next) => {
                        word.push(next);
                        chars.next();
                    }
                    _ => word.push('\\'),
                }
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(core::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("a quote is not closed".to_owned());
    }
    if started {
        out.push(word);
    }
    Ok(out)
}

/// `word` as [`split_words`] reads it back: bare when it can be, else in double quotes.
fn quote_word(word: &str) -> String {
    let plain = !word.is_empty()
        && !word
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
    if plain {
        word.to_owned()
    } else {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// `args` as one line of quoted words.
fn words(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_word(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Env or header entries from `NAME=value` words: `NAME=@SECRET` is the project secret `SECRET`,
/// `NAME=@@text` the literal `@text`, anything else a literal.
fn parse_pairs(text: &str) -> Result<Vec<(String, EnvValue)>, String> {
    // An error names the entry's place, never the word: a value typed without its `NAME=` may be
    // a secret, and the notice would show it.
    split_words(text)?
        .into_iter()
        .enumerate()
        .map(|(index, word)| {
            let place = index + 1;
            let Some((key, value)) = word.split_once('=') else {
                return Err(format!("entry {place} needs the form NAME=value"));
            };
            if key.is_empty() {
                return Err(format!("entry {place} has no name before the `=`"));
            }
            let value = match value.strip_prefix('@') {
                Some(literal) if literal.starts_with('@') => EnvValue::Literal(literal.to_owned()),
                Some("") => return Err(format!("`{key}=@` names no secret after the `@`")),
                Some(secret) => EnvValue::Secret(secret.to_owned()),
                None => EnvValue::Literal(value.to_owned()),
            };
            Ok((key.to_owned(), value))
        })
        .collect()
}

/// One entry as [`parse_pairs`] reads it back.
fn pair(key: &str, value: &EnvValue) -> String {
    let value = match value {
        EnvValue::Literal(text) if text.starts_with('@') => format!("@{text}"),
        EnvValue::Literal(text) => text.clone(),
        EnvValue::Secret(name) => format!("@{name}"),
    };
    format!("{key}={}", quote_word(&value))
}

/// Entries as one line.
fn pairs(entries: &[(String, EnvValue)]) -> String {
    entries
        .iter()
        .map(|(key, value)| pair(key, value))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tool names split on whitespace and commas.
fn tool_names(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The delete question over `name`, its answer `yes` (`ConfirmYes`'s label through `MCP_CONFIRM`).
/// `bindings` counts the attachments the snapshot shows, the scope's own: the delete takes every
/// one the store holds, so the question says there may be more in other workspaces or projects,
/// and the notice after it names the store's exact count.
fn delete_question(name: &str, bindings: usize, yes: Option<&str>) -> String {
    let goes = if bindings == 0 {
        " and any attachments in other workspaces or projects".to_owned()
    } else {
        format!(
            " and its {} here, and any in other workspaces or projects",
            label::plural(bindings, "attachment", "attachments")
        )
    };
    match yes {
        Some(yes) => {
            format!("delete `{name}`{goes}? {yes} deletes, any other key keeps it")
        }
        None => format!("delete `{name}`{goes}?"),
    }
}

/// A transport key on the form's choice row: `choice.toggle` cycles it, `Enter` and `Esc` submit
/// and cancel as a text field's do, anything else passes to the form's stack.
fn transport_key(
    form: &mut ServerForm,
    key: KeyEvent,
    ctx: &Ctx<'_>,
    stack: Stack<'_>,
) -> FieldOutcome {
    if ctx
        .keys()
        .actions(stack, ctx.pressed(&key))
        .contains(&Act::ChoiceToggle)
    {
        form.transport = match form.transport {
            McpTransport::Stdio => McpTransport::Http,
            McpTransport::Http => McpTransport::Stdio,
        };
        return FieldOutcome::Consumed;
    }
    match FieldEvent::of(&key) {
        Some(FieldEvent::Submit) => FieldOutcome::Submit,
        Some(FieldEvent::Cancel) => FieldOutcome::Cancel,
        _ => FieldOutcome::Pass,
    }
}

/// The attachments `server` has in `snapshot`: global, the scope workspace's and the scope
/// projects' rows.
fn bindings_of(snapshot: &SkillsSnapshot, server: McpServerId) -> usize {
    let of = |rows: &[htui_core::model::Binding]| {
        rows.iter()
            .filter(|row| snapshot.mcp_server_of(row).is_some_and(|s| s.id == server))
            .count()
    };
    of(&snapshot.global)
        + snapshot
            .workspaces
            .iter()
            .map(|entry| of(&entry.bindings))
            .sum::<usize>()
        + snapshot
            .projects
            .iter()
            .map(|entry| of(&entry.bindings))
            .sum::<usize>()
}

impl McpView {
    /// The browse mode's stack, whatever mode the view is in.
    pub(super) const fn browse_stack() -> Stack<'static> {
        views::MCP_BROWSE
    }

    /// The current mode's stack, always one: the attachments pane's while it is open.
    pub(super) fn key_stack(&self) -> Stack<'static> {
        if let Some(pane) = &self.attach {
            return pane.key_stack();
        }
        match &self.mode {
            Mode::Browse => views::MCP_BROWSE,
            Mode::Form(_) => views::MCP_FORM,
            Mode::Confirm { .. } => views::MCP_CONFIRM,
        }
    }

    /// Whether the form, the question or the attachments pane's form, picker or question is
    /// taking every key.
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
            || self.attach.as_ref().is_some_and(AttachPane::captures_input)
    }

    /// A bracketed paste into the open form's focused field or the attachments pane; `false` when
    /// nothing here is taking text.
    pub(super) fn on_paste(&mut self, text: &str) -> bool {
        if let Some(attach) = &mut self.attach {
            return attach.on_paste(text);
        }
        match &mut self.mode {
            Mode::Browse => false,
            Mode::Form(form) => {
                let focus = form.focus;
                if let Some(field) = form.field_mut(focus) {
                    field.on_paste(text);
                }
                true
            }
            Mode::Confirm { .. } => true,
        }
    }

    /// The scope changed: the servers, the form, the pane and the write in flight belong to the
    /// workspace that was left. The notice survives.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if self.attach.is_some() {
            return self.on_attach_key(key, ctx);
        }
        match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Form(_) => self.on_form_key(key, ctx),
            Mode::Confirm { .. } => self.on_confirm_key(key, ctx),
        }
    }

    // --- replies -------------------------------------------------------------------------------

    /// A reply addressed to the Skills tab: every one that carries a snapshot refreshes it (a
    /// skill write elsewhere may have moved an attachment), and the write this view has in flight
    /// lands.
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Skills(snapshot) => {
                if self.take(snapshot, ctx) {
                    self.clamp();
                }
            }
            StoreReply::SkillImports(imports) => {
                if self.take(&imports.snapshot, ctx) {
                    self.clamp();
                }
            }
            StoreReply::SkillWritten { snapshot, outcome } => match snapshot {
                Ok(snapshot) => {
                    if self.take(snapshot, ctx) {
                        self.land_binding(outcome, None, ctx.keys());
                        self.clamp();
                    }
                }
                Err(why)
                    if self.busy == Some(outcome.request_name())
                        && super::write_kind(outcome) == BindingKind::McpServer =>
                {
                    self.land_binding(outcome, Some(why), ctx.keys());
                }
                Err(_) => {}
            },
            StoreReply::McpServerWritten { snapshot, outcome } => match snapshot {
                Ok(snapshot) => {
                    if self.take(snapshot, ctx) {
                        self.land(outcome, None, ctx.keys());
                        self.clamp();
                    }
                }
                Err(why) if self.busy == Some(outcome.request_name()) => {
                    self.land(outcome, Some(why), ctx.keys());
                }
                Err(_) => {}
            },
            StoreReply::SkillsStale { snapshot, what } => {
                if !self.take(snapshot, ctx) {
                    return;
                }
                // A skill's write shares `set_skill_binding`'s name: only a server's own reply
                // frees what this view has in flight (MOD-127 M2 R3).
                if super::stale_kind(*what) == BindingKind::McpServer && self.busy.take().is_some()
                {
                    let detach = matches!(
                        self.sent.take(),
                        Some(Sent::Binding {
                            change: BindingChange::Detach,
                            ..
                        })
                    );
                    self.stale(*what, detach, ctx.keys());
                }
                self.clamp();
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message }
                if (MCP_REQUEST_NAMES.contains(request) || *request == REQUEST_NAMES[4])
                    && self.busy == Some(*request) =>
            {
                // A refused write leaves the form or the question's row as it was.
                self.busy = None;
                self.sent = None;
                self.deleting = None;
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
    }

    /// Takes `snapshot` as the view's own when it answers the scope in force.
    fn take(&mut self, snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> bool {
        if !in_scope(snapshot, ctx) {
            return false;
        }
        self.snapshot = Some(snapshot.clone());
        self.unavailable = None;
        true
    }

    /// The server write in flight landed. `unread` is the re-read's failure: the write landed,
    /// and the notice says the list drawn is the one held.
    fn land(&mut self, outcome: &McpServerWrite, unread: Option<&str>, keys: &Keys) {
        if self.busy != Some(outcome.request_name()) {
            return;
        }
        let landed = match outcome {
            McpServerWrite::Created { server, name } => {
                self.select(*server);
                self.mode = Mode::Browse;
                format!("created `{name}`")
            }
            McpServerWrite::Edited { server, name } => {
                self.select(*server);
                self.mode = Mode::Browse;
                format!("saved `{name}`")
            }
            McpServerWrite::Deleted { bindings, .. } => {
                self.mode = Mode::Browse;
                self.attach = None;
                self.scroll.reset();
                let name = self.deleting.take().unwrap_or_default();
                if *bindings == 0 {
                    format!("deleted `{name}`")
                } else {
                    format!(
                        "deleted `{name}` and its {}",
                        label::plural(
                            usize::try_from(*bindings).unwrap_or(usize::MAX),
                            "attachment",
                            "attachments"
                        )
                    )
                }
            }
        };
        self.busy = None;
        self.notice = Some(match unread {
            None => Notice::Info(landed),
            Some(why) => Notice::Error(landed_unread(
                &landed,
                why,
                keys.label(views::MCP_BROWSE, Act::Reload).as_deref(),
            )),
        });
    }

    /// The attachment write in flight landed: the pane's form closes, or keeps later edits.
    fn land_binding(&mut self, outcome: &SkillWrite, unread: Option<&str>, keys: &Keys) {
        if self.busy != Some(outcome.request_name())
            || super::write_kind(outcome) != BindingKind::McpServer
        {
            return;
        }
        let landed = match (outcome, self.sent.take()) {
            (SkillWrite::Attached { key, updated_at }, Some(Sent::Binding { change, target })) => {
                let kept = self
                    .attach
                    .as_mut()
                    .is_some_and(|pane| pane.on_landed(*key, &change, Some(*updated_at)));
                if kept {
                    match keys.label(views::ATTACH_FORM, Act::FormSave) {
                        Some(save) => format!(
                            "attached to {target} \u{2014} later edits kept, {save} saves them"
                        ),
                        None => format!("attached to {target} \u{2014} later edits kept"),
                    }
                } else {
                    format!("attached to {target}")
                }
            }
            (SkillWrite::Detached { key }, Some(Sent::Binding { change, target })) => {
                if let Some(pane) = &mut self.attach {
                    pane.on_landed(*key, &change, None);
                }
                format!("detached from {target}")
            }
            _ => {
                self.busy = None;
                self.attach = None;
                self.notice = Some(Notice::Error(format!(
                    "`{}` landed",
                    outcome.request_name()
                )));
                return;
            }
        };
        self.busy = None;
        self.notice = Some(match unread {
            None => Notice::Info(landed),
            Some(why) => Notice::Error(landed_unread(
                &landed,
                why,
                keys.label(views::MCP_BROWSE, Act::Reload).as_deref(),
            )),
        });
    }

    /// A write lost its token or its row: the snapshot is fresh; the form takes the new token and
    /// keeps its text, a pane's form likewise.
    fn stale(&mut self, what: StaleWhat, detach: bool, keys: &Keys) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match what {
            StaleWhat::McpServer(id) => match (snapshot.mcp_server(id), &mut self.mode) {
                (Some(server), Mode::Form(form)) if form.edit.is_some() => {
                    form.edit = Some((id, server.updated_at));
                    let kept = "changed elsewhere since you opened it \u{2014} your text is kept";
                    self.notice = Some(Notice::Error(
                        match keys.label(views::MCP_FORM, Act::FormSave) {
                            Some(save) => format!("{kept}; {save} saves it over the new row"),
                            None => kept.to_owned(),
                        },
                    ));
                }
                (Some(_), _) => {}
                (None, Mode::Form(_)) => {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Error(SERVER_GONE.to_owned()));
                }
                (None, _) => {
                    self.deleting = None;
                    self.notice = Some(Notice::Error(ALREADY_GONE.to_owned()));
                }
            },
            StaleWhat::Binding(_) => {
                if let Some(pane) = &mut self.attach {
                    pane.on_stale(snapshot);
                }
                let sentence = if detach {
                    "this attachment changed elsewhere \u{2014} nothing was detached; its row \
                     shows it as it is now"
                        .to_owned()
                } else {
                    let kept = "this attachment changed elsewhere \u{2014} your form is kept";
                    match keys.label(views::ATTACH_FORM, Act::FormSave) {
                        Some(save) => format!("{kept}; {save} saves over it"),
                        None => kept.to_owned(),
                    }
                };
                self.notice = Some(Notice::Error(sentence));
            }
            // A skill's write: this view holds none.
            StaleWhat::Skill(_) | StaleWhat::Version(_) => {}
        }
    }

    // --- keys ----------------------------------------------------------------------------------

    /// Browse: each candidate act of the chord through `views::MCP_BROWSE`, the first this view
    /// uses taken; anything else passes for the shell. Motion follows the focus as the Library's
    /// does: the list focused moves the cursor, the pane focused scrolls it.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        let (focus, rows) = (ctx.focus(), self.pane_rows.get());
        for act in ctx.keys().actions(views::MCP_BROWSE, chord) {
            if Step::by_focus(act, focus, |step| self.scroll.follow(step, rows)) {
                return Handled::Consumed;
            }
            match act {
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::Reload => {
                    self.notice = None;
                    ctx.request(StoreRequest::Skills(ctx.scope.clone()));
                }
                Act::New => {
                    if self.snapshot.is_some() {
                        self.notice = None;
                        self.mode = Mode::Form(Box::new(ServerForm::new()));
                    }
                }
                Act::Edit | Act::Delete | Act::LibraryAttach => {
                    self.notice = None;
                    self.on_server_key(act, ctx);
                }
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    return self.scroll.apply(act, self.pane_rows.get());
                }
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// An edit, delete or attach key; with no server under the cursor it does nothing.
    fn on_server_key(&mut self, act: Act, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(server) = snapshot.mcp_servers.get(self.cursor) else {
            return;
        };
        match act {
            Act::Edit => self.mode = Mode::Form(Box::new(ServerForm::of(server))),
            Act::Delete => {
                let bindings = bindings_of(snapshot, server.id);
                let yes = ctx.keys().label(views::MCP_CONFIRM, Act::ConfirmYes);
                self.notice = Some(Notice::Info(delete_question(
                    &server.name,
                    bindings,
                    yes.as_deref(),
                )));
                self.mode = Mode::Confirm {
                    id: server.id,
                    name: server.name.clone(),
                };
            }
            Act::LibraryAttach => self.attach = Some(AttachPane::new(server.id)),
            _ => {}
        }
    }

    /// The delete question, through `views::MCP_CONFIRM`: `confirm.yes` deletes, anything else
    /// keeps the server (a key the stack passes, a tab switch, goes on to the shell).
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Confirm { id, name, .. } = &self.mode else {
            return Handled::Pass;
        };
        let (id, name) = (*id, name.clone());
        let chord = ctx.pressed(&key);
        match ctx.keys().actions(views::MCP_CONFIRM, chord).first() {
            Some(Act::ConfirmYes) => {
                self.mode = Mode::Browse;
                if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else {
                    self.deleting = Some(name);
                    self.send(
                        StoreRequest::DeleteMcpServer {
                            scope: ctx.scope.clone(),
                            id,
                        },
                        ctx,
                    );
                }
                Handled::Consumed
            }
            Some(Act::ConfirmNo) => {
                self.mode = Mode::Browse;
                self.notice = Some(Notice::Info(KEPT.to_owned()));
                Handled::Consumed
            }
            _ if views::MCP_CONFIRM.passes(chord) => Handled::Pass,
            _ => {
                self.mode = Mode::Browse;
                self.notice = Some(Notice::Info(KEPT.to_owned()));
                Handled::Consumed
            }
        }
    }

    /// The form, widget first: the focused field (or the transport's choice) sees the key; what it
    /// passes resolves through `views::MCP_FORM`: `form.next_field`/`prev_field` move, `form.save`
    /// saves. Any other key is `modal_rest`'s.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Form(form) = &mut self.mode else {
            return Handled::Pass;
        };
        let focus = form.focus;
        let outcome = match form.field_mut(focus) {
            Some(field) => field.on_key(key),
            None => transport_key(form, key, ctx, views::MCP_FORM),
        };
        match outcome {
            FieldOutcome::Consumed => {
                // An edit answers the refusal the last save met: the form shows its own now.
                if matches!(self.notice, Some(Notice::Error(_))) {
                    self.notice = None;
                }
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                self.save(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else {
                    self.mode = Mode::Browse;
                    self.notice = None;
                }
                Handled::Consumed
            }
            FieldOutcome::Pass => {
                let chord = ctx.pressed(&key);
                match ctx.keys().actions(views::MCP_FORM, chord).first() {
                    Some(Act::FormNextField) => {
                        form.step(true);
                        Handled::Consumed
                    }
                    Some(Act::FormPrevField) => {
                        form.step(false);
                        Handled::Consumed
                    }
                    Some(Act::FormSave) => {
                        self.save(ctx);
                        Handled::Consumed
                    }
                    _ => modal_rest(views::MCP_FORM, chord),
                }
            }
        }
    }

    /// The form's save: a write in flight; a text the form cannot read; a refusal the store would
    /// give (its sentence, nothing sent); nothing changed; otherwise the create or the edit with
    /// only the columns that differ.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Form(form) = &self.mode else {
            return;
        };
        if let Some((_, sentence)) = form.refusal(self.snapshot.as_ref()) {
            self.notice = Some(Notice::Error(sentence));
            return;
        }
        let Ok(draft) = form.draft() else {
            return;
        };
        let request = match form.edit {
            None => StoreRequest::CreateMcpServer {
                scope: ctx.scope.clone(),
                draft: Box::new(draft),
            },
            Some((id, token)) => {
                let Some(current) = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.mcp_server(id))
                else {
                    self.notice = Some(Notice::Error(SERVER_GONE.to_owned()));
                    return;
                };
                let patch = patch_of(current, &draft);
                if patch == McpServerPatch::default() {
                    self.notice = Some(Notice::Info(NO_CHANGES.to_owned()));
                    return;
                }
                StoreRequest::EditMcpServer {
                    scope: ctx.scope.clone(),
                    id,
                    expected: token,
                    patch: Box::new(patch),
                }
            }
        };
        self.send(request, ctx);
    }

    /// The attachments pane's key: its outcome is a notice, a close, or a write to send.
    fn on_attach_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let (Some(pane), Some(snapshot)) = (&mut self.attach, &self.snapshot) else {
            self.attach = None;
            return Handled::Pass;
        };
        if let Some(busy) = self.busy
            && pane.in_form()
            && FieldEvent::of(&key) == Some(FieldEvent::Cancel)
        {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return Handled::Consumed;
        }
        match pane.on_key(key, snapshot, ctx) {
            AttachOutcome::Consumed => {}
            AttachOutcome::Pass => return Handled::Pass,
            AttachOutcome::Close => self.attach = None,
            AttachOutcome::Notice(notice) => self.notice = Some(notice),
            AttachOutcome::Save { request, sent } => match self.busy {
                Some(busy) => self.notice = Some(Notice::Error(in_flight(busy))),
                None => {
                    self.sent = Some(*sent);
                    self.send(*request, ctx);
                }
            },
        }
        Handled::Consumed
    }

    /// Every write: `busy`, the notice, the request.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.notice = Some(Notice::Info(SAVING.to_owned()));
        ctx.request(request);
    }

    // --- state ---------------------------------------------------------------------------------

    /// `j`/`k`: one row, no wrap.
    fn move_cursor(&mut self, down: bool) {
        let last = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.mcp_servers.len().saturating_sub(1));
        self.cursor = if down {
            (self.cursor + 1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
        self.scroll.reset();
        self.notice = None;
    }

    /// The row under the cursor, for the tab's history location: `{index}:{id}`.
    pub(super) fn location_cursor(&self) -> String {
        match self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.mcp_servers.get(self.cursor))
        {
            Some(server) => format!("{}:{}", self.cursor, server.id),
            None => self.cursor.to_string(),
        }
    }

    /// The cursor back on a [`location_cursor`](Self::location_cursor): onto its server wherever
    /// the list now holds it, else on its row. Only while browsing with nothing open.
    pub(super) fn restore_cursor(&mut self, cursor: &str) -> bool {
        let (index, server) = cursor
            .split_once(':')
            .map_or((cursor, None), |(index, server)| {
                (index, server.parse().ok())
            });
        let Ok(index) = index.parse::<usize>() else {
            return false;
        };
        if !matches!(self.mode, Mode::Browse) || self.attach.is_some() {
            return false;
        }
        let last = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.mcp_servers.len().saturating_sub(1));
        self.cursor = index.min(last);
        match server {
            Some(server) => self.select(server),
            None => self.scroll.reset(),
        }
        true
    }

    /// The cursor onto `server`, the record shown from its top.
    fn select(&mut self, server: McpServerId) {
        if let Some(index) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.mcp_servers.iter().position(|row| row.id == server))
        {
            self.cursor = index;
        }
        self.scroll.reset();
    }

    /// Keeps the cursor on a row after the snapshot changed; the pane closes if its server left.
    fn clamp(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        self.cursor = self
            .cursor
            .min(snapshot.mcp_servers.len().saturating_sub(1));
        if let Some(pane) = &mut self.attach {
            if pane.exists(snapshot) {
                pane.clamp(snapshot);
            } else {
                self.attach = None;
            }
        }
    }

    // --- frames --------------------------------------------------------------------------------

    /// The view over `area`: the attachments pane over the whole content while it is open, else
    /// the list and the record, then the notice.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let (notice_text, tone) = match &self.notice {
            Some(Notice::Info(text)) => (text.as_str(), Tone::Info),
            Some(Notice::Error(text)) => (text.as_str(), Tone::Error),
            None => ("", Tone::Info),
        };
        let mut rows = wrapped(
            notice_text,
            usize::from(area.width.saturating_sub(1)).max(1),
        );
        rows.truncate(NOTICE_LINES);
        let notice_height = u16::try_from(rows.len().max(1)).unwrap_or(1);
        let [content, notice_row] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(notice_height)]).areas(area);
        match (&self.attach, &self.snapshot) {
            (Some(pane), Some(snapshot)) => pane.render(frame, content, snapshot, ctx),
            _ => self.render_browse(frame, content, ctx),
        }
        frame.render_widget(
            Paragraph::new(
                rows.into_iter()
                    .map(|line| notice(&format!(" {line}"), tone, ctx.theme))
                    .collect::<Vec<_>>(),
            ),
            notice_row,
        );
    }

    /// The keys of the shown mode, through [`key_stack`](Self::key_stack).
    pub(super) fn key_hint(&self, keys: &Keys) -> String {
        match (&self.attach, &self.snapshot) {
            (Some(pane), Some(_)) => pane.hint(keys),
            _ => match &self.mode {
                Mode::Browse => keys.hint(views::MCP_BROWSE, BROWSE_HINT),
                Mode::Form(_) => keys.hint(views::MCP_FORM, FORM_HINT),
                Mode::Confirm { .. } => keys.hint(views::MCP_CONFIRM, CONFIRM_HINT),
            },
        }
    }

    /// The list on the left, the pane on the right, each a kit pane. The list leads the view, so
    /// it carries the view chips.
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let areas = ctx.split(area, super::library::LAYOUT);
        let prompt = !matches!(self.mode, Mode::Browse);
        let (list_area, pane_area) = capture_area(&areas, area, prompt);
        let activity = areas.activity.is_some_and(|rect| !rect.is_empty())
            && !(areas.list.is_none() && areas.detail.is_empty());
        let (list_focus, pane_focus) = if prompt {
            (Focus::Unfocused, holder(ctx, activity))
        } else {
            (ctx.pane_focus(PaneId::List), ctx.pane_focus(PaneId::Detail))
        };
        let list_area = list_area.filter(|rect| !rect.is_empty());
        if let Some(list_area) = list_area {
            self.render_list(frame, list_area, list_focus, ctx);
        }
        if !pane_area.is_empty() {
            let leads = list_area.is_none();
            self.render_pane(frame, pane_area, pane_focus, prompt, leads, ctx);
        }
    }

    /// The servers as a kit list: one row each, name and transport, the cursor in the gutter.
    fn render_list(&self, frame: &mut Frame<'_>, area: Rect, focus: Focus, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let inner = KitPane::inner(area);
        let rows: Vec<Row<'static>> = self
            .snapshot
            .iter()
            .flat_map(|snapshot| snapshot.mcp_servers.iter())
            .map(|server| Row {
                line: Line::styled(browse_row(server), theme.base),
                kind: RowKind::Item,
            })
            .collect();
        let selected = (!rows.is_empty()).then_some(self.cursor);
        let offset = list::scroll_into_view(self.cursor, 0, usize::from(inner.height), rows.len());
        list::render(frame, inner, &rows, selected, offset, ctx.accent(), theme);
        let hints = key_line(ctx.keys().hint(views::MCP_BROWSE, LIST_HINT), theme);
        super::framed(view_chips(KitPane::new(""), VIEW), focus, hints, ctx)
            .scrollbar(rows.len(), offset)
            .render(frame, area, theme);
    }

    /// The right-hand pane: the form while it is open, else the record through the views' shared
    /// [`render_detail`].
    fn render_pane(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        prompt: bool,
        leads: bool,
        ctx: &Ctx<'_>,
    ) {
        if let Mode::Form(form) = &self.mode {
            self.render_form(frame, area, form, focus, leads, ctx);
            return;
        }
        let (title, detail) = self.detail(ctx);
        let pane = DetailPane {
            title: &title,
            focus,
            leads: leads.then_some(VIEW),
            scroll: &self.scroll,
            rows: &self.pane_rows,
            stack: views::MCP_BROWSE,
            detail_hint: DETAIL_HINT,
            scroll_hint: SCROLL_HINT,
            prompt_hint: prompt.then(|| self.key_hint(ctx.keys())),
        };
        render_detail(frame, area, pane, detail, ctx);
    }

    /// The form in a kit pane: the inputs of its transport through `kit::form`, scrolled so the
    /// focused one is on screen; the refusal of the draft under the input it is about.
    fn render_form(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        form: &ServerForm,
        focus: Focus,
        leads: bool,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let keys = ctx.keys();
        let title = match form.edit {
            Some(_) => format!("{} \u{b7} edit", form.text(SField::Name)),
            None => "new MCP server".to_owned(),
        };
        let refusal = (!form.is_blank() || form.edit.is_some())
            .then(|| form.refusal(self.snapshot.as_ref()))
            .flatten();
        let transport_help = keys.hint(views::MCP_FORM, TRANSPORT_HINT);
        let visible = SField::visible(form.transport);
        let mut widget = FormWidget::new().editing(form.focus != SField::Transport);
        for (index, field) in visible.iter().copied().enumerate() {
            let value = match form.field(field) {
                Some(text) => Value::Text(text),
                None => Value::Choice(label::mcp_transport(form.transport)),
            };
            let mut row = Field::new(field.label(), value);
            let help = match field {
                SField::Transport if !transport_help.is_empty() => Some(transport_help.as_str()),
                SField::Command => Some(COMMAND_HELP),
                SField::Args => Some(ARGS_HELP),
                SField::Url => Some(URL_HELP),
                SField::Headers | SField::Env => Some(PAIRS_HELP),
                SField::Allow => Some(ALLOW_HELP),
                SField::Deny => Some(DENY_HELP),
                _ => None,
            };
            if let Some(help) = help {
                row = row.help(help);
            }
            if let Some((blamed, sentence)) = &refusal
                && *blamed == field
            {
                row = row.error(sentence.as_str());
            }
            widget = widget.field(row);
            if field == form.focus {
                widget = widget.focus(index);
            }
        }
        let inner = KitPane::inner(area);
        let layout = widget.layout(inner.width, ctx.accent(), theme);
        layout.render(frame, inner, layout.offset(inner.height));
        let hints = key_line(self.key_hint(keys), theme);
        let pane = if leads {
            view_chips_if_room(KitPane::new(&title), &title, VIEW, area.width)
        } else {
            KitPane::new(&title)
        };
        super::framed(pane, focus, hints, ctx).render(frame, area, theme);
    }

    /// The record pane's title and what it shows: a refusal, the empty states, or the server.
    fn detail(&self, ctx: &Ctx<'_>) -> (String, Detail<'_>) {
        let theme = ctx.theme;
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                Detail::Unavailable(format!("{UNAVAILABLE}: {why}")),
            );
        }
        let Some(snapshot) = &self.snapshot else {
            return (
                String::new(),
                Detail::Empty(NOT_READ, NOT_READ_NEXT.to_owned()),
            );
        };
        let Some(server) = snapshot.mcp_servers.get(self.cursor) else {
            let hint = ctx.keys().hint(views::MCP_BROWSE, EMPTY_HINT);
            let next = if hint.is_empty() {
                EMPTY_NEXT.to_owned()
            } else {
                hint
            };
            return (String::new(), Detail::Empty(NO_SERVERS, next));
        };
        let mut lines: Vec<Line<'static>> = Vec::new();
        if !server.description.is_empty() {
            lines.push(Line::styled(server.description.clone(), theme.dim));
            lines.push(Line::default());
        }
        let row = |name: &str, value: String| {
            Line::from(vec![
                Span::styled(format!("{name:<9}"), theme.dim),
                Span::styled(value, theme.base),
            ])
        };
        let more = |value: String| {
            Line::from(vec![
                Span::styled(" ".repeat(9), theme.dim),
                Span::styled(value, theme.base),
            ])
        };
        match server.transport {
            McpTransport::Stdio => {
                let mut command = vec![quote_word(&server.command)];
                command.extend(server.args.iter().map(|arg| quote_word(arg)));
                lines.push(row("command", command.join(" ")));
            }
            McpTransport::Http => {
                lines.push(row("url", server.url.clone().unwrap_or_default()));
            }
        }
        for (name, entries) in [("headers", &server.headers), ("env", &server.env)] {
            for (at, (key, value)) in entries.iter().enumerate() {
                let text = pair(key, value);
                lines.push(if at == 0 { row(name, text) } else { more(text) });
            }
        }
        for (name, tools) in [("allow", &server.allow), ("deny", &server.deny)] {
            if !tools.is_empty() {
                lines.push(row(name, tools.join(" ")));
            }
        }
        (
            format!(
                "{} \u{b7} {}",
                server.name,
                label::mcp_transport(server.transport)
            ),
            Detail::Lines(lines),
        )
    }
}

/// One list row: two spaces, the name fitted to [`NAME_WIDTH`] cells, the transport.
fn browse_row(server: &McpServer) -> String {
    format!(
        "  {} {}",
        cells::fit(&server.name, NAME_WIDTH),
        label::mcp_transport(server.transport)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::tabs::SkillsTab;
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{BindingKey, BindingRef, Scope, SkillId};

    fn empty_snapshot() -> Box<SkillsSnapshot> {
        Box::new(SkillsSnapshot {
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            global: Vec::new(),
            workspaces: Vec::new(),
            projects: Vec::new(),
        })
    }

    /// MOD-127 M2 R3: an MCP attach in flight is this view's own; the replies of a skill's write
    /// (a skill detach in flight in the Library) never free it, whichever arrives first, and its
    /// own reply still does.
    #[test]
    fn a_skills_replies_leave_a_server_attach_in_flight_alone() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: Vec::new(),
        };
        let (top_bar, theme, emit) = (TopBarState::default(), Theme::default(), Emit::default());
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            Keys::compiled(),
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let skill = BindingKey::global(SkillId::new());
        let server = BindingKey::global(McpServerId::new());
        let mut view = McpView {
            busy: Some(REQUEST_NAMES[4]),
            sent: Some(Sent::Binding {
                change: BindingChange::Detach,
                target: "global".to_owned(),
            }),
            ..McpView::default()
        };
        let foreign = [
            StoreReply::SkillsStale {
                snapshot: empty_snapshot(),
                what: StaleWhat::Skill(SkillId::new()),
            },
            StoreReply::SkillsStale {
                snapshot: empty_snapshot(),
                what: StaleWhat::Version(SkillId::new()),
            },
            StoreReply::SkillsStale {
                snapshot: empty_snapshot(),
                what: StaleWhat::Binding(skill),
            },
            StoreReply::SkillWritten {
                snapshot: Ok(empty_snapshot()),
                outcome: SkillWrite::Detached { key: skill },
            },
            StoreReply::SkillWritten {
                snapshot: Ok(empty_snapshot()),
                outcome: SkillWrite::Attached {
                    key: skill,
                    updated_at: Utc::now(),
                },
            },
            StoreReply::SkillWritten {
                snapshot: Err("store unreachable: gone".to_owned()),
                outcome: SkillWrite::Detached { key: skill },
            },
        ];
        for reply in &foreign {
            view.on_reply(reply, &mut ctx);
            assert_eq!(view.busy, Some(REQUEST_NAMES[4]), "{reply:?}");
            assert!(view.sent.is_some(), "{reply:?}");
        }
        view.on_reply(
            &StoreReply::SkillsStale {
                snapshot: empty_snapshot(),
                what: StaleWhat::Binding(server),
            },
            &mut ctx,
        );
        assert_eq!(view.busy, None, "its own stale reply frees it");
        assert!(view.sent.is_none());
    }

    #[test]
    fn words_split_like_a_shell_and_read_back() {
        let args = vec![
            "--stdio".to_owned(),
            "--root".to_owned(),
            "/srv/my docs".to_owned(),
            String::new(),
            "say \"hi\"".to_owned(),
            "C:\\tools\\x".to_owned(),
            "it's".to_owned(),
        ];
        let line = words(&args);
        assert_eq!(split_words(&line).as_deref(), Ok(args.as_slice()));
        assert_eq!(
            split_words("a  'b c'\td\\ e").as_deref(),
            Ok(["a", "b c", "d e"].map(str::to_owned).as_slice())
        );
        assert!(split_words("\"open").is_err());
        assert_eq!(split_words("   ").as_deref(), Ok([].as_slice()));
    }

    #[test]
    fn entries_read_a_reference_a_literal_and_an_escaped_at() {
        let entries = vec![
            ("LOG".to_owned(), EnvValue::Literal("info".to_owned())),
            (
                "KEY".to_owned(),
                EnvValue::Secret("DOCS_API_KEY".to_owned()),
            ),
            ("MAIL".to_owned(), EnvValue::Literal("@home".to_owned())),
            ("NOTE".to_owned(), EnvValue::Literal("a b".to_owned())),
            ("EMPTY".to_owned(), EnvValue::Literal(String::new())),
        ];
        let line = pairs(&entries);
        assert_eq!(
            line,
            "LOG=info KEY=@DOCS_API_KEY MAIL=@@home NOTE=\"a b\" EMPTY=\"\""
        );
        assert_eq!(parse_pairs(&line), Ok(entries));
        assert!(parse_pairs("NOEQUALS").is_err());
        assert!(parse_pairs("=x").is_err());
        assert!(parse_pairs("K=@").is_err());
    }

    #[test]
    fn tool_names_split_on_spaces_and_commas() {
        assert_eq!(
            tool_names("search, fetch  list"),
            ["search", "fetch", "list"]
        );
        assert!(tool_names(" ").is_empty());
    }

    #[test]
    fn a_refusal_is_blamed_on_the_input_it_is_about() {
        assert_eq!(
            blamed(
                "the env `API_TOKEN` looks like a secret: store it",
                SField::Name
            ),
            SField::Env
        );
        assert_eq!(
            blamed(
                "the header `Authorization` looks like a secret",
                SField::Name
            ),
            SField::Headers
        );
        assert_eq!(
            blamed("a stdio mcp_server needs a command", SField::Name),
            SField::Command
        );
        assert_eq!(blamed("something else", SField::Deny), SField::Deny);
    }

    /// MOD-127 M2 R3: a refusal is blamed on the field its lead words name, the text before the
    /// first backtick; a key or value the user typed, which the core's sentence quotes, never
    /// moves it. The sentences are the core's own.
    #[test]
    fn a_refusal_is_blamed_by_its_lead_words_not_by_a_typed_key() {
        let draft = |transport, headers, env| McpServerDraft {
            name: "docs".to_owned(),
            description: String::new(),
            transport,
            command: match transport {
                McpTransport::Stdio => "mcp-docs".to_owned(),
                McpTransport::Http => String::new(),
            },
            args: Vec::new(),
            url: (transport == McpTransport::Http).then(|| "https://docs.example.test".to_owned()),
            headers,
            env,
            allow: Vec::new(),
            deny: Vec::new(),
        };
        let literal = |key: &str| vec![(key.to_owned(), EnvValue::Literal("v".to_owned()))];
        for (draft, field) in [
            (
                draft(McpTransport::Http, literal("x-command-token"), Vec::new()),
                SField::Headers,
            ),
            (
                draft(McpTransport::Http, literal("x-url-token"), Vec::new()),
                SField::Headers,
            ),
            (
                draft(McpTransport::Stdio, Vec::new(), literal("my_command_token")),
                SField::Env,
            ),
            (
                draft(McpTransport::Stdio, Vec::new(), literal("URL_TOKEN")),
                SField::Env,
            ),
            (
                draft(McpTransport::Stdio, Vec::new(), literal("bad key")),
                SField::Env,
            ),
        ] {
            let sentence = draft_refusal(&draft).expect("the core refuses it");
            assert_eq!(blamed(&sentence, SField::Name), field, "{sentence}");
        }
        // The shape sentences, which name no typed value, keep their fields.
        let mut stdio = draft(McpTransport::Stdio, Vec::new(), Vec::new());
        stdio.command.clear();
        let sentence = draft_refusal(&stdio).expect("no command");
        assert_eq!(
            blamed(&sentence, SField::Name),
            SField::Command,
            "{sentence}"
        );
        let mut http = draft(McpTransport::Http, Vec::new(), Vec::new());
        http.url = Some("ftp://docs.example.test".to_owned());
        let sentence = draft_refusal(&http).expect("a wrong scheme");
        assert_eq!(blamed(&sentence, SField::Name), SField::Url, "{sentence}");
    }

    /// MOD-127 M2 R3: a parse error names the entry's place and, at most, its key; the typed
    /// word, which may be a pasted secret, is never echoed.
    #[test]
    fn a_pair_error_never_echoes_a_typed_word() {
        const SENTINEL: &str = "sentinel-secret-0451";
        let text = format!("A=1 {SENTINEL}");
        let why = parse_pairs(&text).expect_err("no `=`");
        assert!(why.contains("entry 2"), "{why}");
        assert!(!why.contains(SENTINEL), "{why}");
        let text = format!("A=1 B=2 ={SENTINEL}");
        let why = parse_pairs(&text).expect_err("no name");
        assert!(why.contains("entry 3"), "{why}");
        assert!(!why.contains(SENTINEL), "{why}");
        assert_eq!(
            parse_pairs("A=1 B=@").expect_err("no secret name"),
            "`B=@` names no secret after the `@`"
        );
    }

    #[test]
    fn the_delete_question_names_the_count_seen_here() {
        assert_eq!(
            delete_question("docs", 2, Some("y")),
            "delete `docs` and its 2 attachments here, and any in other workspaces or projects? \
             y deletes, any other key keeps it"
        );
        assert_eq!(
            delete_question("docs", 1, None),
            "delete `docs` and its 1 attachment here, and any in other workspaces or projects?"
        );
        assert_eq!(
            delete_question("docs", 0, None),
            "delete `docs` and any attachments in other workspaces or projects?"
        );
    }

    #[test]
    fn a_server_binds_as_a_server_reference() {
        let id = McpServerId::new();
        assert_eq!(BindingRef::from(id), BindingRef::McpServer(id));
    }
}

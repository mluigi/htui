//! The editors' agent help (MOD-55 P8, blueprint §4): `Ctrl+G` in the Templates editor (and, from
//! T5, the Library editor) asks an agent to rewrite the body being edited.
//!
//! The component owns the request field, the agent choice and one help turn's life: it sends
//! [`StoreRequest::EditHelp`] with the body as it was when the help opened, follows the turn's
//! frames, and offers the reply's last fenced block as a proposal diffed against that body. The
//! view that embeds it maps its [`HelpOutcome`]s onto its own notice and buffer, so accepting a
//! proposal goes through the view's own gate (`parse` for a template) and saving stays `Ctrl+S`.
//!
//! **Telling frames apart (A-1).** `Tab::on_reply` gets no `seq`, and only `ChatAccepted` carries a
//! `step_id`. What keeps a stray frame harmless is the shell's staleness index (`App::is_fresh`,
//! newest request per kind and origin), one help per tab (the editor that owns it captures input),
//! and the per-state table in [`AgentHelp::on_reply`]: a state reacts only to the frames it can
//! receive. A help that is cancelling cannot be abandoned (A-2): it stays until the stream's
//! terminal frame, which always comes, so that frame never lands on the next help.

use core::cell::Cell;

use crossterm::event::KeyEvent;
use htui_agent::event::{DriverEvent, StopReason};
use htui_core::model::{AgentId, ProjectId, StepId};
use htui_core::prompt::edit_help::{self, HelpPrompt, HelpTarget};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{ChatFrame, EDIT_HELP, StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::modal_rest;
use crate::ui::{FieldOutcome, TextField, Theme, diff};

/// `StoreRequest::ChatCancel`'s request name: its `Failed` (the chat ended before the cancel
/// reached it) is a cancelling help's end (H-1).
const CHAT_CANCEL: &str = "chat_cancel";

/// `StoreRequest::Agents`'s request name: its `Failed` before the list arrived ends the help
/// (MOD-55 review L3).
const AGENTS: &str = "agents";

/// The editor's own chords while the help is open.
const HELP_OPEN: &str = "agent help is open \u{2014} Esc leaves it first";

/// `Enter` on an empty request.
const ASK_SOMETHING: &str = "type what you want changed";

/// `Enter` before the agents were read.
const READING_AGENTS: &str = "reading the agents\u{2026}";

/// `Enter` with no enabled agent.
const NO_AGENT: &str = "no enabled agent \u{2014} add one in Settings > Agents";

/// `Esc` while the help waits.
const CANCELLING: &str = "cancelling\u{2026}";

/// The cancelled help's end.
const CANCELLED: &str = "agent help cancelled";

/// `Esc`/`n` on a proposal.
const DISCARDED: &str = "proposal discarded";

/// The first accept of a proposal that holds a new mask marker (P9).
const MASKED: &str = "the proposal holds [REDACTED] where a masked value was \u{2014} Enter again \
                      replaces the body anyway";

/// What a view says once an accepted proposal replaced its buffer.
pub(super) const ACCEPTED: &str = "proposal accepted \u{2014} Ctrl+S saves it";

/// The hint while the request is typed (MOD-67 M4 D9): `Enter` and `Esc` are the field's.
const ASKING_HINT: HintSpec = &[
    Hint::Text("Enter ask"),
    Hint::Pair(Act::SkillsHelpPrevAgent, Act::SkillsHelpNextAgent, "agent"),
    Hint::Text("Esc back"),
];

/// The hint once a cancel is on its way.
const CANCELLING_HINT: HintSpec = &[Hint::Text(CANCELLING)];

/// The hint while the agent works.
const WAITING_HINT: HintSpec = &[
    Hint::Text("waiting for the agent"),
    Hint::One(Act::SkillsHelpCancel, "cancel"),
];

/// The hint over a proposal.
const PROPOSAL_HINT: HintSpec = &[
    Hint::One(Act::SkillsHelpAccept, "accept"),
    Hint::All(Act::ConfirmNo, "discard"),
    Hint::Pair(Act::PaneScrollDown, Act::PaneScrollUp, "scroll"),
    Hint::Pair(Act::PanePageUp, Act::PanePageDown, "page"),
];

/// The hint over an answer with nothing to accept.
const ANSWERED_HINT: HintSpec = &[
    Hint::Text("nothing to accept"),
    Hint::All(Act::ConfirmNo, "close"),
    Hint::Pair(Act::PaneScrollDown, Act::PaneScrollUp, "scroll"),
];

/// The panel's height under the draft, borders included: the request line and the agent line.
const PANEL_HEIGHT: u16 = 4;

/// The request line's label.
const ASK_LABEL: &str = "ask: ";

/// One line of report for the view's notice row (each view maps it onto its own `Notice`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Report {
    /// Dim.
    Info(String),
    /// `theme.error`.
    Error(String),
}

/// What a key or a reply did to the help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HelpOutcome {
    /// Taken; nothing for the view to do.
    Consumed,
    /// Not the help's: `global.next_tab`/`prev_tab` and what the stack's modal global layer lets
    /// through (CONTROL, ALT, function keys) go to the shell.
    Pass,
    /// The help stays open; the view shows this.
    Note(Report),
    /// The help closes, the buffer as it was; the view shows the report, if any.
    Close(Option<Report>),
    /// The help closes and this text replaces the buffer.
    Accept(String),
}

/// The editors' agent help (MOD-55 P8). `Debug` is hand-written: lengths, never the body, the
/// request, the reply or the proposal.
pub(super) struct AgentHelp {
    /// What is edited, for the request.
    target: HelpTarget,
    /// The run's project (P7).
    project: ProjectId,
    /// The buffer when the help opened: what is sent and what the diff is against (PRD OQ-4).
    sent: String,
    /// Enabled agents from the last `Agents` reply, in its order; `None` before it.
    agents: Option<Vec<(AgentId, String)>>,
    /// Index into `agents`.
    agent: usize,
    /// The request as typed; kept after sending so the panel still shows it.
    request: TextField,
    /// Where it is.
    state: State,
    /// The Proposal/Answered pane's first drawn row.
    scroll: Scroll,
    /// The pane's rows at the last draw, what `scroll` clamps against.
    rows: Cell<usize>,
}

/// Where a help is.
enum State {
    /// Typing the request, choosing the agent.
    Asking,
    /// `EditHelp` sent, no `ChatAccepted` yet.
    Starting {
        /// The agent asked.
        agent: String,
        /// `Esc` was pressed: the `ChatCancel` goes out with the acceptance (A-2).
        cancel: bool,
    },
    /// Accepted; the reply's text accumulates.
    Streaming {
        /// The step a `ChatCancel` names.
        step_id: StepId,
        /// The agent asked.
        agent: String,
        /// The reply so far.
        reply: String,
    },
    /// `ChatCancel` sent; waiting for the stream's terminal frame.
    Cancelling,
    /// The reply's last fenced block, diffed against the body sent.
    Proposal {
        /// The agent that answered.
        agent: String,
        /// The block's content: what replaces the buffer.
        proposed: String,
        /// `diff::unified(sent, proposed)`.
        unified: String,
        /// The proposal holds a mask marker the body sent did not (P9).
        masked: bool,
        /// The first accept of a masked proposal was refused.
        armed: bool,
        /// The reply's closed fenced blocks (MOD-55 review L2): above one, the title says the
        /// last was proposed, which may be a trailing example rather than the body.
        blocks: usize,
    },
    /// A reply with nothing to accept: no block, or the body as sent.
    Answered {
        /// The agent that answered.
        agent: String,
        /// The reply.
        text: String,
        /// The block was the body as sent.
        same: bool,
    },
}

/// Lengths, never the text (H-11).
impl core::fmt::Debug for State {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Asking => f.write_str("Asking"),
            Self::Starting { agent, cancel } => f
                .debug_struct("Starting")
                .field("agent", agent)
                .field("cancel", cancel)
                .finish(),
            Self::Streaming {
                step_id,
                agent,
                reply,
            } => f
                .debug_struct("Streaming")
                .field("step_id", step_id)
                .field("agent", agent)
                .field("reply_len", &reply.len())
                .finish(),
            Self::Cancelling => f.write_str("Cancelling"),
            Self::Proposal {
                agent,
                proposed,
                unified,
                masked,
                armed,
                blocks,
            } => f
                .debug_struct("Proposal")
                .field("agent", agent)
                .field("proposed_len", &proposed.len())
                .field("unified_len", &unified.len())
                .field("masked", masked)
                .field("armed", armed)
                .field("blocks", blocks)
                .finish(),
            Self::Answered { agent, text, same } => f
                .debug_struct("Answered")
                .field("agent", agent)
                .field("text_len", &text.len())
                .field("same", same)
                .finish(),
        }
    }
}

/// Lengths, never the text: `sent` is the body, as the editor's draft is (H-11).
impl core::fmt::Debug for AgentHelp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentHelp")
            .field("target", &self.target)
            .field("project", &self.project)
            .field("sent_len", &self.sent.len())
            .field("agents", &self.agents)
            .field("agent", &self.agent)
            .field("request", &self.request)
            .field("state", &self.state)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

impl From<Handled> for HelpOutcome {
    /// `modal_rest`'s answer as the help's (MOD-67 M4, L-A §6.2).
    fn from(handled: Handled) -> Self {
        match handled {
            Handled::Consumed => Self::Consumed,
            Handled::Pass => Self::Pass,
        }
    }
}

impl AgentHelp {
    /// Opens on `body` and asks for the agents (B-1): read fresh each time, and the activation
    /// requests of the Skills tab stay what they were.
    pub(super) fn open(target: HelpTarget, project: ProjectId, body: &str, ctx: &Ctx<'_>) -> Self {
        ctx.request(StoreRequest::Agents);
        Self {
            target,
            project,
            sent: body.to_owned(),
            agents: None,
            agent: 0,
            request: TextField::new(),
            state: State::Asking,
            scroll: Scroll::default(),
            rows: Cell::new(0),
        }
    }

    /// The stack of the help's state (MOD-67 M4 D4, D7): `HELP_ASKING`, `HELP_WAITING`
    /// (starting, streaming, cancelling) or `HELP_PROPOSAL` (proposal, answered).
    pub(super) const fn key_stack(&self) -> Stack<'static> {
        match self.state {
            State::Asking => views::HELP_ASKING,
            State::Starting { .. } | State::Streaming { .. } | State::Cancelling => {
                views::HELP_WAITING
            }
            State::Proposal { .. } | State::Answered { .. } => views::HELP_PROPOSAL,
        }
    }

    /// The stack of the editor under the help, whose verbs (`form.save`,
    /// `form.external_editor`, `skills.ask_agent`) are refused while the help is open.
    const fn editor_stack(&self) -> Stack<'static> {
        match self.target {
            HelpTarget::Skill { .. } => views::LIBRARY_EDITOR,
            HelpTarget::Template { .. } => views::TEMPLATES_EDITOR,
        }
    }

    /// A key while the help is open (blueprint §4.2, MOD-67 M4 D7). The request field sees a
    /// key first while asking (`Enter` asks, `Esc` closes); then the state's own acts through
    /// [`key_stack`](Self::key_stack); then the editor's verbs, resolved through the editor's
    /// stack (a rebound one too), are refused with a note; `global.next_tab`/`prev_tab` pass,
    /// the draft kept (PA-2); anything else is `modal_rest`'s: CONTROL, ALT and function keys
    /// pass, the rest is taken, so the draft under the help is locked.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> HelpOutcome {
        if matches!(self.state, State::Asking) {
            match self.request.on_key(key) {
                FieldOutcome::Consumed => return HelpOutcome::Consumed,
                FieldOutcome::Submit => return self.ask(ctx),
                FieldOutcome::Cancel => return HelpOutcome::Close(None),
                FieldOutcome::Pass => {}
            }
        }
        let stack = self.key_stack();
        let chord = KeyChord::from_event(key);
        let act = ctx.keys().actions(stack, chord).first().copied();
        if let Some(outcome) = act.and_then(|act| self.on_act(act, ctx)) {
            return outcome;
        }
        let refused = ctx
            .keys()
            .actions(self.editor_stack(), chord)
            .iter()
            .any(|act| {
                matches!(
                    act,
                    Act::FormSave | Act::FormExternalEditor | Act::SkillsAskAgent
                )
            });
        if refused {
            return HelpOutcome::Note(Report::Info(HELP_OPEN.to_owned()));
        }
        if matches!(act, Some(Act::NextTab | Act::PrevTab)) {
            return HelpOutcome::Pass;
        }
        modal_rest(stack, chord).into()
    }

    /// One of the state's own acts; `None` when the state has no use for `act`.
    fn on_act(&mut self, act: Act, ctx: &Ctx<'_>) -> Option<HelpOutcome> {
        let count = self.agents.as_ref().map_or(0, Vec::len);
        let outcome = match (&mut self.state, act) {
            (State::Asking, Act::SkillsHelpPrevAgent | Act::SkillsHelpNextAgent) => {
                if count > 0 {
                    self.agent = if act == Act::SkillsHelpNextAgent {
                        (self.agent + 1) % count
                    } else {
                        (self.agent + count - 1) % count
                    };
                }
                HelpOutcome::Consumed
            }
            (State::Starting { cancel, .. }, Act::SkillsHelpCancel) => {
                *cancel = true;
                HelpOutcome::Note(Report::Info(CANCELLING.to_owned()))
            }
            (State::Streaming { step_id, .. }, Act::SkillsHelpCancel) => {
                ctx.request(StoreRequest::ChatCancel { step_id: *step_id });
                self.state = State::Cancelling;
                HelpOutcome::Note(Report::Info(CANCELLING.to_owned()))
            }
            (State::Cancelling, Act::SkillsHelpCancel) => HelpOutcome::Consumed,
            (
                State::Proposal {
                    proposed,
                    masked,
                    armed,
                    ..
                },
                Act::SkillsHelpAccept,
            ) => {
                if *masked && !*armed {
                    *armed = true;
                    HelpOutcome::Note(Report::Error(MASKED.to_owned()))
                } else {
                    HelpOutcome::Accept(core::mem::take(proposed))
                }
            }
            (State::Proposal { .. }, Act::ConfirmNo) => {
                HelpOutcome::Close(Some(Report::Info(DISCARDED.to_owned())))
            }
            (State::Answered { .. }, Act::ConfirmNo | Act::SkillsHelpAccept) => {
                HelpOutcome::Close(None)
            }
            (
                State::Proposal { .. } | State::Answered { .. },
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp,
            ) => self.scroll.apply(act, self.rows.get()).into(),
            _ => return None,
        };
        Some(outcome)
    }

    /// `Enter` while asking: the request goes out, or a note says why not.
    fn ask(&mut self, ctx: &Ctx<'_>) -> HelpOutcome {
        let request = self.request.text().unwrap_or_default();
        if request.trim().is_empty() {
            return HelpOutcome::Note(Report::Error(ASK_SOMETHING.to_owned()));
        }
        let Some(agents) = &self.agents else {
            return HelpOutcome::Note(Report::Info(READING_AGENTS.to_owned()));
        };
        let Some((agent_id, name)) = agents.get(self.agent) else {
            return HelpOutcome::Note(Report::Error(NO_AGENT.to_owned()));
        };
        ctx.request(StoreRequest::EditHelp {
            project_id: self.project,
            agent_id: *agent_id,
            prompt: HelpPrompt {
                target: self.target.clone(),
                body: self.sent.clone(),
                request: request.to_owned(),
            },
        });
        self.state = State::Starting {
            agent: name.clone(),
            cancel: false,
        };
        HelpOutcome::Consumed
    }

    /// A bracketed paste: into the request while asking; dropped otherwise (the draft is locked).
    pub(super) fn on_paste(&mut self, text: &str) {
        if matches!(self.state, State::Asking) {
            self.request.on_paste(text);
        }
    }

    /// A reply addressed to the tab (blueprint §4.3, A-1): each state reacts only to the frames
    /// it can receive; everything else is `Consumed`, which means "nothing to do".
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &Ctx<'_>) -> HelpOutcome {
        let state = core::mem::replace(&mut self.state, State::Cancelling);
        let (state, outcome) = match (state, reply) {
            (State::Asking, StoreReply::Agents(list)) => {
                let enabled: Vec<(AgentId, String)> = list
                    .iter()
                    .filter(|summary| summary.agent.enabled)
                    .map(|summary| (summary.agent.id, summary.agent.name.clone()))
                    .collect();
                self.agent = self.agent.min(enabled.len().saturating_sub(1));
                self.agents = Some(enabled);
                (State::Asking, HelpOutcome::Consumed)
            }
            // MOD-55 review L3: without the list nothing can be asked, and "reading the agents…"
            // would never end; once read, another view's failed read is not this help's.
            (State::Asking, StoreReply::Failed { request, message })
                if *request == AGENTS && self.agents.is_none() =>
            {
                (
                    State::Asking,
                    HelpOutcome::Close(Some(Report::Error(format!(
                        "could not read the agents \u{2014} {message}"
                    )))),
                )
            }
            (State::Starting { agent, cancel }, StoreReply::ChatAccepted { step_id, .. }) => {
                if cancel {
                    ctx.request(StoreRequest::ChatCancel { step_id: *step_id });
                    (State::Cancelling, HelpOutcome::Consumed)
                } else {
                    let streaming = State::Streaming {
                        step_id: *step_id,
                        agent,
                        reply: String::new(),
                    };
                    (streaming, HelpOutcome::Consumed)
                }
            }
            (
                State::Starting { .. } | State::Streaming { .. },
                StoreReply::Failed { request, message },
            ) if *request == EDIT_HELP => (
                State::Cancelling,
                HelpOutcome::Close(Some(Report::Error(message.clone()))),
            ),
            (
                State::Starting { .. } | State::Streaming { .. },
                StoreReply::Chat(ChatFrame::Failed { message }),
            ) => (
                State::Cancelling,
                HelpOutcome::Close(Some(Report::Error(message.clone()))),
            ),
            (
                State::Streaming {
                    step_id,
                    agent,
                    mut reply,
                },
                StoreReply::Chat(ChatFrame::Event(envelope)),
            ) => {
                if let DriverEvent::AssistantChunk(chunk) = &envelope.event {
                    reply.push_str(&chunk.text);
                }
                let streaming = State::Streaming {
                    step_id,
                    agent,
                    reply,
                };
                (streaming, HelpOutcome::Consumed)
            }
            (
                State::Streaming { agent, reply, .. },
                StoreReply::Chat(ChatFrame::Ended { stop_reason }),
            ) if *stop_reason == StopReason::EndTurn => {
                (self.finish(agent, reply), HelpOutcome::Consumed)
            }
            (State::Streaming { .. }, StoreReply::Chat(ChatFrame::Ended { stop_reason })) => (
                State::Cancelling,
                HelpOutcome::Close(Some(Report::Error(format!(
                    "the agent stopped ({stop_reason}) \u{2014} nothing to accept"
                )))),
            ),
            (
                State::Cancelling,
                StoreReply::Chat(ChatFrame::Ended { .. } | ChatFrame::Failed { .. }),
            ) => (
                State::Cancelling,
                HelpOutcome::Close(Some(Report::Info(CANCELLED.to_owned()))),
            ),
            (State::Cancelling, StoreReply::Failed { request, .. })
                if *request == CHAT_CANCEL || *request == EDIT_HELP =>
            {
                (
                    State::Cancelling,
                    HelpOutcome::Close(Some(Report::Info(CANCELLED.to_owned()))),
                )
            }
            (state, _) => (state, HelpOutcome::Consumed),
        };
        self.state = state;
        outcome
    }

    /// `Ended { EndTurn }`: the reply's last fenced block becomes a proposal, unless there is
    /// none or it is the body as sent.
    fn finish(&mut self, agent: String, reply: String) -> State {
        self.scroll.reset();
        let Some(mut proposed) = edit_help::proposal(&reply) else {
            return State::Answered {
                agent,
                text: reply,
                same: false,
            };
        };
        // A block's content always ends in a line ending; the body sent may not, and a lone
        // "no newline at end" hunk would be all the diff said.
        if !self.sent.ends_with('\n') && proposed.ends_with('\n') {
            proposed.pop();
        }
        if proposed == self.sent {
            return State::Answered {
                agent,
                text: reply,
                same: true,
            };
        }
        State::Proposal {
            unified: diff::unified(&self.sent, &proposed, "sent", "proposed"),
            masked: edit_help::holds_mask(&self.sent, &proposed),
            armed: false,
            blocks: edit_help::closed_blocks(&reply),
            agent,
            proposed,
        }
    }

    /// The hint row while the help is open, through [`key_stack`](Self::key_stack) (MOD-67 M4
    /// D9).
    pub(super) fn hint(&self, keys: &Keys) -> String {
        let spec = match self.state {
            State::Asking => ASKING_HINT,
            State::Starting { cancel: true, .. } | State::Cancelling => CANCELLING_HINT,
            State::Starting { .. } | State::Streaming { .. } => WAITING_HINT,
            State::Proposal { .. } => PROPOSAL_HINT,
            State::Answered { .. } => ANSWERED_HINT,
        };
        keys.hint(self.key_stack(), spec)
    }

    /// Draws into `area` (B-2); returns the rect left for the locked draft, `None` when the help
    /// takes it all.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Option<Rect> {
        match &self.state {
            State::Proposal {
                agent,
                unified,
                masked,
                blocks,
                ..
            } => {
                let mut title = format!(" proposal from {agent} \u{b7} sent \u{2192} proposed ");
                if *masked {
                    title.push_str("\u{b7} holds [REDACTED] ");
                }
                // MOD-55 review L2: the last block is proposed; with several, it may be a
                // trailing example rather than the body.
                if *blocks > 1 {
                    title.push_str(&format!("\u{b7} last of {blocks} blocks "));
                }
                let title = if *masked {
                    Line::styled(title, theme.error)
                } else if *blocks > 1 {
                    Line::styled(title, theme.warning)
                } else {
                    Line::from(title)
                };
                self.render_pane(frame, area, title, diff::lines(unified, theme));
                None
            }
            State::Answered { agent, text, same } => {
                let why = if *same {
                    "the body as sent"
                } else {
                    "no fenced block"
                };
                let lines = text
                    .lines()
                    .map(|line| Line::styled(line.to_owned(), theme.base))
                    .collect();
                let title = Line::from(format!(" {agent} replied \u{b7} {why} "));
                self.render_pane(frame, area, title, lines);
                None
            }
            State::Asking
            | State::Starting { .. }
            | State::Streaming { .. }
            | State::Cancelling => {
                let [draft, panel] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(PANEL_HEIGHT)])
                        .areas(area);
                self.render_panel(frame, panel, theme);
                Some(draft)
            }
        }
    }

    /// The panel under the draft: the request, then the agent or what the help is doing.
    fn render_panel(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let block = Block::new().borders(Borders::ALL).title(" ask an agent ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let budget = inner
            .width
            .saturating_sub(u16::try_from(cell_width(ASK_LABEL)).unwrap_or(u16::MAX));
        let asking = matches!(self.state, State::Asking);
        let mut ask = vec![Span::styled(ASK_LABEL, theme.dim)];
        ask.extend(self.request.line(budget, asking, theme).spans);
        let status = match &self.state {
            State::Asking => {
                let name = match &self.agents {
                    None => "reading\u{2026}",
                    Some(agents) => agents
                        .get(self.agent)
                        .map_or("none enabled", |(_, name)| name.as_str()),
                };
                Line::from(vec![
                    Span::styled("agent: ", theme.dim),
                    Span::styled(name.to_owned(), theme.base),
                ])
            }
            State::Starting { cancel: true, .. } | State::Cancelling => {
                Line::styled(CANCELLING, theme.dim)
            }
            State::Starting { agent, .. } => {
                Line::styled(format!("{agent} is starting\u{2026}"), theme.dim)
            }
            State::Streaming { agent, reply, .. } => Line::styled(
                format!(
                    "{agent} is answering\u{2026} {} chars",
                    reply.chars().count()
                ),
                theme.dim,
            ),
            State::Proposal { .. } | State::Answered { .. } => Line::default(),
        };
        frame.render_widget(Paragraph::new(vec![Line::from(ask), status]), inner);
    }

    /// The Proposal/Answered pane: wrapped and scrolled, its row count kept for the clamp (the
    /// Templates view's `render_browse` rule).
    fn render_pane(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: Line<'static>,
        lines: Vec<Line<'static>>,
    ) {
        let width = area.width.saturating_sub(2);
        let rows: usize = lines
            .iter()
            .map(|line| line.width().div_ceil(usize::from(width).max(1)).max(1))
            .sum();
        self.rows.set(rows);
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            inner,
        );
    }
}

/// Rows and frames the help's tests and the embedding views' tests share.
#[cfg(test)]
pub(super) mod fixtures {
    use chrono::{TimeZone as _, Utc};
    use htui_agent::driver::DriverCaps;
    use htui_agent::event::{DriverEnvelope, DriverEvent, StopReason, TextChunk};
    use htui_core::model::{Agent, AgentId, AgentSummary, Billing, StepId, Transport};
    use serde_json::json;

    use crate::store_worker::{ChatFrame, StoreReply};

    /// One registry row's summary.
    pub(in crate::ui::tabs::skills) fn summary(name: &str, enabled: bool) -> AgentSummary {
        AgentSummary {
            agent: Agent {
                id: AgentId::new(),
                name: name.to_owned(),
                transport: Transport::Acp,
                billing: Billing::Subscription,
                models: Vec::new(),
                default_model: None,
                launch: json!({ "command": "unused", "args": [] }),
                settings: json!({}),
                enabled,
                created_at: htui_core::fixtures::demo_at(0, 0),
                updated_at: htui_core::fixtures::demo_at(0, 0),
            },
            on_box: None,
            user_off: false,
        }
    }

    /// A `ChatAccepted` on `step_id`, no capabilities.
    pub(in crate::ui::tabs::skills) fn accepted(step_id: StepId) -> StoreReply {
        StoreReply::ChatAccepted {
            step_id,
            session_ref: None,
            caps: DriverCaps {
                permission_requests: false,
                edit_proposals: false,
                plans: false,
                thoughts: false,
                follow_up_in_session: false,
                resume: false,
                usage: false,
                usage_mid_turn: false,
                authenticate: false,
            },
            prompts: false,
        }
    }

    /// One `AssistantChunk` frame.
    pub(in crate::ui::tabs::skills) fn chunk(text: &str) -> StoreReply {
        StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
            event: DriverEvent::AssistantChunk(TextChunk {
                text: text.to_owned(),
                message_id: None,
            }),
            raw: None,
            at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
        })))
    }

    /// The stream's end.
    pub(in crate::ui::tabs::skills) fn ended(stop_reason: StopReason) -> StoreReply {
        StoreReply::Chat(ChatFrame::Ended { stop_reason })
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::Scope;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::fixtures::{accepted, chunk, ended, summary};
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::keys::{Keys, load_str, views};
    use crate::store_worker::Origin;
    use crate::ui::tabs::SkillsTab;

    /// What a `Ctx` borrows, held by the test (`templates.rs`'s `Bench`).
    struct Bench {
        /// The Harness's startup scope.
        scope: Scope,
        /// The top bar.
        top_bar: TopBarState,
        /// The default keymap.
        keymap: Keymap,
        /// The default theme.
        theme: Theme,
        /// What the help emitted.
        emit: Emit,
        /// A rebound table, else the compiled defaults.
        keys: Option<Keys>,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: ids::WORKSPACE_GRAPHICS,
                    project_ids: vec![ids::PROJECT_VULKAN],
                },
                top_bar: TopBarState::default(),
                keymap: Keymap::default_global(),
                theme: Theme::default(),
                emit: Emit::default(),
                keys: None,
            }
        }

        /// A bench whose `Ctx` hands out `src` loaded as a key file.
        fn with_keys(src: &str) -> Self {
            Self {
                keys: Some(load_str(src).expect("the test's key file loads")),
                ..Self::new()
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            let ctx = Ctx::new(
                &self.scope,
                &[],
                &self.top_bar,
                &self.keymap,
                &self.theme,
                Origin::Tab(SkillsTab::ID),
                &self.emit,
            );
            match &self.keys {
                Some(keys) => ctx.with_keys(keys),
                None => ctx,
            }
        }

        /// The requests emitted since the last drain.
        fn sent(&self) -> Vec<StoreRequest> {
            self.emit
                .take()
                .into_iter()
                .filter_map(|action| match action {
                    Action::Store(request) => Some(request),
                    _ => None,
                })
                .collect()
        }
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ch(c: char) -> KeyEvent {
        plain(KeyCode::Char(c))
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// The agents `[b (disabled), c, d]`: `c` is the first enabled one.
    fn agents() -> StoreReply {
        StoreReply::Agents(vec![
            summary("b", false),
            summary("c", true),
            summary("d", true),
        ])
    }

    fn target() -> HelpTarget {
        HelpTarget::Template {
            name: "implement".to_owned(),
        }
    }

    /// A help opened on `sent`, its agents read and drained.
    fn asking(bench: &Bench, sent: &str) -> AgentHelp {
        let ctx = bench.ctx();
        let mut help = AgentHelp::open(target(), ids::PROJECT_VULKAN, sent, &ctx);
        assert_eq!(help.on_reply(&agents(), &ctx), HelpOutcome::Consumed);
        bench.sent();
        help
    }

    fn type_request(help: &mut AgentHelp, bench: &Bench, text: &str) {
        for c in text.chars() {
            assert_eq!(help.on_key(ch(c), &bench.ctx()), HelpOutcome::Consumed);
        }
    }

    /// A help on `sent` whose request went out and was accepted on `step`.
    fn streaming(bench: &Bench, sent: &str, step: StepId) -> AgentHelp {
        let mut help = asking(bench, sent);
        type_request(&mut help, bench, "shorter");
        help.on_key(plain(KeyCode::Enter), &bench.ctx());
        bench.sent();
        assert_eq!(
            help.on_reply(&accepted(step), &bench.ctx()),
            HelpOutcome::Consumed
        );
        assert!(matches!(help.state, State::Streaming { .. }), "{help:?}");
        help
    }

    /// A help on `sent` whose reply was `reply`, ended `EndTurn`.
    fn answered(bench: &Bench, sent: &str, reply: &str) -> AgentHelp {
        let mut help = streaming(bench, sent, StepId::new());
        help.on_reply(&chunk(reply), &bench.ctx());
        assert_eq!(
            help.on_reply(&ended(StopReason::EndTurn), &bench.ctx()),
            HelpOutcome::Consumed
        );
        help
    }

    /// H-1 (B-1): the agents are read when the help opens.
    #[test]
    fn open_asks_for_the_agents() {
        let bench = Bench::new();
        let help = AgentHelp::open(target(), ids::PROJECT_VULKAN, "body", &bench.ctx());
        assert!(matches!(bench.sent().as_slice(), [StoreRequest::Agents]));
        assert!(matches!(help.state, State::Asking));
        assert_eq!(
            help.hint(Keys::compiled()),
            "Enter ask \u{b7} Up/Down agent \u{b7} Esc back"
        );
    }

    /// H-2: the first enabled agent is the default, and `Enter` sends the body as it was when the
    /// help opened, for the editor's project and target.
    #[test]
    fn enter_sends_edit_help_to_the_first_enabled_agent() {
        let bench = Bench::new();
        let mut help = asking(&bench, "old body\n");
        // `asking` read a fresh list: the default is read back from the help itself.
        let first = help.agents.as_ref().expect("read")[0].clone();
        assert_eq!(first.1, "c", "the disabled `b` is not offered");
        type_request(&mut help, &bench, "shorter");
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &bench.ctx()),
            HelpOutcome::Consumed
        );
        let sent = bench.sent();
        let [
            StoreRequest::EditHelp {
                project_id,
                agent_id,
                prompt,
            },
        ] = sent.as_slice()
        else {
            panic!("exactly the EditHelp: {sent:?}");
        };
        assert_eq!(*project_id, ids::PROJECT_VULKAN);
        assert_eq!(*agent_id, first.0);
        assert_eq!(prompt.target, target());
        assert_eq!(prompt.body, "old body\n");
        assert_eq!(prompt.request, "shorter");
        assert!(
            matches!(&help.state, State::Starting { agent, cancel: false } if agent == "c"),
            "{help:?}"
        );
        assert_eq!(
            help.hint(Keys::compiled()),
            "waiting for the agent \u{b7} Esc cancel"
        );
    }

    /// H-3: `Up`/`Down` cycle the enabled agents and wrap.
    #[test]
    fn up_and_down_cycle_the_agents_and_wrap() {
        let bench = Bench::new();
        let mut help = asking(&bench, "body");
        let name = |help: &AgentHelp| help.agents.as_ref().expect("read")[help.agent].1.clone();
        assert_eq!(name(&help), "c");
        help.on_key(plain(KeyCode::Down), &bench.ctx());
        assert_eq!(name(&help), "d");
        help.on_key(plain(KeyCode::Down), &bench.ctx());
        assert_eq!(name(&help), "c", "wraps forward");
        help.on_key(plain(KeyCode::Up), &bench.ctx());
        assert_eq!(name(&help), "d", "wraps back");
        assert!(bench.sent().is_empty());
    }

    /// H-4: an empty request, an unread list and an empty list each say so and send nothing.
    #[test]
    fn an_empty_request_or_no_agent_sends_nothing() {
        let bench = Bench::new();
        let ctx = bench.ctx();
        let mut help = AgentHelp::open(target(), ids::PROJECT_VULKAN, "body", &ctx);
        bench.sent();
        type_request(&mut help, &bench, "  ");
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &ctx),
            HelpOutcome::Note(Report::Error(ASK_SOMETHING.to_owned()))
        );
        type_request(&mut help, &bench, "x");
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &ctx),
            HelpOutcome::Note(Report::Info(READING_AGENTS.to_owned()))
        );
        help.on_reply(&StoreReply::Agents(vec![summary("off", false)]), &ctx);
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &ctx),
            HelpOutcome::Note(Report::Error(NO_AGENT.to_owned()))
        );
        assert!(bench.sent().is_empty());
        assert!(matches!(help.state, State::Asking));
    }

    /// H-5: chunks accumulate, a block split across them is one block, and `EndTurn` proposes
    /// its content as a diff against the body sent.
    #[test]
    fn chunks_accumulate_and_end_turn_proposes_the_last_block() {
        let bench = Bench::new();
        let mut help = streaming(&bench, "old\n", StepId::new());
        help.on_reply(&chunk("Here:\n```\nne"), &bench.ctx());
        help.on_reply(&chunk("w\n```\n"), &bench.ctx());
        assert!(
            matches!(&help.state, State::Streaming { reply, .. } if reply == "Here:\n```\nnew\n```\n")
        );
        help.on_reply(&ended(StopReason::EndTurn), &bench.ctx());
        let State::Proposal {
            agent,
            proposed,
            unified,
            masked,
            armed,
            blocks,
        } = &help.state
        else {
            panic!("a proposal: {help:?}");
        };
        assert_eq!(*blocks, 1);
        assert_eq!(agent, "c");
        assert_eq!(proposed, "new\n");
        assert!(unified.contains("-old"), "{unified}");
        assert!(unified.contains("+new"), "{unified}");
        assert!(!masked && !armed);
        assert_eq!(
            help.hint(Keys::compiled()),
            "Enter accept \u{b7} n/Esc discard \u{b7} J/K scroll \u{b7} PgUp/PgDn page"
        );
    }

    /// H-6: a reply with no block, or whose block is the body as sent, has nothing to accept.
    #[test]
    fn a_reply_with_no_block_or_the_same_body_is_answered() {
        let bench = Bench::new();
        let help = answered(&bench, "old\n", "It reads fine as it is.");
        assert!(
            matches!(&help.state, State::Answered { text, same: false, .. } if text == "It reads fine as it is."),
            "{help:?}"
        );
        assert_eq!(
            help.hint(Keys::compiled()),
            "nothing to accept \u{b7} n/Esc close \u{b7} J/K scroll"
        );

        let help = answered(&bench, "old\n", "Unchanged:\n```\nold\n```\n");
        assert!(
            matches!(&help.state, State::Answered { same: true, .. }),
            "{help:?}"
        );
    }

    /// H-7 (A-7, B-4): only `EndTurn` proposes; any other stop closes with nothing to accept.
    #[test]
    fn a_stop_other_than_end_turn_proposes_nothing() {
        let bench = Bench::new();
        for stop in [StopReason::Cancelled, StopReason::MaxTokens] {
            let mut help = streaming(&bench, "old\n", StepId::new());
            help.on_reply(&chunk("```\nnew\n```\n"), &bench.ctx());
            assert_eq!(
                help.on_reply(&ended(stop), &bench.ctx()),
                HelpOutcome::Close(Some(Report::Error(format!(
                    "the agent stopped ({stop}) \u{2014} nothing to accept"
                ))))
            );
        }
    }

    /// H-8: `Enter`/`y` accept the proposal; `Esc`/`n` discard it.
    #[test]
    fn enter_accepts_and_esc_or_n_discards() {
        let bench = Bench::new();
        for key in [plain(KeyCode::Enter), ch('y')] {
            let mut help = answered(&bench, "old\n", "```\nnew\n```");
            assert_eq!(
                help.on_key(key, &bench.ctx()),
                HelpOutcome::Accept("new\n".to_owned())
            );
        }
        for key in [plain(KeyCode::Esc), ch('n')] {
            let mut help = answered(&bench, "old\n", "```\nnew\n```");
            assert_eq!(
                help.on_key(key, &bench.ctx()),
                HelpOutcome::Close(Some(Report::Info(DISCARDED.to_owned())))
            );
        }
        assert!(bench.sent().is_empty());
    }

    /// H-9 (P9): a proposal holding a new mask marker takes a second accept; a marker the sent
    /// body already held is not flagged.
    #[test]
    fn a_masked_proposal_needs_a_second_accept() {
        let bench = Bench::new();
        let mut help = answered(&bench, "key: abc\n", "```\nkey: [REDACTED]\n```");
        assert!(matches!(
            help.state,
            State::Proposal {
                masked: true,
                armed: false,
                ..
            }
        ));
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &bench.ctx()),
            HelpOutcome::Note(Report::Error(MASKED.to_owned()))
        );
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &bench.ctx()),
            HelpOutcome::Accept("key: [REDACTED]\n".to_owned())
        );

        let mut help = answered(&bench, "key: [REDACTED]\n", "```\nthe key: [REDACTED]\n```");
        assert!(matches!(help.state, State::Proposal { masked: false, .. }));
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &bench.ctx()),
            HelpOutcome::Accept("the key: [REDACTED]\n".to_owned())
        );
    }

    /// H-10: `Esc` while the reply streams sends `ChatCancel` for the step; the stream's end
    /// closes the help, whatever its reason, and proposes nothing.
    #[test]
    fn esc_while_streaming_cancels_and_the_end_closes() {
        let bench = Bench::new();
        let step = StepId::new();
        for end in [
            ended(StopReason::Cancelled),
            ended(StopReason::EndTurn),
            StoreReply::Chat(ChatFrame::Failed {
                message: "gone".to_owned(),
            }),
            StoreReply::Failed {
                request: "chat_cancel",
                message: "this chat has ended".to_owned(),
            },
        ] {
            let mut help = streaming(&bench, "old\n", step);
            help.on_reply(&chunk("```\nnew\n```\n"), &bench.ctx());
            assert_eq!(
                help.on_key(plain(KeyCode::Esc), &bench.ctx()),
                HelpOutcome::Note(Report::Info(CANCELLING.to_owned()))
            );
            let sent = bench.sent();
            assert!(
                matches!(sent.as_slice(), [StoreRequest::ChatCancel { step_id }] if *step_id == step),
                "{sent:?}"
            );
            assert_eq!(help.hint(Keys::compiled()), "cancelling\u{2026}");
            assert_eq!(
                help.on_key(plain(KeyCode::Esc), &bench.ctx()),
                HelpOutcome::Consumed,
                "a cancelling help cannot be abandoned (A-2)"
            );
            assert_eq!(
                help.on_reply(&chunk("more"), &bench.ctx()),
                HelpOutcome::Consumed
            );
            assert_eq!(
                help.on_reply(&end, &bench.ctx()),
                HelpOutcome::Close(Some(Report::Info(CANCELLED.to_owned()))),
                "{end:?}"
            );
        }
    }

    /// H-11 (A-2): `Esc` before the acceptance sends nothing (there is no step yet); the
    /// acceptance then sends the `ChatCancel`.
    #[test]
    fn esc_before_acceptance_cancels_on_acceptance() {
        let bench = Bench::new();
        let mut help = asking(&bench, "old\n");
        type_request(&mut help, &bench, "shorter");
        help.on_key(plain(KeyCode::Enter), &bench.ctx());
        bench.sent();
        assert_eq!(
            help.on_key(plain(KeyCode::Esc), &bench.ctx()),
            HelpOutcome::Note(Report::Info(CANCELLING.to_owned()))
        );
        assert!(bench.sent().is_empty(), "no step to cancel yet");
        let step = StepId::new();
        assert_eq!(
            help.on_reply(&accepted(step), &bench.ctx()),
            HelpOutcome::Consumed
        );
        let sent = bench.sent();
        assert!(
            matches!(sent.as_slice(), [StoreRequest::ChatCancel { step_id }] if *step_id == step),
            "{sent:?}"
        );
        assert!(matches!(help.state, State::Cancelling));
        assert_eq!(
            help.on_reply(&ended(StopReason::Cancelled), &bench.ctx()),
            HelpOutcome::Close(Some(Report::Info(CANCELLED.to_owned())))
        );
    }

    /// H-12: a refused start closes the help with the refusal's own sentence.
    #[test]
    fn a_failed_start_closes_with_its_message() {
        let bench = Bench::new();
        for failure in [
            StoreReply::Failed {
                request: EDIT_HELP,
                message: "not sent: the body matches the github_token rule".to_owned(),
            },
            StoreReply::Chat(ChatFrame::Failed {
                message: "not sent: the body matches the github_token rule".to_owned(),
            }),
        ] {
            let mut help = asking(&bench, "old\n");
            type_request(&mut help, &bench, "shorter");
            help.on_key(plain(KeyCode::Enter), &bench.ctx());
            assert_eq!(
                help.on_reply(&failure, &bench.ctx()),
                HelpOutcome::Close(Some(Report::Error(
                    "not sent: the body matches the github_token rule".to_owned()
                )))
            );
        }
        // While streaming, the same.
        let mut help = streaming(&bench, "old\n", StepId::new());
        assert_eq!(
            help.on_reply(
                &StoreReply::Chat(ChatFrame::Failed {
                    message: "recorder refused".to_owned()
                }),
                &bench.ctx()
            ),
            HelpOutcome::Close(Some(Report::Error("recorder refused".to_owned())))
        );
    }

    /// H-13 (A-1, H-3): a stray frame — an earlier help's late end, another request's failure —
    /// changes nothing in a state that does not expect it.
    #[test]
    fn a_stray_end_before_acceptance_is_ignored() {
        let bench = Bench::new();
        let mut help = asking(&bench, "old\n");
        for stray in [
            ended(StopReason::Cancelled),
            chunk("late"),
            accepted(StepId::new()),
        ] {
            assert_eq!(help.on_reply(&stray, &bench.ctx()), HelpOutcome::Consumed);
            assert!(matches!(help.state, State::Asking));
        }
        type_request(&mut help, &bench, "shorter");
        help.on_key(plain(KeyCode::Enter), &bench.ctx());
        for stray in [
            ended(StopReason::Cancelled),
            ended(StopReason::EndTurn),
            chunk("late"),
            StoreReply::Failed {
                request: "chat_cancel",
                message: "this chat has ended".to_owned(),
            },
        ] {
            assert_eq!(help.on_reply(&stray, &bench.ctx()), HelpOutcome::Consumed);
            assert!(
                matches!(help.state, State::Starting { .. }),
                "{stray:?}: {help:?}"
            );
        }
        // Streaming ignores another request's failure and a second acceptance.
        let mut help = streaming(&bench, "old\n", StepId::new());
        for stray in [
            StoreReply::Failed {
                request: "chat_cancel",
                message: "no live chat for step x".to_owned(),
            },
            accepted(StepId::new()),
        ] {
            assert_eq!(help.on_reply(&stray, &bench.ctx()), HelpOutcome::Consumed);
            assert!(matches!(help.state, State::Streaming { .. }), "{stray:?}");
        }
        // A finished help ignores everything.
        let mut help = answered(&bench, "old\n", "```\nnew\n```");
        for stray in [ended(StopReason::EndTurn), chunk("late")] {
            assert_eq!(help.on_reply(&stray, &bench.ctx()), HelpOutcome::Consumed);
            assert!(matches!(help.state, State::Proposal { .. }));
        }
    }

    /// H-14: `Tab` and `Shift+Tab` pass to the shell; every other key is the help's while it
    /// waits, so the draft under it is locked.
    #[test]
    fn tab_passes_and_the_buffer_is_locked() {
        let bench = Bench::new();
        let mut help = streaming(&bench, "old\n", StepId::new());
        assert_eq!(
            help.on_key(plain(KeyCode::Tab), &bench.ctx()),
            HelpOutcome::Pass
        );
        assert_eq!(
            help.on_key(plain(KeyCode::BackTab), &bench.ctx()),
            HelpOutcome::Pass
        );
        assert_eq!(help.on_key(ch('x'), &bench.ctx()), HelpOutcome::Consumed);
        assert_eq!(
            help.on_key(ctrl('c'), &bench.ctx()),
            HelpOutcome::Pass,
            "a chord the help does not own goes to the shell"
        );
        assert!(bench.sent().is_empty());
    }

    /// H-15: the editor's own chords are refused while the help is open, in every state.
    #[test]
    fn ctrl_s_ctrl_e_ctrl_g_are_refused_while_open() {
        let bench = Bench::new();
        let open = HelpOutcome::Note(Report::Info(HELP_OPEN.to_owned()));
        let mut helps = vec![
            asking(&bench, "old\n"),
            streaming(&bench, "old\n", StepId::new()),
        ];
        let mut starting = asking(&bench, "old\n");
        type_request(&mut starting, &bench, "x");
        starting.on_key(plain(KeyCode::Enter), &bench.ctx());
        helps.push(starting);
        let mut cancelling = streaming(&bench, "old\n", StepId::new());
        cancelling.on_key(plain(KeyCode::Esc), &bench.ctx());
        helps.push(cancelling);
        helps.push(answered(&bench, "old\n", "```\nnew\n```"));
        helps.push(answered(&bench, "old\n", "no block"));
        for mut help in helps {
            for c in ['s', 'S', 'e', 'E', 'g', 'G'] {
                assert_eq!(help.on_key(ctrl(c), &bench.ctx()), open, "{c}: {help:?}");
            }
        }
    }

    /// H-16: a paste goes into the request while asking and nowhere afterwards.
    #[test]
    fn paste_reaches_the_request_only_while_asking() {
        let bench = Bench::new();
        let mut help = asking(&bench, "old\n");
        help.on_paste("make it\nshorter");
        assert_eq!(help.request.text(), Some("make itshorter"));
        help.on_key(plain(KeyCode::Enter), &bench.ctx());
        help.on_paste("more");
        assert_eq!(help.request.text(), Some("make itshorter"));
    }

    /// H-17: a body sent without a last line ending gets a proposal without one, so the diff is
    /// not a lone "no newline" hunk.
    #[test]
    fn the_proposal_keeps_the_sent_body_s_last_line_ending() {
        let bench = Bench::new();
        let mut help = answered(&bench, "a", "```\nb\n```");
        assert!(
            matches!(&help.state, State::Proposal { proposed, .. } if proposed == "b"),
            "{help:?}"
        );
        assert_eq!(
            help.on_key(plain(KeyCode::Enter), &bench.ctx()),
            HelpOutcome::Accept("b".to_owned())
        );
        let help = answered(&bench, "a", "```\na\n```");
        assert!(matches!(help.state, State::Answered { same: true, .. }));
    }

    /// H-18 (H-11): `Debug` prints lengths, never the body, the request, the reply or the
    /// proposal.
    #[test]
    fn help_debugs_lengths_only() {
        let bench = Bench::new();
        let mut help = asking(&bench, "secret-body\n");
        type_request(&mut help, &bench, "secret-request");
        assert!(!format!("{help:?}").contains("secret"), "{help:?}");
        help.on_key(plain(KeyCode::Enter), &bench.ctx());
        help.on_reply(&accepted(StepId::new()), &bench.ctx());
        help.on_reply(&chunk("secret-reply\n```\nsecret-"), &bench.ctx());
        let shown = format!("{help:?}");
        assert!(!shown.contains("secret"), "{shown}");
        help.on_reply(&chunk("proposal\n```\n"), &bench.ctx());
        help.on_reply(&ended(StopReason::EndTurn), &bench.ctx());
        assert!(matches!(help.state, State::Proposal { .. }), "{help:?}");
        let shown = format!("{help:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("sent_len: 12"), "{shown}");
        let help = answered(&bench, "secret-body\n", "secret-answer");
        let shown = format!("{help:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }

    /// B-2: while asking, the help takes a 4-row panel under the draft and hands the rest back;
    /// a proposal takes the whole area and flags a mask in its title.
    #[test]
    fn the_panel_sits_under_the_draft_and_a_proposal_takes_the_area() {
        let bench = Bench::new();
        let mut help = asking(&bench, "old\n");
        type_request(&mut help, &bench, "shorter");
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("a test terminal");
        let mut left = None;
        terminal
            .draw(|frame| left = help.render(frame, frame.area(), &bench.theme))
            .expect("draws");
        assert_eq!(left, Some(Rect::new(0, 0, 60, 8)));
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains(" ask an agent "), "{text}");
        assert!(text.contains("ask: shorter"), "{text}");
        assert!(text.contains("agent: c"), "{text}");

        let help = answered(&bench, "key: abc\n", "```\nkey: [REDACTED]\n```");
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("a test terminal");
        terminal
            .draw(|frame| left = help.render(frame, frame.area(), &bench.theme))
            .expect("draws");
        assert_eq!(left, None);
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("proposal from c \u{b7} sent \u{2192} proposed"),
            "{text}"
        );
        assert!(text.contains("holds [REDACTED]"), "{text}");
        assert!(text.contains("+key: [REDACTED]"), "{text}");
    }

    /// MOD-55 review L3: a failed `Agents` read closes the help with the store's sentence rather
    /// than leaving it on "reading the agents…"; once the agents are read, a stray one changes
    /// nothing.
    #[test]
    fn a_failed_agents_read_closes_the_help() {
        let bench = Bench::new();
        let ctx = bench.ctx();
        let failed = StoreReply::Failed {
            request: "agents",
            message: "the store is unavailable".to_owned(),
        };
        let mut help = AgentHelp::open(target(), ids::PROJECT_VULKAN, "body", &ctx);
        assert_eq!(
            help.on_reply(&failed, &ctx),
            HelpOutcome::Close(Some(Report::Error(
                "could not read the agents \u{2014} the store is unavailable".to_owned()
            )))
        );

        let mut help = asking(&bench, "body");
        assert_eq!(help.on_reply(&failed, &ctx), HelpOutcome::Consumed);
        assert!(matches!(help.state, State::Asking));
        assert_eq!(help.agents.as_ref().map(Vec::len), Some(2));
    }

    /// MOD-55 review L2: a reply with more than one closed block still proposes the last, and
    /// the proposal's title says how many there were; a one-block reply's title does not.
    #[test]
    fn a_reply_with_several_blocks_flags_the_last_one_proposed() {
        let bench = Bench::new();
        let help = answered(
            &bench,
            "old\n",
            "```\nnew\n```\nRun it:\n```sh\ncargo test\n```\n",
        );
        assert!(
            matches!(&help.state, State::Proposal { proposed, blocks: 2, .. } if proposed == "cargo test\n"),
            "{help:?}"
        );
        assert!(format!("{help:?}").contains("blocks: 2"), "{help:?}");
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("a test terminal");
        terminal
            .draw(|frame| {
                help.render(frame, frame.area(), &bench.theme);
            })
            .expect("draws");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("\u{b7} last of 2 blocks"), "{text}");

        let help = answered(&bench, "old\n", "```\nnew\n```");
        assert!(
            matches!(help.state, State::Proposal { blocks: 1, .. }),
            "{help:?}"
        );
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("a test terminal");
        terminal
            .draw(|frame| {
                help.render(frame, frame.area(), &bench.theme);
            })
            .expect("draws");
        let text = buffer_text(terminal.backend().buffer());
        assert!(!text.contains("blocks"), "{text}");
    }

    /// The buffer's symbols, one line per row.
    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let width = usize::from(buffer.area.width);
        buffer
            .content
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// MOD-67 M4 D7: the editor's verbs are refused through its own stack, so a rebound one is
    /// refused and the old chord is just a chord the help passes.
    #[test]
    fn a_rebound_ask_agent_is_refused_while_open() {
        let bench = Bench::with_keys("[skills]\nask_agent = \"f3\"\n");
        let open = HelpOutcome::Note(Report::Info(HELP_OPEN.to_owned()));
        let mut helps = vec![
            asking(&bench, "old\n"),
            streaming(&bench, "old\n", StepId::new()),
        ];
        helps.push(answered(&bench, "old\n", "```\nnew\n```"));
        for mut help in helps {
            assert_eq!(
                help.on_key(plain(KeyCode::F(3)), &bench.ctx()),
                open,
                "{help:?}"
            );
            assert_eq!(
                help.on_key(ctrl('g'), &bench.ctx()),
                HelpOutcome::Pass,
                "{help:?}"
            );
        }
    }

    /// MOD-67 M4 D7 (ANA-26 ALT defect): accepting compares whole chords.
    #[test]
    fn alt_y_accepts_nothing() {
        let bench = Bench::new();
        let mut help = answered(&bench, "old\n", "```\nnew\n```");
        assert_eq!(
            help.on_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
                &bench.ctx()
            ),
            HelpOutcome::Pass
        );
        assert!(matches!(help.state, State::Proposal { .. }), "{help:?}");
    }

    /// MOD-67 M4 (blueprint §1 item 5): `F1` reaches the shell's help from every state.
    #[test]
    fn f1_passes_from_every_state() {
        let bench = Bench::new();
        let helps = [
            asking(&bench, "old\n"),
            streaming(&bench, "old\n", StepId::new()),
            answered(&bench, "old\n", "```\nnew\n```"),
        ];
        for mut help in helps {
            assert_eq!(
                help.on_key(plain(KeyCode::F(1)), &bench.ctx()),
                HelpOutcome::Pass,
                "{help:?}"
            );
        }
        assert!(bench.sent().is_empty());
    }

    /// MOD-67 M4 D4, D7: the stack is the state's.
    #[test]
    fn the_stack_follows_the_state() {
        let bench = Bench::new();
        let mut help = asking(&bench, "old\n");
        assert_eq!(help.key_stack(), views::HELP_ASKING);
        type_request(&mut help, &bench, "x");
        help.on_key(plain(KeyCode::Enter), &bench.ctx());
        assert_eq!(help.key_stack(), views::HELP_WAITING);
        let help = streaming(&bench, "old\n", StepId::new());
        assert_eq!(help.key_stack(), views::HELP_WAITING);
        let help = answered(&bench, "old\n", "```\nnew\n```");
        assert_eq!(help.key_stack(), views::HELP_PROPOSAL);
        let help = answered(&bench, "old\n", "no block");
        assert_eq!(help.key_stack(), views::HELP_PROPOSAL);
    }
}

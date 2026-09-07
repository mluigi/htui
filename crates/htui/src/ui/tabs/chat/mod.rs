//! The Chat tab: a live conversation with an agent (`R-TUI-6`, MOD-2 milestone 3).
//!
//! The tab holds no store handle and no session (`R-NF-3`): it asks for a chat with
//! [`StoreRequest::ChatStart`], is answered with [`StoreReply::ChatAccepted`], and then receives
//! one [`StoreReply::Chat`] frame per recorded event until the session ends. Everything it renders
//! comes out of those frames, which are the recorder's **scrubbed** copies — the screen is never
//! less masked than the row (`R-SEC-3`).

pub mod composer;
pub mod permission;
pub mod transcript;

use htui_agent::driver::{AgentSessionRef, DriverCaps, PermissionAnswer};
use htui_core::model::{AgentSummary, Scope, StepId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::store_worker::{ChatFrame, StoreReply, StoreRequest};
use crate::ui::tabs::registry::{Tab, TabId};
use composer::{Composer, ComposerOutcome};
use crossterm::event::{KeyCode, KeyEvent};
use permission::PermissionStrip;
use transcript::{Transcript, TranscriptRow};

pub use composer::ComposerOutcome as ChatComposerOutcome;

/// What replaces a value the session does not have yet.
const NONE: &str = "\u{2014}";

/// The live chat, once one has been accepted.
#[derive(Debug, Clone)]
pub struct ChatSessionState {
    /// The step every event is recorded against; the key for every later command.
    pub step_id: StepId,
    /// The agent-side session id, for a later `session/load`.
    pub session_ref: Option<AgentSessionRef>,
    /// What this transport can do.
    pub caps: DriverCaps,
    /// How the session ended, once it has.
    pub ended: Option<htui_agent::event::StopReason>,
}

/// The tab.
#[derive(Debug)]
pub struct ChatTab {
    /// The registry, enabled rows only, in name order.
    agents: Vec<AgentSummary>,
    /// Which agent `a` has landed on.
    agent_index: usize,
    /// The live session, or `None` before the first prompt.
    session: Option<ChatSessionState>,
    /// A start that has been asked for and not yet answered.
    pending_start: bool,
    transcript: Transcript,
    composer: Composer,
    /// `Esc` once arms the cancel; `Esc` again ends the session. Any other key disarms it, so an
    /// `Esc` typed to leave the composer cannot end a conversation by itself.
    cancel_armed: bool,
    /// The last refusal, shown in place of the transcript.
    refusal: Option<String>,
}

impl Default for ChatTab {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatTab {
    /// Identity of the Chat tab.
    pub const ID: TabId = TabId("chat");

    /// A tab with no session and no agents read yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            agent_index: 0,
            session: None,
            pending_start: false,
            transcript: Transcript::new(),
            composer: Composer::default(),
            cancel_armed: false,
            refusal: None,
        }
    }

    /// The agent a new chat would talk to.
    #[must_use]
    pub fn agent(&self) -> Option<&AgentSummary> {
        self.agents.get(self.agent_index)
    }

    /// The live session, for assertions.
    #[must_use]
    pub const fn session(&self) -> Option<&ChatSessionState> {
        self.session.as_ref()
    }

    /// Sends what the composer submitted: the first text starts a chat, later ones are follow-ups.
    fn submit(&mut self, text: String, ctx: &mut Ctx<'_>) {
        // A session that has **ended** is history: its step accepts no more turns, so the next
        // prompt opens a new chat rather than being refused by a step nobody is listening on.
        let live = self
            .session
            .as_ref()
            .filter(|session| session.ended.is_none());
        match live {
            Some(session) => ctx.request(StoreRequest::ChatSend {
                step_id: session.step_id,
                text,
            }),
            None => {
                let started = self
                    .agent()
                    .map(|summary| summary.agent.id)
                    .zip(ctx.projects.first().map(|project| project.project_id));
                let Some((agent_id, project_id)) = started else {
                    self.refusal = Some("no agent and project to chat in".to_owned());
                    return;
                };
                self.pending_start = true;
                self.refusal = None;
                // The transcript of the finished chat stays on screen until the new one is
                // accepted, which is when it is replaced (`on_reply`).
                ctx.request(StoreRequest::ChatStart {
                    project_id,
                    agent_id,
                    model: None,
                    prompt: text,
                });
            }
        }
    }

    /// Answers the parked request with the option a digit picked.
    fn answer(&mut self, digit: char, ctx: &mut Ctx<'_>) -> Handled {
        let (Some(session), Some(row)) = (self.session.as_ref(), self.transcript.parked()) else {
            return Handled::Pass;
        };
        let TranscriptRow::Permission { request_id, .. } = row else {
            return Handled::Pass;
        };
        let Some(option) = PermissionStrip::pick(row, digit) else {
            return Handled::Pass;
        };
        ctx.request(StoreRequest::ChatAnswer {
            step_id: session.step_id,
            request_id: request_id.clone(),
            answer: PermissionAnswer::Selected(option.id),
        });
        Handled::Consumed
    }

    /// The header: who is talking, about which project, in which session.
    fn header(&self, ctx: &Ctx<'_>) -> Line<'static> {
        let agent = self
            .agent()
            .map_or(NONE.to_owned(), |summary| summary.agent.name.clone());
        let model = self
            .agent()
            .and_then(|summary| summary.agent.default_model.clone())
            .unwrap_or_else(|| "default".to_owned());
        let project = ctx
            .projects
            .first()
            .map_or(NONE.to_owned(), |project| project.name.clone());
        let session = self
            .session
            .as_ref()
            .and_then(|state| state.session_ref.as_ref())
            .map_or(NONE.to_owned(), |reference| reference.as_str().to_owned());
        Line::from(vec![
            Span::styled(agent, ctx.theme.accent),
            Span::styled(
                format!(" · {model} · {project} · session {session}"),
                ctx.theme.dim,
            ),
        ])
    }

    /// The line naming what this transport cannot do, or `None` when it can do everything.
    ///
    /// `DriverCaps` is authoritative and `agent.transport` is not (§4.3): the banner is computed
    /// from what the driver answered, so a degraded CLI session says so without the tab knowing
    /// what a CLI session is.
    fn caps_banner(&self) -> Option<String> {
        let caps = self.session.as_ref()?.caps;
        let mut missing: Vec<&str> = Vec::new();
        if !caps.permission_requests {
            missing.push("permission requests");
        }
        if !caps.edit_proposals {
            missing.push("edit proposals");
        }
        if !caps.plans {
            missing.push("plans");
        }
        (!missing.is_empty()).then(|| format!("this agent cannot: {}", missing.join(", ")))
    }

    /// The hint under the transcript.
    ///
    /// A refusal takes this line while there is a live conversation to keep on screen, and is
    /// cleared by the next key or the next frame.
    fn hint(&self) -> String {
        if let Some(refusal) = &self.refusal
            && self.session.is_some()
        {
            return refusal.clone();
        }
        if self.composer.is_active() {
            return "Enter send · Esc leave".to_owned();
        }
        if self.cancel_armed {
            return "Esc again ends this chat".to_owned();
        }
        if self.transcript.parked().is_some() {
            return "1-9 answer · i compose · t thoughts".to_owned();
        }
        match &self.session {
            Some(state) if state.ended.is_some() => "this chat has ended · i compose".to_owned(),
            Some(_) => "i compose · Esc Esc end · t thoughts · j/k scroll".to_owned(),
            None => "i compose the first prompt · a next agent".to_owned(),
        }
    }
}

impl Tab for ChatTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Chat"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped, like the Settings section that lists the same rows: `agent` is a global table.
        vec![StoreRequest::Agents]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // A live session belongs to the project it started in and is **not** dropped by a scope
        // change: the process is still running, and abandoning the handle would orphan it. The
        // header keeps naming the new scope's first project, which is where the *next* chat goes.
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A refusal is transient: it says what the last key could not do, and the next one clears
        // it whatever it was.
        self.refusal = None;
        // The composer owns every key while it is open, so the tab's own single letters stay
        // single letters everywhere else.
        match self.composer.on_key(key) {
            ComposerOutcome::Consumed => return Handled::Consumed,
            ComposerOutcome::Submit(text) => {
                self.submit(text, ctx);
                return Handled::Consumed;
            }
            ComposerOutcome::Leave => return Handled::Consumed,
            ComposerOutcome::Pass => {}
        }

        // A digit answers the parked request, and **every** digit is consumed while one is
        // waiting: a `4` with three options on screen must not fall through to the global
        // tab-select binding and move the user off the conversation the agent is blocked on.
        if let KeyCode::Char(digit @ '1'..='9') = key.code
            && self.transcript.parked().is_some()
        {
            self.answer(digit, ctx);
            self.cancel_armed = false;
            return Handled::Consumed;
        }

        let handled = match key.code {
            KeyCode::Char('i') | KeyCode::Enter => {
                self.composer.enter();
                Handled::Consumed
            }
            KeyCode::Char('a') if self.session.is_none() && !self.agents.is_empty() => {
                self.agent_index = (self.agent_index + 1) % self.agents.len();
                Handled::Consumed
            }
            KeyCode::Esc => {
                match (self.cancel_armed, self.session.as_ref()) {
                    (true, Some(session)) => {
                        ctx.request(StoreRequest::ChatCancel {
                            step_id: session.step_id,
                        });
                        self.cancel_armed = false;
                    }
                    (false, Some(_)) => {
                        self.cancel_armed = true;
                        return Handled::Consumed;
                    }
                    _ => {}
                }
                Handled::Consumed
            }
            _ => self.transcript.on_key(key),
        };
        if key.code != KeyCode::Esc {
            self.cancel_armed = false;
        }
        handled
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Agents(agents) => {
                self.agents = agents
                    .iter()
                    .filter(|summary| summary.agent.enabled)
                    .cloned()
                    .collect();
                self.agent_index = self.agent_index.min(self.agents.len().saturating_sub(1));
            }
            StoreReply::ChatAccepted {
                step_id,
                session_ref,
                caps,
            } => {
                self.pending_start = false;
                self.refusal = None;
                self.session = Some(ChatSessionState {
                    step_id: *step_id,
                    session_ref: session_ref.clone(),
                    caps: *caps,
                    ended: None,
                });
            }
            StoreReply::Chat(ChatFrame::Event(envelope)) => {
                self.transcript.apply(envelope);
                // The banner carries the agent-side session id, and it arrives as a frame rather
                // than in the acceptance because only the session task ever sees it.
                if let (Some(session), Some(reference)) =
                    (self.session.as_mut(), self.transcript.session_ref())
                {
                    session.session_ref = Some(reference.clone());
                }
            }
            StoreReply::Chat(ChatFrame::Ended { stop_reason }) => {
                if let Some(session) = self.session.as_mut() {
                    session.ended = Some(*stop_reason);
                }
            }
            StoreReply::Chat(ChatFrame::Failed { message }) => {
                self.pending_start = false;
                self.refusal = Some(message.clone());
            }
            StoreReply::Failed { request, message } if request.starts_with("chat_") => {
                self.pending_start = false;
                self.refusal = Some(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let block = Block::new().borders(Borders::ALL).title(" Chat ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let banner = self.caps_banner();
        let parked = self.transcript.parked();
        let [header, banner_area, body, strip, composer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(u16::from(banner.is_some())),
            Constraint::Min(1),
            Constraint::Length(u16::from(parked.is_some())),
            Constraint::Length(1),
        ])
        .areas(inner);

        frame.render_widget(Paragraph::new(self.header(ctx)), header);
        if let Some(text) = banner {
            frame.render_widget(
                Paragraph::new(Line::styled(text, ctx.theme.error)),
                banner_area,
            );
        }

        // A refusal never replaces a live conversation: it is one line under it, because losing
        // the transcript to "answer the permission request first" would cost far more than the
        // message is worth. Only a chat that never started shows it in the body.
        if self.session.is_none()
            && let Some(refusal) = &self.refusal
        {
            frame.render_widget(
                Paragraph::new(Line::styled(refusal.clone(), ctx.theme.error))
                    .wrap(Wrap { trim: true }),
                body,
            );
        } else if self.session.is_none() && !self.pending_start {
            let text = if self.agents.is_empty() {
                "no agent is registered on this box".to_owned()
            } else {
                "type a prompt to start a chat".to_owned()
            };
            frame.render_widget(Paragraph::new(Line::styled(text, ctx.theme.dim)), body);
        } else {
            let lines = self.transcript.lines(body.height as usize, ctx.theme);
            frame.render_widget(Paragraph::new(lines), body);
        }

        if let Some(row) = parked {
            PermissionStrip::render(frame, strip, row, ctx.theme);
        }
        Composer::render(frame, composer, &self.composer, &self.hint(), ctx.theme);
    }
}

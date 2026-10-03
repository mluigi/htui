//! The Chat tab: a live conversation with an agent (`R-TUI-6`, MOD-2 milestone 3).
//!
//! The tab holds no store handle and no session (`R-NF-3`): it asks for a chat with
//! [`StoreRequest::ChatStart`], is answered with [`StoreReply::ChatAccepted`], and then receives
//! one [`StoreReply::Chat`] frame per recorded event until the session ends. Everything it renders
//! comes out of those frames, which are the recorder's **scrubbed** copies — the screen is never
//! less masked than the row (`R-SEC-3`).
//!
//! Since milestone 4 the tab has a second mode: a [`ReplayState`] holds one **past** step, opened
//! by a [`StoreReply::StepEvents`] the shell addressed here on the Runs pane's behalf (D39). It
//! is a read of persisted rows and nothing more — the rows are decoded into the same envelopes a
//! live session sends and rendered by the same [`Transcript`] (`R-HIS-2`, `R-TUI-6`), and while
//! it is open the tab sends nothing at all (D40). A live session underneath keeps receiving its
//! frames the whole time and is exactly where it was when `Esc` closes the replay.
//!
//! Milestone 4 also let a chat start with the store unreachable, in which case its rows went to the
//! offline buffer instead of to Postgres, and the header said so (D42) from a writer label the
//! acceptance carried. Since MOD-25 an offline chat is refused before it starts and the buffer has
//! been removed; CLEAN-7 took the label off `StoreReply::ChatAccepted`.
//!
//! Since MOD-4 milestone 6 the tab also drives a **promoted graph step** (plan D165): the Runs
//! pane's `p` asks the shell to promote on this tab's behalf, the `Orch(Promoted)` reply names
//! the step, and the session the chat runtime binds to it answers at the same address. The header
//! then reads `promoted · <phase> · <resumed | handoff> · …`, and every command is keyed by the
//! step's own id as it is for any chat. `Esc Esc` ends the session and leaves the step promoted.

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
use crate::run_worker::{OrchReply, Via};
use crate::store_worker::{ChatFrame, StoreReply, StoreRequest};
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::registry::{Tab, TabId};
use composer::{Composer, ComposerOutcome};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use permission::PermissionStrip;
use transcript::{Transcript, TranscriptRow};

pub use composer::ComposerOutcome as ChatComposerOutcome;

/// What replaces a value the session does not have yet.
const NONE: &str = "\u{2014}";

/// How much of a [`StepId`] the replay header shows — the **last** eight hex digits, never the
/// first.
///
/// A `StepId` is a UUIDv7, so its head is a timestamp: two steps of one run share it, and the
/// demo fixture's ids share their first ten characters by construction. The tail is the part that
/// differs, which is the only part worth eight columns of a header that has to name *which* step
/// this is.
const STEP_TAIL: usize = 8;

/// The last line while a replay is open: the three keys that do anything, and the way out.
const REPLAY_HINT: &str = "Esc leave replay · t thoughts · j/k scroll";

/// `StoreRequest::name` of a promotion (blueprint D209): its refusal is this tab's to show, since
/// the shell asked for it on the tab's behalf.
const PROMOTE_STEP: &str = crate::run_worker::ORCH_NAMES[5];

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

/// The graph step the live chat drives, when it was opened by a promotion (MOD-4 plan D165)
/// rather than by a prompt: what the header names instead of the agent picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotedHeader {
    /// The promoted step, which the session records against.
    pub step: StepId,
    /// `run_step.phase_name`.
    pub phase: String,
    /// The step's agent.
    pub agent: String,
    /// The step's model.
    pub model: Option<String>,
    /// Whether the chat resumed the step's session or opened with the handoff prompt.
    pub via: Via,
}

/// A past step, reopened read-only (D40, `R-HIS-2`).
///
/// "Read-only" is a property of what the tab can **send**, not of what it draws greyed out: while
/// this is `Some` the whole key path is `ChatTab::on_key_replay`, which is handed no [`Ctx`] and
/// so cannot issue a request even by mistake. The live session underneath is untouched and comes
/// back exactly as it was when `Esc` closes this.
#[derive(Debug)]
pub struct ReplayState {
    /// The step whose log is on screen.
    pub step_id: StepId,
    /// Its rows, decoded and rendered through the live path (D37, `R-TUI-6`).
    pub transcript: Transcript,
    /// The reply carried `None`: this box has no rows for the step (D38). Distinct from a step
    /// that recorded nothing, which is a conversation that happened and said nothing.
    pub missing: bool,
}

/// The one line a replay shows instead of a transcript, or `None` when it has rows to show.
///
/// The two messages are two different facts and must not read as one (D38): a step this box never
/// synced is *unknown*, and rendering it as an empty conversation is the one reading `R-HIS-1`
/// forbids.
fn replay_body(replay: &ReplayState) -> Option<&'static str> {
    if replay.missing {
        Some("this step is not on this box")
    } else if replay.transcript.is_empty() {
        Some("this step recorded nothing")
    } else {
        None
    }
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
    /// The promoted step whose `ChatFollow` this tab has already sent, when its `Promoted` reply
    /// asked for it (T7). The first acceptance of that step then sends none: a second follow
    /// would supersede the first in the shell's staleness index and reopen the window.
    followed: Option<StepId>,
    transcript: Transcript,
    composer: Composer,
    /// `Esc` once arms the cancel; `Esc` again ends the session. Any other key disarms it, so an
    /// `Esc` typed to leave the composer cannot end a conversation by itself.
    ///
    /// Opening a replay disarms it too. The pair is a sequence over the *live* view, and the `Esc`
    /// that leaves a replay is navigation: reading it as the second half would end the
    /// conversation the user opened the replay over, which is the one thing `Esc Esc` exists to
    /// make deliberate.
    cancel_armed: bool,
    /// The last refusal, shown in place of the transcript.
    refusal: Option<String>,
    /// A past step open over the top of all of it, or `None` for the live view (D40).
    replay: Option<ReplayState>,
    /// The promoted step the chat drives, or `None` for a chat the tab started itself (D165).
    promoted: Option<PromotedHeader>,
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
            followed: None,
            transcript: Transcript::new(),
            composer: Composer::default(),
            cancel_armed: false,
            refusal: None,
            replay: None,
            promoted: None,
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

    /// The past step on screen, for assertions.
    #[must_use]
    pub const fn replay(&self) -> Option<&ReplayState> {
        self.replay.as_ref()
    }

    /// The promoted step the chat drives, for assertions.
    #[must_use]
    pub const fn promoted(&self) -> Option<&PromotedHeader> {
        self.promoted.as_ref()
    }

    /// Every key while a replay is open — the whole of D40's read-only guarantee.
    ///
    /// It takes no [`Ctx`], which is the point: a mode that *cannot* reach the request sink is
    /// one a reviewer can check by looking at this signature, rather than by trusting that no
    /// arm below grew a `ctx.request` later. `Esc` leaves; the keys the live tab would act on
    /// (`i`, `Enter`, a digit, `a`) are consumed and do nothing, because a finished step takes no
    /// prompt and answers no permission request (D41); everything else is the transcript's, so
    /// scrolling and `t` work exactly as they do live.
    ///
    /// The digits are *consumed* rather than passed on for the same reason the live tab consumes
    /// them: a `2` typed at a permission row must not fall through to the global tab-select
    /// binding and move the user off the conversation they are reading.
    fn on_key_replay(&mut self, key: KeyEvent) -> Handled {
        if key.code == KeyCode::Esc {
            // Nothing else is touched: the session, its transcript and the composer's text are
            // all exactly as replay found them. The cancel is disarmed rather than restored, and
            // deliberately so - it was disarmed when the replay opened, precisely so that *this*
            // key cannot be the second half of an `Esc Esc` that ends the live chat.
            self.replay = None;
            return Handled::Consumed;
        }
        match key.code {
            KeyCode::Char('1'..='9' | 'i' | 'a') | KeyCode::Enter => Handled::Consumed,
            _ => self
                .replay
                .as_mut()
                .map_or(Handled::Pass, |replay| replay.transcript.on_key(key)),
        }
    }

    /// The replay header: which step is on screen, how much of it there is, and that it is over.
    fn replay_header(replay: &ReplayState, ctx: &Ctx<'_>) -> Line<'static> {
        let id = replay.step_id.to_string();
        let step = id.get(id.len().saturating_sub(STEP_TAIL)..).unwrap_or(&id);
        Line::from(vec![
            Span::styled(
                format!("replay · step …{step} · {} rows", replay.transcript.len()),
                ctx.theme.accent,
            ),
            Span::styled(" · read-only · Esc leave", ctx.theme.dim),
        ])
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
    ///
    /// A promoted step's chat names the step instead (D165): `promoted · <phase> · <resumed |
    /// handoff> · <agent> · <model> · session <ref>`, fitted to `width`. Every other chat's header
    /// is unchanged.
    fn header(&self, ctx: &Ctx<'_>, width: u16) -> Line<'static> {
        let session = self
            .session
            .as_ref()
            .and_then(|state| state.session_ref.as_ref())
            .map_or(NONE.to_owned(), |reference| reference.as_str().to_owned());
        if let Some(promoted) = &self.promoted {
            let via = match promoted.via {
                Via::Resumed => "resumed",
                Via::Handoff => "handoff",
            };
            let model = promoted.model.as_deref().unwrap_or("default");
            // The session ref is the one field here that exists to be read back (a later
            // `session/load`), so the border never cuts it: a line too wide drops the `session `
            // label first, then cuts the fields before the ref with an ellipsis.
            let width = usize::from(width);
            let lead = format!("promoted · {}", promoted.phase);
            let middle = format!(" · {via} · {} · {model}", promoted.agent);
            let mut tail = format!(" · session {session}");
            if cell_width(&lead) + cell_width(&middle) + cell_width(&tail) > width {
                tail = format!(" · {session}");
            }
            let room = width.saturating_sub(cell_width(&tail));
            // A lead cut through a wide cluster comes back a cell short; that leftover cell would
            // only hold a second ellipsis, so a cut lead takes the middle's room with it.
            let middle = if cell_width(&lead) > room {
                String::new()
            } else {
                cells::clip(&middle, room - cell_width(&lead))
            };
            let lead = cells::clip(&lead, room);
            return Line::from(vec![
                Span::styled(lead, ctx.theme.accent),
                Span::styled(format!("{middle}{tail}"), ctx.theme.dim),
            ]);
        }
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
        // A replay offers three keys and no refusal can apply to it: nothing it can do fails.
        if self.replay.is_some() {
            return REPLAY_HINT.to_owned();
        }
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
        // A chord is the shell's (`Ctrl+F`, `Ctrl+C`), never composer text: every other text widget
        // passes it the same way (`TextField::on_key`, MOD-64 F7).
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Handled::Pass;
        }
        // Before the composer, before the digits, before anything that could send: a replay is a
        // mode with no command path at all (D40), and `on_key_replay` is handed no `ctx` to prove
        // it. An open composer underneath keeps its text — it simply cannot be typed into until
        // `Esc` puts the live view back.
        if self.replay.is_some() {
            return self.on_key_replay(key);
        }
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

    /// MOD-22 review M-1: a bracketed paste into the open composer. A replay has no command path
    /// and a closed composer takes nothing, so both drop it.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        if self.replay.is_some() || !self.composer.on_paste(text) {
            return Handled::Pass;
        }
        self.refusal = None;
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
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
                // Blueprint D185: a promotion's chat streams at the promotion's address, an `Orch`
                // one, and the next `Orch` request from this tab (a refused second promotion)
                // would make every frame after it stale. Its frames follow this tab to a chat
                // request's address instead, which nothing else it sends supersedes.
                //
                // The follow went out when the promotion answered (T7), so that no frame can
                // arrive between the two; only an acceptance sent again by a refused promotion
                // (D185) follows here.
                if self.promoted.is_some() && self.followed != Some(*step_id) {
                    ctx.request(StoreRequest::ChatFollow { step_id: *step_id });
                }
                self.followed = None;
                // A chat the tab started itself, after a promoted one has ended, is not the
                // promoted step's: its header goes back to naming the agent picker.
                if self
                    .promoted
                    .as_ref()
                    .is_some_and(|promoted| promoted.step != *step_id)
                {
                    self.promoted = None;
                }
                self.session = Some(ChatSessionState {
                    step_id: *step_id,
                    session_ref: session_ref.clone(),
                    caps: *caps,
                    ended: None,
                });
            }
            // The reply is addressed to this tab even though the Runs pane pressed the key
            // (D39): the shell stamped the request with the Chat tab's origin, so replay arrives
            // through the ordinary read path and the tab needs no store handle for it (`R-NF-3`).
            // Decoding runs here, on the UI task — it is serde over rows the reply already
            // carries, with no I/O.
            StoreReply::StepEvents { step_id, events } => {
                // An `Esc` armed before the replay opened is not the first half of the pair that
                // leaves it: disarming here is what stops the *next* `Esc` after the replay closes
                // from cancelling the live chat outright.
                self.cancel_armed = false;
                self.replay = Some(ReplayState {
                    step_id: *step_id,
                    transcript: Transcript::from_rows(events.as_deref().unwrap_or_default()),
                    missing: events.is_none(),
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
            // MOD-4 plan D165: the shell asked for the promotion on this tab's behalf, and the
            // engine's writes are done. The chat opens on the promoted step: its acceptance and
            // every frame after it arrive at this reply's address. No chat of this process is
            // live, because the promotion's `chat_open` guard refused otherwise (D185), so the
            // old conversation is history and is cleared here.
            StoreReply::Orch(OrchReply::Promoted {
                step,
                phase,
                agent,
                model,
                via,
                ..
            }) => {
                self.promoted = Some(PromotedHeader {
                    step: *step,
                    phase: phase.clone(),
                    agent: agent.clone(),
                    model: model.clone(),
                    via: *via,
                });
                self.transcript = Transcript::new();
                self.session = None;
                self.pending_start = true;
                self.refusal = None;
                self.cancel_armed = false;
                self.replay = None;
                // The chat's frames are addressed at this reply's request, an `Orch` one that any
                // later `Orch` request supersedes, and they can start before the acceptance is
                // read: follow now, not then (T7).
                ctx.request(StoreRequest::ChatFollow { step_id: *step });
                self.followed = Some(*step);
            }
            StoreReply::Failed { request, message }
                if request.starts_with("chat_") || *request == PROMOTE_STEP =>
            {
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

        let replay = self.replay.as_ref();
        // A replay draws neither: the capability banner is a property of the *live* transport,
        // and a strip offering options over a finished step would be a question nobody can
        // answer (D41). The rows themselves still render the request, as history.
        let (banner, parked) = match replay {
            Some(_) => (None, None),
            None => (self.caps_banner(), self.transcript.parked()),
        };
        let [header, banner_area, body, strip, composer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(u16::from(banner.is_some())),
            Constraint::Min(1),
            Constraint::Length(u16::from(parked.is_some())),
            Constraint::Length(1),
        ])
        .areas(inner);

        let header_line = match replay {
            Some(replay) => Self::replay_header(replay, ctx),
            None => self.header(ctx, header.width),
        };
        frame.render_widget(Paragraph::new(header_line), header);
        if let Some(text) = banner {
            frame.render_widget(
                Paragraph::new(Line::styled(text, ctx.theme.error)),
                banner_area,
            );
        }

        if let Some(replay) = replay {
            // The same renderer the live view uses, over rows instead of frames (`R-TUI-6`).
            match replay_body(replay) {
                Some(text) => {
                    frame.render_widget(Paragraph::new(Line::styled(text, ctx.theme.dim)), body)
                }
                None => {
                    let lines = replay.transcript.lines(body.height as usize, ctx.theme);
                    frame.render_widget(Paragraph::new(lines), body);
                }
            }
        }
        // A refusal never replaces a live conversation: it is one line under it, because losing
        // the transcript to "answer the permission request first" would cost far more than the
        // message is worth. Only a chat that never started shows it in the body.
        else if self.session.is_none()
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
        match replay {
            // Not `Composer::render`: the composer belongs to the live view, and a replay must
            // not show an input line for a step that can take no input.
            Some(_) => frame.render_widget(
                Paragraph::new(Line::styled(self.hint(), ctx.theme.dim)),
                composer,
            ),
            None => Composer::render(frame, composer, &self.composer, &self.hint(), ctx.theme),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use chrono::{TimeZone as _, Utc};
    use htui_agent::event::{DriverEnvelope, DriverEvent, TextChunk};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        EventKind, EventRole, ProjectRef, Scope, SessionEvent, StepId, WorkspaceId,
    };
    use serde_json::json;

    /// Everything a [`Ctx`] borrows, kept alive for the length of a test — and the [`Emit`] that
    /// makes "the tab sent nothing" an assertion rather than a claim.
    struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Shell {
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: Vec::new(),
                },
                projects: Vec::new(),
                top_bar: TopBarState::default(),
                keymap: Keymap::new(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keymap,
                &self.theme,
                Origin::Tab(ChatTab::ID),
                &self.emit,
            )
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn row(seq: i32, kind: EventKind, payload: serde_json::Value) -> SessionEvent {
        SessionEvent {
            run_step_id: ids::STEP_PLAN,
            seq,
            turn: 0,
            kind,
            role: EventRole::Agent,
            tool_call_id: None,
            payload,
            raw: None,
            at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
        }
    }

    /// A tab with a live, unfinished chat on screen: what a replay must leave exactly as it is.
    fn live(shell: &Shell) -> ChatTab {
        let mut tab = ChatTab::new();
        tab.on_reply(
            &StoreReply::ChatAccepted {
                step_id: StepId::new(),
                session_ref: Some(AgentSessionRef::new("live-1")),
                caps: DriverCaps {
                    permission_requests: true,
                    edit_proposals: true,
                    plans: true,
                    thoughts: true,
                    follow_up_in_session: true,
                    resume: true,
                    usage: true,
                    usage_mid_turn: true,
                    authenticate: true,
                },
            },
            &mut shell.ctx(),
        );
        tab.on_reply(
            &StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
                event: DriverEvent::AssistantChunk(TextChunk {
                    text: "still talking".to_owned(),
                    message_id: Some("m1".to_owned()),
                }),
                raw: None,
                at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
            }))),
            &mut shell.ctx(),
        );
        // Starting a chat is not what these tests are about: the queue starts empty at the key.
        drop(shell.emit.take());
        tab
    }

    /// Opens a replay of two rows on the tab, as the `StepEvents` reply does.
    fn replay(tab: &mut ChatTab, shell: &Shell, events: Option<Vec<SessionEvent>>) {
        tab.on_reply(
            &StoreReply::StepEvents {
                step_id: ids::STEP_PLAN,
                events,
            },
            &mut shell.ctx(),
        );
    }

    fn two_rows() -> Option<Vec<SessionEvent>> {
        Some(vec![
            row(
                0,
                EventKind::Prompt,
                json!({ "text": "what happened here" }),
            ),
            row(1, EventKind::AssistantText, json!({ "text": "this did" })),
        ])
    }

    /// D40, the property the whole mode exists for: while a replay is open the tab issues no
    /// request at all — not a `ChatStart` from `i`/`Enter`, not a `ChatAnswer` from a digit, not
    /// a `ChatCancel` from `Esc`. The `Emit` is empty afterwards, which is the same thing the
    /// shell would have turned into a `StoreRequest`.
    #[test]
    fn every_key_the_live_tab_acts_on_sends_nothing_in_replay() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        replay(&mut tab, &shell, two_rows());

        let keys = [
            key(KeyCode::Char('i')),
            key(KeyCode::Enter),
            key(KeyCode::Char('1')),
            key(KeyCode::Char('2')),
            key(KeyCode::Char('9')),
            key(KeyCode::Char('a')),
        ];
        for pressed in keys {
            assert_eq!(
                tab.on_key(pressed, &mut shell.ctx()),
                Handled::Consumed,
                "{pressed:?} must not fall through to the global table either"
            );
            assert!(
                shell.emit.is_empty(),
                "{pressed:?} sent something while a finished step was open"
            );
        }
        assert!(
            !tab.composer.is_active(),
            "the composer never opens over a step that can take no more turns"
        );
        assert!(tab.replay().is_some(), "and none of them left replay");

        // What replay *can* do is read: scrolling and folding are the transcript's own keys, and
        // they reach it unchanged.
        for pressed in [key(KeyCode::Char('t')), key(KeyCode::Char('j'))] {
            assert_eq!(
                tab.on_key(pressed, &mut shell.ctx()),
                Handled::Consumed,
                "{pressed:?} is the transcript's, and a replay is still a transcript"
            );
        }
        assert!(shell.emit.is_empty(), "reading sends nothing either");
    }

    /// A parked-looking row is history, not a question (D41): the digit that would have answered
    /// it live answers nothing here, and the row keeps saying nobody answered.
    #[test]
    fn an_unanswered_permission_replays_parked_and_stays_unanswered() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        replay(
            &mut tab,
            &shell,
            Some(vec![
                row(0, EventKind::Prompt, json!({ "text": "remove the build" })),
                row(
                    1,
                    EventKind::PermissionRequest,
                    json!({ "request_id": "req-1", "tool_call_id": "call-1",
                            "options": [{ "id": "allow", "label": "Allow once",
                                          "kind": "allow_once" }] }),
                ),
            ]),
        );

        let open = tab.replay().expect("the replay is open");
        assert!(
            open.transcript.parked().is_some(),
            "the log says the agent asked and nobody answered"
        );

        tab.on_key(key(KeyCode::Char('1')), &mut shell.ctx());
        assert!(shell.emit.is_empty(), "a finished step is not answerable");
        assert!(
            tab.replay()
                .expect("still open")
                .transcript
                .parked()
                .is_some()
        );
    }

    /// The live session is a state the replay reads over, never one it writes to.
    #[test]
    fn esc_leaves_replay_and_the_live_chat_is_exactly_as_it_was() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        let before = tab.transcript.rows().to_vec();
        let session = tab.session().expect("a live session").step_id;

        replay(&mut tab, &shell, two_rows());
        assert_eq!(
            tab.session().map(|state| state.step_id),
            Some(session),
            "entering replay does not touch the session underneath"
        );
        assert_eq!(tab.transcript.rows(), before.as_slice());

        assert_eq!(
            tab.on_key(key(KeyCode::Esc), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(tab.replay().is_none(), "`Esc` leaves replay");
        assert!(
            shell.emit.is_empty(),
            "and does not cancel the chat on the way out"
        );
        assert_eq!(
            tab.transcript.rows(),
            before.as_slice(),
            "the live transcript is back, unchanged"
        );
    }

    /// An `Esc` pressed to leave a replay must not be read as the second half of `Esc Esc`.
    ///
    /// The arming is a two-key sequence over the *live* view, and a replay opening between the two
    /// keys breaks the sequence: the `Esc` that closes the replay is navigation, and treating it as
    /// a confirmation would end the conversation the user opened the replay over. Entering replay
    /// therefore disarms.
    #[test]
    fn entering_replay_disarms_the_cancel() {
        let shell = Shell::new();
        let mut tab = live(&shell);

        assert_eq!(
            tab.on_key(key(KeyCode::Esc), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(
            shell.emit.is_empty(),
            "the first `Esc` only arms the cancel"
        );

        replay(&mut tab, &shell, two_rows());
        tab.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(tab.replay().is_none(), "`Esc` leaves the replay");
        assert!(shell.emit.is_empty(), "and cancels nothing on the way out");

        tab.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(
            shell.emit.is_empty(),
            "the next `Esc` arms afresh: it is the first of a new pair, not the second of a pair \
             a replay interrupted",
        );

        tab.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(
            matches!(
                shell.emit.take().as_slice(),
                [Action::Store(StoreRequest::ChatCancel { .. })]
            ),
            "and `Esc Esc` over the live view still ends the chat",
        );
    }

    /// A live frame that arrives while a past step is on screen belongs to the live session and
    /// is applied to it — the replay is a second view, not a takeover.
    #[test]
    fn a_live_frame_keeps_applying_under_an_open_replay() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        replay(&mut tab, &shell, two_rows());

        tab.on_reply(
            &StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
                event: DriverEvent::AssistantChunk(TextChunk {
                    text: " and more".to_owned(),
                    message_id: Some("m1".to_owned()),
                }),
                raw: None,
                at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
            }))),
            &mut shell.ctx(),
        );

        tab.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert_eq!(
            tab.transcript.rows(),
            &[TranscriptRow::Assistant {
                text: "still talking and more".to_owned(),
                message_id: Some("m1".to_owned()),
            }]
        );
    }

    /// MOD-4 plan D165: a promotion clears the finished conversation and waits for the promoted
    /// step's acceptance; a refused promotion is the tab's to show; and a chat the tab starts
    /// itself afterwards is not the promoted step's.
    #[test]
    fn a_promotion_opens_on_its_step_and_a_later_chat_forgets_it() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        let promoted = StepId::new();
        tab.on_reply(
            &StoreReply::Orch(OrchReply::Promoted {
                step: promoted,
                run: htui_core::model::RunId::new(),
                phase: "research".to_owned(),
                agent: "scripted".to_owned(),
                model: None,
                via: Via::Resumed,
            }),
            &mut shell.ctx(),
        );
        assert!(tab.session().is_none(), "the old conversation is history");
        assert!(tab.transcript.is_empty());
        assert!(
            tab.pending_start,
            "the promoted step's acceptance is awaited"
        );
        assert_eq!(tab.promoted().map(|header| header.step), Some(promoted));

        tab.on_reply(
            &StoreReply::Failed {
                request: PROMOTE_STEP,
                message: "the step's log is not on this box".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(!tab.pending_start);
        assert_eq!(
            tab.refusal.as_deref(),
            Some("the step's log is not on this box")
        );

        let fresh = live(&shell);
        tab.on_reply(
            &StoreReply::ChatAccepted {
                step_id: fresh.session().expect("a session").step_id,
                session_ref: None,
                caps: fresh.session().expect("a session").caps,
            },
            &mut shell.ctx(),
        );
        assert!(
            tab.promoted().is_none(),
            "a chat on another step is the tab's own, and its header names the agent again"
        );
    }

    /// T7: the follow is asked for when the promotion answers, so no chat frame can arrive between
    /// the acceptance and the follow; the first acceptance of that step adds none, and one sent
    /// again by a refused promotion (D185) follows as before.
    #[test]
    fn a_promoted_reply_asks_for_the_follow_at_once() {
        let shell = Shell::new();
        let mut tab = ChatTab::new();
        let step = StepId::new();
        tab.on_reply(
            &StoreReply::Orch(OrchReply::Promoted {
                step,
                run: htui_core::model::RunId::new(),
                phase: "research".to_owned(),
                agent: "scripted".to_owned(),
                model: None,
                via: Via::Resumed,
            }),
            &mut shell.ctx(),
        );
        assert!(
            matches!(
                shell.emit.take().as_slice(),
                [Action::Store(StoreRequest::ChatFollow { step_id })] if *step_id == step
            ),
            "the promotion's answer asks for the follow",
        );
        let accepted = StoreReply::ChatAccepted {
            step_id: step,
            session_ref: None,
            caps: live(&shell).session().expect("a session").caps,
        };
        drop(shell.emit.take());
        tab.on_reply(&accepted, &mut shell.ctx());
        assert!(
            shell.emit.is_empty(),
            "the first acceptance adds no second follow"
        );
        tab.on_reply(&accepted, &mut shell.ctx());
        assert!(
            matches!(
                shell.emit.take().as_slice(),
                [Action::Store(StoreRequest::ChatFollow { step_id })] if *step_id == step
            ),
            "an acceptance sent again follows",
        );
    }

    /// MOD-37 M5 (ANA-27 T5): a promotion that asked to resume says `resumed` until the worker's
    /// `resume_failed` notice arrives; then the chat opened with the handoff, and the header says
    /// so for the rest of the chat. The transcript shows the sentence once. The notice on a chat
    /// no promotion opened changes no header.
    #[test]
    fn a_resume_failed_event_turns_the_header_to_handoff() {
        use htui_agent::event::OtherEvent;
        use htui_orch::promote::CONTEXT_NOT_CARRIED;

        const REASON: &str = "session/load failed: gone";
        let shell = Shell::new();
        let other = |update: &str, body: serde_json::Value| {
            StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
                event: DriverEvent::Other(OtherEvent {
                    update: update.to_owned(),
                    body,
                }),
                raw: None,
                at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
            })))
        };
        let resume_failed = other(
            htui_agent::event::RESUME_FAILED,
            json!({ "session_id": "s1", "reason": REASON, "note": CONTEXT_NOT_CARRIED }),
        );
        let header = |tab: &ChatTab| -> String {
            tab.header(&shell.ctx(), 200)
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        };

        let mut tab = ChatTab::new();
        let step = StepId::new();
        tab.on_reply(
            &StoreReply::Orch(OrchReply::Promoted {
                step,
                run: htui_core::model::RunId::new(),
                phase: "research".to_owned(),
                agent: "scripted".to_owned(),
                model: None,
                via: Via::Resumed,
            }),
            &mut shell.ctx(),
        );
        tab.on_reply(
            &StoreReply::ChatAccepted {
                step_id: step,
                session_ref: None,
                caps: live(&shell).session().expect("a session").caps,
            },
            &mut shell.ctx(),
        );
        assert!(header(&tab).contains(" · resumed · "), "{}", header(&tab));

        tab.on_reply(&resume_failed, &mut shell.ctx());
        assert_eq!(tab.promoted().map(|header| header.via), Some(Via::Handoff));
        let text = header(&tab);
        assert!(
            text.contains(" · handoff · ") && !text.contains("resumed"),
            "{text}"
        );
        assert_eq!(
            tab.transcript.rows(),
            &[TranscriptRow::ResumeFailed {
                reason: REASON.to_owned(),
                note: CONTEXT_NOT_CARRIED.to_owned(),
            }],
            "the sentence, once"
        );

        tab.on_reply(
            &other("follow_up", json!({ "text": "the handoff prompt" })),
            &mut shell.ctx(),
        );
        assert_eq!(
            tab.promoted().map(|header| header.via),
            Some(Via::Handoff),
            "the handoff's opening leaves the header where the notice put it"
        );

        let mut fresh = live(&shell);
        let before = header(&fresh);
        fresh.on_reply(&resume_failed, &mut shell.ctx());
        assert!(
            fresh.promoted().is_none(),
            "no promotion, no promoted header"
        );
        assert_eq!(header(&fresh), before);
    }

    /// Verifier finding (D165): the session ref is what a later `session/load` reads back, so a
    /// header too wide for its line drops the `session ` label, then cuts the fields before the
    /// ref — never the ref itself.
    #[test]
    fn a_narrow_promoted_header_keeps_the_whole_session_ref() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        tab.promoted = Some(PromotedHeader {
            step: StepId::new(),
            phase: "implementation".to_owned(),
            agent: "scripted".to_owned(),
            model: Some("sonnet".to_owned()),
            via: Via::Handoff,
        });
        let text = |width: u16| -> String {
            tab.header(&shell.ctx(), width)
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        };

        assert_eq!(
            text(200),
            "promoted · implementation · handoff · scripted · sonnet · session live-1"
        );
        assert_eq!(
            text(66),
            "promoted · implementation · handoff · scripted · sonnet · live-1",
            "the label goes first"
        );
        assert_eq!(text(40), "promoted · implementation · ha\u{2026} · live-1");
        assert_eq!(cell_width(&text(40)), 40, "and the line fits");
    }

    /// The promoted header's text at `width`, for a step with these fields.
    fn promoted_text(phase: &str, agent: &str, width: u16) -> String {
        let shell = Shell::new();
        let mut tab = live(&shell);
        tab.promoted = Some(PromotedHeader {
            step: StepId::new(),
            phase: phase.to_owned(),
            agent: agent.to_owned(),
            model: Some("sonnet".to_owned()),
            via: Via::Handoff,
        });
        tab.header(&shell.ctx(), width)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// MOD-60 D9: a halfwidth sound mark is a cell of its own, as `ratatui` draws it. `Span::width`
    /// calls `ｶﾞ` one cell, so the header measured that way ran ten cells past its width.
    #[test]
    fn a_halfwidth_phase_keeps_the_promoted_header_within_its_width() {
        let phase = "\u{ff76}\u{ff9e}".repeat(10);
        assert_eq!(cell_width(&phase), 20);
        let text = promoted_text(&phase, "scripted", 40);
        assert!(cell_width(&text) <= 40, "{text:?} against 40");
        assert!(
            text.ends_with(" · live-1"),
            "{text:?} keeps the session ref"
        );
    }

    /// MOD-60 D9: a phase cut through a wide cluster comes back a cell short of its room, and that
    /// lone leftover cell is not handed to the fields after it — one ellipsis marks the cut, not
    /// two in a row.
    #[test]
    fn a_wide_phase_cut_short_marks_the_cut_once() {
        let phase = "\u{6f22}".repeat(8);
        for width in 20..=40 {
            let text = promoted_text(&phase, "scripted", width);
            assert!(
                !text.contains("\u{2026}\u{2026}"),
                "{text:?} at {width} doubles the ellipsis"
            );
            assert!(
                cell_width(&text) <= usize::from(width),
                "{text:?} against {width}"
            );
            assert!(
                text.ends_with(" · live-1"),
                "{text:?} keeps the session ref"
            );
        }
    }

    /// MOD-60 D9: the header is cut at a grapheme boundary, so a ZWJ family is drawn whole or not
    /// at all — never a lone joiner left dangling at the cut.
    #[test]
    fn a_family_emoji_is_never_split_in_the_header() {
        const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let agent = FAMILY.repeat(10);
        for width in 20..=80 {
            let text = promoted_text("p", &agent, width);
            assert!(
                !text.replace(FAMILY, "").contains('\u{200d}'),
                "{text:?} splits a family at {width}"
            );
            assert!(
                cell_width(&text) <= usize::from(width),
                "{text:?} against {width}"
            );
        }
    }

    /// D38's two answers are two different facts and must not read as one.
    #[test]
    fn a_step_not_on_this_box_does_not_read_as_a_step_that_said_nothing() {
        let shell = Shell::new();
        let mut tab = live(&shell);

        replay(&mut tab, &shell, None);
        assert_eq!(
            replay_body(tab.replay().expect("open")),
            Some("this step is not on this box")
        );

        replay(&mut tab, &shell, Some(Vec::new()));
        assert_eq!(
            replay_body(tab.replay().expect("open")),
            Some("this step recorded nothing")
        );

        replay(&mut tab, &shell, two_rows());
        assert_eq!(
            replay_body(tab.replay().expect("open")),
            None,
            "a step with rows shows the rows"
        );
    }

    /// MOD-64 F7 (D256): with the composer open, a chord is the shell's (`Ctrl+F` opens the
    /// concepts search, `Ctrl+C` quits), never composer text.
    #[test]
    fn a_chord_passes_to_the_shell_with_the_composer_open() {
        let shell = Shell::new();
        let mut tab = ChatTab::new();
        assert_eq!(
            tab.on_key(key(KeyCode::Char('i')), &mut shell.ctx()),
            Handled::Consumed
        );
        tab.on_key(key(KeyCode::Char('h')), &mut shell.ctx());
        assert!(tab.composer.is_active(), "`i` opens the composer");

        for chord in [
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
        ] {
            assert_eq!(
                tab.on_key(chord, &mut shell.ctx()),
                Handled::Pass,
                "{chord:?} reaches the shell"
            );
        }
        assert_eq!(tab.composer.text(), "h", "no chord typed into the composer");
        assert!(tab.composer.is_active(), "and the composer stays open");
        assert!(shell.emit.is_empty(), "the tab sent nothing");
    }
}

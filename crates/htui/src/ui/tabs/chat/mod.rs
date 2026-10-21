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
//!
//! Since MOD-115 M3 (D5) the tab draws one kit [`Pane`], titled by the session (or the replayed
//! step): the transcript's scrollbar runs on its right border beside the transcript's rows only,
//! the composer is its last inner row and the mode's hints sit on its bottom border.

pub mod composer;
pub mod permission;
pub mod transcript;

use htui_agent::driver::{AgentSessionRef, DriverCaps, PermissionAnswer};
use htui_agent::event::DriverEvent;
use htui_core::model::{AgentSummary, Scope, StepId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Context, Hint, HintSpec, KeyChord, Keys, Stack, is_plain, views};
use crate::run_worker::{OrchReply, Via};
use crate::store_worker::{ChatFrame, StoreReply, StoreRequest};
use crate::ui::cells::{self, cell_width};
use crate::ui::kit::chips::key_line;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::label::plural;
use crate::ui::kit::pane::Pane;
use crate::ui::kit::text::{Tone, notice};
use crate::ui::shell::PaneId;
use crate::ui::tabs::registry::{Tab, TabId};
use composer::{Composer, ComposerOutcome};
use crossterm::event::KeyEvent;
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

/// The last line while a replay is open (`CHAT_REPLAY`): the three keys that do anything, and the
/// way out.
const REPLAY_HINT: HintSpec = &[
    Hint::One(Act::Back, "leave replay"),
    Hint::One(Act::ChatThoughts, "thoughts"),
    Hint::Pair(Act::ListDown, Act::ListUp, "scroll"),
];

/// The open composer's keys (`CAPTURE`): `field.submit` and `field.cancel` through the keys in
/// force (MOD-115 M2 RV1 H2, M3 MD-A).
const COMPOSE_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "send"),
    Hint::One(Act::FieldCancel, "leave"),
];

/// A parked permission request (`CHAT_LIVE`): the digits are `answer.option_*` (MOD-115 M2 D7).
const PARKED_HINT: HintSpec = &[
    Hint::Many(permission::ANSWERS, "answer"),
    Hint::One(Act::ChatCompose, "compose"),
    Hint::One(Act::ChatThoughts, "thoughts"),
];

/// A chat that has ended (`CHAT_LIVE`).
const ENDED_HINT: HintSpec = &[
    Hint::Text("this chat has ended"),
    Hint::One(Act::ChatCompose, "compose"),
];

/// A live chat (`CHAT_LIVE`), before the doubled end key.
const LIVE_HINT: HintSpec = &[Hint::One(Act::ChatCompose, "compose")];

/// A live chat (`CHAT_LIVE`), after the doubled end key.
const LIVE_HINT_TAIL: HintSpec = &[
    Hint::One(Act::ChatThoughts, "thoughts"),
    Hint::Pair(Act::ListDown, Act::ListUp, "scroll"),
];

/// No chat yet (`CHAT_LIVE`).
const IDLE_HINT: HintSpec = &[
    Hint::One(Act::ChatCompose, "compose the first prompt"),
    Hint::One(Act::ChatNextAgent, "next agent"),
];

/// The empty Chat when no agent is enabled, and its next step.
const NO_AGENT: &str = "no agent is registered on this box";
const NO_AGENT_NEXT: &str = "Settings \u{203a} Agents adds one";

/// The empty Chat with an agent to talk to.
const NO_CHAT: &str = "type a prompt to start a chat";

/// The empty Chat's next step: the compose key in force (`chat.compose` through `CHAT_LIVE`), or
/// where to bind one when it is unbound.
fn compose_next(keys: &Keys) -> String {
    keys.label(views::CHAT_LIVE, Act::ChatCompose).map_or_else(
        || "bind [chat] compose to write the first prompt".to_owned(),
        |key| format!("{key} composes the first prompt"),
    )
}

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
    /// MOD-11 D18: the session answers permission requests through `htui`'s prompt port, whatever
    /// `caps` says.
    pub prompts: bool,
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
    /// Whether the chat resumed the step's session or opened with the handoff prompt: what
    /// `Promoted` asked for, turned to `Handoff` by a `resume_failed` event (MOD-37 M5).
    pub via: Via,
}

/// A past step, reopened read-only (D40, `R-HIS-2`).
///
/// "Read-only" is a property of what the tab can **send**, not of what it draws greyed out: while
/// this is `Some` the whole key path is `ChatTab::on_key_replay`, which is handed the keys in force
/// and no [`Ctx`], and so cannot issue a request even by mistake. The live session underneath is
/// untouched and comes back exactly as it was when `Esc` closes this.
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
    /// It takes the keys in force and no [`Ctx`], which is the point: a mode that *cannot* reach
    /// the request sink is one a reviewer can check by looking at this signature, rather than by
    /// trusting that no arm below grew a `ctx.request` later. A key resolves through
    /// [`views::CHAT_REPLAY`] first: `common.back` (`Esc`) leaves, and the transcript's acts
    /// scroll and fold exactly as they do live. A key none of them took that the live tab would
    /// act on (`i`, `Enter`, a digit, `a`) is consumed and does nothing, because a finished step
    /// takes no prompt and answers no permission request (D41): the chords `CHAT_LIVE` binds to a
    /// `[chat]` or `[answer]` row, so `alt-2` still reaches the shell. Anything else is the shell's
    /// (`q`, `Tab`, `?`).
    /// Resolving first lets a key file give the replay's own acts one of those keys
    /// (`[common] back = ["i"]`, M5 R1 M1); under the defaults none of them has one.
    ///
    /// The digits are *consumed* rather than passed on for the same reason the live tab consumes
    /// them: a `2` typed at a permission row must not fall through to the global tab-select
    /// binding and move the user off the conversation they are reading. The swallowed keys are
    /// what `CHAT_LIVE` binds to a `[chat]` or `[answer]` row (MOD-115 M2 D7), so they follow a
    /// rebind.
    fn on_key_replay(&mut self, key: KeyEvent, keys: &Keys) -> Handled {
        let handled = match keys
            .actions(views::CHAT_REPLAY, KeyChord::from_event(key))
            .first()
            .copied()
        {
            Some(Act::Back) => {
                // Nothing else is touched: the session, its transcript and the composer's text
                // are all exactly as replay found them. The cancel is disarmed rather than
                // restored, and deliberately so - it was disarmed when the replay opened,
                // precisely so that *this* key cannot be the second half of an `Esc Esc` that
                // ends the live chat.
                self.replay = None;
                return Handled::Consumed;
            }
            // A global act (the tab select on a digit) is no transcript act: `Pass` here.
            Some(act) => self
                .replay
                .as_mut()
                .map_or(Handled::Pass, |replay| replay.transcript.on_act(act)),
            None => Handled::Pass,
        };
        // MOD-115 M2 D7: what the live tab would do with the chord (a `[chat]` or `[answer]` row
        // through `CHAT_LIVE`: the digits, `i`, `a`, `Enter` by default) is swallowed, derived
        // from the keys in force rather than listed.
        if handled == Handled::Pass
            && keys
                .actions(views::CHAT_LIVE, KeyChord::from_event(key))
                .iter()
                .any(|act| {
                    act.spec()
                        .is_some_and(|spec| matches!(spec.context, Context::Chat | Context::Answer))
                })
        {
            return Handled::Consumed;
        }
        handled
    }

    /// The replay header, the pane's title while a replay is open: which step is on screen, how
    /// much of it there is, and that it is over. The key that leaves it is the bottom border's
    /// (`REPLAY_HINT`, MOD-115 M3 D5), so the title names none.
    fn replay_header(replay: &ReplayState) -> String {
        let id = replay.step_id.to_string();
        let step = id.get(id.len().saturating_sub(STEP_TAIL)..).unwrap_or(&id);
        format!(
            "replay · step …{step} · {} · read-only",
            plural(replay.transcript.len(), "row", "rows")
        )
    }

    /// The current mode's stack, which [`Tab::key_stack`] reports and the hints label through.
    fn stack(&self) -> Stack<'static> {
        if self.replay.is_some() {
            views::CHAT_REPLAY
        } else if self.composer.is_active() {
            views::CAPTURE
        } else {
            views::CHAT_LIVE
        }
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

    /// The header, the pane's title (MOD-115 M3 D5): who is talking, about which project, in
    /// which session — `<agent> · <model> · <project> · session <ref>`.
    ///
    /// A promoted step's chat names the step instead (D165): `promoted · <phase> · <resumed |
    /// handoff> · <agent> · <model> · session <ref>`. Either is fitted to `width`, the title's
    /// room, so the pane never clips it ([`fit_header`]).
    fn header(&self, ctx: &Ctx<'_>, width: u16) -> String {
        let session = self
            .session
            .as_ref()
            .and_then(|state| state.session_ref.as_ref())
            .map_or(NONE.to_owned(), |reference| reference.as_str().to_owned());
        let (lead, middle) = match &self.promoted {
            Some(promoted) => {
                let via = match promoted.via {
                    Via::Resumed => "resumed",
                    Via::Handoff => "handoff",
                };
                let model = promoted.model.as_deref().unwrap_or("default");
                (
                    format!("promoted · {}", promoted.phase),
                    format!(" · {via} · {} · {model}", promoted.agent),
                )
            }
            None => {
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
                (agent, format!(" · {model} · {project}"))
            }
        };
        fit_header(&lead, &middle, &session, usize::from(width))
    }

    /// The line naming what this transport cannot do, or `None` when it can do everything.
    ///
    /// `DriverCaps` is authoritative and `agent.transport` is not (§4.3): the banner is computed
    /// from what the driver answered, so a degraded CLI session says so without the tab knowing
    /// what a CLI session is.
    ///
    /// MOD-11 D18: a session with `htui`'s prompt port announces and answers permission requests
    /// although its caps stay the row's (all-false for a CLI row, which the interlock reads), so it
    /// is not bannered as unable to.
    fn caps_banner(&self) -> Option<String> {
        let state = self.session.as_ref()?;
        let caps = state.caps;
        let mut missing: Vec<&str> = Vec::new();
        if !caps.permission_requests && !state.prompts {
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

    /// The Chat's keys for the shell's hint row (MOD-67 M6 T2), each labelled through the current
    /// mode's stack (MOD-67 M5 D9), so a rebound key shows its new chord.
    ///
    /// A refusal takes this line while there is a live conversation to keep on screen, and is
    /// cleared by the next key or the next frame.
    fn hint_text(&self, keys: &Keys) -> String {
        let stack = self.stack();
        // A replay offers three keys and no refusal can apply to it: nothing it can do fails.
        if self.replay.is_some() {
            return keys.hint(stack, REPLAY_HINT);
        }
        if let Some(refusal) = &self.refusal
            && self.session.is_some()
        {
            return refusal.clone();
        }
        if self.composer.is_active() {
            return keys.hint(stack, COMPOSE_HINT);
        }
        if self.cancel_armed {
            // MOD-67 M6 D4: the end key in force, as the live hint doubles it. Only `ChatEnd`
            // arms, so it is bound whenever this shows; unbound, the line is empty.
            return keys
                .label(stack, Act::ChatEnd)
                .map(|end| format!("{end} again ends this chat"))
                .unwrap_or_default();
        }
        if self.transcript.parked().is_some() {
            return keys.hint(stack, PARKED_HINT);
        }
        match &self.session {
            Some(state) if state.ended.is_some() => keys.hint(stack, ENDED_HINT),
            // The end key is pressed twice, which no `Hint` element spells: its label is doubled
            // by hand, and an unbound end drops out like any element.
            Some(_) => [
                keys.hint(stack, LIVE_HINT),
                keys.label(stack, Act::ChatEnd)
                    .map(|label| format!("{label} {label} end"))
                    .unwrap_or_default(),
                keys.hint(stack, LIVE_HINT_TAIL),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
            None => keys.hint(stack, IDLE_HINT),
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

    /// The stack of the Chat's current mode (MOD-67 M5 D4, PA-4): the replay's while one is open,
    /// `CAPTURE` while the composer is, else the live view's (idle, live, parked or ended: one key
    /// set, the agent cycle declined once a session exists, the answers ahead of it).
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    /// The live view's stack, whatever mode the chat is in (MOD-115 review): what the about panel
    /// labels the tab's actions through, so an open composer does not blank them.
    fn browse_stack(&self) -> Stack<'static> {
        views::CHAT_LIVE
    }

    /// The current mode's keys (MOD-67 M6 T2): `hint_text`, in `theme.dim`. The pane draws them
    /// on its bottom border itself (MOD-115 M3 D5); the shell reads them for the activity
    /// placeholder only (C2).
    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        let hint = self.hint_text(ctx.keys());
        key_line(hint, ctx.theme)
    }

    /// MOD-115 M3 D5: the Chat draws its own kit pane, so the shell paints no focus or hints
    /// over it.
    fn draws_chrome(&self) -> bool {
        true
    }

    /// Each mode resolves through its stack (MOD-67 M5 D4, D5): the open composer reads its
    /// canonical events (the shell's `[field]` translation, MOD-115 M2 D7), a parked request
    /// answers on `answer.option_*` through `CHAT_LIVE`, and the replay's swallow takes what the
    /// replay's own acts left (M5 R1 M1) of the live tab's `[chat]` and `[answer]` chords.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A refusal is transient: it says what the last key could not do, and the next one clears
        // it whatever it was.
        self.refusal = None;
        // The delivered (canonical) chord is the composer's, so a `[field]` chord the shell
        // translated (`field.cancel = "ctrl-o"` into `Esc`) is never passed as a CONTROL chord;
        // the live view's own acts resolve from the chord pressed (MOD-115 M3 MD-A).
        let chord = KeyChord::from_event(key);
        let pressed = ctx.pressed(&key);
        // Before the composer, before the digits, before anything that could send: a replay is a
        // mode with no command path at all (D40), and `on_key_replay` is handed no `ctx` to prove
        // it. An open composer underneath keeps its text — it simply cannot be typed into until
        // `Esc` puts the live view back.
        if self.replay.is_some() {
            return self.on_key_replay(key, ctx.keys());
        }
        // The composer owns every key while it is open, so the tab's own single letters stay
        // single letters everywhere else; but it is offered only what `CAPTURE` does not pass. A
        // chord or a function key is the shell's (`Ctrl+F`, `Ctrl+C`, `F1`), never composer text:
        // every other text widget passes it the same way (`TextField::on_key`, MOD-64 F7).
        if self.composer.is_active() {
            if views::CAPTURE.passes(chord) {
                return Handled::Pass;
            }
            if let ComposerOutcome::Submit(text) = self.composer.on_key(key) {
                self.submit(text, ctx);
            }
            // Every other outcome is consumed too: an open composer answers `Pass` for nothing,
            // and were it to, the key must still not reach the live view below (M5 R1 L3).
            return Handled::Consumed;
        }

        // A digit answers the parked request, and **every** digit is consumed while one is
        // waiting: a `4` with three options on screen must not fall through to the global
        // tab-select binding and move the user off the conversation the agent is blocked on.
        // MOD-115 M2 D7: the answers are `answer.option_*` rows (the digits by default).
        if let Some(index) = ctx
            .keys()
            .actions(views::CHAT_LIVE, pressed)
            .first()
            .and_then(|act| act.answer_index())
            && let Some(digit) = u32::try_from(index + 1)
                .ok()
                .and_then(|n| char::from_digit(n, 10))
            && self.transcript.parked().is_some()
        {
            self.answer(digit, ctx);
            self.cancel_armed = false;
            return Handled::Consumed;
        }

        let first = ctx
            .keys()
            .actions(views::CHAT_LIVE, pressed)
            .first()
            .copied();
        let handled = match first {
            Some(Act::ChatCompose) => {
                self.composer.enter();
                Handled::Consumed
            }
            // Declined with a session or no agent: `chat.next_agent` is no transcript act, so the
            // arm below passes it.
            Some(Act::ChatNextAgent) if self.session.is_none() && !self.agents.is_empty() => {
                self.agent_index = (self.agent_index + 1) % self.agents.len();
                Handled::Consumed
            }
            Some(Act::ChatEnd) => {
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
            Some(act) => self.transcript.on_act(act),
            None => Handled::Pass,
        };
        // Any key but the end disarms it; a chord never did (the old guard returned before
        // this), so a rebound `[chat] end = "ctrl-e"` still arms on `ctrl-e`.
        if first != Some(Act::ChatEnd) && is_plain(&key) {
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
                prompts,
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
                    prompts: *prompts,
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
                // MOD-37 M5 (ANA-27 T5): the resume failed and the chat fell back to the handoff
                // prompt, so the header stops saying `resumed`. `via` was set by `Promoted` before
                // the bind; no other reply carries the outcome.
                if let DriverEvent::Other(other) = &envelope.event
                    && other.update == htui_agent::event::RESUME_FAILED
                    && let Some(promoted) = self.promoted.as_mut()
                {
                    promoted.via = Via::Handoff;
                }
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

    /// MOD-115 M3 D5: one kit pane titled by the session (or the replayed step), its one pane
    /// being `PaneId::Detail` (C1). The content is drawn into the pane's inner rect first, so the
    /// transcript's scrollbar knows its numbers when the frame is drawn (C3): the banner, the
    /// body, the permission strip and the composer, top to bottom, the bar beside the body only.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let inner = Pane::inner(area);
        let replay = self.replay.as_ref();
        // A replay draws neither: the capability banner is a property of the *live* transport,
        // and a strip offering options over a finished step would be a question nobody can
        // answer (D41). The rows themselves still render the request, as history.
        let (banner, parked) = match replay {
            Some(_) => (None, None),
            None => (self.caps_banner(), self.transcript.parked()),
        };
        let [banner_area, body, strip, composer] = Layout::vertical([
            Constraint::Length(u16::from(banner.is_some())),
            Constraint::Min(1),
            Constraint::Length(u16::from(parked.is_some())),
            Constraint::Length(1),
        ])
        .areas(inner);

        if let Some(text) = banner {
            frame.render_widget(
                Paragraph::new(notice(&text, Tone::Error, ctx.theme)),
                banner_area,
            );
        }

        // The transcript's numbers for the scrollbar, when a transcript is what the body shows.
        let mut scroll = None;
        if let Some(replay) = replay {
            // The same renderer the live view uses, over rows instead of frames (`R-TUI-6`).
            match replay_body(replay) {
                Some(text) => {
                    frame.render_widget(Paragraph::new(notice(text, Tone::Info, ctx.theme)), body);
                }
                None => {
                    let unfold = ctx.keys().label(views::CHAT_REPLAY, Act::ChatThoughts);
                    let lines =
                        replay
                            .transcript
                            .lines(body.height as usize, ctx.theme, unfold.as_deref());
                    frame.render_widget(Paragraph::new(lines), body);
                    scroll = Some(replay.transcript.scroll_position());
                }
            }
        }
        // A refusal never replaces a live conversation: it is one line under it, because losing
        // the transcript to "answer the permission request first" would cost far more than the
        // message is worth. Only a chat that never started shows it in the body.
        else if self.session.is_none()
            && let Some(refusal) = &self.refusal
        {
            // MOD-37 review L-6: a resume whose handoff failed too has a `resume_failed` line, and
            // it goes above the refusal: it is why the header says `handoff`.
            let unfold = ctx.keys().label(views::CHAT_LIVE, Act::ChatThoughts);
            let mut lines = self
                .transcript
                .resume_failed_lines(ctx.theme, unfold.as_deref());
            lines.push(Line::styled(refusal.clone(), ctx.theme.error));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), body);
        } else if self.session.is_none() && !self.pending_start {
            // MOD-115 M3: an empty state says what is missing and the next step (audit item 2).
            let (what, next) = if self.agents.is_empty() {
                (NO_AGENT, NO_AGENT_NEXT.to_owned())
            } else {
                (NO_CHAT, compose_next(ctx.keys()))
            };
            empty::render(frame, body, what, NextStep(&next), ctx.theme);
        } else {
            let unfold = ctx.keys().label(views::CHAT_LIVE, Act::ChatThoughts);
            let lines = self
                .transcript
                .lines(body.height as usize, ctx.theme, unfold.as_deref());
            frame.render_widget(Paragraph::new(lines), body);
            scroll = Some(self.transcript.scroll_position());
        }

        if let Some(row) = parked {
            PermissionStrip::render(frame, strip, row, ctx.keys(), views::CHAT_LIVE, ctx.theme);
        }
        // Not under a replay: the composer belongs to the live view, and a replay must not show
        // an input line for a step that can take no input.
        if replay.is_none() {
            Composer::render(frame, composer, &self.composer, ctx.accent(), ctx.theme);
        }

        // The title's room: the pane draws ` {title} ` two cells past the corner and stops one
        // rule cell short of the other, so a header fitted to six cells under the width is never
        // clipped by it — the promoted header's session ref stays whole (D165).
        let title = match replay {
            Some(replay) => Self::replay_header(replay),
            None => self.header(ctx, area.width.saturating_sub(6)),
        };
        let mut pane = Pane::new(&title)
            .focus(ctx.pane_focus(PaneId::Detail))
            .accent(ctx.accent());
        if let Some(hints) = key_line(self.hint_text(ctx.keys()), ctx.theme) {
            pane = pane.hints(hints);
        }
        if let Some((total, first)) = scroll {
            pane = pane.scrollbar(total, first).scrollbar_span(body);
        }
        pane.render(frame, area, ctx.theme);
    }
}

/// `lead`, `middle` and the session ref as one header line of at most `width` cells (D165, MOD-60
/// D9). The session ref is the one field here that exists to be read back (a later
/// `session/load`), so it is never cut: a line too wide drops the `session ` label first, then
/// cuts the fields before the ref with an ellipsis.
fn fit_header(lead: &str, middle: &str, session: &str, width: usize) -> String {
    let mut tail = format!(" · session {session}");
    if cell_width(lead) + cell_width(middle) + cell_width(&tail) > width {
        tail = format!(" · {session}");
    }
    let room = width.saturating_sub(cell_width(&tail));
    // A lead cut through a wide cluster comes back a cell short; that leftover cell would only
    // hold a second ellipsis, so a cut lead takes the middle's room with it.
    let middle = if cell_width(lead) > room {
        String::new()
    } else {
        cells::clip(middle, room - cell_width(lead))
    };
    let lead = cells::clip(lead, room);
    format!("{lead}{middle}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keys::Keys;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::shell::PaneId;
    use chrono::{TimeZone as _, Utc};
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_agent::event::{DriverEnvelope, DriverEvent, TextChunk};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        EventKind, EventRole, ProjectRef, Scope, SessionEvent, StepId, WorkspaceId,
    };
    use ratatui::style::{Color, Modifier};
    use serde_json::json;

    /// Everything a [`Ctx`] borrows, kept alive for the length of a test — and the [`Emit`] that
    /// makes "the tab sent nothing" an assertion rather than a claim.
    struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keys: Keys,
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
                keys: Keys::compiled().clone(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keys,
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

    /// MOD-11 D18 (adversarial review T9-ADV-3): a CLI session keeps the row's all-false caps, but
    /// with `htui`'s prompt port it does announce and answer permission requests — so the banner
    /// names only what it still cannot do.
    #[test]
    fn a_session_with_a_prompt_port_is_not_bannered_for_permission_requests() {
        let shell = Shell::new();
        for (prompts, banner) in [
            (
                false,
                "this agent cannot: permission requests, edit proposals, plans",
            ),
            (true, "this agent cannot: edit proposals, plans"),
        ] {
            let mut tab = ChatTab::new();
            tab.on_reply(
                &StoreReply::ChatAccepted {
                    step_id: StepId::new(),
                    session_ref: None,
                    caps: DriverCaps::default(),
                    prompts,
                },
                &mut shell.ctx(),
            );
            assert_eq!(
                tab.caps_banner().as_deref(),
                Some(banner),
                "prompts: {prompts}"
            );
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
                prompts: false,
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

    /// MOD-115 M2 D7: the replay swallows what the live tab binds in force (its `[chat]` and
    /// `[answer]` chords), not a fixed list: a rebound answer or compose key is swallowed, and the
    /// old letter or digit is the shell's again.
    #[test]
    fn the_replay_swallows_what_the_live_tab_binds_in_force() {
        let mut shell = Shell::new();
        shell.keys = crate::keys::load_str(
            "version = 2\n[answer]\noption_1 = \"f8\"\n[chat]\ncompose = \"I\"\n",
        )
        .expect("the keys load");
        let mut tab = live(&shell);
        replay(&mut tab, &shell, two_rows());
        for (pressed, handled) in [
            (key(KeyCode::F(8)), Handled::Consumed),
            (key(KeyCode::Char('I')), Handled::Consumed),
            (key(KeyCode::Char('2')), Handled::Consumed),
            (key(KeyCode::Char('i')), Handled::Pass),
            (key(KeyCode::Char('1')), Handled::Pass),
        ] {
            assert_eq!(
                tab.on_key(pressed, &mut shell.ctx()),
                handled,
                "{pressed:?}"
            );
        }
        assert!(shell.emit.is_empty(), "a replay sends nothing");
        assert!(tab.replay().is_some(), "and is still open");
    }

    /// MOD-115 M2 RV1 H2, M3 MD-A: the open composer's hint names the submit and the cancel keys
    /// in force through `CAPTURE` (`Enter` and `Esc` under the compiled keys).
    #[test]
    fn the_compose_hint_names_the_submit_key_in_force() {
        let mut shell = Shell::new();
        let mut tab = live(&shell);
        tab.on_key(KeyEvent::from(KeyCode::Char('i')), &mut shell.ctx());
        assert!(tab.composer.is_active(), "`i` opens the composer");
        assert_eq!(tab.stack(), views::CAPTURE);
        assert_eq!(
            tab.hint_text(Keys::compiled()),
            "Enter send \u{b7} Esc leave"
        );
        shell.keys = crate::keys::load_str("version = 2\n[field]\nsubmit = \"ctrl-t\"\n")
            .expect("the keys load");
        assert_eq!(tab.hint_text(&shell.keys), "Ctrl+t send \u{b7} Esc leave");
        shell.keys = crate::keys::load_str("version = 2\n[field]\ncancel = \"ctrl-o\"\n")
            .expect("the keys load");
        assert_eq!(
            tab.hint_text(&shell.keys),
            "Enter send \u{b7} Ctrl+o leave",
            "the leave key is `field.cancel`'s, not a literal `Esc` (MD-A)"
        );
    }

    /// `tab` drawn into a `width` by `height` bench under `theme`, the shell's focus on the
    /// Chat's one pane (`PaneId::Detail`, C1) when `focused`; a bench without it focuses the list
    /// (M2 B7), so the pane reads unfocused.
    fn draw_with(
        tab: &ChatTab,
        shell: &Shell,
        (width, height): (u16, u16),
        focused: bool,
        theme: &Theme,
    ) -> ratatui::buffer::Buffer {
        let ctx = Ctx::new(
            &shell.scope,
            &shell.projects,
            &shell.top_bar,
            &shell.keys,
            theme,
            Origin::Tab(ChatTab::ID),
            &shell.emit,
        );
        let ctx = if focused {
            ctx.with_focus(PaneId::Detail)
        } else {
            ctx
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), &ctx))
            .expect("the frame draws");
        terminal.backend().buffer().clone()
    }

    /// [`draw_with`] under the default theme.
    fn draw(
        tab: &ChatTab,
        shell: &Shell,
        size: (u16, u16),
        focused: bool,
    ) -> ratatui::buffer::Buffer {
        draw_with(tab, shell, size, focused, &shell.theme)
    }

    fn rows_of(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// MOD-115 M3 D5: the Chat draws its own kit pane, so the shell paints no focus or hints
    /// over it.
    #[test]
    fn chat_draws_its_own_chrome() {
        assert!(ChatTab::new().draws_chrome());
    }

    /// MOD-115 M3 D5: one rounded pane titled by the session; the `┌ Chat ─` box and the header
    /// row inside it are gone, so the transcript starts on the first inner row, and the focused
    /// pane carries the mode's hints on its bottom border.
    #[test]
    fn the_frame_is_one_rounded_pane_titled_by_the_session() {
        let shell = Shell::new();
        let tab = live(&shell);
        let rows = rows_of(&draw(&tab, &shell, (60, 8), true));
        assert!(
            rows[0].starts_with("\u{256d}\u{2500} \u{2014} \u{b7} default \u{b7} \u{2014} \u{b7} session live-1 \u{2500}"),
            "{rows:#?}"
        );
        assert!(rows[0].ends_with('\u{256e}'), "{rows:#?}");
        assert!(rows[1].contains("still talking"), "{rows:#?}");
        assert!(
            rows[7].starts_with("\u{2570}\u{2500}i compose \u{b7} Esc Esc end"),
            "{rows:#?}"
        );
        assert!(rows[7].ends_with('\u{256f}'), "{rows:#?}");
        assert!(
            rows.iter()
                .all(|row| !row.contains('\u{250c}') && !row.contains(" Chat ")),
            "no square box and no tab-name title: {rows:#?}"
        );
    }

    /// C1: on a bench nothing focuses the Chat's pane, so it carries no hints and a dim title.
    #[test]
    fn an_unfocused_chat_draws_no_hints() {
        let shell = Shell::new();
        let tab = live(&shell);
        let buffer = draw(&tab, &shell, (60, 8), false);
        let rows = rows_of(&buffer);
        assert!(!rows[7].contains("compose"), "{rows:#?}");
        assert_eq!(buffer[(3, 0)].style().fg, shell.theme.dim.fg, "a dim title");
    }

    /// MOD-115 M3 D5, MOD-85: the session header is the pane's title, in the tab accent and bold
    /// while focused; without colour it is still bold.
    #[test]
    fn the_title_takes_the_tab_accent_bold_when_focused() {
        let shell = Shell::new();
        let tab = live(&shell);
        let buffer = draw(&tab, &shell, (60, 8), true);
        let title = buffer[(3, 0)].style();
        assert_eq!(buffer[(3, 0)].symbol(), "\u{2014}");
        assert_eq!(title.fg, shell.ctx().accent().fg, "the Chat's accent");
        assert!(title.add_modifier.contains(Modifier::BOLD));

        let mono = Theme::monochrome();
        let buffer = draw_with(&tab, &shell, (60, 8), true, &mono);
        let title = buffer[(3, 0)].style();
        assert_eq!(
            (title.fg, title.bg),
            (Some(Color::Reset), Some(Color::Reset)),
            "no colour"
        );
        assert!(title.add_modifier.contains(Modifier::BOLD));
    }

    /// MOD-115 M3 D-17, MOD-85: the open composer's `> ` takes the Chat's accent (the one focus
    /// use left in the tab), not the theme's modal accent.
    #[test]
    fn the_composer_takes_the_tab_accent() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        tab.on_key(key(KeyCode::Char('i')), &mut shell.ctx());
        let buffer = draw(&tab, &shell, (60, 8), true);
        // The composer is the pane's last inner row.
        assert_eq!(buffer[(1, 6)].symbol(), ">");
        assert_eq!(
            Some(buffer[(1, 6)].fg),
            shell.ctx().accent().fg,
            "the tab accent"
        );
    }

    /// MOD-115 M3: an empty Chat says what is missing and what to do next (audit item 2).
    #[test]
    fn the_empty_chat_names_a_next_step() {
        let shell = Shell::new();
        let tab = ChatTab::new();
        let rows = rows_of(&draw(&tab, &shell, (60, 10), true));
        let screen = rows.join("\n");
        assert!(
            screen.contains("no agent is registered on this box"),
            "{screen}"
        );
        assert!(
            screen.contains("Settings \u{203a} Agents adds one"),
            "{screen}"
        );
    }

    /// MOD-115 M3 D-11: the transcript's scrollbar sits on the right border beside the
    /// transcript's rows only, never beside the banner, the permission strip or the composer.
    #[test]
    fn the_scrollbar_runs_beside_the_transcript_only() {
        use htui_agent::driver::PermissionRequestId;
        use htui_agent::event::{PermissionOption, PermissionOptionKind, PermissionRequestEvent};
        let shell = Shell::new();
        let mut tab = ChatTab::new();
        // All-false caps: a banner row above the transcript.
        tab.on_reply(
            &StoreReply::ChatAccepted {
                step_id: StepId::new(),
                session_ref: None,
                caps: DriverCaps::default(),
                prompts: false,
            },
            &mut shell.ctx(),
        );
        let event = |event: DriverEvent| {
            StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
                event,
                raw: None,
                at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
            })))
        };
        let text = (0..30)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        tab.on_reply(
            &event(DriverEvent::AssistantChunk(TextChunk {
                text,
                message_id: Some("m1".to_owned()),
            })),
            &mut shell.ctx(),
        );
        // A parked request: a strip row under the transcript.
        tab.on_reply(
            &event(DriverEvent::PermissionRequest(PermissionRequestEvent {
                request_id: PermissionRequestId::new("req-1"),
                tool_call_id: None,
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            })),
            &mut shell.ctx(),
        );
        // 12 rows: the top border, the banner (1), the transcript (2..=8), the strip (9), the
        // composer (10) and the bottom border.
        let buffer = draw(&tab, &shell, (40, 12), true);
        let thumb: Vec<u16> = (0..12)
            .filter(|&y| buffer[(39, y)].symbol() == "\u{2503}")
            .collect();
        assert!(!thumb.is_empty(), "{:#?}", rows_of(&buffer));
        assert!(
            thumb.iter().all(|y| (2..=8).contains(y)),
            "the thumb stays beside the transcript: {thumb:?} {:#?}",
            rows_of(&buffer)
        );
        assert_eq!(thumb.last(), Some(&8), "the tail is at the bottom of it");

        // Scrolled to the top, the thumb is at the top of the transcript's rows.
        let mut tab = tab;
        tab.transcript.on_act(Act::ChatTop);
        let buffer = draw(&tab, &shell, (40, 12), true);
        assert_eq!(buffer[(39, 2)].symbol(), "\u{2503}");
    }

    /// MOD-115 M3 D5: a replay is titled by its step, its rows counted in words; the key that
    /// leaves it is on the bottom border (`REPLAY_HINT`), no longer in the title.
    #[test]
    fn a_replay_is_titled_by_its_step_and_its_hint_names_the_way_out() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        replay(&mut tab, &shell, two_rows());
        let rows = rows_of(&draw(&tab, &shell, (80, 8), true));
        let id = ids::STEP_PLAN.to_string();
        let tail = &id[id.len() - STEP_TAIL..];
        assert!(
            rows[0].starts_with(&format!(
                "\u{256d}\u{2500} replay \u{b7} step \u{2026}{tail} \u{b7} 2 rows \u{b7} read-only \u{2500}"
            )),
            "{rows:#?}"
        );
        assert!(
            rows[7].starts_with("\u{2570}\u{2500}Esc leave replay"),
            "{rows:#?}"
        );

        replay(
            &mut tab,
            &shell,
            Some(vec![row(0, EventKind::Prompt, json!({ "text": "one" }))]),
        );
        assert!(
            ChatTab::replay_header(tab.replay().expect("open")).contains(" \u{b7} 1 row \u{b7} "),
            "one row, not `1 rows`"
        );
    }

    /// The plain header is fitted to the title's room the way the promoted one is: the
    /// `session ` label goes first, then the fields before the ref are cut, never the ref.
    #[test]
    fn a_narrow_plain_header_keeps_the_whole_session_ref() {
        let shell = Shell::new();
        let tab = live(&shell);
        let text = |width: u16| tab.header(&shell.ctx(), width);
        assert_eq!(
            text(200),
            "\u{2014} \u{b7} default \u{b7} \u{2014} \u{b7} session live-1"
        );
        assert_eq!(
            text(30),
            "\u{2014} \u{b7} default \u{b7} \u{2014} \u{b7} live-1",
            "the label goes first"
        );
        assert_eq!(text(15), "\u{2014} \u{b7} d\u{2026} \u{b7} live-1");
        assert_eq!(cell_width(&text(15)), 15, "and the line fits");
    }

    /// MOD-115 M2 D7: a parked request answers on `answer.option_*`: a rebound option answers,
    /// and the old digit no longer does (it is the tab select's again).
    #[test]
    fn a_rebound_answer_answers_the_parked_request() {
        use htui_agent::driver::PermissionRequestId;
        use htui_agent::event::{PermissionOption, PermissionOptionKind, PermissionRequestEvent};
        let mut shell = Shell::new();
        shell.keys = crate::keys::load_str("version = 2\n[answer]\noption_1 = \"f8\"\n")
            .expect("the keys load");
        let mut tab = live(&shell);
        tab.on_reply(
            &StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
                event: DriverEvent::PermissionRequest(PermissionRequestEvent {
                    request_id: PermissionRequestId::new("req-1"),
                    tool_call_id: Some("call-1".to_owned()),
                    options: vec![PermissionOption {
                        id: "allow".to_owned(),
                        label: "Allow".to_owned(),
                        kind: PermissionOptionKind::AllowOnce,
                    }],
                }),
                raw: None,
                at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
            }))),
            &mut shell.ctx(),
        );
        assert!(tab.transcript.parked().is_some(), "the agent asks");
        // T5 repair: the hint names the keys that answer, not the digits.
        assert!(
            tab.hint_text(&shell.keys)
                .starts_with("F8/2/3/4/5/6/7/8/9 answer · "),
            "{}",
            tab.hint_text(&shell.keys)
        );
        assert!(
            tab.hint_text(Keys::compiled()).starts_with("1-9 answer · "),
            "the defaults read as before"
        );
        drop(shell.emit.take());
        assert_eq!(
            tab.on_key(key(KeyCode::Char('1')), &mut shell.ctx()),
            Handled::Pass,
            "`1` selects a tab now"
        );
        assert!(shell.emit.is_empty(), "`1` answers nothing");
        assert_eq!(
            tab.on_key(key(KeyCode::F(8)), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(
            matches!(
                shell.emit.take().as_slice(),
                [Action::Store(StoreRequest::ChatAnswer { .. })]
            ),
            "`F8` answers"
        );
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
                prompts: false,
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
            prompts: false,
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
        let header = |tab: &ChatTab| -> String { tab.header(&shell.ctx(), 200) };

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
                prompts: false,
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

    /// MOD-37 review L-6: when the resume and its handoff both fail, the body shows the
    /// `resume_failed` line above the refusal, so the header's `handoff` is explained.
    #[test]
    fn a_chat_whose_resume_and_handoff_both_failed_shows_why_above_the_refusal() {
        use htui_agent::event::OtherEvent;
        use htui_orch::promote::CONTEXT_NOT_CARRIED;

        let shell = Shell::new();
        let mut tab = ChatTab::new();
        tab.on_reply(
            &StoreReply::Orch(OrchReply::Promoted {
                step: StepId::new(),
                run: htui_core::model::RunId::new(),
                phase: "research".to_owned(),
                agent: "scripted".to_owned(),
                model: None,
                via: Via::Resumed,
            }),
            &mut shell.ctx(),
        );
        tab.on_reply(
            &StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope {
                event: DriverEvent::Other(OtherEvent {
                    update: htui_agent::event::RESUME_FAILED.to_owned(),
                    body: json!({
                        "session_id": "s1",
                        "reason": "session/load failed: gone",
                        "note": CONTEXT_NOT_CARRIED,
                    }),
                }),
                raw: None,
                at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
            }))),
            &mut shell.ctx(),
        );
        tab.on_reply(
            &StoreReply::Failed {
                request: PROMOTE_STEP,
                message: "second refusal".to_owned(),
            },
            &mut shell.ctx(),
        );
        tab.on_reply(
            &StoreReply::Chat(ChatFrame::Failed {
                message: "second refusal".to_owned(),
            }),
            &mut shell.ctx(),
        );

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 10)).expect("terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), &shell.ctx()))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        let row_of = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("`{needle}` is drawn: {rows:#?}"))
        };
        // MOD-115 M3 D5: the header is the pane's title, on the top border.
        assert!(rows[0].contains(" · handoff · "), "{rows:#?}");
        let reason = row_of("resume failed  session/load failed: gone");
        let note = row_of(CONTEXT_NOT_CARRIED);
        let refusal = row_of("second refusal");
        assert!(
            reason < note && note < refusal,
            "the reason, its note, then the refusal: {rows:#?}"
        );
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
        let text = |width: u16| -> String { tab.header(&shell.ctx(), width) };

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

    /// MOD-67 M5 R1 L3: an open composer never lets a key on to the live dispatch: the live
    /// view's letters and a digit are composer text, a named key the composer has no use for is
    /// swallowed, and nothing folds, scrolls, arms the end or is sent.
    #[test]
    fn an_open_composer_never_lets_a_key_reach_the_live_view() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        tab.on_key(key(KeyCode::Char('i')), &mut shell.ctx());
        assert!(tab.composer.is_active(), "`i` opens the composer");
        let before = format!("{:?}", tab.transcript);
        for code in [
            KeyCode::Char('t'),
            KeyCode::Char('j'),
            KeyCode::Char('G'),
            KeyCode::Char('a'),
            KeyCode::Char('1'),
            KeyCode::Down,
            KeyCode::Home,
        ] {
            assert_eq!(
                tab.on_key(key(code), &mut shell.ctx()),
                Handled::Consumed,
                "{code:?} is the composer's"
            );
        }
        assert_eq!(tab.composer.text(), "tjGa1");
        assert_eq!(
            format!("{:?}", tab.transcript),
            before,
            "nothing folded or scrolled"
        );
        assert!(!tab.cancel_armed, "nothing armed the end");
        assert!(shell.emit.is_empty(), "the tab sent nothing");
    }

    /// MOD-86 D5: a follow-up the runtime refused for a credential shows on the hint line, and
    /// the session stays live: nothing is added to the transcript and nothing is sent again (the
    /// refused text is not kept, so the user retypes it without the secret).
    #[test]
    fn a_refused_follow_up_shows_on_the_hint_and_keeps_the_session_live() {
        let shell = Shell::new();
        let mut tab = live(&shell);
        let rows = tab.transcript.len();
        let sentence = "not sent: the follow-up matches the github_token rule";

        tab.on_reply(
            &StoreReply::Failed {
                request: "chat_send",
                message: sentence.to_owned(),
            },
            &mut shell.ctx(),
        );

        assert_eq!(tab.hint_text(Keys::compiled()), sentence);
        assert!(
            tab.session().is_some_and(|session| session.ended.is_none()),
            "the session is still live"
        );
        assert_eq!(tab.transcript.len(), rows, "nothing joins the transcript");
        assert!(shell.emit.take().is_empty(), "nothing is sent again");
    }
}

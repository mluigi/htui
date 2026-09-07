//! Live chat sessions, owned by the store worker's task (MOD-2 plan D27, D28).
//!
//! One [`AgentRuntime`] lives inside the store worker loop, because a chat needs two things only
//! that loop has: the [`Backend`] (for the writer, the box, the user and the registry row) and the
//! reply channel every view is answered through. The chat tab holds neither — it asks for a chat
//! with [`StoreRequest::ChatStart`] and is answered
//! in `on_reply`, exactly as it is for a list of items (`R-NF-3`).
//!
//! **The stream is one request, many replies.** Every frame a session produces is sent as a
//! [`ReplyEnvelope`] carrying the `ChatStart` request's own `seq` and origin, so `App::is_fresh`
//! passes all of them for as long as that chat is the tab's newest `ChatStart`
//! (`docs/ANA-4.md` §8: the stream gets its own discriminant and nothing else uses it).
//!
//! **`run_chat` is a plain `async fn`, not a spawned task.** Production spawns it; the test
//! harness awaits it inline, which is what keeps chat-tab snapshots byte-stable with no sleeps —
//! the same trade `Harness::settle` makes for store requests.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::acp::SESSION_STARTED;
use htui_agent::driver::{
    AgentDriver, AgentSession, DriverCaps, PermissionAnswer, PermissionPolicy, PermissionRequestId,
    SessionSpec,
};
use htui_agent::error::DriverError;
use htui_agent::event::{DriverEnvelope, DriverEvent, StopReason, ToolCallEvent};
use htui_agent::launch::AgentSettings;
use htui_agent::record::{AnsweredBy, Recorder};
use htui_agent::registry::DriverFactory;
use htui_core::model::{ChatRunSpec, RunStatus, StepId};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{StoreError, WriteStore};
use htui_store::{Backend, Writer};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::store_worker::{
    ChatFrame, Origin, ReplyEnvelope, RequestEnvelope, Seq, StoreReply, StoreRequest,
};

/// How long a cancelled session may take the graceful path before its tree is killed.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// Depth of the recorder's render channel.
///
/// Drained by the same task after every `record`, so a full channel is structurally impossible
/// here; the bound exists so a bug cannot buffer a turn without limit.
pub const UI_FRAMES: usize = 256;

/// `HTUI_KEEP_RAW_EVENTS=1` keeps the verbatim wire message on every row (plan D29).
///
/// `project.settings.keep_raw_events` is the real source and has no reader until MOD-15; this is
/// the stand-in, and the default is `false`.
pub const KEEP_RAW_ENV: &str = "HTUI_KEEP_RAW_EVENTS";

/// Where a reply goes: the request that asked, by `seq` and origin.
#[derive(Debug, Clone)]
pub struct ReplyAddr {
    /// The `seq` of the request being answered.
    pub seq: Seq,
    /// Who asked.
    pub origin: Origin,
}

/// What the worker asks a live chat to do. Each carries the address of the request that asked, so
/// exactly one reply goes back for it.
#[derive(Debug)]
pub enum ChatCommand {
    /// A follow-up turn.
    Send {
        /// The user's text.
        text: String,
        /// Who to answer.
        reply: ReplyAddr,
    },
    /// An answer to a parked permission request.
    Answer {
        /// Which request.
        request_id: PermissionRequestId,
        /// The chosen option, or a cancellation.
        answer: PermissionAnswer,
        /// Who to answer.
        reply: ReplyAddr,
    },
    /// End the session.
    Cancel {
        /// Who to answer; `None` when the runtime is shutting every chat down.
        reply: Option<ReplyAddr>,
    },
}

/// One live chat, as the worker sees it.
pub struct LiveChat {
    /// The command channel into [`run_chat`].
    commands: mpsc::UnboundedSender<ChatCommand>,
    /// What the driver can do, for the tab's capability banner.
    caps: DriverCaps,
    /// The spawned task, when production spawned one.
    task: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for LiveChat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveChat")
            .field("caps", &self.caps)
            .field("closed", &self.commands.is_closed())
            .field("spawned", &self.task.is_some())
            .finish()
    }
}

/// A chat's session future: production spawns it, the harness polls it inline (plan D30).
pub type ChatTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// What [`AgentRuntime::serve`] decided about one request.
pub enum Served {
    /// Answer with this reply, now.
    Reply(StoreReply),
    /// The session task will answer this request itself.
    Deferred,
    /// A chat is starting; the caller spawns (or polls) the future and attaches the handle.
    Start {
        /// The step the chat records against.
        step_id: StepId,
        /// The session future.
        task: ChatTask,
    },
}

impl core::fmt::Debug for Served {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Reply(reply) => f.debug_tuple("Reply").field(reply).finish(),
            Self::Deferred => f.write_str("Deferred"),
            Self::Start { step_id, .. } => {
                f.debug_struct("Start").field("step_id", step_id).finish()
            }
        }
    }
}

/// Every live chat this process owns.
pub struct AgentRuntime {
    factory: DriverFactory,
    live: HashMap<StepId, LiveChat>,
    started: Vec<StepId>,
    grace: Duration,
}

impl core::fmt::Debug for AgentRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentRuntime")
            .field("adapters", &self.factory.adapter_ids())
            .field("live", &self.live.len())
            .finish()
    }
}

impl AgentRuntime {
    /// A runtime over a transport registry.
    #[must_use]
    pub fn new(factory: DriverFactory) -> Self {
        Self {
            factory,
            live: HashMap::new(),
            started: Vec::new(),
            grace: CANCEL_GRACE,
        }
    }

    /// The production runtime: the ACP transport and nothing else (milestone 8 adds the CLI one).
    #[must_use]
    pub fn production() -> Self {
        Self::new(DriverFactory::with_acp())
    }

    /// A runtime whose cancels use this grace window. Tests use zero.
    #[must_use]
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Every step this runtime has started, oldest first.
    #[must_use]
    pub fn steps(&self) -> Vec<StepId> {
        self.started.clone()
    }

    /// Records the task handle of a chat the caller spawned.
    pub fn attach(&mut self, step_id: StepId, task: JoinHandle<()>) {
        if let Some(chat) = self.live.get_mut(&step_id) {
            chat.task = Some(task);
        }
    }

    /// What a tab may ask about a chat that is running.
    #[must_use]
    pub fn caps(&self, step_id: StepId) -> Option<DriverCaps> {
        self.live.get(&step_id).map(|chat| chat.caps)
    }

    /// Serves one chat request.
    ///
    /// # Panics
    ///
    /// Never: a request that is not one of the four chat variants is answered with a `Failed`
    /// naming it rather than by panicking, because the worker's match is the only caller and a
    /// widened enum should not become a crash.
    pub async fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
    ) -> Served {
        // A finished chat leaves its entry behind; sweep before anything looks one up, so a second
        // chat on a finished step is a start and not a "no live chat".
        self.live.retain(|_, chat| !chat.commands.is_closed());

        let addr = ReplyAddr {
            seq: envelope.seq,
            origin: envelope.origin.clone(),
        };
        match &envelope.request {
            StoreRequest::ChatStart {
                project_id,
                agent_id,
                model,
                prompt,
            } => {
                match self
                    .start(
                        backend,
                        replies,
                        addr,
                        *project_id,
                        *agent_id,
                        model.clone(),
                        prompt.clone(),
                    )
                    .await
                {
                    Ok(started) => started,
                    Err(err) => Served::Reply(failed("chat_start", &err)),
                }
            }
            StoreRequest::ChatSend { step_id, text } => self.command(
                *step_id,
                "chat_send",
                ChatCommand::Send {
                    text: text.clone(),
                    reply: addr,
                },
            ),
            StoreRequest::ChatAnswer {
                step_id,
                request_id,
                answer,
            } => self.command(
                *step_id,
                "chat_answer",
                ChatCommand::Answer {
                    request_id: request_id.clone(),
                    answer: answer.clone(),
                    reply: addr,
                },
            ),
            StoreRequest::ChatCancel { step_id } => self.command(
                *step_id,
                "chat_cancel",
                ChatCommand::Cancel { reply: Some(addr) },
            ),
            other => Served::Reply(StoreReply::Failed {
                request: other.name(),
                message: "not a chat request".to_owned(),
            }),
        }
    }

    /// Cancels every live chat and waits for it, then gives up on stragglers.
    ///
    /// Called when the UI is gone. Without it the runtime drops every session task at its first
    /// await and the agent processes are orphaned (`docs/ANA-4.md` §11 criterion 11).
    pub async fn shutdown(&mut self, grace: Duration) {
        for (step, chat) in self.live.drain() {
            let _ = chat.commands.send(ChatCommand::Cancel { reply: None });
            let Some(task) = chat.task else { continue };
            if tokio::time::timeout(grace * 2, task).await.is_err() {
                tracing::warn!(%step, "a chat did not end within the grace window");
            }
        }
    }

    /// Forwards a command to a live chat.
    fn command(&mut self, step_id: StepId, request: &'static str, command: ChatCommand) -> Served {
        let Some(chat) = self.live.get(&step_id) else {
            return Served::Reply(StoreReply::Failed {
                request,
                message: format!("no live chat for step {step_id}"),
            });
        };
        if chat.commands.send(command).is_err() {
            self.live.remove(&step_id);
            return Served::Reply(StoreReply::Failed {
                request,
                message: "this chat has ended".to_owned(),
            });
        }
        Served::Deferred
    }

    /// The `ChatStart` path: identity, registry row, driver, the two rows, the session future.
    #[expect(
        clippy::too_many_arguments,
        reason = "the request's own fields plus the three the worker supplies; a struct would \
                  rename the arity"
    )]
    async fn start(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        project_id: htui_core::model::ProjectId,
        agent_id: htui_core::model::AgentId,
        model: Option<String>,
        prompt: String,
    ) -> Result<Served, StoreError> {
        // Milestone 4 owns the offline session path (`append_pending`); until then a chat that
        // cannot record is refused rather than run into memory nobody will ever read.
        let writer = backend
            .writer()
            .ok_or_else(|| StoreError::Unreachable("a chat needs a writable store".to_owned()))?;
        let box_id = backend
            .box_info()
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "box",
                id: "this box is not registered".to_owned(),
            })?
            .box_id;
        let user = backend.this_user().await?;
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;

        let summary = backend
            .agents()
            .await?
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "agent",
                id: agent_id.to_string(),
            })?;
        if !summary.agent.enabled {
            return Err(StoreError::Constraint(format!(
                "agent `{}` is disabled",
                summary.agent.name
            )));
        }
        let driver = self
            .factory
            .driver_for(&summary.agent, summary.on_box.as_ref())
            .map_err(|err| StoreError::Backend(err.to_string()))?;
        // The registry's own rule: settings that do not parse are settings that are not set
        // (`htui_agent::registry`), because the column is `JSONB NOT NULL DEFAULT '{}'` and
        // hand-editable.
        let settings: AgentSettings =
            serde_json::from_value(summary.agent.settings.clone()).unwrap_or_default();
        let model = model.or_else(|| summary.agent.default_model.clone());

        let chat = ChatRunSpec::mint(project_id, box_id, user, Some(agent_id), model.clone());
        writer.start_chat_run(&chat).await?;

        let spec = SessionSpec {
            agent_id,
            step_id: chat.step_id,
            // The chat's working directory is this process's own until MOD-13 and MOD-7 give a
            // project a repo path per box (`docs/ANA-2.md` §4.7); the header shows the project's
            // name, never a path it does not have.
            //
            // A failure here **refuses the chat**: the path guard admits a file only under an
            // absolute session directory, and `"."` as a fallback would be a session with no scope
            // at all rather than a session scoped to somewhere unexpected.
            cwd,
            extra_dirs: Vec::new(),
            // MOD-10 fills this from the secret provider; until then a session carries none, and
            // the scrubber below therefore masks the credential prefixes only.
            env: std::collections::BTreeMap::new(),
            model: model.clone(),
            tools: htui_agent::driver::ToolExposure::default(),
            mcp: Vec::new(),
            permission: settings.permission.clone(),
            retain_raw: std::env::var(KEEP_RAW_ENV).is_ok_and(|value| value == "1"),
            resume: None,
        };

        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let caps = driver.caps();
        self.live.insert(
            chat.step_id,
            LiveChat {
                commands: commands_tx,
                caps,
                task: None,
            },
        );
        self.started.push(chat.step_id);

        let step_id = chat.step_id;
        let args = ChatArgs {
            driver,
            writer,
            chat,
            spec,
            prompt,
            policy: settings.permission,
            caps,
            commands: commands_rx,
            frames: Frames {
                tx: replies.clone(),
                addr,
            },
            grace: self.grace,
        };
        Ok(Served::Start {
            step_id,
            task: Box::pin(run_chat(args)),
        })
    }
}

/// Everything one chat session needs.
pub struct ChatArgs {
    driver: Box<dyn AgentDriver>,
    writer: Writer,
    chat: ChatRunSpec,
    spec: SessionSpec,
    prompt: String,
    policy: PermissionPolicy,
    caps: DriverCaps,
    commands: mpsc::UnboundedReceiver<ChatCommand>,
    frames: Frames,
    grace: Duration,
}

impl core::fmt::Debug for ChatArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChatArgs")
            .field("driver", &self.driver.name())
            .field("step", &self.chat.step_id)
            .field("spec", &self.spec)
            .finish()
    }
}

/// The reply-channel side of one chat: one address, one sender, one place frames are shaped.
struct Frames {
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    addr: ReplyAddr,
}

impl Frames {
    /// One recorded, scrubbed envelope.
    fn event(&self, envelope: DriverEnvelope) {
        self.send(
            self.addr.clone(),
            StoreReply::Chat(ChatFrame::Event(Box::new(envelope))),
        );
    }

    /// One of the three rows `htui` authors itself, shaped as an `other` event for transport only.
    ///
    /// The recorder has already written the real row with its real kind; this is the copy the tab
    /// renders, and shaping it as `other` is what keeps [`ChatFrame`] one type instead of four.
    fn local(&self, update: &str, body: Value, at: DateTime<Utc>) {
        self.event(DriverEnvelope {
            event: DriverEvent::Other(htui_agent::event::OtherEvent {
                update: update.to_owned(),
                body,
            }),
            raw: None,
            at,
        });
    }

    /// The session is over.
    fn ended(&self, stop_reason: StopReason) {
        self.send(
            self.addr.clone(),
            StoreReply::Chat(ChatFrame::Ended { stop_reason }),
        );
    }

    /// The session died.
    fn failed(&self, message: String) {
        self.send(
            self.addr.clone(),
            StoreReply::Chat(ChatFrame::Failed { message }),
        );
    }

    /// Answers one request by its own address.
    fn reply(&self, addr: &ReplyAddr, reply: StoreReply) {
        self.send(addr.clone(), reply);
    }

    fn send(&self, addr: ReplyAddr, reply: StoreReply) {
        // A UI that has gone away is not an error: the rows still matter, the frames do not.
        let _ = self.tx.send(ReplyEnvelope {
            seq: addr.seq,
            origin: addr.origin,
            reply,
        });
    }
}

/// How one turn ended.
enum TurnEnd {
    /// The agent finished it.
    Done(StopReason),
    /// The user cancelled it, and the session is over.
    Cancelled,
}

/// One chat session, start to finish.
///
/// Owns the driver, the writer, the recorder and the session handle; answers the `ChatStart`
/// request once the handshake is through (a spawn plus `initialize` plus `session/new` takes
/// seconds, and the store worker must not be blocked behind them).
pub async fn run_chat(args: ChatArgs) {
    let ChatArgs {
        driver,
        writer,
        chat,
        spec,
        prompt,
        policy,
        caps,
        mut commands,
        frames,
        grace,
    } = args;

    let scrubber = MinimalScrubber::new(spec.env.values().cloned());
    let step_id = chat.step_id;
    let start_addr = frames.addr.clone();

    let mut session = match driver.start(spec, prompt.clone()).await {
        Ok(session) => session,
        Err(err) => {
            let message = err.to_string();
            frames.reply(
                &start_addr,
                StoreReply::Failed {
                    request: "chat_start",
                    message: message.clone(),
                },
            );
            close_run(&writer, &chat, RunStatus::Failed).await;
            frames.failed(message);
            return;
        }
    };

    frames.reply(
        &start_addr,
        StoreReply::ChatAccepted {
            step_id,
            session_ref: session.session_ref().cloned(),
            caps,
        },
    );

    let (ui_tx, mut ui_rx) = mpsc::channel(UI_FRAMES);
    let mut recorder = Recorder::new(
        &writer,
        &scrubber,
        step_id,
        std::env::var(KEEP_RAW_ENV).is_ok_and(|value| value == "1"),
        Some(ui_tx),
    );

    let now = Utc::now();
    if let Err(err) = recorder
        .record_prompt(&prompt, prompt_sections(), now)
        .await
    {
        tracing::error!(%err, "the prompt row could not be written");
    }
    frames.local("prompt", json!({ "text": prompt }), now);

    // Every call this session has seen, so a permission request can be evaluated against the tool
    // call it gates (`htui_agent::permission`).
    let mut calls: HashMap<String, ToolCallEvent> = HashMap::new();
    let mut status = RunStatus::Done;
    let mut last_stop = StopReason::EndTurn;

    loop {
        match run_turn(
            session.as_mut(),
            &mut recorder,
            &mut ui_rx,
            &mut commands,
            &policy,
            &mut calls,
            &frames,
            grace,
        )
        .await
        {
            Ok(TurnEnd::Done(stop)) => last_stop = stop,
            Ok(TurnEnd::Cancelled) => {
                status = RunStatus::Cancelled;
                last_stop = StopReason::Cancelled;
                break;
            }
            Err(err) => {
                tracing::warn!(%err, "the chat session ended with a transport error");
                status = RunStatus::Failed;
                frames.failed(err.to_string());
                break;
            }
        }

        // Between turns the session idles on the user, not on the wire: a closed transport is
        // noticed by the next command rather than here (a `next_event` here would end every chat
        // the moment its first turn closed).
        match commands.recv().await {
            Some(ChatCommand::Send { text, reply }) => {
                if let Err(err) = session.send_follow_up(text.clone()).await {
                    frames.reply(
                        &reply,
                        StoreReply::Failed {
                            request: "chat_send",
                            message: err.to_string(),
                        },
                    );
                    status = RunStatus::Failed;
                    frames.failed(err.to_string());
                    break;
                }
                let at = Utc::now();
                if let Err(err) = recorder.record_follow_up(&text, at).await {
                    tracing::error!(%err, "the follow-up row could not be written");
                }
                // **Once**, at the request's own address: the frame passes `App::is_fresh` from
                // either address, so sending it to the stream as well would render the same
                // follow-up twice. Every request is answered exactly once, including the ones that
                // succeed (`ChatCommand`'s contract).
                frames.reply(
                    &reply,
                    StoreReply::Chat(ChatFrame::Event(Box::new(follow_up_frame(&text, at)))),
                );
            }
            Some(ChatCommand::Answer { reply, .. }) => {
                frames.reply(
                    &reply,
                    StoreReply::Failed {
                        request: "chat_answer",
                        message: "no permission request is waiting".to_owned(),
                    },
                );
            }
            // Ending a chat **between** turns is not a cancellation: nothing was cut, the user is
            // simply finished, and the run closes `done` with the last turn's own stop reason
            // (a turn cut mid-flight is the `TurnEnd::Cancelled` arm above, which closes
            // `cancelled`).
            Some(ChatCommand::Cancel { reply }) => {
                let _ = session.cancel(grace).await;
                drain(session.as_mut(), &mut recorder, &mut ui_rx, &frames).await;
                if let Some(reply) = reply {
                    frames.reply(
                        &reply,
                        StoreReply::Chat(ChatFrame::Ended {
                            stop_reason: last_stop,
                        }),
                    );
                }
                break;
            }
            // The runtime is gone: end the session rather than leave a child running. Same rule —
            // no turn was open, so the run is done rather than cancelled.
            None => {
                let _ = session.cancel(grace).await;
                drain(session.as_mut(), &mut recorder, &mut ui_rx, &frames).await;
                break;
            }
        }
    }

    if let Err(err) = recorder.finish().await {
        tracing::error!(%err, "the recorder did not close cleanly");
        status = RunStatus::Failed;
    }
    close_run(&writer, &chat, status).await;
    frames.ended(last_stop);
}

/// Drives one turn: pulls events, records them, and serves commands while a request is parked.
///
/// This is [`htui_agent::record::pump`]'s shape with one difference the plan's D28 did not have:
/// `pump` cannot cross a parked permission request — `next_event` refuses while one is
/// outstanding, on every transport — so the pull and the command channel have to be served by the
/// same loop.
#[expect(
    clippy::too_many_arguments,
    reason = "one turn's collaborators; a struct would rename the arity without reducing it"
)]
async fn run_turn(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, Writer>,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    commands: &mut mpsc::UnboundedReceiver<ChatCommand>,
    policy: &PermissionPolicy,
    calls: &mut HashMap<String, ToolCallEvent>,
    frames: &Frames,
    grace: Duration,
) -> Result<TurnEnd, DriverError> {
    let mut parked: Option<PermissionRequestId> = None;

    loop {
        if parked.is_some() {
            // Nothing may be pulled until the agent has its answer.
            match commands.recv().await {
                Some(ChatCommand::Answer {
                    request_id,
                    answer,
                    reply,
                }) => {
                    // A second tap on the same digit, or an answer that raced the frame saying it
                    // was already answered, is a stale request — **not** a reason to end the
                    // conversation. It is refused at its own address and the turn goes on.
                    if parked.as_ref() != Some(&request_id) {
                        frames.reply(
                            &reply,
                            StoreReply::Failed {
                                request: "chat_answer",
                                message: format!("no permission request `{request_id}` is waiting"),
                            },
                        );
                        continue;
                    }
                    session
                        .answer_permission(request_id.clone(), answer.clone())
                        .await?;
                    // **One** frame, addressed to the request that caused it: it passes
                    // `App::is_fresh` either way, and a second copy at the stream's address would
                    // render the same answer twice.
                    record_answer(
                        recorder,
                        &request_id,
                        &answer,
                        AnsweredBy::User,
                        frames,
                        Some(&reply),
                    )
                    .await;
                    parked = None;
                }
                Some(ChatCommand::Cancel { reply }) => {
                    session.cancel(grace).await?;
                    if let Some(request_id) = parked.take() {
                        record_answer(
                            recorder,
                            &request_id,
                            &PermissionAnswer::Cancelled,
                            AnsweredBy::Policy,
                            frames,
                            None,
                        )
                        .await;
                    }
                    drain(session, recorder, ui, frames).await;
                    if let Some(reply) = reply {
                        frames.reply(
                            &reply,
                            StoreReply::Chat(ChatFrame::Ended {
                                stop_reason: StopReason::Cancelled,
                            }),
                        );
                    }
                    return Ok(TurnEnd::Cancelled);
                }
                Some(ChatCommand::Send { reply, .. }) => {
                    frames.reply(
                        &reply,
                        StoreReply::Failed {
                            request: "chat_send",
                            message: "answer the permission request first".to_owned(),
                        },
                    );
                }
                None => return Ok(TurnEnd::Cancelled),
            }
            continue;
        }

        let Some(envelope) = session.next_event().await? else {
            // The stream ended without a `done`. That is a transport that died mid-turn, not a
            // turn that finished: reporting it as `EndTurn` would close the run `done`, leave no
            // `done` row in the log, and tell the tab nothing.
            return Err(DriverError::Closed);
        };
        let event = envelope.event.clone();
        record(recorder, envelope, ui, frames).await;

        match event {
            DriverEvent::ToolCall(call) => {
                calls.insert(call.tool_call_id.clone(), call);
            }
            DriverEvent::PermissionRequest(request) => {
                let call = request.tool_call_id.as_ref().and_then(|id| calls.get(id));
                // Stages 1 and 2 of ANA-4 §4.3 decide here, where the recorder is; stage 3 parks
                // the request and the user decides.
                match htui_agent::permission::evaluate(policy, call, &request.options) {
                    Some(answered) => {
                        let answer = PermissionAnswer::Selected(answered.option_id.clone());
                        session
                            .answer_permission(request.request_id.clone(), answer.clone())
                            .await?;
                        tracing::info!(
                            stage = ?answered.stage,
                            reason = %answered.reason,
                            "a permission request was answered by policy"
                        );
                        record_answer(
                            recorder,
                            &request.request_id,
                            &answer,
                            AnsweredBy::Policy,
                            frames,
                            None,
                        )
                        .await;
                    }
                    None => parked = Some(request.request_id.clone()),
                }
            }
            DriverEvent::Done(done) => return Ok(TurnEnd::Done(done.stop_reason)),
            _ => {}
        }
    }
}

/// Records one envelope and forwards the scrubbed copy the recorder made of it.
///
/// The frame comes from the recorder's own channel, never from the envelope this function was
/// handed: what reaches the screen must be masked exactly as what reached the store (`R-SEC-3`).
async fn record(
    recorder: &mut Recorder<'_, Writer>,
    envelope: DriverEnvelope,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    frames: &Frames,
) {
    if let Err(err) = recorder.record(envelope).await {
        tracing::error!(%err, "an event could not be recorded");
    }
    while let Ok(frame) = ui.try_recv() {
        frames.event(frame);
    }
}

/// Pulls what is left of a cancelled session into the log.
async fn drain(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, Writer>,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    frames: &Frames,
) {
    while let Ok(Some(envelope)) = session.next_event().await {
        let done = matches!(envelope.event, DriverEvent::Done(_));
        record(recorder, envelope, ui, frames).await;
        if done {
            break;
        }
    }
}

/// Writes the `permission_answer` row and its frame.
async fn record_answer(
    recorder: &mut Recorder<'_, Writer>,
    request_id: &PermissionRequestId,
    answer: &PermissionAnswer,
    by: AnsweredBy,
    frames: &Frames,
    // Where the frame goes: the request that asked, when a user's answer caused it, else the
    // stream's own address (a policy answer nobody asked for).
    to: Option<&ReplyAddr>,
) {
    let (option_id, cancelled) = match answer {
        PermissionAnswer::Selected(option_id) => (Some(option_id.clone()), false),
        PermissionAnswer::Cancelled => (None, true),
    };
    let at = Utc::now();
    if let Err(err) = recorder
        .record_permission_answer(request_id, option_id.as_deref(), by, cancelled, at)
        .await
    {
        tracing::error!(%err, "the permission answer row could not be written");
    }
    let frame = DriverEnvelope {
        event: DriverEvent::Other(htui_agent::event::OtherEvent {
            update: "permission_answer".to_owned(),
            body: json!({
                "request_id": request_id.as_str(),
                "option_id": option_id,
                "by": by.as_str(),
                "cancelled": cancelled,
            }),
        }),
        raw: None,
        at,
    };
    match to {
        Some(addr) => frames.reply(addr, StoreReply::Chat(ChatFrame::Event(Box::new(frame)))),
        None => frames.event(frame),
    }
}

/// The frame a follow-up produces, shaped as the `other` row the tab renders.
fn follow_up_frame(text: &str, at: DateTime<Utc>) -> DriverEnvelope {
    DriverEnvelope {
        event: DriverEvent::Other(htui_agent::event::OtherEvent {
            update: "follow_up".to_owned(),
            body: json!({ "text": text }),
        }),
        raw: None,
        at,
    }
}

/// The `sections[]` of the `prompt` row.
///
/// A chat's prompt is what the user typed, so it is one section. ANA-5's assembler fills this with
/// the real section list in milestone 9, and the row's shape does not change when it does.
fn prompt_sections() -> Value {
    json!([{ "name": "chat", "tokens": Value::Null, "trimmed": false }])
}

/// Closes the chat's `run` / `run_step` pair, so it stops counting as an active run.
async fn close_run(writer: &Writer, chat: &ChatRunSpec, status: RunStatus) {
    if let Err(err) = writer
        .finish_chat_run(chat.run_id, chat.step_id, status, Utc::now())
        .await
    {
        tracing::error!(%err, "the chat run could not be closed");
    }
}

/// The `session_started` banner body, for a caller that wants the agent-side id out of a frame.
#[must_use]
pub fn session_ref_of(envelope: &DriverEnvelope) -> Option<&str> {
    match &envelope.event {
        DriverEvent::Other(other) if other.update == SESSION_STARTED => {
            other.body.get("session_id").and_then(Value::as_str)
        }
        _ => None,
    }
}

/// Renders a store error into the reply the asking view receives.
fn failed(request: &'static str, err: &StoreError) -> StoreReply {
    StoreReply::Failed {
        request,
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_agent::conformance::{Script, ScriptEvent};
    use htui_agent::event::{
        DoneEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent, TextChunk,
        ToolKind,
    };
    use htui_agent::fake::FakeAdapter;
    use htui_core::fixtures::ids;
    use htui_core::model::{Agent, AgentId, EventKind, Scope, Transport};
    use htui_core::store::MemStore;
    use htui_core::store::ReadStore as _;
    use std::sync::Arc;

    /// A registry row for the fake transport: `cli`, stream `fake`, so the factory reaches the
    /// adapter by **row data** and not by name (`R-AGT-5`, plan D12).
    fn fake_row(id: AgentId) -> Agent {
        Agent {
            id,
            name: "scripted".to_owned(),
            transport: Transport::Cli,
            billing: htui_core::model::Billing::PerToken,
            models: Vec::new(),
            default_model: None,
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({ "cli": { "stream": "fake", "permission_mode": "ask",
                                       "extra_args": [] } }),
            enabled: true,
            created_at: htui_core::fixtures::demo_at(0, 0),
            updated_at: htui_core::fixtures::demo_at(0, 0),
        }
    }

    /// A store holding the demo fixture plus the fake row, and the runtime that can drive it.
    async fn fixture(script: Script) -> (MemStore, Backend, AgentRuntime, AgentId) {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&fake_row(agent_id))
            .await
            .expect("the fake row lands");

        let adapter = Arc::new(FakeAdapter::new());
        adapter.load(script);
        let mut factory = DriverFactory::new();
        factory.register("cli/fake", Box::new(FakeBuilder(Arc::clone(&adapter))));

        let backend = Backend::memory(store.clone());
        let runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));
        (store, backend, runtime, agent_id)
    }

    /// Lets one `FakeAdapter` be shared between the test and the factory.
    #[derive(Debug)]
    struct FakeBuilder(Arc<FakeAdapter>);

    impl htui_agent::registry::TransportBuilder for FakeBuilder {
        fn build(
            &self,
            agent: &Agent,
            on_box: Option<&htui_core::model::AgentBox>,
            caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            self.0.build(agent, on_box, caps)
        }
    }

    fn envelope(seq: Seq, request: StoreRequest) -> RequestEnvelope {
        RequestEnvelope {
            seq,
            origin: Origin::Tab(crate::ui::tabs::TabId("chat")),
            request,
        }
    }

    fn start(agent_id: AgentId, prompt: &str) -> StoreRequest {
        StoreRequest::ChatStart {
            project_id: ids::PROJECT_HTUI,
            agent_id,
            model: None,
            prompt: prompt.to_owned(),
        }
    }

    fn scope() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        }
    }

    /// Drives a `ChatStart` to completion inline and returns every reply it produced.
    ///
    /// The cancel is queued **before** the future is polled, because a chat does not end by
    /// itself: after its turn it waits on the user, exactly as it does in the running binary. The
    /// command channel is unbounded, so the session plays its whole turn and then finds the
    /// waiting `Cancel` — which is the same sequence as a user pressing `Esc Esc`.
    async fn run(
        runtime: &mut AgentRuntime,
        backend: &Backend,
        request: StoreRequest,
    ) -> (StepId, Vec<ReplyEnvelope>) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let envelope = envelope(7, request);
        let Served::Start { step_id, task } = runtime.serve(backend, &tx, &envelope).await else {
            panic!("a chat start opens a session")
        };
        let cancel_envelope = RequestEnvelope {
            seq: 8,
            origin: envelope.origin.clone(),
            request: StoreRequest::ChatCancel { step_id },
        };
        let cancel = runtime.serve(backend, &tx, &cancel_envelope).await;
        assert!(
            matches!(cancel, Served::Deferred),
            "the session answers its own cancel: {cancel:?}"
        );
        task.await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }
        (step_id, replies)
    }

    #[tokio::test]
    async fn a_scripted_chat_records_its_turn_and_closes_its_run() {
        let script = Script::one_turn(vec![
            ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
                text: "hello".to_owned(),
                message_id: Some("m1".to_owned()),
            })),
            ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
        ]);
        let (store, backend, mut runtime, agent_id) = fixture(script).await;
        let before = store.active_runs(&scope()).await.expect("count");

        let (step_id, replies) = run(&mut runtime, &backend, start(agent_id, "say hello")).await;

        // Every *stream* frame answers the `ChatStart` request, so `App::is_fresh` passes all of
        // them; the `ChatCancel` request gets its own answer at its own `seq`, which is what
        // "exactly one reply per request" means for a request that is not the stream's.
        assert!(
            replies
                .iter()
                .filter(|reply| matches!(reply.reply, StoreReply::Chat(ChatFrame::Event(_))))
                .all(|reply| reply.seq == 7),
            "every stream frame carries the ChatStart seq: {replies:?}"
        );
        assert_eq!(
            replies.iter().filter(|reply| reply.seq == 8).count(),
            1,
            "the cancel is answered exactly once"
        );
        assert!(
            matches!(replies[0].reply, StoreReply::ChatAccepted { .. }),
            "the first reply is the acceptance: {:?}",
            replies[0].reply
        );
        assert!(
            matches!(
                replies.last().map(|reply| &reply.reply),
                Some(StoreReply::Chat(ChatFrame::Ended { .. }))
            ),
            "the last reply ends the stream: {:?}",
            replies.last()
        );

        let log = store
            .step_events(step_id)
            .await
            .expect("the log reads")
            .expect("the chat step has a log");
        let kinds: Vec<EventKind> = log.iter().map(|row| row.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::Prompt,
                EventKind::Other,
                EventKind::AssistantText,
                EventKind::Done
            ],
            "the prompt, the session banner, the coalesced text and the done"
        );
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before,
            "a finished chat stops counting as an active run"
        );
    }

    #[tokio::test]
    async fn a_command_for_an_unknown_step_is_refused() {
        let (_, backend, mut runtime, _) = fixture(Script::default()).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::ChatSend {
                        step_id: StepId::new(),
                        text: "hi".to_owned(),
                    },
                ),
            )
            .await;
        assert!(
            matches!(
                served,
                Served::Reply(StoreReply::Failed {
                    request: "chat_send",
                    ..
                })
            ),
            "{served:?}"
        );
    }

    #[tokio::test]
    async fn an_offline_backend_refuses_to_start_a_chat() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "chat-test", 1)
            .await
            .expect("mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let mut runtime = AgentRuntime::new(DriverFactory::new());
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(1, start(AgentId::new(), "hi")))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "chat_start");
                assert!(message.contains("writable"), "{message}");
            }
            other => panic!("an offline chat must be refused: {other:?}"),
        }
        cache.close().await;
    }

    /// A policy rule answers stages 1–2 of ANA-4 §4.3 without ever reaching the user.
    #[tokio::test]
    async fn a_policy_rule_answers_a_permission_request_and_records_it() {
        let script = Script::one_turn(vec![
            ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
                tool_call_id: "call-1".to_owned(),
                title: "Read".to_owned(),
                tool_kind: ToolKind::Read,
                input: json!({ "path": "src/main.rs" }),
                locations: Vec::new(),
            })),
            ScriptEvent::ParkPermission(PermissionRequestEvent {
                request_id: PermissionRequestId::new("req-1"),
                tool_call_id: Some("call-1".to_owned()),
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            }),
            ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
        ]);
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        let mut row = fake_row(agent_id);
        // Every read is allowed, by rule.
        row.settings = json!({
            "cli": { "stream": "fake", "permission_mode": "ask", "extra_args": [] },
            "permission": {
                "default": "ask",
                "rules": [ { "match": { "tool_kind": "read" }, "answer": "allow_once",
                             "reason": "reads are safe" } ],
                "remembered": []
            }
        });
        store.upsert_agent(&row).await.expect("the row lands");

        let adapter = Arc::new(FakeAdapter::new());
        adapter.load(script);
        let mut factory = DriverFactory::new();
        factory.register("cli/fake", Box::new(FakeBuilder(Arc::clone(&adapter))));
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));

        let (step_id, _replies) = run(&mut runtime, &backend, start(agent_id, "read it")).await;

        let log = store
            .step_events(step_id)
            .await
            .expect("the log reads")
            .expect("a log");
        let answer = log
            .iter()
            .find(|row| row.kind == EventKind::PermissionAnswer)
            .expect("the rule answered the request without asking the user");
        assert_eq!(
            answer.payload.get("by").and_then(Value::as_str),
            Some("policy"),
            "a rule's answer is recorded as policy, not as the user's (ANA-9 §4.3)"
        );
        assert_eq!(
            answer.payload.get("option_id").and_then(Value::as_str),
            Some("allow")
        );
    }
}

//! What a chat looks like: frames in, lines out (`R-TUI-6`).
//!
//! The transcript keeps its **own** coalescing. The recorder's is about rows — one
//! `assistant_text` per contiguous run, flushed on a bound — and it happens after the frame has
//! already gone to the screen, so a tab that waited for it would render a turn only once it was
//! over. Two coalescers with one rule (`message_id` groups a run) is the price of streaming.

use htui_agent::driver::{AgentSessionRef, PermissionRequestId};
use htui_agent::event::{
    DriverEnvelope, DriverEvent, PermissionOption, PlanEntry, StopReason, TerminalReason, ToolKind,
};
use htui_core::model::SessionEvent;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::app::Handled;
use crate::ui::Theme;
use crossterm::event::{KeyCode, KeyEvent};

/// Where a tool call stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallStatus {
    /// The agent has not reported an end yet.
    Running,
    /// It finished.
    Completed,
    /// It failed, was rejected, or was cancelled.
    Failed {
        /// Set only on a row `htui` synthesized (`docs/ANA-4.md` §4.3).
        reason: Option<TerminalReason>,
    },
}

/// How a permission request was answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The chosen option, or `None` for a cancellation or a refusal.
    pub option_id: Option<String>,
    /// `user` or `policy` (ANA-9 §4.3).
    pub by: String,
    /// `true` when the answer **refused** the call rather than settling it some other way.
    ///
    /// An absent `option_id` alone cannot tell a refusal from a cancellation, and they are
    /// different facts about what happened to the user's work: a cancellation is something they
    /// asked for, a denial is something a policy did to them. MOD-2 D85's whole purpose is to make
    /// the second visible, so the transcript has to be able to say which.
    pub denied: bool,
}

/// One rendered thing in a chat.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptRow {
    /// The assembled prompt that opened the session.
    Prompt {
        /// What was sent.
        text: String,
    },
    /// A follow-up the user typed.
    FollowUp {
        /// What was sent.
        text: String,
    },
    /// Agent text, coalesced per `message_id`.
    Assistant {
        /// The text so far.
        text: String,
        /// The run's grouping key.
        message_id: Option<String>,
    },
    /// Agent reasoning, coalesced the same way and foldable.
    Thought {
        /// The text so far.
        text: String,
        /// The run's grouping key.
        message_id: Option<String>,
    },
    /// A tool call and how it ended.
    ToolCall {
        /// The transport's call id.
        id: String,
        /// What the agent called it.
        title: String,
        /// The ten-value ACP kind.
        kind: ToolKind,
        /// Where it stands.
        status: CallStatus,
    },
    /// A proposed edit, as a diff.
    EditProposal {
        /// The enclosing call, when there was one.
        id: Option<String>,
        /// The file.
        path: String,
        /// The unified diff.
        diff: String,
        /// `true` allowed, `false` rejected, `None` while a request is parked.
        accepted: Option<bool>,
    },
    /// A permission request, with its options and its answer once it has one.
    Permission {
        /// Correlates the request with its answer.
        request_id: PermissionRequestId,
        /// The call being gated.
        tool_call_id: Option<String>,
        /// What the agent offered.
        options: Vec<PermissionOption>,
        /// `None` while the user has not answered.
        resolved: Option<Resolution>,
    },
    /// The agent's complete plan.
    Plan {
        /// Every entry, as of this update.
        entries: Vec<PlanEntry>,
    },
    /// A usage report, already rendered to one line.
    Usage {
        /// The line.
        line: String,
    },
    /// Something went wrong.
    Error {
        /// The machine-readable code.
        code: String,
        /// The message.
        message: String,
    },
    /// The end of a turn.
    Done {
        /// Why it ended.
        stop_reason: StopReason,
    },
    /// Anything else the protocol said, named but not interpreted.
    Other {
        /// The transport's own name for it.
        update: String,
    },
}

/// The rendered conversation.
#[derive(Debug, Default)]
pub struct Transcript {
    rows: Vec<TranscriptRow>,
    /// Thoughts collapse to one line each by default (`R-TUI-6`: "collapsed thoughts").
    fold_thoughts: bool,
    /// Where the view is.
    scroll: Scroll,
    /// Lines the last render produced, so `k` from the tail can step back one **line** rather
    /// than mistaking the row count for a line index (a row can render many lines).
    rendered: std::cell::Cell<usize>,
    /// Height the last render was given, for the same reason.
    height: std::cell::Cell<usize>,
    /// The agent-side session id, taken from the banner rather than rendered as a row.
    session_ref: Option<AgentSessionRef>,
}

/// Where the view is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Scroll {
    /// Pinned to the newest line: a streaming turn stays visible without a keystroke.
    #[default]
    Tail,
    /// Parked at a line the user scrolled to.
    At(usize),
}

impl Transcript {
    /// An empty transcript with thoughts folded.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fold_thoughts: true,
            ..Self::default()
        }
    }

    /// A finished step's persisted rows, rendered through the live path (D37, `R-TUI-6`).
    ///
    /// The rows are decoded by [`htui_agent::replay`] and then handed to [`Transcript::apply`]
    /// one by one — the *same* method a live frame goes through. That is the whole point: a
    /// replay that rendered through a second code path would drift from the live one, and the
    /// first thing it would drift on is the kind an adapter added last week.
    ///
    /// The lossy decode is deliberate (D37): a row nobody can read still reaches the screen as a
    /// dim `<kind>` line rather than costing the reader the rest of the conversation.
    #[must_use]
    pub fn from_rows(rows: &[SessionEvent]) -> Self {
        let mut transcript = Self::new();
        for envelope in htui_agent::replay::envelopes(rows) {
            transcript.apply(&envelope);
        }
        transcript
    }

    /// Every row, for assertions.
    #[must_use]
    pub fn rows(&self) -> &[TranscriptRow] {
        &self.rows
    }

    /// How many rows are on screen — which is not how many rows were persisted: a `tool_result`
    /// folds into its call and the session banner becomes the header.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there is nothing to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The agent-side session id the banner carried.
    #[must_use]
    pub fn session_ref(&self) -> Option<&AgentSessionRef> {
        self.session_ref.as_ref()
    }

    /// The newest permission request nobody has answered.
    #[must_use]
    pub fn parked(&self) -> Option<&TranscriptRow> {
        self.rows
            .iter()
            .rev()
            .find(|row| matches!(row, TranscriptRow::Permission { resolved: None, .. }))
    }

    /// Applies one frame.
    pub fn apply(&mut self, envelope: &DriverEnvelope) {
        match &envelope.event {
            DriverEvent::AssistantChunk(chunk) => {
                self.append_text(&chunk.text, chunk.message_id.as_deref(), false);
            }
            DriverEvent::ThoughtChunk(chunk) => {
                self.append_text(&chunk.text, chunk.message_id.as_deref(), true);
            }
            DriverEvent::ToolCall(call) => self.rows.push(TranscriptRow::ToolCall {
                id: call.tool_call_id.clone(),
                title: call.title.clone(),
                kind: call.tool_kind,
                status: CallStatus::Running,
            }),
            // A result is not its own row: it is how the call it belongs to ended, which is what
            // makes a turn readable as "what the agent did" rather than as a protocol log.
            DriverEvent::ToolResult(result) => {
                if let Some(TranscriptRow::ToolCall { status, .. }) =
                    self.rows.iter_mut().rev().find(|row| {
                        matches!(row, TranscriptRow::ToolCall { id, .. } if id == &result.tool_call_id)
                    })
                {
                    *status = match result.status {
                        htui_agent::event::ToolResultStatus::Completed => CallStatus::Completed,
                        htui_agent::event::ToolResultStatus::Failed => CallStatus::Failed {
                            reason: result.terminal_reason,
                        },
                    };
                }
            }
            DriverEvent::EditProposal(proposal) => {
                let existing = self.rows.iter_mut().find(|row| {
                    matches!(row, TranscriptRow::EditProposal { id, path, .. }
                             if id == &proposal.tool_call_id && path == &proposal.path)
                });
                match existing {
                    // The recorder dedups `(tool_call_id, path)` in the store; the screen shows the
                    // same one row updated, not a second copy of the same file.
                    Some(TranscriptRow::EditProposal { diff, accepted, .. }) => {
                        diff.clone_from(&proposal.diff);
                        *accepted = proposal.accepted;
                    }
                    _ => self.rows.push(TranscriptRow::EditProposal {
                        id: proposal.tool_call_id.clone(),
                        path: proposal.path.clone(),
                        diff: proposal.diff.clone(),
                        accepted: proposal.accepted,
                    }),
                }
            }
            DriverEvent::PermissionRequest(request) => self.rows.push(TranscriptRow::Permission {
                request_id: request.request_id.clone(),
                tool_call_id: request.tool_call_id.clone(),
                options: request.options.clone(),
                resolved: None,
            }),
            DriverEvent::Plan(plan) => {
                // A plan update is always the complete list: replace the row, never append to it.
                if let Some(TranscriptRow::Plan { entries }) = self
                    .rows
                    .iter_mut()
                    .find(|row| matches!(row, TranscriptRow::Plan { .. }))
                {
                    entries.clone_from(&plan.entries);
                } else {
                    self.rows.push(TranscriptRow::Plan {
                        entries: plan.entries.clone(),
                    });
                }
            }
            DriverEvent::Usage(usage) => self.rows.push(TranscriptRow::Usage {
                line: usage_line(usage),
            }),
            DriverEvent::Error(error) => self.rows.push(TranscriptRow::Error {
                code: error.code.clone(),
                message: error.message.clone(),
            }),
            DriverEvent::Done(done) => self.rows.push(TranscriptRow::Done {
                stop_reason: done.stop_reason,
            }),
            DriverEvent::Other(other) => self.apply_other(other),
            // Plan D93: a transport whose own policy settled a request reports the answer instead
            // of parking it. It resolves the request the same way `htui`'s own answer does — and
            // when there is no parked row to resolve, because a transport with no permission
            // channel never announced one, what the user saw live is the verbatim `other` the
            // transport sent beside it (ANA-4 §6.2) and the answer is a row of the replay.
            DriverEvent::PermissionAnswer(answer) => {
                let resolution = Resolution {
                    option_id: answer.option_id.clone(),
                    by: answer.by.as_str().to_owned(),
                    denied: answer.denied,
                };
                if !self.resolve(answer.request_id.as_str(), resolution.clone()) {
                    // Nothing to resolve, because this transport never announced a request: the
                    // CLI's own `--permission-mode` refused the call and only *reported* it
                    // (§4.3, D85). The answer is still the one thing the user needs to see — it is
                    // why their tool call failed — so it becomes a row of its own rather than
                    // being dropped. Without this the chat tab showed the failed `tool_result` and
                    // no reason for it, live **and** on replay, which is the whole of what D85 was
                    // for.
                    self.rows.push(TranscriptRow::Permission {
                        request_id: answer.request_id.clone(),
                        tool_call_id: answer.tool_call_id.clone(),
                        // No options: there was never a choice to offer. The renderer reads the
                        // resolution, not the list, so an empty one costs nothing and inventing
                        // entries would claim the user could have answered.
                        options: Vec::new(),
                        resolved: Some(resolution),
                    });
                }
            }
        }
    }

    /// The three rows `htui` authors itself arrive shaped as `other`; so does the banner.
    fn apply_other(&mut self, other: &htui_agent::event::OtherEvent) {
        let text = || {
            other
                .body
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match other.update.as_str() {
            "prompt" => self.rows.push(TranscriptRow::Prompt { text: text() }),
            "follow_up" => self.rows.push(TranscriptRow::FollowUp { text: text() }),
            "permission_answer" => self.resolve_permission(&other.body),
            // The banner is the header's, not a row's: it says which session this is, which is a
            // property of the whole conversation.
            htui_agent::acp::SESSION_STARTED => {
                self.session_ref = other
                    .body
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .map(AgentSessionRef::new);
            }
            update => self.rows.push(TranscriptRow::Other {
                update: update.to_owned(),
            }),
        }
    }

    /// Marks the matching permission row answered.
    fn resolve_permission(&mut self, body: &serde_json::Value) {
        let request_id = body
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let resolution = Resolution {
            option_id: body
                .get("option_id")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            by: body
                .get("by")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("policy")
                .to_owned(),
            // Absent on every row written before MOD-2 milestone 8, which is why the event's own
            // field is `#[serde(default)]` (D94) and why this reads the same way: a row that
            // carries no `denied` key was written when a denial could not be reported at all, and
            // an answer that picked an option is not one.
            denied: body
                .get("denied")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        };
        // The replay path, where the same answer arrives as a persisted row. Its return is
        // discarded on purpose: a replayed log carries the denial's own `permission_answer` row,
        // and `apply` has already pushed a `Permission` row for it if there was nothing to
        // resolve, so a second one here would double it.
        let _ = self.resolve(request_id, resolution);
    }

    /// Marks the newest matching `permission_request` row answered, whoever answered it.
    ///
    /// Answers **whether a row matched**. Nothing matching is not an error — it is a transport
    /// that answered before it asked, which is exactly what a CLI policy denial is — but the
    /// caller has to know, because an answer with no request still has to reach the screen
    /// somehow.
    fn resolve(&mut self, request_id: &str, resolution: Resolution) -> bool {
        if let Some(TranscriptRow::Permission { resolved, .. }) =
            self.rows.iter_mut().rev().find(|row| {
                matches!(row, TranscriptRow::Permission { request_id: id, .. }
                         if id.as_str() == request_id)
            })
        {
            *resolved = Some(resolution);
            return true;
        }
        false
    }

    /// Appends a chunk to the open run of the same kind and key, or starts a new one.
    fn append_text(&mut self, text: &str, message_id: Option<&str>, thought: bool) {
        let open = self.rows.last_mut().and_then(|row| match (row, thought) {
            (TranscriptRow::Assistant { text, message_id }, false)
            | (TranscriptRow::Thought { text, message_id }, true) => Some((text, message_id)),
            _ => None,
        });
        if let Some((buffer, key)) = open
            && key.as_deref() == message_id
        {
            buffer.push_str(text);
            return;
        }
        let (text, message_id) = (text.to_owned(), message_id.map(ToOwned::to_owned));
        self.rows.push(if thought {
            TranscriptRow::Thought { text, message_id }
        } else {
            TranscriptRow::Assistant { text, message_id }
        });
    }

    /// Scrolling and folding.
    pub fn on_key(&mut self, key: KeyEvent) -> Handled {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll = match self.scroll {
                    Scroll::Tail => Scroll::Tail,
                    Scroll::At(line) => Scroll::At(line + 1),
                };
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll = match self.scroll {
                    // The tail shows the last `height` lines, so stepping back from it means the
                    // line just above that window — not `rows.len()`, which counts rows.
                    Scroll::Tail => Scroll::At(
                        self.rendered
                            .get()
                            .saturating_sub(self.height.get())
                            .saturating_sub(1),
                    ),
                    Scroll::At(line) => Scroll::At(line.saturating_sub(1)),
                };
                Handled::Consumed
            }
            KeyCode::Char('g') => {
                self.scroll = Scroll::At(0);
                Handled::Consumed
            }
            KeyCode::Char('G') => {
                self.scroll = Scroll::Tail;
                Handled::Consumed
            }
            KeyCode::Char('t') => {
                self.fold_thoughts = !self.fold_thoughts;
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    /// The transcript as lines, tail-first when nothing has been scrolled.
    #[must_use]
    pub fn lines(&self, height: usize, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in &self.rows {
            lines.extend(self.render_row(row, theme));
        }
        self.rendered.set(lines.len());
        self.height.set(height);
        let first = match self.scroll {
            Scroll::Tail => lines.len().saturating_sub(height),
            Scroll::At(line) => line.min(lines.len().saturating_sub(1)),
        };
        lines.into_iter().skip(first).take(height).collect()
    }

    /// One row's lines.
    fn render_row(&self, row: &TranscriptRow, theme: &Theme) -> Vec<Line<'static>> {
        match row {
            TranscriptRow::Prompt { text } | TranscriptRow::FollowUp { text } => {
                vec![Line::from(vec![
                    Span::styled("you  ", theme.accent),
                    Span::styled(text.clone(), theme.base),
                ])]
            }
            TranscriptRow::Assistant { text, .. } => text
                .lines()
                .map(|line| Line::styled(line.to_owned(), theme.base))
                .collect(),
            TranscriptRow::Thought { text, .. } => {
                if self.fold_thoughts {
                    let summary = text.lines().next().unwrap_or_default();
                    vec![Line::styled(
                        format!("thought  {summary} (t to unfold)"),
                        theme.dim,
                    )]
                } else {
                    text.lines()
                        .map(|line| Line::styled(format!("thought  {line}"), theme.dim))
                        .collect()
                }
            }
            TranscriptRow::ToolCall {
                title,
                kind,
                status,
                ..
            } => {
                let mark = match status {
                    CallStatus::Running => "…",
                    CallStatus::Completed => "ok",
                    CallStatus::Failed { reason: None } => "failed",
                    CallStatus::Failed {
                        reason: Some(TerminalReason::Rejected),
                    } => "rejected",
                    CallStatus::Failed {
                        reason: Some(TerminalReason::Cancelled),
                    } => "cancelled",
                };
                let style = match status {
                    CallStatus::Failed { .. } => theme.error,
                    _ => theme.dim,
                };
                vec![Line::from(vec![
                    Span::styled(format!("{} {title}", kind.as_str()), theme.base),
                    Span::styled(format!("  [{mark}]"), style),
                ])]
            }
            TranscriptRow::EditProposal {
                path,
                diff,
                accepted,
                ..
            } => {
                let state = match accepted {
                    Some(true) => "accepted",
                    Some(false) => "rejected",
                    None => "waiting",
                };
                let mut lines = vec![Line::styled(
                    format!("edit  {path}  [{state}]"),
                    theme.title,
                )];
                lines.extend(diff.lines().map(|line| {
                    let style = diff_style(line, theme);
                    Line::styled(line.to_owned(), style)
                }));
                lines
            }
            TranscriptRow::Permission {
                options, resolved, ..
            } => {
                let text = match resolved {
                    Some(Resolution {
                        option_id,
                        by,
                        denied,
                    }) => {
                        // `denied` before the absent option: both a refusal and a cancellation
                        // carry `option_id: None`, and reporting a policy refusal as "cancelled"
                        // would tell the user they stopped their own tool call.
                        let chosen = match (denied, option_id.as_deref()) {
                            (true, _) => "denied",
                            (false, Some(option)) => option,
                            (false, None) => "cancelled",
                        };
                        format!("permission  {chosen} (by {by})")
                    }
                    None => format!("permission  waiting on you ({} options)", options.len()),
                };
                let style = if resolved.is_some() {
                    theme.dim
                } else {
                    theme.accent
                };
                vec![Line::styled(text, style)]
            }
            TranscriptRow::Plan { entries } => {
                let mut lines = vec![Line::styled("plan".to_owned(), theme.title)];
                lines.extend(entries.iter().map(|entry| {
                    Line::styled(
                        format!("  [{}] {}", entry.status.as_str(), entry.content),
                        theme.dim,
                    )
                }));
                lines
            }
            TranscriptRow::Usage { line } => vec![Line::styled(line.clone(), theme.dim)],
            TranscriptRow::Error { code, message } => vec![Line::styled(
                format!("error  {code}: {message}"),
                theme.error,
            )],
            TranscriptRow::Done { stop_reason } => vec![Line::styled(
                format!("— turn ended ({}) —", stop_reason.as_str()),
                theme.dim,
            )],
            TranscriptRow::Other { update } => {
                vec![Line::styled(format!("· {update}"), theme.dim)]
            }
        }
    }
}

/// `+` accents, `-` errors, everything else is plain — the two gutters a reader looks for.
fn diff_style(line: &str, theme: &Theme) -> Style {
    if line.starts_with("+++") || line.starts_with("---") {
        theme.dim
    } else if line.starts_with('+') {
        theme.accent
    } else if line.starts_with('-') {
        theme.error
    } else {
        theme.dim
    }
}

/// A usage report as one line: what it cost and how full the context is.
fn usage_line(usage: &htui_agent::event::UsageEvent) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let (Some(used), Some(size)) = (usage.context_used, usage.context_size) {
        parts.push(format!("context {used}/{size}"));
    }
    if let Some(total) = usage.cost_micros_total {
        // Labelled an estimate because both the SDK and the CLI document it as one (ANA-4 risk 13).
        parts.push(format!("~${:.4}", total as f64 / 1_000_000.0));
    }
    if parts.is_empty() {
        "usage".to_owned()
    } else {
        format!("usage  {}", parts.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use htui_agent::event::{
        DoneEvent, EditProposalEvent, OtherEvent, PermissionAnswerEvent, PermissionOptionKind,
        PermissionRequestEvent, TextChunk, ToolCallEvent, ToolResultEvent, ToolResultStatus,
    };
    use htui_agent::record::AnsweredBy;
    use serde_json::json;

    fn envelope(event: DriverEvent) -> DriverEnvelope {
        DriverEnvelope {
            event,
            raw: None,
            at: Utc::now(),
        }
    }

    fn chunk(text: &str, id: &str) -> DriverEnvelope {
        envelope(DriverEvent::AssistantChunk(TextChunk {
            text: text.to_owned(),
            message_id: Some(id.to_owned()),
        }))
    }

    #[test]
    fn chunks_of_one_message_coalesce_and_a_new_key_starts_a_row() {
        let mut transcript = Transcript::new();
        transcript.apply(&chunk("hel", "m1"));
        transcript.apply(&chunk("lo", "m1"));
        transcript.apply(&chunk("next", "m2"));
        assert_eq!(
            transcript.rows(),
            &[
                TranscriptRow::Assistant {
                    text: "hello".to_owned(),
                    message_id: Some("m1".to_owned())
                },
                TranscriptRow::Assistant {
                    text: "next".to_owned(),
                    message_id: Some("m2".to_owned())
                },
            ]
        );
    }

    #[test]
    fn a_tool_result_folds_into_its_call_rather_than_becoming_a_row() {
        let mut transcript = Transcript::new();
        transcript.apply(&envelope(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "Read main.rs".to_owned(),
            tool_kind: ToolKind::Read,
            input: json!({}),
            locations: Vec::new(),
        })));
        transcript.apply(&envelope(DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: "call-1".to_owned(),
            status: ToolResultStatus::Failed,
            output: None,
            locations: Vec::new(),
            terminal_reason: Some(TerminalReason::Rejected),
        })));
        assert_eq!(transcript.rows().len(), 1, "one call, one row");
        assert!(matches!(
            &transcript.rows()[0],
            TranscriptRow::ToolCall {
                status: CallStatus::Failed {
                    reason: Some(TerminalReason::Rejected)
                },
                ..
            }
        ));
    }

    #[test]
    fn a_second_proposal_for_one_file_updates_the_row_it_already_has() {
        let mut transcript = Transcript::new();
        let proposal = |diff: &str, accepted: Option<bool>| {
            envelope(DriverEvent::EditProposal(EditProposalEvent {
                tool_call_id: Some("call-1".to_owned()),
                path: "src/a.rs".to_owned(),
                diff: diff.to_owned(),
                accepted,
            }))
        };
        transcript.apply(&proposal("@@ first", None));
        transcript.apply(&proposal("@@ second", Some(true)));
        assert_eq!(transcript.rows().len(), 1);
        assert!(matches!(
            &transcript.rows()[0],
            TranscriptRow::EditProposal { diff, accepted: Some(true), .. } if diff == "@@ second"
        ));
    }

    #[test]
    fn a_permission_request_is_parked_until_its_answer_arrives() {
        let mut transcript = Transcript::new();
        transcript.apply(&envelope(DriverEvent::PermissionRequest(
            PermissionRequestEvent {
                request_id: PermissionRequestId::new("req-1"),
                tool_call_id: Some("call-1".to_owned()),
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            },
        )));
        assert!(transcript.parked().is_some(), "nobody has answered yet");

        transcript.apply(&envelope(DriverEvent::Other(OtherEvent {
            update: "permission_answer".to_owned(),
            body: json!({ "request_id": "req-1", "option_id": "allow", "by": "user" }),
        })));
        assert!(
            transcript.parked().is_none(),
            "an answered request is no longer waiting on the user"
        );
    }

    /// A refusal by a transport that never asked is still the reason the user's tool call failed,
    /// so it is on the screen (MOD-2 D85, review gate MEDIUM).
    ///
    /// The CLI transport declares `permission_requests: false` and announces nothing; its
    /// `--permission-mode` refuses a call and *reports* the answer. Before this, `resolve` found no
    /// parked row and returned silently, so the chat tab showed a failed `tool_result` and no
    /// reason for it — live **and** on replay. The comment then in the code claimed the user saw
    /// "the verbatim `other` the transport sent beside it", and no such row exists: the mapper
    /// emits the typed answer alone.
    #[test]
    fn a_policy_denial_with_no_request_is_still_a_row() {
        let mut transcript = Transcript::new();
        transcript.apply(&envelope(DriverEvent::PermissionAnswer(
            PermissionAnswerEvent {
                request_id: PermissionRequestId::new("toolu_1"),
                tool_call_id: Some("toolu_1".to_owned()),
                option_id: None,
                by: AnsweredBy::Policy,
                cancelled: false,
                denied: true,
            },
        )));
        let rows = transcript.rows();
        assert_eq!(rows.len(), 1, "the refusal is a row of its own: {rows:?}");
        let theme = Theme::default();
        let text = transcript
            .render_row(&rows[0], &theme)
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(
            text.contains("denied") && text.contains("policy"),
            "and it reads as a refusal by a policy, not as something the user did: {text:?}"
        );
        assert!(
            !text.contains("cancelled"),
            "a denial is not a cancellation — both carry no `option_id`, and calling a policy \
             refusal `cancelled` tells the user they stopped their own tool call: {text:?}"
        );
        assert!(
            transcript.parked().is_none(),
            "and nothing is waiting on the user, because nothing ever asked"
        );
    }

    #[test]
    fn the_banner_becomes_the_session_id_and_not_a_row() {
        let mut transcript = Transcript::new();
        transcript.apply(&envelope(DriverEvent::Other(OtherEvent {
            update: htui_agent::acp::SESSION_STARTED.to_owned(),
            body: json!({ "session_id": "abc-123", "protocol_version": 1 }),
        })));
        assert!(transcript.rows().is_empty(), "the header shows it");
        assert_eq!(
            transcript.session_ref().map(AgentSessionRef::as_str),
            Some("abc-123")
        );
    }

    #[test]
    fn a_plan_update_replaces_the_plan_rather_than_appending_one() {
        let mut transcript = Transcript::new();
        let plan = |entries: Vec<PlanEntry>| {
            envelope(DriverEvent::Plan(htui_agent::event::PlanEvent { entries }))
        };
        let entry = |content: &str| PlanEntry {
            content: content.to_owned(),
            status: htui_agent::event::PlanEntryStatus::Pending,
            priority: htui_agent::event::PlanEntryPriority::High,
        };
        transcript.apply(&plan(vec![entry("one")]));
        transcript.apply(&plan(vec![entry("one"), entry("two")]));
        assert_eq!(transcript.rows().len(), 1, "one plan, replaced in place");
        assert!(matches!(
            &transcript.rows()[0],
            TranscriptRow::Plan { entries } if entries.len() == 2
        ));
    }

    #[test]
    fn thoughts_fold_to_one_line_until_t_unfolds_them() {
        let mut transcript = Transcript::new();
        transcript.apply(&envelope(DriverEvent::ThoughtChunk(TextChunk {
            text: "first\nsecond\nthird".to_owned(),
            message_id: None,
        })));
        let theme = Theme::default();
        assert_eq!(transcript.lines(20, &theme).len(), 1, "folded");

        transcript.on_key(KeyEvent::from(KeyCode::Char('t')));
        assert_eq!(transcript.lines(20, &theme).len(), 3, "unfolded");
    }

    /// The fixture's `plan` step is eight persisted rows and seven rendered ones: the
    /// `tool_result` folds into its call, and every other row keeps its place and its order.
    #[tokio::test]
    async fn from_rows_renders_a_persisted_step_the_way_the_live_path_would() {
        use htui_core::fixtures::ids;
        use htui_core::store::{MemStore, ReadStore as _};

        let rows = MemStore::demo()
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the memory store never fails")
            .expect("the fixture's plan step has a log");
        assert_eq!(rows.len(), 8, "the fixture's own row count");

        let transcript = Transcript::from_rows(&rows);
        assert_eq!(transcript.len(), 7);
        assert!(matches!(
            transcript.rows(),
            [
                TranscriptRow::Prompt { .. },
                TranscriptRow::Assistant { .. },
                TranscriptRow::Thought { .. },
                TranscriptRow::ToolCall {
                    status: CallStatus::Completed,
                    ..
                },
                TranscriptRow::Plan { .. },
                TranscriptRow::Usage { .. },
                TranscriptRow::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        ));
    }

    /// The decoder stamps one grouping key per persisted row, so two text rows stay two rows
    /// rather than being glued into one by the transcript's own coalescing.
    #[tokio::test]
    async fn two_recorded_text_rows_stay_two_rows() {
        use chrono::TimeZone as _;
        use htui_core::fixtures::ids;
        use htui_core::model::{EventKind, EventRole, SessionEvent};

        let text = |seq: i32, text: &str| SessionEvent {
            run_step_id: ids::STEP_PLAN,
            seq,
            turn: 0,
            kind: EventKind::AssistantText,
            role: EventRole::Agent,
            tool_call_id: None,
            payload: json!({ "text": text }),
            raw: None,
            at: Utc.timestamp_opt(0, 0).single().expect("epoch is a time"),
        };
        let transcript = Transcript::from_rows(&[text(0, "first"), text(1, "second")]);
        assert_eq!(transcript.len(), 2, "one persisted row, one rendered row");
    }

    #[test]
    fn the_view_follows_the_tail_until_it_is_scrolled() {
        let mut transcript = Transcript::new();
        for n in 0..10 {
            transcript.apply(&envelope(DriverEvent::Done(DoneEvent {
                stop_reason: if n % 2 == 0 {
                    StopReason::EndTurn
                } else {
                    StopReason::Cancelled
                },
            })));
        }
        let theme = Theme::default();
        let tail = transcript.lines(3, &theme);
        assert_eq!(tail.len(), 3);
        assert!(
            tail[2].to_string().contains("cancelled"),
            "the newest line is visible: {:?}",
            tail[2].to_string()
        );

        transcript.on_key(KeyEvent::from(KeyCode::Char('g')));
        let top = transcript.lines(3, &theme);
        assert!(
            top[0].to_string().contains("end_turn"),
            "`g` goes to the first line: {:?}",
            top[0].to_string()
        );
    }
}

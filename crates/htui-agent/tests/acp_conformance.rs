//! The ACP transport against the shared conformance suite (`docs/ANA-4.md` §11 criterion 1).
//!
//! Second binding of one `CASES` list, and it **adds no case**: everything transport-specific
//! lives in the harness below, which turns a [`Script`] into wire traffic.
//!
//! The agent side of the duplex speaks **raw newline-delimited JSON-RPC** and imports no SDK type.
//! That is the point: if both ends used the same library, the suite would prove the library is
//! self-consistent rather than that `htui` speaks the protocol. What it exercises instead is the
//! real client — handshake, `session/new`, `session/set_config_option`, `session/update` decoding,
//! `session/request_permission` parking, `session/cancel`, and the `session/prompt` response.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use htui_agent::acp::{AcpDriver, AcpIo, Stamp};
use htui_agent::conformance::{self, CaseHarness, Script, ScriptEvent};
use htui_agent::driver::AgentDriver;
use htui_agent::event::{DriverEvent, PermissionRequestEvent};
use htui_agent::registry::caps_for;
use htui_core::model::Agent;
use htui_core::store::MemStore;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The buffer each half of the in-process pipe gets. Larger than the 16 KiB flush bound so a
/// `chunk_flush_at_16kib` script never deadlocks on a full pipe.
const DUPLEX_BYTES: usize = 256 * 1024;

/// The `claude` seed row, which is the row every case runs against.
fn claude_row() -> Agent {
    htui_core::model::agent::seed_rows(conformance::epoch())
        .into_iter()
        .find(|agent| agent.name == "claude")
        .expect("the seed rows carry `claude`")
}

/// Builds an [`AcpDriver`] over a duplex whose far end is a scripted agent.
#[derive(Debug)]
struct AcpHarness {
    row: Agent,
    sessions: Arc<AtomicU64>,
}

impl AcpHarness {
    fn new() -> Self {
        Self {
            row: claude_row(),
            sessions: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl CaseHarness for AcpHarness {
    fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
        let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
        let (reader, writer) = tokio::io::split(client_end);
        let n = self.sessions.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(scripted_agent(
            agent_end,
            script,
            format!("acp-session-{n}"),
        ));
        Box::new(AcpDriver::over(
            AcpIo {
                reader: Box::new(reader),
                writer: Box::new(writer),
                child: None,
            },
            &self.row,
            caps_for(&self.row),
            // The suite compares `at` byte for byte across two replays (criterion 2), so the
            // transport's clock is the harness's, not the wall's.
            Stamp::Fixed {
                epoch: conformance::epoch(),
            },
        ))
    }
}

#[tokio::test]
async fn the_case_list_is_the_shared_one() {
    assert_eq!(
        conformance::CASES.len(),
        13,
        "adding a transport must add no case (`docs/ANA-4.md` §11 criterion 1)"
    );
}

#[tokio::test]
async fn the_acp_transport_passes_every_case() {
    conformance::run_all(&AcpHarness::new(), || async { MemStore::demo() }).await;
}

// ---------------------------------------------------------------------------------------------
// The scripted agent: raw JSON-RPC, no SDK
// ---------------------------------------------------------------------------------------------

/// Plays `script` as an ACP agent over `stream`.
///
/// One loop, alternating between writing the current turn's events and reading whatever the client
/// sends. It writes without reading only while it has events to write and nothing is blocking; a
/// parked permission request and an `ExpectCancel` marker are the two things that block, and both
/// are released by a line from the client.
async fn scripted_agent(stream: tokio::io::DuplexStream, script: Script, session_id: String) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();

    let mut turns: VecDeque<Vec<ScriptEvent>> =
        script.turns.into_iter().map(|turn| turn.events).collect();
    let mut current: VecDeque<ScriptEvent> = VecDeque::new();
    // The `session/prompt` request this turn answers, and the cumulative cost the client turns
    // back into deltas (ANA-4 §7: ACP reports a session total).
    let mut prompt_id: Option<Value> = None;
    let mut cost_micros_total: i64 = 0;
    let mut awaiting_permission = false;
    let mut waiting_for_cancel = false;
    let mut next_request_id: i64 = 1;

    loop {
        // Write, while there is something to write and nothing is blocking.
        if !awaiting_permission
            && !waiting_for_cancel
            && prompt_id.is_some()
            && let Some(event) = current.pop_front()
        {
            {
                match event {
                    ScriptEvent::Emit(DriverEvent::Done(done)) => {
                        let id = prompt_id.take().expect("a turn answers its prompt");
                        let reason = serde_json::to_value(done.stop_reason)
                            .expect("a stop reason serialises");
                        send(
                            &mut writer,
                            &json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": reason } }),
                        )
                        .await;
                    }
                    ScriptEvent::Emit(event) => {
                        let update = wire_update(&event, &mut cost_micros_total);
                        send(
                            &mut writer,
                            &json!({
                                "jsonrpc": "2.0",
                                "method": "session/update",
                                "params": { "sessionId": session_id, "update": update },
                            }),
                        )
                        .await;
                    }
                    ScriptEvent::ParkPermission(request) => {
                        let id = next_request_id;
                        next_request_id += 1;
                        send(&mut writer, &permission_request(id, &session_id, &request)).await;
                        awaiting_permission = true;
                    }
                    // The turn has no `done` of its own: only a cancel ends it.
                    ScriptEvent::ExpectCancel => waiting_for_cancel = true,
                }
                continue;
            }
        }

        // Otherwise read.
        let Ok(Some(line)) = lines.next_line().await else {
            return;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        // A response to our own permission request releases the turn.
        if message.get("method").is_none() {
            awaiting_permission = false;
            continue;
        }

        let method = message["method"].as_str().unwrap_or_default();
        let id = message.get("id").cloned();
        match method {
            "initialize" => {
                send(
                    &mut writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentCapabilities": { "loadSession": false },
                            "agentInfo": { "name": "scripted", "version": "0.0.0-scripted" },
                            "authMethods": [],
                        },
                    }),
                )
                .await;
            }
            "session/new" => {
                send(
                    &mut writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessionId": session_id,
                            "configOptions": [ model_option() ],
                        },
                    }),
                )
                .await;
            }
            // The id path of D23: the client picks the option that lists the model it wants.
            "session/set_config_option" => {
                send(
                    &mut writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "configOptions": [ model_option() ] },
                    }),
                )
                .await;
            }
            "session/prompt" => {
                prompt_id = id;
                current = turns.pop_front().unwrap_or_default().into();
            }
            "session/cancel" => {
                waiting_for_cancel = false;
                awaiting_permission = false;
                current.clear();
                if let Some(id) = prompt_id.take() {
                    send(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "stopReason": "cancelled" },
                        }),
                    )
                    .await;
                }
            }
            _ => {}
        }
    }
}

/// The one config option this agent offers: the model selector of D23, by **id**.
fn model_option() -> Value {
    json!({
        "id": "model",
        "name": "Model",
        "category": "model",
        "type": "select",
        "currentValue": "sonnet",
        "options": [ { "value": "sonnet", "name": "Sonnet" } ],
    })
}

/// A `session/request_permission` request carrying the script's options.
fn permission_request(id: i64, session_id: &str, request: &PermissionRequestEvent) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": {
            "sessionId": session_id,
            "toolCall": { "toolCallId": request.tool_call_id.clone().unwrap_or_default() },
            "options": request
                .options
                .iter()
                .map(|option| json!({
                    "optionId": option.id,
                    "name": option.label,
                    "kind": option.kind.as_str(),
                }))
                .collect::<Vec<_>>(),
        },
    })
}

/// One script event as the `update` object of a `session/update` notification — the inverse of
/// `htui_agent::acp::map`.
///
/// A script event with no ACP shape panics naming itself rather than being silently skipped: a
/// future case that needs one must extend this table, which is the transport's own business and
/// not the suite's.
fn wire_update(event: &DriverEvent, cost_micros_total: &mut i64) -> Value {
    match event {
        DriverEvent::AssistantChunk(chunk) => json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": chunk.text },
            "messageId": chunk.message_id,
        }),
        DriverEvent::ThoughtChunk(chunk) => json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": chunk.text },
            "messageId": chunk.message_id,
        }),
        DriverEvent::ToolCall(call) => json!({
            "sessionUpdate": "tool_call",
            "toolCallId": call.tool_call_id,
            "title": call.title,
            "kind": call.tool_kind.as_str(),
            "status": "pending",
            "rawInput": call.input,
            "locations": call
                .locations
                .iter()
                .map(|location| json!({ "path": location.path, "line": location.line }))
                .collect::<Vec<_>>(),
        }),
        DriverEvent::ToolResult(result) => json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": result.tool_call_id,
            "status": result.status.as_str(),
            "rawOutput": result.output,
        }),
        // ACP carries old/new full text, never a diff (§3): the script's marker travels as the new
        // text and the client synthesizes the unified diff that contains it.
        //
        // A proposal the script marks `accepted` rides a **terminal** update, because that is what
        // an accepted edit is on the wire: the call that made it completed. The client's rule is
        // the same one from the other side — `accepted` stays `null` until the call settles (§4.3),
        // and an agent that has not finished the call has not made the edit.
        DriverEvent::EditProposal(proposal) => {
            let mut update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": proposal.tool_call_id,
                "content": [ {
                    "type": "diff",
                    "path": proposal.path,
                    "oldText": Value::Null,
                    "newText": proposal.diff,
                } ],
            });
            if proposal.accepted == Some(true) {
                update["status"] = json!("completed");
            }
            update
        }
        DriverEvent::Plan(plan) => json!({
            "sessionUpdate": "plan",
            "entries": plan
                .entries
                .iter()
                .map(|entry| json!({
                    "content": entry.content,
                    "status": entry.status.as_str(),
                    "priority": entry.priority.as_str(),
                }))
                .collect::<Vec<_>>(),
        }),
        // The agent reports the session's cumulative cost; the client re-derives the delta (§7).
        DriverEvent::Usage(usage) => {
            *cost_micros_total += usage.cost_micros.unwrap_or_default();
            #[expect(
                clippy::cast_precision_loss,
                reason = "a test's cost fits a f64 exactly; the wire field is a currency amount"
            )]
            let amount = *cost_micros_total as f64 / 1_000_000.0;
            json!({
                "sessionUpdate": "usage_update",
                "used": usage.context_used.unwrap_or_default(),
                "size": usage.context_size.unwrap_or_default(),
                "cost": { "amount": amount, "currency": "USD" },
            })
        }
        DriverEvent::Other(other) => {
            let mut update = json!({ "sessionUpdate": other.update });
            match &other.body {
                Value::Object(fields) => {
                    for (key, value) in fields {
                        update[key] = value.clone();
                    }
                }
                body => update["body"] = body.clone(),
            }
            update
        }
        DriverEvent::PermissionRequest(_) | DriverEvent::Error(_) | DriverEvent::Done(_) => {
            panic!("{event:?} has no `session/update` shape; it is sent by another route")
        }
    }
}

/// Writes one JSON-RPC message as a line.
async fn send(writer: &mut (impl AsyncWriteExt + Unpin), message: &Value) {
    let mut line = serde_json::to_string(message).expect("a message serialises");
    line.push('\n');
    if writer.write_all(line.as_bytes()).await.is_err() {
        return;
    }
    let _ = writer.flush().await;
}

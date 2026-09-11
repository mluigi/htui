//! The CLI transport against the shared conformance suite (`docs/ANA-4.md` §11 criterion 1).
//!
//! **Third binding of one `CASES` list, and it adds no case.** That is the whole claim this file
//! exists to make: a transport with no permission channel, no edit-proposal channel, no plans and
//! no mid-turn cost report reaches the same fifteen names as ACP does, because everything
//! transport-specific lives in the harness below, which turns a [`Script`] into wire traffic.
//!
//! The agent side of the pipe writes **raw `claude` stream-json NDJSON** and imports no adapter
//! type — no [`htui_agent::cli::claude::Mapper`], no helper of it, nothing that could agree with
//! the mapper by construction. `acp_conformance.rs:5-10`'s reason, sharpened: over ACP the risk was
//! that both ends would use one SDK and the suite would prove the library self-consistent. Here
//! there is no library at all, so the risk is subtler and worse — a harness that shared one line
//! builder with `src/cli/claude.rs` would prove that `htui` can read back what `htui` wrote. What
//! this file exercises instead is the real supervisor (`crate::cli`): the prompt on stdin, the
//! `system/init` banner, the `stream_event` delta channel and its dedup against the assembled
//! `assistant` envelope, the terminal `result`'s cost and denials, and a cancel that is a closed
//! stdin rather than a notification.
//!
//! **Six capability gates take their right-hand arm here** (plan D80, D91), and this binding is the
//! first thing in the tree to make a *real transport over a real pipe* take them:
//! `DriverCaps { permission_requests: false, edit_proposals: false, plans: false,
//! usage_mid_turn: false, .. }` — §4.3's triple plus D91's fourth — read off the seeded
//! `claude-cli` row by `caps_for`, which is also where the driver gets them. `open_case` asserts
//! the two agree, so a harness that declared a profile its driver does not report would fail its
//! first case rather than silently script the wrong thing.
//!
//! # The duplex buffer
//!
//! [`DUPLEX_BYTES`] is `acp_conformance.rs:27-29`'s number for its reason: `chunk_flush_at_16kib`
//! puts 17 KiB of deltas on the wire, and a script that also sends the assembled message doubles
//! that, so a pipe smaller than the flush bound would deadlock the scripted agent against a client
//! that has not read yet. It is not a tuning knob — it is the bound that makes the case's *script*
//! legal.
//!
//! # What the fixtures said and B.7 did not
//!
//! Blueprint B.7 has the scripted agent echo "the line's `session_id`" back on `system/init`. There
//! is no such key: every one of the fourteen recorded transcripts writes the prompt as
//! `{"type":"user","message":{"role":"user","content":[{"type":"text","text":…}]}}` and nothing
//! else, which is the shape T50 implemented (`cli/mod.rs::stdin_line`). So the scripted agent below
//! cannot learn the id `htui` minted and reports [`CLI_SESSION_ID`] instead — which turns out to be
//! the more useful fixture, because it is what makes `session_banner_is_first_other_row`'s
//! `session_id` assertion mean blueprint H-17 ("the banner carries `htui`'s mint, whatever the CLI
//! echoes") rather than a tautology.

use std::collections::{BTreeMap, VecDeque};

use htui_agent::cli::CliDriver;
use htui_agent::conformance::{self, CaseHarness, Script, ScriptEvent};
use htui_agent::driver::{AgentDriver, DriverCaps};
use htui_agent::event::{DriverEvent, Stamp, StopReason, TextChunk, ToolKind, ToolResultStatus};
use htui_agent::launch::ChildIo;
use htui_agent::registry::caps_for;
use htui_core::model::Agent;
use htui_core::store::MemStore;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

/// The buffer each half of the in-process pipe gets. Larger than the 16 KiB flush bound so a
/// `chunk_flush_at_16kib` script never deadlocks on a full pipe — and larger again because this
/// dialect sends every run **twice**, as deltas and as the assembled message (blueprint H-8).
const DUPLEX_BYTES: usize = 256 * 1024;

/// The session id the scripted agent reports on `system/init`.
///
/// Deliberately **not** the one `htui` minted, and deliberately a constant rather than a counter:
/// the id `htui` minted is a UUIDv7 the harness never sees (see this module's header), and a
/// constant keeps the `raw` column byte-identical across the two replays criterion 2 compares.
const CLI_SESSION_ID: &str = "the-cli-picked-this";

/// The text the CLI puts in a refused call's `tool_result`, from
/// `tests/fixtures/claude_stream_json_policy_denied.jsonl` line 10, shortened to its first
/// sentence.
///
/// Shortened and not paraphrased: the fixture's own prose is what the wire carries, and a harness
/// that invented a denial notice would be inventing the one thing D85 says is measurable.
const DENIAL_MESSAGE: &str = "Permission for this tool use was denied. It requires approval, and \
                              this session has no approval surface.";

/// `system/permission_denied.decision_reason`, verbatim from the same fixture (plan F-12b).
const DENIAL_REASON: &str =
    "no approval surface in this session; permission request denied automatically";

/// The `claude-cli` seed row, which is the row every case runs against.
///
/// The **row**, not a hand-written profile: `caps_for` reads `agent.transport` and the settings
/// block, so the six capability gates below are answered by the thing `DriverFactory` would have
/// answered them with in production. Nothing here reads the row's name to decide anything
/// (`R-AGT-5`) — the name is only how the seed list is searched.
fn cli_row() -> Agent {
    htui_core::model::agent::seed_rows(conformance::epoch())
        .into_iter()
        .find(|agent| agent.name == "claude-cli")
        .expect("the seed rows carry `claude-cli`")
}

/// Builds a [`CliDriver`] over a duplex whose far end is a scripted `claude` process.
#[derive(Debug)]
struct CliHarness {
    row: Agent,
}

impl CliHarness {
    fn new() -> Self {
        Self { row: cli_row() }
    }
}

impl CaseHarness for CliHarness {
    fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
        let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
        let (reader, writer) = tokio::io::split(client_end);
        tokio::spawn(scripted_cli(agent_end, script));
        Box::new(CliDriver::over(
            // `child: None` — there is no process, which is the one thing this harness cannot
            // exercise and says so: a cancel here is a closed stdin and nothing else, because
            // `cli::interrupt` has no `Spawned` to signal. The signal half of the cancel sequence
            // (plan D81 step 2, F-2) is `tests/cli_driver.rs`'s, over a real `/bin/sh` child.
            ChildIo {
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

    /// The row's own profile, which is where the driver above gets it too: the suite asserts the
    /// two agree on every case it opens.
    fn caps(&self) -> DriverCaps {
        caps_for(&self.row)
    }
}

#[tokio::test]
async fn the_case_list_is_the_shared_one() {
    assert_eq!(
        conformance::CASES.len(),
        15,
        "adding a transport must add no case (`docs/ANA-4.md` §11 criterion 1)"
    );
}

/// The profile this binding runs under, asserted before the suite does: §4.3's triple and D91's
/// fourth, all `false`, so every one of T53's six gates takes its right-hand arm.
///
/// A separate case from `the_cli_transport_passes_every_case` because a regression that flipped one
/// of the four to `true` would otherwise show up as fifteen cases failing for reasons that never
/// name the capability.
#[tokio::test]
async fn this_binding_declares_the_four_capabilities_it_has_not() {
    let caps = CliHarness::new().caps();
    assert!(!caps.permission_requests, "§4.3: nothing asks");
    assert!(
        !caps.edit_proposals,
        "§4.3: a write is reported, not proposed"
    );
    assert!(!caps.plans, "§4.3: this dialect has no plan block");
    assert!(
        !caps.usage_mid_turn,
        "D91: cost arrives once, on the terminal `result`"
    );
}

#[tokio::test]
async fn the_cli_transport_passes_every_case() {
    conformance::run_all(&CliHarness::new(), || async { MemStore::demo() }).await;
}

// ---------------------------------------------------------------------------------------------
// The scripted agent: raw stream-json, no adapter type
// ---------------------------------------------------------------------------------------------

/// The agent's stdin, as the scripted agent reads it.
type Stdin = tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>;

/// The agent's stdout, as the scripted agent writes it.
type Stdout = WriteHalf<DuplexStream>;

/// Which content-block vocabulary an open delta run is speaking.
///
/// Two, because the dialect spells the same run two ways: a reply is `text_delta` then a `text`
/// block, a thought is `thinking_delta` then a `thinking` block (§6.2, plan F-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Text,
    Thinking,
}

/// A run of deltas not yet closed by its assembled `assistant` envelope.
#[derive(Debug)]
struct Run {
    message: String,
    block: Block,
    text: String,
}

/// Per-session wire state: everything the scripted agent has to remember across turns.
#[derive(Debug, Default)]
struct Wire {
    /// The session's cumulative spend in USD micros.
    ///
    /// Cumulative, and that is measured rather than chosen (plan F-4): `total_cost_usd` on the
    /// terminal `result` is a session total, not a turn's, and the mapper turns it back into a
    /// delta. A harness that reported each turn's own spend would make the second turn of a
    /// two-turn script look free.
    cost_micros: i64,
    /// `result.num_turns`, which the CLI counts across the process.
    turns: i64,
    /// The open delta run, flushed as an assembled `assistant` envelope when anything else happens.
    open: Option<Run>,
    /// How many synthetic `message.id`s have been minted for envelopes the script gave none.
    messages: u64,
    /// How many `tool_use` ids have been minted for scripted edit proposals.
    edits: u64,
    /// `tool_use_id` → the tool name it was announced under, so a refusal can name the tool the way
    /// `system/permission_denied` does.
    tools: BTreeMap<String, String>,
}

impl Wire {
    /// A fresh `message.id` for an envelope the script gave none.
    ///
    /// Minted from a counter and not from a clock or a UUID: two replays of one script must put
    /// the same bytes on the wire, which is what criterion 2 compares the rows of.
    fn mint_message(&mut self) -> String {
        let id = format!("msg-{}", self.messages);
        self.messages += 1;
        id
    }
}

/// Plays `script` as a `claude -p --output-format stream-json` process over `stream`.
///
/// The shape of a real one, in order: the prompt arrives on stdin **first** (the supervisor writes
/// it before it waits for anything, blueprint P-2), `system/init` answers it, and every later stdin
/// line opens the next turn. Nothing is written before the prompt arrives — F-9 measured hook
/// envelopes arriving ahead of `init` on a real box, and the supervisor's pre-`init` buffer is
/// covered by `tests/cli_driver.rs`; writing one here would put a second `other` row in every case's
/// log and `coalesce_across_message_id` asserts its kinds exactly.
async fn scripted_cli(stream: DuplexStream, script: Script) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut stdin = BufReader::new(reader).lines();
    let mut turns: VecDeque<Vec<ScriptEvent>> =
        script.turns.into_iter().map(|turn| turn.events).collect();
    let mut wire = Wire::default();

    // The prompt. A stream that ends here is a session nobody ever opened.
    if !matches!(stdin.next_line().await, Ok(Some(_))) {
        return;
    }
    if !send(&mut writer, &init_line()).await {
        return;
    }

    loop {
        wire.turns += 1;
        let events = turns.pop_front().unwrap_or_default();
        if !play_turn(&mut writer, &mut stdin, &mut wire, events).await {
            return;
        }
        // The follow-up that opens the next turn, or the stdin close that ends the process (F-1).
        if !matches!(stdin.next_line().await, Ok(Some(_))) {
            return;
        }
    }
}

/// `system/init`: the envelope the supervisor turns into the session banner (D84).
///
/// `claude_code_version` and not `version` — the key the recorded transcripts actually carry, which
/// is the sort of thing only a fixture can settle (`cli/mod.rs::agent_version`).
fn init_line() -> Value {
    json!({
        "type": "system",
        "subtype": "init",
        "cwd": ".",
        "session_id": CLI_SESSION_ID,
        "claude_code_version": "0.0.0-scripted",
        "model": "sonnet",
        "tools": [],
        "permissionMode": "acceptEdits",
    })
}

/// Plays one turn; `false` once the session is over and the process should exit.
///
/// Returns after writing the turn's terminal `result`, which is the only thing on this wire that
/// ends a turn — so a script with no `done` of its own still gets one, rather than hanging a case
/// that ran out of turns.
async fn play_turn(
    writer: &mut Stdout,
    stdin: &mut Stdin,
    wire: &mut Wire,
    events: Vec<ScriptEvent>,
) -> bool {
    // The turn's refusals, repeated on its `result` the way the CLI repeats them (F-12b). The
    // mapper is expected to suppress the repeat by `tool_use_id`, and putting it here is what puts
    // that rule under test on every run rather than only in `cli_map.rs`'s snapshot.
    let mut denials: Vec<Value> = Vec::new();

    for event in events {
        match event {
            ScriptEvent::Emit(DriverEvent::AssistantChunk(chunk)) => {
                if !delta(writer, wire, Block::Text, &chunk).await {
                    return false;
                }
            }
            ScriptEvent::Emit(DriverEvent::ThoughtChunk(chunk)) => {
                if !delta(writer, wire, Block::Thinking, &chunk).await {
                    return false;
                }
            }
            ScriptEvent::Emit(DriverEvent::Done(done)) => {
                if !flush_run(writer, wire).await {
                    return false;
                }
                return send(writer, &result_line(wire, &denials, done.stop_reason)).await;
            }
            ScriptEvent::PolicyDenied(call) => {
                if !flush_run(writer, wire).await || !deny(writer, wire, &call, &mut denials).await
                {
                    return false;
                }
            }
            ScriptEvent::ExpectCancel => {
                if !flush_run(writer, wire).await {
                    return false;
                }
                return stall_until_stdin_closes(writer, stdin, wire).await;
            }
            // Refused **by name**, exactly as the ACP harness refuses a `PolicyDenied` — the
            // mirror image of the same rule (plan D80, blueprint H-22). This transport has no
            // permission channel: `caps.permission_requests` is `false`, so every case that would
            // script a park takes its other arm and never reaches here, and a harness that
            // fabricated a request the stream cannot carry would be proving the harness rather than
            // the transport.
            ScriptEvent::ParkPermission(request) => panic!(
                "script marker `park_permission({})`: this transport has no permission channel and \
                 no shape to carry one; the case that scripted it read `caps.permission_requests` \
                 wrong",
                request.request_id
            ),
            ScriptEvent::Emit(event) => {
                if !flush_run(writer, wire).await {
                    return false;
                }
                for line in wire_lines(wire, &event) {
                    if !send(writer, &line).await {
                        return false;
                    }
                }
            }
        }
    }

    // A turn whose script never said `done` — an empty turn, from a case that sent one follow-up
    // more than it scripted — still ends, because a turn that did not end is a suite that hangs.
    if !flush_run(writer, wire).await {
        return false;
    }
    send(writer, &result_line(wire, &denials, StopReason::EndTurn)).await
}

/// [`ScriptEvent::ExpectCancel`]: the turn has no ending of its own, so it waits for one.
///
/// **What "cancel" means on this pipe.** `cli::cancel_session`'s first step is to close stdin (plan
/// D81 step 1), and its second is a SIGINT to the child's process group — which there is not one
/// of here, so the close is the whole of the signal this harness can observe. The answer it writes
/// is F-2's measured shape: a real terminal `result`, error-dressed, carrying
/// `terminal_reason: "aborted_streaming"` and **no usage at all**, because a cancelled turn reports
/// `total_cost_usd: 0` and an empty `modelUsage` and the mapper writes no `usage` row for one.
///
/// The suite cancels with a zero grace, so the supervisor never drains and this envelope never
/// reaches the mapper — the `done { cancelled }` the case reads is the synthesized one. It is
/// written anyway rather than left off: a harness whose scripted agent went silent would be
/// describing a process that was killed (F-3, SIGTERM, exit 143), and this one was interrupted.
async fn stall_until_stdin_closes(writer: &mut Stdout, stdin: &mut Stdin, wire: &Wire) -> bool {
    while matches!(stdin.next_line().await, Ok(Some(_))) {}
    send(
        writer,
        &json!({
            "type": "result",
            "subtype": "error_during_execution",
            "is_error": true,
            "terminal_reason": "aborted_streaming",
            "total_cost_usd": 0,
            "modelUsage": {},
            "permission_denials": [],
            "num_turns": wire.turns,
            "duration_ms": 0,
            "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"],
        }),
    )
    .await;
    false
}

/// One chunk of a reply or a thought, as **both** halves of the dialect's double report.
///
/// `--include-partial-messages` sends a run twice: once as `stream_event` deltas while it is being
/// written, and again inside the assembled `assistant` envelope when it is done. The mapper drops
/// the second (blueprint H-8), and this harness sends both on **every** run so that rule is
/// exercised by every case rather than by a snapshot — a mapper that stopped deduping would double
/// every reply in the log and `coalesce_across_message_id` would say so.
///
/// The delta's `index` is `0` and the assembled envelope carries one block, so the mapper's
/// coalescing key `(message.id, content-block index)` (F-7) is `<id>#0` on both sides. That the key
/// is composite and not `message.id` alone is what keeps a thought and a reply sharing one message
/// from folding into each other; nothing here has to know that, which is the point — the harness
/// writes the wire and the mapper decides what it means.
async fn delta(writer: &mut Stdout, wire: &mut Wire, block: Block, chunk: &TextChunk) -> bool {
    let message = match chunk.message_id.clone() {
        Some(id) => id,
        None => wire.mint_message(),
    };
    let continues = wire
        .open
        .as_ref()
        .is_some_and(|run| run.message == message && run.block == block);
    if !continues {
        if !flush_run(writer, wire).await {
            return false;
        }
        if !send(
            writer,
            &json!({
                "type": "stream_event",
                "event": {
                    "type": "message_start",
                    "message": {
                        "id": message,
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                    },
                },
            }),
        )
        .await
        {
            return false;
        }
        wire.open = Some(Run {
            message,
            block,
            text: String::new(),
        });
    }
    if let Some(run) = wire.open.as_mut() {
        run.text.push_str(&chunk.text);
    }
    let delta = match block {
        Block::Text => json!({ "type": "text_delta", "text": chunk.text }),
        Block::Thinking => json!({ "type": "thinking_delta", "thinking": chunk.text }),
    };
    send(
        writer,
        &json!({
            "type": "stream_event",
            "event": { "type": "content_block_delta", "index": 0, "delta": delta },
        }),
    )
    .await
}

/// Closes the open delta run with its assembled `assistant` envelope — the half the mapper must
/// drop.
///
/// A no-op when nothing is open, which is what makes it safe to call in front of every other line:
/// a real `assistant` envelope arrives when its message is complete, so anything that is not part
/// of the run has to come after it.
async fn flush_run(writer: &mut Stdout, wire: &mut Wire) -> bool {
    let Some(run) = wire.open.take() else {
        return true;
    };
    let block = match run.block {
        Block::Text => json!({ "type": "text", "text": run.text }),
        // The signature beside a thinking block is a cryptographic block signature and not the
        // thought (F-6); it is written empty here because inventing one would put an opaque
        // kilobyte in a fixture that is about coalescing.
        Block::Thinking => json!({ "type": "thinking", "thinking": run.text, "signature": "" }),
    };
    send(
        writer,
        &json!({
            "type": "assistant",
            "message": {
                "id": run.message,
                "type": "message",
                "role": "assistant",
                "content": [block],
            },
        }),
    )
    .await
}

/// [`ScriptEvent::PolicyDenied`]: the refusal, **twice**, which is what the wire does.
///
/// Plan F-12b measured both sources and this harness writes both: the live
/// `system/permission_denied` envelope at the moment it happens, and the same `tool_use_id` again
/// in the terminal `result`'s `permission_denials[]`. One refusal is one row, so the mapper is
/// expected to take the live one and suppress the repeat — a rule that is only under test if the
/// repeat is actually sent, which is why it is.
///
/// Then the `user`/`tool_result` the CLI sends the model so it knows the call did not run. That is
/// what settles the call on this transport: there is no permission channel to answer and nothing to
/// synthesize, so the refused call is closed by an `is_error` result like any other failure — and
/// the script's own later result for the same call is dropped by the supervisor
/// (`cli::emit`'s `settled_calls`), which is §4.3's "one result per call" surviving a change of who
/// refused.
async fn deny(writer: &mut Stdout, wire: &Wire, call: &str, denials: &mut Vec<Value>) -> bool {
    let tool = wire
        .tools
        .get(call)
        .cloned()
        .unwrap_or_else(|| "Other".to_owned());
    if !send(
        writer,
        &json!({
            "type": "system",
            "subtype": "permission_denied",
            "tool_name": tool,
            "tool_use_id": call,
            "decision_reason_type": "asyncAgent",
            "decision_reason": DENIAL_REASON,
            "message": DENIAL_MESSAGE,
        }),
    )
    .await
    {
        return false;
    }
    denials.push(json!({ "tool_name": tool, "tool_use_id": call, "tool_input": {} }));
    send(writer, &tool_result_line(call, DENIAL_MESSAGE, true)).await
}

/// The turn's terminal `result`: its cost, its denials and its end, in one envelope.
///
/// **No `modelUsage`.** The five token figures are `null` over this harness on purpose: the script
/// language has no token counts to report (`usage_deltas_sum_to_step_usage` says why it must not),
/// and an invented per-model breakdown would be a number the case would then have to believe.
/// `total_cost_usd` is written **only when something was spent**, and that is not tidiness: an
/// absent cost and an absent `modelUsage` are an absence of measurement, and a `result` that
/// measured nothing must not produce a `usage` row — `coalesce_across_message_id` asserts its
/// kinds exactly, and a row of five nulls and no cost would be a row that says nothing sitting
/// between the reply and the `done`.
///
/// `subtype` is a label and not a verdict (F-8): the outcome is `is_error` plus `terminal_reason`,
/// which is what the mapper reads and therefore what this has to get right.
fn result_line(wire: &Wire, denials: &[Value], stop: StopReason) -> Value {
    let (subtype, terminal, is_error) = match stop {
        StopReason::EndTurn => ("success", "completed", false),
        StopReason::Cancelled => ("error_during_execution", "aborted_streaming", true),
        StopReason::MaxTurnRequests => ("error_max_turns", "completed", false),
        // `max_tokens` and `refusal` are ACP `stopReason`s with no `terminal_reason` behind them in
        // any recorded transcript, so there is no shape to write and a guess would be a fixture
        // nobody measured (blueprint H-22). No case scripts one.
        other => panic!(
            "script `done({})`: this dialect has no terminal `result` shape for it; the fixtures \
             recorded `completed`, `aborted_streaming`, `api_error` and `budget_exhausted`",
            other.as_str()
        ),
    };
    let mut line = json!({
        "type": "result",
        "subtype": subtype,
        "is_error": is_error,
        "terminal_reason": terminal,
        "num_turns": wire.turns,
        "duration_ms": 0,
        "permission_denials": denials,
    });
    if wire.cost_micros > 0 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a suite's spend is three digits of micros, which an f64 carries exactly; the \
                      wire field is whole US dollars"
        )]
        let usd = wire.cost_micros as f64 / 1_000_000.0;
        line["total_cost_usd"] = json!(usd);
    }
    line
}

/// One script event as the stream-json lines that carry it — the inverse of
/// `htui_agent::cli::claude`, written from the fixtures rather than from that module.
///
/// A script event with no shape in this dialect panics **naming itself** rather than being silently
/// skipped, the rule `acp_conformance.rs:298-303` states: a future case that needs one extends this
/// table, which is the transport's own business and not the suite's.
fn wire_lines(wire: &mut Wire, event: &DriverEvent) -> Vec<Value> {
    match event {
        // `tool_use` rides an `assistant` envelope of its own, under a minted `message.id`: the
        // tool name is derived from the script's `ToolKind` so that `claude::tool_kind` maps it
        // back to the kind the case asserts on, and the script's human `title` has no place on
        // this wire — the dialect carries a tool *name*, and the mapper's `title` is that name.
        DriverEvent::ToolCall(call) => {
            let name = tool_name(call.tool_kind);
            wire.tools
                .insert(call.tool_call_id.clone(), name.to_owned());
            let id = wire.mint_message();
            vec![json!({
                "type": "assistant",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": call.tool_call_id,
                        "name": name,
                        "input": call.input,
                    }],
                },
            })]
        }
        DriverEvent::ToolResult(result) => vec![tool_result_line(
            &result.tool_call_id,
            &output_text(result.output.as_ref()),
            result.status == ToolResultStatus::Failed,
        )],
        // §4.3 (`:545-546`): a transport with no proposal channel "surfaces them only as post-hoc
        // `Edit`/`Write` tool calls". So a scripted proposal becomes a call that already happened,
        // with its own id — `<call>#<n>`, because three proposals under one `tool_call_id` are
        // three writes and a dialect that reused the id would be reporting one call three times —
        // followed by the result that closed it. **No diff on the wire, ever**: the dialect carries
        // the new text as a tool input, and the case's assertion is on `locations[0].path`.
        DriverEvent::EditProposal(proposal) => {
            let call = format!(
                "{}#{}",
                proposal.tool_call_id.clone().unwrap_or_default(),
                wire.edits
            );
            wire.edits += 1;
            wire.tools.insert(call.clone(), "Edit".to_owned());
            let id = wire.mint_message();
            vec![
                json!({
                    "type": "assistant",
                    "message": {
                        "id": id,
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": call,
                            "name": "Edit",
                            "input": {
                                "file_path": proposal.path,
                                "old_string": "",
                                "new_string": proposal.diff,
                            },
                        }],
                    },
                }),
                tool_result_line(&call, "ok", false),
            ]
        }
        // **Folded, not written** (plan D91). Cost exists on the terminal `result` and nowhere else
        // on this wire, so a scripted mid-turn report contributes its micros to the session total
        // and puts no line out. Its vendor blob does have a mid-turn shape — `rate_limit_event` —
        // and travels now, where the mapper holds it for the turn's single `usage` row.
        DriverEvent::Usage(usage) => {
            wire.cost_micros += usage.cost_micros.unwrap_or_default();
            usage
                .quota
                .iter()
                .map(|blob| json!({ "type": "rate_limit_event", "rate_limit_info": blob }))
                .collect()
        }
        // §6.2's wildcard from the other side: an envelope kind this build does not recognize is
        // its `type` plus its body's keys, which is exactly what `body_of` takes back apart.
        DriverEvent::Other(other) => {
            let mut line = json!({ "type": other.update });
            match &other.body {
                Value::Object(fields) => {
                    for (key, value) in fields {
                        line[key] = value.clone();
                    }
                }
                body => line["body"] = body.clone(),
            }
            vec![line]
        }
        // The four this dialect cannot say. `PermissionRequest` and `Plan` are §4.3's declared
        // gaps — `caps.permission_requests` and `caps.plans` are `false`, so no case reaches here —
        // `PermissionAnswer` is reported through `system/permission_denied` and has no other route,
        // and `Error` and `Done` ride the terminal `result` rather than a line of their own.
        DriverEvent::PermissionRequest(_)
        | DriverEvent::PermissionAnswer(_)
        | DriverEvent::Plan(_)
        | DriverEvent::Error(_)
        | DriverEvent::Done(_)
        | DriverEvent::AssistantChunk(_)
        | DriverEvent::ThoughtChunk(_) => {
            panic!("{event:?} has no stream-json line of its own; it is sent by another route")
        }
    }
}

/// A `user` envelope carrying one `tool_result` block — how this dialect closes a call.
fn tool_result_line(call: &str, content: &str, is_error: bool) -> Value {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": call,
                "content": content,
                "is_error": is_error,
            }],
        },
    })
}

/// A script's structured tool output as the text the wire carries.
///
/// The dialect's `tool_result.content` is prose or a list of text blocks, never a bare object, so a
/// structured output is serialised rather than embedded: it is what a real tool's stdout would have
/// been, and it keeps every byte the scrubber has to find (`env_values_masked_in_rows`,
/// `scrub_residue_refuses_write` both hide their needle in here).
fn output_text(output: Option<&Value>) -> String {
    match output {
        None => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(value) => value.to_string(),
    }
}

/// A [`ToolKind`] as the tool **name** this dialect would have carried.
///
/// The inverse of `claude::tool_kind`'s table, and the round trip is the assertion: a script that
/// says `edit` has to reach the log as `edit`, or `edit_proposal_deduped_per_call_and_path`'s
/// second arm is checking the harness's opinion of its own name. A kind this dialect has no tool
/// for panics by name rather than being mapped onto a plausible neighbour — `Delete` is not `Bash`,
/// and a fixture that said so would be a fixture nobody measured.
fn tool_name(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "Read",
        ToolKind::Edit => "Edit",
        ToolKind::Execute => "Bash",
        ToolKind::Search => "Glob",
        ToolKind::Fetch => "WebFetch",
        // Nothing in the table maps to it, which is exactly what `Other` means.
        ToolKind::Other => "Task",
        other => panic!(
            "tool kind `{}` has no tool in this dialect's vocabulary (§6.2); the script that asked \
             for one is asserting about a transport that cannot produce it",
            other.as_str()
        ),
    }
}

/// Writes one envelope as an NDJSON line; `false` once the reader is gone.
async fn send(writer: &mut Stdout, line: &Value) -> bool {
    let mut text = serde_json::to_string(line).expect("an envelope serialises");
    text.push('\n');
    if writer.write_all(text.as_bytes()).await.is_err() {
        return false;
    }
    writer.flush().await.is_ok()
}

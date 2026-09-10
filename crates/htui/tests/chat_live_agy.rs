//! One real `agy` conversation through the production seam, and the three §11.14 answers it was
//! built to collect (`docs/ANA-4.md` §11.14, plan MOD-2 `T34`, blueprint B.8 and milestone 6's
//! G-T34).
//!
//! `#[ignore]` by default, for two reasons at once: it spawns the ~1.9 GB `agy_acp_server.par`
//! this box has installed, and it spends real tokens of the maintainer's own subscription
//! (`docs/decisions/mod/mod-21.md` — the login that unblocked this file). Run it by hand:
//!
//! ```text
//! HTUI_KEEP_RAW_EVENTS=1 HTUI_AGY_FIXTURE_OUT=/tmp/agy_acp_turn.jsonl \
//!   cargo test -p htui --features testkit --test chat_live_agy -- --ignored --nocapture
//! ```
//!
//! `testkit` is `htui`'s only test feature; `HTUI_KEEP_RAW_EVENTS` has to arrive on the command
//! line because `SessionSpec.retain_raw` is read from the process environment
//! (`agent_worker::KEEP_RAW_ENV`) and `set_var` is forbidden in this workspace (milestone 6 H-12).
//! `HTUI_AGY_FIXTURE_OUT` is **read, never written**: when it names a path the recorded
//! `session/update` raws are written there, one JSON object per line, in
//! `crates/htui-agent/tests/fixtures/claude_acp_turn.jsonl`'s shape, ready to be redacted by hand
//! and committed (blueprint H-13).
//!
//! ## What it proves, and what it merely observes
//!
//! The **assertions** are `tests/chat_live.rs`'s structural floor, transposed: the seeded `agy`
//! row, a real probe, the production [`AgentRuntime`], the ACP transport, the recorder and the
//! chat seam turn three typed prompts into three live turns whose rows land in a store — a
//! `prompt` row at `seq 0`, some `assistant_text`, a `done`, the session banner as the first
//! `other` row at protocol 1 — and then end the session and leave no `agy_acp_server` behind.
//!
//! The **observations** are the point of the file. ANA-4 §11.14 lists three questions this repo
//! could only answer from the wire, and §7's `agy` row is marked "Unverified — MOD-2 must
//! confirm". A live probe answers them by printing enough of every frame to read the answer off
//! the output, and "it emits nothing" is a valid answer rather than a failure (the
//! `tests/agy_live.rs` case-2 precedent). The three, and where each is answered:
//!
//! | § | Question | Answered by |
//! |---|---|---|
//! | `:1384` | does `agy_acp_server` emit `usage_update` at all, under what field, with `cost` or context occupancy only? | every `DriverEvent::Usage` frame is printed as JSON, and [`Observed::usage`] counts them; the raw `session/update` behind each is in the fixture |
//! | `:1385-1386` | does it issue `session/request_permission` in `default` mode, with what option ids and `PermissionOptionKind` values? | every `DriverEvent::PermissionRequest` prints `options[].{id, kind}` **before** it is answered; the seed's `permission.default` is `ask`, so nothing is auto-answered and every request reaches this test |
//! | `:1387-1388` | do its edits arrive as a standard `tool_call` with `kind: "edit"` and a `diff`, or in a vendor shape landing in `other`? | turn 3 asks for a file write; [`Observed`] records whether an `EditProposal` (with its diff), a `ToolCall { tool_kind: Edit }`, or neither arrived, and prints every `other` row's update name so a vendor shape is visible by name |
//!
//! Nothing about the adapter's behaviour is asserted. Per plan D62 the mapper is **not** amended
//! from here: a shape it cannot read is a finding for the maintainer (blueprint H-15), not a quiet
//! change.
//!
//! ## Why it probes first
//!
//! Only a written `agent_box` row makes `AgentRuntime::start` → `driver_for(agent, Some(on_box))`
//! → `launch_for` spawn the **recorded** launch, which on Linux is the only one carrying the
//! `--uid=` the server needs (milestone 6 E-1, H-3). Without it the chat would resolve through
//! `tools::resolve` and launch without the flag. So the test runs the production probe over the
//! unmodified seed row, writes what it decided, and refuses to go on unless the row says `ready` —
//! `htui` cannot log this box in from here (`docs/ANA-4.md` §4.5, plan D63).
//!
//! With `tests/agy_live.rs` and `tests/probe_live.rs` this is one of the few files allowed to
//! probe an **unmodified seed row** (milestone 6 H-18); every other suite uses a registry whose
//! tools cannot resolve, so a plain `cargo test` never starts a real adapter.
//!
//! ## The working directory
//!
//! `ChatStart` carries no directory: `AgentRuntime::start` sets `SessionSpec.cwd` from
//! `std::env::current_dir()` (`agent_worker.rs:1081`). So the only way to bound turn 3's file
//! write is to move this **process** into a `tempfile::tempdir()` — `std::env::set_current_dir`,
//! which is a safe function, so `unsafe_code = "forbid"` is untouched (blueprint P-7). It is
//! process-wide, which is acceptable for exactly one reason: this file is a single `#[ignore]`
//! test in its own binary, nothing else runs beside it, and [`RestoreCwd`] puts the directory back
//! on every exit path including a panic (blueprint H-14). The tempdir then removes turn 3's file
//! for us, so milestone 6's H-10 manual cleanup is gone.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use htui::agent_worker::{AgentRuntime, Served};
use htui::store_worker::{
    ChatFrame, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest,
};
use htui::ui::tabs::ChatTab;
use htui_agent::driver::PermissionAnswer;
use htui_agent::event::{DriverEvent, PermissionOptionKind, ToolKind};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeStatus, SpawnTier2, probe_agent,
};
use htui_agent::registry::caps_for;
use htui_core::fixtures::ids;
use htui_core::model::EventKind;
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use htui_store::Backend;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Where the recorded `session/update` raws go, when the maintainer names a path.
///
/// Read with `std::env::var` and never written: the fixture path is an input to this test, and
/// `set_var` is forbidden in this workspace anyway.
const FIXTURE_OUT_ENV: &str = "HTUI_AGY_FIXTURE_OUT";

/// How long the whole three-turn conversation may take.
///
/// Generous rather than tuned. The handshake alone was ~1.2 s in milestone 6, but each turn here
/// is a real model round trip and turn 3 asks for a tool call, so a shorter deadline would report
/// a busy service as a broken seam. `tests/chat_live.rs` gives one `claude` turn 120 s; this is
/// three `agy` turns and the same order of magnitude per turn.
const CONVERSATION_TIMEOUT: Duration = Duration::from_secs(420);

/// The three prompts, in order, and what each one is for.
///
/// Turn 3 is the permission-worthy one (blueprint B.8 amendment 2): a file write in the working
/// directory is the request `default` mode should gate, and the whole of §11.14's second question
/// is what the adapter does with it. Turn 2 is a **read**, which is the other half of the same
/// question — a mode that gates writes may or may not gate reads.
const TURNS: [&str; 3] = [
    "Reply with exactly the word ok, nothing else. Do not use any tools.",
    "Read the file README.md in the working directory and reply with its first heading, \
     using a tool.",
    "Create a file named htui-agy-probe.txt in the working directory containing the word ok.",
];

/// What turn 2 reads. The scratch directory is empty, so the test puts it there itself rather than
/// asking the agent to read a file that does not exist.
const SCRATCH_README: &str = "# htui agy probe\n\nA scratch tree for the MOD-2 T34 live probe.\n";

/// The file turn 3 is asked to create.
const PROBE_FILE: &str = "htui-agy-probe.txt";

/// Puts the process's working directory back, whatever happens (blueprint H-14).
///
/// A `Drop` rather than a line at the end of the test: a panic mid-turn — a failed assertion, a
/// deadline — must not leave the rest of this process, and any `cargo test` reporting after it,
/// standing in a directory that is about to be deleted.
struct RestoreCwd(PathBuf);

impl Drop for RestoreCwd {
    fn drop(&mut self) {
        if let Err(err) = std::env::set_current_dir(&self.0) {
            eprintln!(
                "the working directory could not be restored to {}: {err}",
                self.0.display()
            );
        }
    }
}

/// Everything the run *observed*, as opposed to everything it asserted.
///
/// Printed as one block at the end so the three §11.14 answers can be read off the output without
/// scrolling through the frame log, and so an answer of "nothing arrived" is stated in words
/// rather than inferred from silence.
#[derive(Debug, Default)]
struct Observed {
    /// Every `usage` frame, as the JSON the recorder would persist.
    usage: Vec<Value>,
    /// Every permission request, as `{ request_id, tool_call_id, options: [{id, kind}] }`.
    permissions: Vec<Value>,
    /// Every `tool_call` frame, as `{ tool_call_id, tool_kind, title, input }`.
    tool_calls: Vec<Value>,
    /// Every `edit_proposal` frame, as `{ tool_call_id, path, diff_lines, accepted }`.
    edits: Vec<Value>,
    /// The `update` name of every `other` frame, in order and with duplicates: a vendor shape the
    /// mapper cannot classify is visible here by name (§6.1's wildcard row).
    others: Vec<String>,
    /// Every `error` frame, which is where a refused path or a failed write would land.
    errors: Vec<Value>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "spawns the 1.9 GB agy ACP adapter installed on this box and spends subscription tokens"]
async fn a_real_agy_session_streams_three_turns_into_the_store_and_answers_11_14() {
    // ---------------------------------------------------------------------------------------
    // The scratch tree, and the guard that gives the directory back (blueprint P-7, H-14).
    // ---------------------------------------------------------------------------------------
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let home = std::env::current_dir().expect("this process has a working directory");
    std::fs::write(scratch.path().join("README.md"), SCRATCH_README)
        .expect("turn 2's file is written");
    std::env::set_current_dir(scratch.path()).expect("the process moves into the scratch tree");
    // Declared **after** `scratch` so it drops **before** it: the directory is restored first and
    // removed second, never the other way round.
    let _cwd = RestoreCwd(home);
    println!("scratch tree: {}", scratch.path().display());

    // Processes matching before anything spawns, so the final assertion is about **new**
    // survivors: `pgrep -f` matches any command line holding the text, including the shell that
    // ran the test, so an absolute count would fail for reasons unrelated to this code.
    let before = matching_processes();

    // ---------------------------------------------------------------------------------------
    // The probe, and the row that makes the chat spawn the recorded launch (milestone 6 E-1).
    // ---------------------------------------------------------------------------------------
    let store = MemStore::demo();
    // The fixture's `agy` row **is** the seed row, re-stamped (`htui_core::fixtures`), so this
    // launches exactly what a real database would.
    let agent = store
        .agents()
        .await
        .expect("the registry reads")
        .into_iter()
        .find(|summary| summary.agent.name == "agy")
        .expect("the fixture carries `agy`")
        .agent;
    let agent_id = agent.id;

    let ctx = ProbeContext {
        env: ProbeEnv::host(scratch.path().to_path_buf()),
        now: Utc::now(),
    };
    let started = std::time::Instant::now();
    let row = match probe_agent(&agent, ids::BOX, None, &ctx, &SpawnTier2::default()).await {
        ProbeOutcome::Row(row) => row,
        ProbeOutcome::Kept { reason } => panic!("the probe wrote nothing: {reason}"),
    };
    println!("probe_agent took {:?}", started.elapsed());
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    println!(
        "agent_box.probe = {}",
        serde_json::to_string_pretty(&probe).expect("the snapshot re-serialises")
    );
    let status = probe["status"].as_str().unwrap_or("<none>");
    assert_eq!(
        status,
        ProbeStatus::Ready.as_str(),
        "precondition: this box's `agy` row probes `{status}`, not `ready` (credential tier: {}). \
         Run `agy_acp_server`'s own login first — `htui` cannot do it from here (ANA-4 §4.5, plan \
         D63): open Settings > Agents, put the cursor on `agy` and press `a`.",
        probe["credential"].as_str().unwrap_or("<none>")
    );
    store
        .upsert_agent_box(&row)
        .await
        .expect("the probed row lands");

    // ---------------------------------------------------------------------------------------
    // Turn 1: the production runtime, exactly as `tests/chat_live.rs` starts it.
    // ---------------------------------------------------------------------------------------
    let backend = Backend::memory(store.clone());
    let mut runtime = AgentRuntime::production().with_grace(Duration::from_secs(1));
    let (tx, mut rx) = mpsc::unbounded_channel::<ReplyEnvelope>();
    let mut seq: u64 = 1;

    let start = RequestEnvelope {
        seq,
        origin: Origin::Tab(ChatTab::ID),
        request: StoreRequest::ChatStart {
            project_id: ids::PROJECT_HTUI,
            agent_id,
            model: None,
            prompt: TURNS[0].to_owned(),
        },
    };
    let Served::Start { step_id, task } = runtime.serve(&backend, &tx, &start).await else {
        panic!("a chat start opens a session")
    };
    let session = tokio::spawn(task);
    runtime.attach(step_id, tokio::spawn(async {}));

    // ---------------------------------------------------------------------------------------
    // The stream: print every frame, answer every permission request, send the next turn on each
    // `done`, and end the session the way `Esc Esc` does after the last one.
    // ---------------------------------------------------------------------------------------
    let mut observed = Observed::default();
    let mut turn = 0_usize;
    let mut ended = false;
    let deadline = tokio::time::Instant::now() + CONVERSATION_TIMEOUT;
    while let Ok(Some(envelope)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match envelope.reply {
            StoreReply::ChatAccepted {
                session_ref,
                caps,
                writer_label,
                ..
            } => {
                println!("accepted: session {session_ref:?} into `{writer_label}`");
                // Milestone 6's criterion 11: an `agy` chat differs from a `claude` one by the
                // registry row and the capability banner, and the banner is the same because both
                // rows are `acp` with `session.resume` on.
                assert_eq!(
                    caps,
                    caps_for(&agent),
                    "the accepted caps are the row's own profile"
                );
            }
            StoreReply::Chat(ChatFrame::Event(frame)) => {
                report(&frame.event, &mut observed);
                match &frame.event {
                    DriverEvent::PermissionRequest(request) => {
                        // Blueprint B.8 amendment 2: **print every option first**, then answer
                        // with the first `AllowOnce`. The print is §11.14's answer; the answer is
                        // what lets turn 3 finish.
                        let chosen = choose(&request.options);
                        println!(
                            "  answering `{}` with option `{chosen}`",
                            request.request_id
                        );
                        seq += 1;
                        let answer = RequestEnvelope {
                            seq,
                            origin: Origin::Tab(ChatTab::ID),
                            request: StoreRequest::ChatAnswer {
                                step_id,
                                request_id: request.request_id.clone(),
                                answer: PermissionAnswer::Selected(chosen),
                            },
                        };
                        runtime.serve(&backend, &tx, &answer).await;
                    }
                    DriverEvent::Done(done) => {
                        turn += 1;
                        println!("--- turn {turn} closed: {}", done.stop_reason.as_str());
                        seq += 1;
                        let request = match TURNS.get(turn) {
                            Some(text) => {
                                println!("--- turn {} prompt: {text}", turn + 1);
                                StoreRequest::ChatSend {
                                    step_id,
                                    text: (*text).to_owned(),
                                }
                            }
                            None => StoreRequest::ChatCancel { step_id },
                        };
                        let next = RequestEnvelope {
                            seq,
                            origin: Origin::Tab(ChatTab::ID),
                            request,
                        };
                        runtime.serve(&backend, &tx, &next).await;
                    }
                    _ => {}
                }
            }
            StoreReply::Chat(ChatFrame::Ended { stop_reason }) => {
                println!("ended: {}", stop_reason.as_str());
                ended = true;
                break;
            }
            StoreReply::Chat(ChatFrame::Failed { message }) => {
                panic!("the session failed: {message}")
            }
            StoreReply::Failed { request, message } => panic!("{request} failed: {message}"),
            other => println!("reply: {other:?}"),
        }
    }
    assert!(
        ended,
        "the session ended within {CONVERSATION_TIMEOUT:?} (turns closed: {turn})"
    );
    let _ = tokio::time::timeout(Duration::from_secs(30), session).await;

    // ---------------------------------------------------------------------------------------
    // The log: the structural floor of `tests/chat_live.rs:115-153`, transposed.
    // ---------------------------------------------------------------------------------------
    let log = store
        .step_events(step_id)
        .await
        .expect("the log reads")
        .expect("the chat step has a log");
    for event in &log {
        println!("row {:>3} turn {} {:?}", event.seq, event.turn, event.kind);
    }

    assert_eq!(log[0].kind, EventKind::Prompt, "the prompt opens the log");
    assert_eq!(log[0].seq, 0, "and it is `seq` 0");
    assert!(
        log.iter().any(|row| row.kind == EventKind::AssistantText),
        "the agent answered"
    );
    assert!(
        log.iter().any(|row| row.kind == EventKind::Done),
        "the turn closed"
    );
    // An `other` row's payload is `{ update, body }`, so the banner's fields are one level down.
    // The banner is the session's first row after the prompt (§4.4): it is emitted inside the
    // handshake, before `session/prompt` is ever sent, so nothing the agent says can precede it.
    let banner = log
        .iter()
        .find(|row| row.kind == EventKind::Other)
        .expect("the chat recorded an `other` row");
    assert_eq!(
        banner.payload["update"],
        json!(htui_agent::acp::SESSION_STARTED),
        "the session banner is the first `other` row (§4.4): {}",
        banner.payload
    );
    let body = &banner.payload["body"];
    assert!(body["session_id"].is_string(), "{body}");
    assert_eq!(body["protocol_version"], 1, "MOD-2 speaks wire protocol 1");
    assert!(
        body["agent_version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "the adapter's version is recorded per session (risk 6: version skew per box)"
    );
    println!(
        "session banner = {}",
        serde_json::to_string_pretty(body).expect("the banner re-serialises")
    );

    // ---------------------------------------------------------------------------------------
    // The fixture (blueprint B.8 amendment 4): the raw `session/update` lines, in
    // `claude_acp_turn.jsonl`'s shape. Redacted by hand before commit (H-13).
    // ---------------------------------------------------------------------------------------
    let updates: Vec<String> = log
        .iter()
        .filter_map(|row| row.raw.as_ref())
        .filter(|raw| raw["method"] == "session/update")
        .map(|raw| {
            json!({
                "direction": "agent",
                "method": raw["method"],
                "params": raw["params"],
            })
            .to_string()
        })
        .collect();
    match std::env::var(FIXTURE_OUT_ENV) {
        Ok(path) if !path.is_empty() => {
            let body = updates.iter().fold(String::new(), |mut text, line| {
                text.push_str(line);
                text.push('\n');
                text
            });
            std::fs::write(&path, body).expect("the fixture is written");
            println!(
                "wrote {} `session/update` line(s) to {path} — redact $HOME paths and the session \
                 id by hand before committing (H-13)",
                updates.len()
            );
        }
        _ => {
            println!(
                "{FIXTURE_OUT_ENV} is unset; the {} `session/update` line(s) follow:",
                updates.len()
            );
            for line in &updates {
                println!("{line}");
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // The three §11.14 answers, stated in words. "Nothing arrived" is one of them.
    // ---------------------------------------------------------------------------------------
    let probe_file = scratch.path().join(PROBE_FILE);
    println!("\n=== ANA-4 §11.14, answered from this run ===");
    println!("adapter: {}", body["agent_name"]);
    println!("version: {}", body["agent_version"]);
    println!("models projected onto the banner: {}", body["models"]);
    println!();
    println!(
        "1. `usage_update` (:1384): {} frame(s)",
        observed.usage.len()
    );
    for usage in &observed.usage {
        println!("   {usage}");
    }
    if observed.usage.is_empty() {
        println!(
            "   NONE. `agy_acp_server` reported no usage over three turns, so §7's `agy` row is \
             confirmed as `quota.source: \"none\"` and the Settings quota column reads `—` by \
             design (plan D65)."
        );
    }
    println!();
    println!(
        "2. `session/request_permission` in `default` mode (:1385-1386): {} request(s)",
        observed.permissions.len()
    );
    for request in &observed.permissions {
        println!("   {request}");
    }
    if observed.permissions.is_empty() {
        println!(
            "   NONE — no `session/request_permission` for a read or a write in `default` mode. \
             `htui`'s own policy is `ask`, so nothing here auto-answered: the adapter simply never \
             asked."
        );
    }
    println!();
    println!("3. the edit shape (:1387-1388):");
    println!("   tool_call frames: {}", observed.tool_calls.len());
    for call in &observed.tool_calls {
        println!("     {call}");
    }
    println!("   edit_proposal frames: {}", observed.edits.len());
    for edit in &observed.edits {
        println!("     {edit}");
    }
    let edit_kinds = observed
        .tool_calls
        .iter()
        .any(|call| call["tool_kind"] == json!(ToolKind::Edit.as_str()));
    match (!observed.edits.is_empty(), edit_kinds) {
        (true, _) => println!(
            "   ANSWER: the edit reached `htui` as an `edit_proposal` with a diff (either a \
             `tool_call` diff content block or the `fs/write_text_file` route, which synthesizes \
             one — `acp/mod.rs:1609-1628`)."
        ),
        (false, true) => println!(
            "   ANSWER: a `tool_call` with `kind: \"edit\"` arrived but carried **no** diff \
             content block, so no `edit_proposal` was produced. That is the vendor shape half of \
             the question."
        ),
        (false, false) => println!(
            "   ANSWER: neither an `edit_proposal` nor a `tool_call{{kind: edit}}` arrived. \
             Whatever the adapter did is in the `other` update names below and in the fixture."
        ),
    }
    println!("   `other` update names, in order: {:?}", observed.others);
    println!("   `error` frames: {:?}", observed.errors);
    println!(
        "   {PROBE_FILE} exists in the scratch tree: {}",
        probe_file.is_file()
    );
    if let Ok(text) = std::fs::read_to_string(&probe_file) {
        println!("   its contents: {text:?}");
    }
    println!("=== end of the §11.14 answers ===\n");

    // Criterion 11, the process-group half: nothing this session started outlives it.
    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = matching_processes();
        let new: Vec<&String> = after.iter().filter(|pid| !before.contains(*pid)).collect();
        assert!(new.is_empty(), "the session left processes behind: {new:?}");
    }
}

/// Prints one frame with enough of its body to answer §11.14, and records it in [`Observed`].
///
/// Deliberately per-variant rather than a blanket `{:?}`: a `Debug` of a turn's worth of text
/// chunks buries the four frames that matter, and the point of a live probe is that its output can
/// be read.
fn report(event: &DriverEvent, observed: &mut Observed) {
    match event {
        DriverEvent::AssistantChunk(chunk) => println!("frame: assistant_text {:?}", chunk.text),
        DriverEvent::ThoughtChunk(chunk) => println!("frame: thought {:?}", chunk.text),
        DriverEvent::ToolCall(call) => {
            let record = json!({
                "tool_call_id": call.tool_call_id,
                "tool_kind": call.tool_kind.as_str(),
                "title": call.title,
                "input": call.input,
                "locations": call.locations.iter().map(|at| at.path.clone()).collect::<Vec<_>>(),
            });
            println!("frame: tool_call {record}");
            observed.tool_calls.push(record);
        }
        DriverEvent::ToolResult(result) => println!(
            "frame: tool_result {} {:?}",
            result.tool_call_id, result.status
        ),
        DriverEvent::EditProposal(edit) => {
            let record = json!({
                "tool_call_id": edit.tool_call_id,
                "path": edit.path,
                "accepted": edit.accepted,
                "diff_lines": edit.diff.lines().count(),
            });
            println!("frame: edit_proposal {record}");
            println!("  diff:\n{}", edit.diff);
            observed.edits.push(record);
        }
        DriverEvent::PermissionRequest(request) => {
            let record = json!({
                "request_id": request.request_id.as_str(),
                "tool_call_id": request.tool_call_id,
                "options": request
                    .options
                    .iter()
                    .map(|option| json!({
                        "id": option.id,
                        "label": option.label,
                        "kind": option.kind.as_str(),
                    }))
                    .collect::<Vec<_>>(),
            });
            println!("frame: permission_request {record}");
            observed.permissions.push(record);
        }
        DriverEvent::Plan(plan) => println!("frame: plan with {} entries", plan.entries.len()),
        DriverEvent::Usage(usage) => {
            // Serialized rather than `Debug`ged: this is the exact document the recorder writes to
            // `session_event.payload`, which is what §11.14's first question is about, and once
            // T37 lands its `quota` key appears here without touching this line.
            let record = serde_json::to_value(usage).unwrap_or(Value::Null);
            println!("frame: usage {record}");
            observed.usage.push(record);
        }
        DriverEvent::Error(error) => {
            let record = json!({ "code": error.code, "message": error.message });
            println!("frame: error {record}");
            observed.errors.push(record);
        }
        DriverEvent::Done(done) => println!("frame: done {}", done.stop_reason.as_str()),
        DriverEvent::Other(other) => {
            println!("frame: other `{}` {}", other.update, other.body);
            observed.others.push(other.update.clone());
        }
    }
}

/// The option id this test answers a permission request with.
///
/// The first `AllowOnce`, per blueprint B.8 amendment 2. The two fallbacks exist so an adapter
/// that offers no `allow_once` produces a **recorded observation** rather than a hung turn: an
/// unanswered request blocks `next_event` on every transport (`acp/mod.rs:632`), so refusing to
/// choose would fail this test at the deadline and answer nothing.
///
/// # Panics
///
/// When the request carries no options at all, which would be a protocol violation worth failing
/// on: there would be nothing to send back.
fn choose(options: &[htui_agent::event::PermissionOption]) -> String {
    if let Some(option) = options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
    {
        return option.id.clone();
    }
    if let Some(option) = options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowAlways)
    {
        println!(
            "  OBSERVED: no `allow_once` option; falling back to `allow_always` `{}`",
            option.id
        );
        return option.id.clone();
    }
    let option = options
        .first()
        .expect("a permission request offers at least one option");
    println!(
        "  OBSERVED: no allow option at all; answering with the first offered, `{}` ({})",
        option.id,
        option.kind.as_str()
    );
    option.id.clone()
}

/// The pids whose command line mentions the adapter, as a set.
///
/// By pattern rather than by `/proc` parentage, as `tests/chat_live.rs` does it: the session's
/// child belongs to a task this test spawned, not to this thread, and the pattern is the same one
/// the maintainer would type. `pgrep -f` also matches the shell that launched `cargo test`, which
/// is why the assertion is about the **difference** between two readings.
#[cfg(unix)]
fn matching_processes() -> BTreeSet<String> {
    let output = std::process::Command::new("pgrep")
        .args(["-f", "agy_acp_server"])
        .output()
        .expect("pgrep runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// No pattern scan off unix; the survivor assertion is `cfg(unix)` too.
#[cfg(not(unix))]
fn matching_processes() -> BTreeSet<String> {
    BTreeSet::new()
}

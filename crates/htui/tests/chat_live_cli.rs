//! One real conversation over the **`claude-cli` row**, through the whole seam
//! (`docs/ANA-4.md` §11 criteria 7, 9 and 11; MOD-2 milestone 8 `T57`, blueprint F-T57).
//!
//! This is the milestone's live proof. Everything `T49`–`T54` built — the third seed row, the
//! `cli/claude_stream_json` adapter, the supervisor, the stream mapper, the conformance arms — has
//! so far been exercised against scripts and recorded transcripts. Here the production
//! [`AgentRuntime`] launches the `claude` binary this box actually has installed, streams two turns
//! of a real conversation into a store through the production recorder, ends the session the way
//! `Esc Esc` does, and leaves nothing running.
//!
//! ```text
//! cargo test -p htui --features testkit --test chat_live_cli -- --ignored --nocapture
//! ```
//!
//! `testkit` is `htui`'s only test feature (`crates/htui/Cargo.toml:19-23`, blueprint E-6 — the
//! plan's `--features demo,test-support` line names features this crate does not declare).
//!
//! ## The two tests, and why only one is `#[ignore]`d
//!
//! [`the_seeded_cli_row_banners_its_three_missing_capabilities`] spawns nothing and costs nothing:
//! it drives the **real seed row** through the **real `caps_for`** into a **real `ChatTab`** over a
//! scripted transport registered under the production adapter id, and photographs the one line the
//! milestone promises the user. It runs in every `cargo test -p htui --features testkit`, so the
//! promise is defended continuously rather than only on the days someone spends tokens.
//!
//! [`a_real_claude_cli_session_streams_two_turns_into_the_store_and_then_ends`] is the `#[ignore]`d
//! live half, and it asserts that the capabilities the live driver reports on `ChatAccepted` are
//! the same profile. Those two together are §4.3's claim end to end: the degradation is
//! **declared**, not hidden.
//!
//! ## What the live half asserts, and the authority for each
//!
//! | Assertion | Authority |
//! |---|---|
//! | the accepted caps are `caps_for(row)`, with `permission_requests`, `edit_proposals` and `plans` all `false` | §4.3, plan D80 |
//! | the session banner is the step's **first** `other` row, at `protocol_version: null` | D84 for the banner, **F-9** for the buffer that keeps it first — hook envelopes arrive before `system/init` on this dialect, and a supervisor that mapped them as they landed would put an `other` row in front of it |
//! | the banner's `agent_name` is the row's own name and `agent_version` is non-empty | the stream carries no agent identity, so the name comes from the row and the version from the `--version` capture (risk 6, version skew per box) |
//! | one `usage` row per turn, and no more | **D91** / **F-4**: cost exists only on the terminal `result`, so `usage_mid_turn` is `false` here and a turn reports exactly once |
//! | `UsageTotals::from_rows` equals a hand-written sum of the persisted rows, and its `cost_micros` equals the last `cost_micros_total` | §11 criterion 7, both clauses. `modelUsage` is **cumulative** and the mapper reports deltas (F-4); a mapper that forwarded the cumulative figure would fail clause two on turn 2, which is the exact double-count F-4 was raised to catch |
//! | no `permission_request`, `edit_proposal` or `plan` row ever lands | the other side of the banner: a transport that declares it cannot do a thing must not then do it |
//! | no process survives the session | §11 criterion 11, the CLI half |
//!
//! ### Why `run_step.usage` is summed from the rows rather than read from the column
//!
//! Criterion 7 is a statement about `run_step.usage`, and no `ReadStore` method returns that column
//! (`chat_usage_pg.rs:113-126` says why, and reaches it with a raw `SELECT`). This file records
//! into a [`MemStore`], as `chat_live.rs` does, so the column is not reachable at all from here.
//! What is reachable is the **one summing rule** both writers share
//! ([`UsageTotals::from_rows`](htui_core::model::UsageTotals::from_rows), plan D36): the online
//! recorder sums the deltas as they arrive and writes that document to the column, and the offline
//! uploader re-derives it from the same rows. So the sum asserted here **is** the document, checked
//! against an independent hand sum of the persisted payloads, and the column-side identity is
//! pinned on real Postgres by `chat_usage_pg.rs` and transport-neutrally by the store conformance
//! case `usage_deltas_sum_to_step_usage`. Blueprint F-T57 names this the route and puts the
//! Postgres variant of *this* file outside the milestone's scope.

#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use std::time::Duration;

use htui::agent_worker::{AgentRuntime, Served};
use htui::store_worker::{
    ChatFrame, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest,
};
use htui::testkit::Harness;
use htui::ui::tabs::ChatTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason, TextChunk};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::{DriverFactory, caps_for};
use htui_core::fixtures::ids;
use htui_core::model::{Agent, EventKind, SessionEvent, Transport, UsageTotals};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use htui_store::Backend;
use serde_json::Value;
use tokio::sync::mpsc;

/// The line the Chat tab draws for a session whose driver answers §4.3's degraded profile.
///
/// Spelled out rather than rebuilt from `DriverCaps`, because a test that recomputed the tab's own
/// rule would agree with itself whatever the rule became. `ChatTab::caps_banner`
/// (`ui/tabs/chat/mod.rs:320-333`) joins the missing capabilities in declaration order, and this is
/// the sentence a user of the `claude-cli` row sees above the transcript.
const CAPS_BANNER: &str = "this agent cannot: permission requests, edit proposals, plans";

/// The two prompts, in order.
///
/// One word of answer each, deliberately: this file spends the maintainer's own subscription, and
/// a turn's cost is dominated by the tokens the model produces. Two turns rather than one because
/// the second is what makes criterion 7 non-trivial — a single-turn chat has exactly one `usage`
/// row and "the sum equals the row" is arithmetic, while two rows make the sum a claim about the
/// mapper's deltas (F-4).
const TURNS: [&str; 2] = [
    "reply with exactly the word ok and nothing else. do not use any tools.",
    "reply with exactly the word two and nothing else. do not use any tools.",
];

/// How long the whole two-turn conversation may take.
///
/// `chat_live.rs` gives one ACP turn 120 s. These are two turns of a real model round trip through
/// a CLI that also runs this repo's hooks before it reads stdin (F-9), so a shorter deadline would
/// report a busy service as a broken seam.
const CONVERSATION_TIMEOUT: Duration = Duration::from_secs(240);

/// The seeded `claude-cli` row, as the store holds it.
///
/// By name, which a **test** may do: `R-AGT-5` forbids a *code path* keyed on an agent's name, and
/// `extensibility.rs`'s sweeps scan `src/` for exactly that. Reading a seed row's own data is what
/// every live suite here does (`chat_live.rs:36-44`).
async fn cli_row(store: &MemStore) -> Agent {
    store
        .agents()
        .await
        .expect("the registry reads")
        .into_iter()
        .find(|summary| summary.agent.name == "claude-cli")
        .expect(
            "the fixture carries `claude-cli` (D79's third seed row, H-10's three-element list)",
        )
        .agent
}

/// Types `text` into the composer and submits it, as `tests/chat.rs:123-133` does.
///
/// A space is `"space"`: `KeyChord::parse` trims its input, so a bare `" "` is not a chord — the
/// composer sees the same `KeyCode::Char(' ')` either way.
fn compose(harness: &mut Harness, text: &str) {
    harness.key("i");
    for ch in text.chars() {
        if ch == ' ' {
            harness.key("space");
        } else {
            harness.key(&ch.to_string());
        }
    }
    harness.key("enter");
}

/// §4.3's claim, on the seeded row, all the way to the screen — with no process anywhere.
///
/// The transport is milestone 2's fake, registered under the **production** adapter id
/// `cli/claude_stream_json`, so the factory reaches it exactly as it reaches the real one: by the
/// row's `transport` and its `settings.cli.stream`, never by the agent's name (plan D12). The
/// capabilities the session is built with are therefore the real [`caps_for`] over the real seed
/// row, and the banner on screen is what the `claude-cli` row's user will read.
///
/// `tests/chat.rs::a_degraded_transport_banners_what_it_cannot_do` photographs the same line over a
/// hand-built `cli` row; what this one adds is that the **shipped** row is such a row.
#[tokio::test]
async fn the_seeded_cli_row_banners_its_three_missing_capabilities() {
    let store = MemStore::demo();
    let agent = cli_row(&store).await;
    assert_eq!(agent.transport, Transport::Cli, "D79's row is a CLI row");

    let caps = caps_for(&agent);
    assert!(!caps.permission_requests, "§4.3: no inline approvals");
    assert!(!caps.edit_proposals, "§4.3: no diff to accept or reject");
    assert!(!caps.plans, "§4.3: no plan updates");
    assert!(
        !caps.usage_mid_turn,
        "D91: cost arrives once, on the terminal `result`"
    );

    // The other two fixture rows are `acp`, and this build's ACP adapter would try to spawn one.
    // Disabling them leaves the tab a single agent to talk to, which is what makes the choice
    // deliberate rather than alphabetical (`tests/chat.rs:86-93`).
    for summary in store.agents().await.expect("the registry reads") {
        if summary.agent.id != agent.id {
            let mut row = summary.agent;
            row.enabled = false;
            store.upsert_agent(&row).await.expect("the row is disabled");
        }
    }

    let adapter = FakeAdapter::new();
    adapter.load(Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
            text: "ok".to_owned(),
            message_id: Some("m1".to_owned()),
        })),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ]));
    let mut factory = DriverFactory::new();
    factory.register(htui_agent::cli::ADAPTER_ID, Box::new(adapter));

    let mut harness = Harness::over(store.clone())
        .with_tab(Box::new(ChatTab::new()))
        .with_replay_tab(ChatTab::ID)
        .with_agent_runtime(AgentRuntime::new(factory).with_grace(Duration::from_millis(0)));
    harness.drive().await;
    compose(&mut harness, "hello");
    harness.drive().await;

    let screen = harness.render();
    assert!(
        screen.contains(CAPS_BANNER),
        "the `claude-cli` row's session must name what it cannot do (§4.3); the screen was:\n\
         {screen}"
    );
}

/// Two real turns of `claude -p --output-format stream-json`, recorded and then ended.
///
/// `#[ignore]` for two reasons at once: it spawns the `claude` this box has installed, and it
/// spends real tokens of the maintainer's own subscription. One authorised run per change — the
/// `--nocapture` output below is the evidence, and re-running it to explore is spending someone
/// else's money.
///
/// The structure is `tests/chat_live.rs:29-154` transposed to a second turn: seeded row, production
/// [`AgentRuntime`], real transport, recorder and chat seam turning typed prompts into streamed
/// rows, then `ChatCancel` — the `Esc Esc` path — and the process check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "spawns the `claude` CLI installed on this box and spends subscription tokens"]
async fn a_real_claude_cli_session_streams_two_turns_into_the_store_and_then_ends() {
    // Processes matching **before** the chat, so the assertion at the end is about new survivors:
    // the pattern below also matches the maintainer's own editor session, and an absolute count
    // would fail for reasons that have nothing to do with this code.
    let before = matching_processes();

    let store = MemStore::demo();
    // The fixture's `claude-cli` row **is** `crates/htui-core/seeds/agent_claude_cli.json`,
    // re-stamped (`htui_core::fixtures`), so this launches exactly what a real database would
    // (D13, and D88 for how an existing box gets the row at all).
    let agent = cli_row(&store).await;
    let agent_id = agent.id;
    let expected_caps = caps_for(&agent);
    println!("row: {} ({:?})", agent.name, agent.transport);
    println!("caps: {expected_caps:?}");

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
    println!("--- turn 1 prompt: {}", TURNS[0]);
    let Served::Start { step_id, task } = runtime.serve(&backend, &tx, &start).await else {
        panic!("a chat start opens a session")
    };
    let session = tokio::spawn(task);
    runtime.attach(step_id, tokio::spawn(async {}));

    // The stream: print every frame, send the next turn on each `done`, and end the session the
    // way `Esc Esc` does after the last one.
    let mut accepted_caps = None;
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
                accepted_caps = Some(caps);
            }
            StoreReply::Chat(ChatFrame::Event(frame)) => {
                report(&frame.event);
                if let DriverEvent::Done(done) = &frame.event {
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

    // -------------------------------------------------------------------------------------------
    // §4.3: the capabilities the live driver reported.
    // -------------------------------------------------------------------------------------------
    let caps = accepted_caps.expect("the chat was accepted");
    assert_eq!(
        caps, expected_caps,
        "the accepted caps are the row's own profile, computed by `caps_for` and not by the tab"
    );
    assert!(
        !caps.permission_requests && !caps.edit_proposals && !caps.plans,
        "§4.3: the CLI transport declares three missing capabilities, and the tab turns exactly \
         those into `{CAPS_BANNER}` — {caps:?}"
    );

    // -------------------------------------------------------------------------------------------
    // The rows.
    // -------------------------------------------------------------------------------------------
    let log = store
        .step_events(step_id)
        .await
        .expect("the log reads")
        .expect("the chat step has a log");
    for row in &log {
        let detail = match row.kind {
            EventKind::Other | EventKind::Usage | EventKind::Done | EventKind::Error => {
                format!(" {}", row.payload)
            }
            _ => String::new(),
        };
        println!(
            "row {:>3} turn {} {:?}{detail}",
            row.seq, row.turn, row.kind
        );
    }

    assert_eq!(log[0].kind, EventKind::Prompt, "the prompt opens the log");
    assert_eq!(log[0].seq, 0, "and it is `seq` 0");
    assert!(
        log.iter().any(|row| row.kind == EventKind::AssistantText),
        "the agent answered"
    );
    assert_eq!(
        log.iter().filter(|row| row.kind == EventKind::Done).count(),
        TURNS.len(),
        "one `done` per turn"
    );
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::FollowUp)
            .count(),
        TURNS.len() - 1,
        "and one `follow_up` for every turn after the first, in the same process"
    );

    // D84 and F-9 together: the banner is the step's first `other` row **because** the supervisor
    // buffers the hook envelopes that arrive before `system/init`. Finding it by position rather
    // than by name is the whole point — searching for it by `update` would pass even if three hook
    // rows sat in front of it.
    let banner = log
        .iter()
        .find(|row| row.kind == EventKind::Other)
        .expect("the chat recorded an `other` row");
    assert_eq!(
        banner.payload["update"],
        Value::from(htui_agent::event::SESSION_STARTED),
        "the session banner is the step's first `other` row (D84, and F-9's pre-`init` buffer is \
         what keeps it first on this dialect): {}",
        banner.payload
    );
    let body = &banner.payload["body"];
    println!(
        "session banner = {}",
        serde_json::to_string_pretty(body).expect("the banner re-serialises")
    );
    assert!(body["session_id"].is_string(), "{body}");
    assert_eq!(
        body["protocol_version"],
        Value::Null,
        "the stream negotiates nothing, so there is no version to report — a `1` copied from ACP \
         would be a claim about a handshake that never happened: {body}"
    );
    assert_eq!(
        body["agent_name"],
        Value::from(agent.name.as_str()),
        "the stream carries no agent identity of its own, so the banner's name is the row's: {body}"
    );
    assert!(
        body["agent_version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "the CLI's version is recorded per session, from the `--version` capture (risk 6: version \
         skew per box): {body}"
    );

    // The other side of the banner: three capabilities declared missing, and three kinds of row
    // that therefore never land.
    for kind in [
        EventKind::PermissionRequest,
        EventKind::EditProposal,
        EventKind::Plan,
    ] {
        assert!(
            !log.iter().any(|row| row.kind == kind),
            "a transport that declares it cannot do a thing must not then do it: {kind:?}"
        );
    }

    // -------------------------------------------------------------------------------------------
    // §11 criterion 7.
    // -------------------------------------------------------------------------------------------
    let usage_rows: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Usage)
        .collect();
    assert_eq!(
        usage_rows.len(),
        TURNS.len(),
        "D91/F-4: cost exists only on the terminal `result`, so a turn reports usage exactly once"
    );
    for row in &usage_rows {
        assert_eq!(
            row.payload["usage_scope"],
            Value::from("model_usage"),
            "the row's `settings.usage.scope` travels on every usage payload (§5.2): {}",
            row.payload
        );
    }

    let summed = UsageTotals::from_rows(&log);
    // The same five keys, added up by hand off the persisted payloads. Independent of
    // `UsageTotals`, which is the point: the shared rule is what writes `run_step.usage`, and a
    // test that only re-ran it would agree with itself.
    let expected = UsageTotals {
        input_tokens: key_sum(&usage_rows, "input_tokens"),
        output_tokens: key_sum(&usage_rows, "output_tokens"),
        cache_read_tokens: key_sum(&usage_rows, "cache_read_tokens"),
        cache_write_tokens: key_sum(&usage_rows, "cache_write_tokens"),
        cost_micros: key_sum(&usage_rows, "cost_micros"),
    };
    println!("usage totals = {}", summed.to_value());
    assert_eq!(
        summed, expected,
        "criterion 7, clause one: `run_step.usage` is the sum of the step's `usage` rows"
    );

    let last_total = usage_rows
        .iter()
        .filter_map(|row| row.payload.get("cost_micros_total").and_then(Value::as_i64))
        .max();
    assert!(
        summed.cost_micros.is_some_and(|micros| micros > 0),
        "the CLI reports `total_cost_usd` on every `result`, so a `None` or a zero here would make \
         clause two vacuous: {}",
        summed.to_value()
    );
    assert_eq!(
        summed.cost_micros, last_total,
        "criterion 7, clause two: the summed deltas meet the last cumulative total. `modelUsage` \
         is cumulative and the mapper reports deltas (F-4); a mapper that forwarded the cumulative \
         figure would double-count turn 1 into turn 2 and fail exactly here"
    );

    // -------------------------------------------------------------------------------------------
    // Criterion 11, the CLI half: nothing this session started outlives it.
    // -------------------------------------------------------------------------------------------
    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = matching_processes();
        let new: Vec<&String> = after.iter().filter(|pid| !before.contains(*pid)).collect();
        assert!(new.is_empty(), "the session left processes behind: {new:?}");
    }
}

/// `Σ payload[key]` over the rows that carry an integer there, or `None` when none did.
///
/// The distinction `UsageTotals` draws and this has to draw too: a key no row reported stays
/// `null`, which is not the same fact as a reported zero.
fn key_sum(rows: &[&SessionEvent], key: &str) -> Option<i64> {
    rows.iter()
        .filter_map(|row| row.payload.get(key).and_then(Value::as_i64))
        .fold(None, |total, delta| {
            Some(total.unwrap_or(0).saturating_add(delta))
        })
}

/// Prints one frame with enough of its body to read the run off the output.
///
/// Per-variant rather than a blanket `{:?}`, for `chat_live_agy.rs:532-536`'s reason: a `Debug` of
/// a turn's worth of text chunks buries the frames that matter.
fn report(event: &DriverEvent) {
    match event {
        DriverEvent::AssistantChunk(chunk) => println!("frame: assistant_text {:?}", chunk.text),
        DriverEvent::ThoughtChunk(chunk) => println!("frame: thought {:?}", chunk.text),
        DriverEvent::Usage(usage) => println!(
            "frame: usage {}",
            serde_json::to_value(usage).unwrap_or(Value::Null)
        ),
        DriverEvent::Other(other) => println!("frame: other `{}` {}", other.update, other.body),
        DriverEvent::Error(error) => {
            println!("frame: error {} {}", error.code, error.message);
        }
        DriverEvent::Done(done) => println!("frame: done {}", done.stop_reason.as_str()),
        other => println!("frame: {other:?}"),
    }
}

/// The pids whose command line carries the argv only this transport spells, as a set.
///
/// **Not** `pgrep -f claude` (blueprint P-9, hazard H-27): the test binary is named
/// `chat_live_cli`, the maintainer's own editor is a `claude` process, and the shell that ran
/// `cargo test` holds the word too — so that pattern would match this very run and the assertion
/// would fail for reasons unrelated to the code. `--output-format stream-json` is in
/// `cli::argv`'s fixed flag block (`cli/mod.rs:112-125`) and in nothing else here.
#[cfg(unix)]
fn matching_processes() -> BTreeSet<String> {
    let output = std::process::Command::new("pgrep")
        .args(["-f", "--", "--output-format stream-json"])
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

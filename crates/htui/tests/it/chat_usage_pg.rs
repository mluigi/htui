//! `docs/ANA-4.md` §11 criterion 7 and the §7 quota latch, against live Postgres (MOD-2 T43,
//! blueprint B.10, P-3).
//!
//! Criterion 7 (`docs/ANA-4.md:1356-1357`) is one sentence with two clauses: `run_step.usage`
//! equals the sum of the step's `usage` rows **and**, for an ACP session, its `cost_micros` equals
//! the last `cost_micros_total` the agent reported. The store conformance case
//! `usage_deltas_sum_to_step_usage` covers the first clause transport-neutrally and cannot cover
//! the second at all: `cost_micros_total` is a *mapper* key, produced by `acp::map` from the
//! cumulative `cost` ACP reports, and a hand-built row set would prove only that this file agrees
//! with itself.
//!
//! So the transcript is real - `crates/htui-agent/tests/fixtures/claude_acp_turn.jsonl`, recorded
//! off `claude-code-acp` - and every piece under it is the production one: the ACP mapper, the
//! recorder, `Writer::Online` over a throwaway database, and the two columns read back with a raw
//! `SELECT`. This is **not** in `crates/htui-store/tests/pg_criteria.rs` for the reason blueprint
//! P-3 gives and `chat_offline.rs:1-14` set the precedent for: `htui-store` has no `htui-agent`
//! dependency, so it can construct neither a `Mapper` nor a `Recorder`, and `htui` is the one
//! crate holding both.
//!
//! The second case is the latch's Postgres line. `agent_box.quota` / `.quota_at` are
//! **single-writer** since plan D74: `WriteStore::set_agent_box_quota` is their only writer, and
//! `upsert_agent_box` can neither set nor clear them - which is why the `probe` document this case
//! latches over is seeded through `upsert_agent_box` while the allowance is not. The
//! `probe`-byte-identical assertion is that property from the other side: the narrow setter the
//! latch calls must not be the statement that writes the §4.6 snapshot, because a re-probe of the
//! same row may be running beside a chat (plan D55/D60/D67).
//!
//! Both cases create and drop their own database and print `testkit::SKIP` with
//! `HTUI_TEST_DATABASE_URL` unset (plan D13), like every other Postgres-backed suite.

use chrono::{DateTime, Utc};
use htui_agent::acp::map::Mapper;
use htui_agent::event::{DriverEnvelope, DriverEvent};
use htui_agent::record::{QuotaLatch, Recorder};
use htui_core::fixtures::ids;
use htui_core::model::{
    AgentBox, Billing, ChatRunSpec, EventKind, Quota, QuotaSource, SessionEvent, StepId,
    UsageTotals,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{ReadStore, WriteStore as _};
use htui_store::{Writer, testkit};
use serde_json::{Value, json};
use sqlx::Row as _;
use sqlx::postgres::PgPool;

/// The recorded ACP turn this suite drives: three `usage_update` reports, one of which carries the
/// `_meta` rate-limit blob and one of which carries the session's cumulative `cost`.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../htui-agent/tests/fixtures/claude_acp_turn.jsonl"
);

/// A fixed capture time, so the rows and the latched document are a function of the fixture and of
/// nothing else (`replay.rs:32-35`). Milliseconds, which `timestamptz` keeps exactly - this suite
/// compares `quota_at` to the `observed_at` inside the document for equality.
fn at() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the demo epoch is a valid instant")
}

/// An envelope with no `raw`: `retain_raw` is off, and a `raw` blob is not what these cases assert.
fn env(event: DriverEvent) -> DriverEnvelope {
    DriverEnvelope {
        event,
        raw: None,
        at: at(),
    }
}

/// Every `session/update` of [`FIXTURE`], mapped in order by the production ACP mapper
/// (`replay.rs:411-424`).
///
/// One `Mapper` for the whole file, because that is what makes `cost_micros` a delta of the
/// cumulative `cost_micros_total` it also records - the identity clause two is about.
fn mapped_turn() -> Vec<DriverEvent> {
    let text = std::fs::read_to_string(FIXTURE).unwrap_or_else(|err| panic!("{FIXTURE}: {err}"));
    let mut mapper = Mapper::new();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("a fixture line is JSON"))
        .filter(|line| line["method"] == "session/update")
        .flat_map(|line| mapper.map(&line["params"]["update"]))
        .collect()
}

/// The §4.6 probe snapshot the latched row already carries.
///
/// Big enough that "byte-identical" is a claim about a payload rather than about a two-key object
/// `JSONB` might normalise either way (`pg_criteria.rs:1618-1639`'s reason).
fn probe_snapshot() -> Value {
    json!({
        "transport": "acp",
        "resolved": {
            "command": "/usr/bin/node",
            "args": ["/opt/claude-code-acp/dist/index.js", "--stdio"],
            "env": { "CLAUDE_CODE_EXECUTABLE": "/usr/bin/claude" },
        },
        "tools": { "claude": "2.1.263", "claude_agent_acp": null, "node": "22.19.0" },
        "handshake": {
            "at": "2026-09-08T12:00:00Z",
            "protocol_version": 1,
            "agent_name": "claude-code-acp",
            "agent_version": "0.7.1",
            "capabilities": { "loadSession": false },
            "auth_methods": [],
        },
        "status": "ready",
        "stderr_tail": null,
        "source": "probe",
    })
}

/// `run_step.usage`, which no `ReadStore` method returns (`pg_criteria.rs:93-105`).
///
/// Runtime-checked rather than `query!` for the reason that file gives: a new `query!` string
/// would need a `cargo sqlx prepare` pass, and `.sqlx/` belongs to `htui-store`, not to a test in
/// this crate.
async fn step_usage(pool: &PgPool, step: StepId) -> Value {
    sqlx::query("SELECT usage FROM run_step WHERE id = $1")
        .bind(step.as_uuid())
        .fetch_one(pool)
        .await
        .expect("read run_step")
        .get::<Option<Value>, _>("usage")
        .expect("a recorded step has a usage document")
}

/// The step's persisted log, in `seq` order.
async fn step_rows<S: ReadStore>(store: &S, step: StepId) -> Vec<SessionEvent> {
    store
        .step_events(step)
        .await
        .expect("the log reads")
        .expect("the chat step has a log")
}

/// The highest `cost_micros_total` any `usage` row of the step reported - i.e. the last one, since
/// ACP's cumulative figure only grows.
///
/// Read off the **persisted rows**, not off the typed events: what criterion 7 is about is what the
/// log holds, and `UsageTotals::add_payload` reads five fixed names that do not include this one.
fn max_cost_micros_total(rows: &[SessionEvent]) -> Option<i64> {
    rows.iter()
        .filter(|row| row.kind == EventKind::Usage)
        .filter_map(|row| row.payload.get("cost_micros_total").and_then(Value::as_i64))
        .max()
}

/// §11 criterion 7, both clauses, on a real ACP transcript against a real server.
///
/// The recorder sums the `cost_micros` **deltas** as they arrive and writes the sum to
/// `run_step.usage`; the mapper derives each delta from the cumulative `cost_micros_total` it puts
/// on the same row. The two therefore have to meet at the last total (blueprint P-5), and the day
/// they part - a mapper that stops carrying the total forward, a recorder that starts taking the
/// last row instead of summing - is the day this case goes red.
#[tokio::test(flavor = "multi_thread")]
async fn an_acp_step_usage_equals_the_last_cost_total_on_postgres() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        db.store.this_box(),
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        None,
    );
    let writer = Writer::Online(db.store.clone());
    writer
        .start_chat_run(&chat)
        .await
        .expect("the chat run and step mint");

    let scrubber = MinimalScrubber::new(Vec::<String>::new());
    let mut recorder = Recorder::new(&writer, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt("what did that turn cost", json!({}), at())
        .await
        .expect("the prompt records");
    for event in mapped_turn() {
        // The cap verdict, which this session has no cap to reach: `record` answers
        // `Option<CapBreach>` since plan D70 and a caller that holds no session may ignore it.
        recorder
            .record(env(event))
            .await
            .expect("every mapped row records");
    }
    recorder.finish().await.expect("the step closes");

    let persisted = step_usage(&db.pool, chat.step_id).await;
    let rows = step_rows(&db.store, chat.step_id).await;
    let summed = UsageTotals::from_rows(&rows).to_value();
    let last_total = max_cost_micros_total(&rows);
    let step_cost = persisted.get("cost_micros").and_then(Value::as_i64);

    assert!(
        step_cost.is_some_and(|micros| micros > 0),
        "the fixture reports a USD cost, so a `None` here would make both clauses vacuous: \
         {persisted}"
    );
    // Clause one, in full: not just `cost_micros` but every one of the five keys.
    assert_eq!(
        persisted, summed,
        "`run_step.usage` is the sum of the step's persisted `usage` rows"
    );
    // Clause two: the ACP half no transport-neutral case can state.
    assert_eq!(
        step_cost, last_total,
        "and for an ACP session it equals the last `cost_micros_total` observed"
    );

    db.drop_db().await;
}

/// §7's passive latch against live Postgres (plan D66-D68, D74): the document lands in
/// `agent_box.quota`, `quota_at` mirrors its `observed_at`, and the `probe` snapshot beside it
/// comes back byte for byte.
///
/// The recorder's own cases pin what the seven keys contain against a `MemStore`
/// (`htui-agent/tests/recorder.rs`); what this one owns is that the same drive against the real
/// backend writes the real columns - a `JSONB` round trip, a `timestamptz` comparison, and the
/// narrow `UPDATE` leaving the neighbouring document alone.
#[tokio::test(flavor = "multi_thread")]
async fn the_latch_lands_on_postgres_and_leaves_probe_alone() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let agent = ids::AGENT_CLAUDE;
    let on_box = db.store.this_box();
    // `quota` / `quota_at` are deliberately **not** seeded here: since plan D74 this statement
    // cannot write them, and `set_agent_box_quota` - the latch's own path - is their only writer.
    db.store
        .upsert_agent_box(&AgentBox {
            agent_id: agent,
            box_id: on_box,
            enabled: true,
            version: Some("0.7.1".to_owned()),
            path: Some("/usr/bin/node".to_owned()),
            probed_at: Some(at()),
            quota: None,
            quota_at: None,
            updated_at: at(),
            probe: Some(probe_snapshot()),
        })
        .await
        .expect("the probed row lands");

    let chat = ChatRunSpec::mint(ids::PROJECT_HTUI, on_box, ids::USER, Some(agent), None);
    let writer = Writer::Online(db.store.clone());
    writer
        .start_chat_run(&chat)
        .await
        .expect("the chat run and step mint");

    let scrubber = MinimalScrubber::new(Vec::<String>::new());
    // The two row-side facts the document carries, as the `agent` row declares them: the source is
    // `agent.settings.quota.source` and the billing is `agent.billing`, never the agent's name
    // (`R-AGT-5`).
    let mut recorder = Recorder::new(&writer, &scrubber, chat.step_id, false, None)
        .with_quota_latch(QuotaLatch {
            agent_id: agent,
            box_id: on_box,
            source: QuotaSource::AcpMetaRateLimit,
            billing: Billing::Subscription,
        });
    recorder
        .record_prompt("what is my allowance", json!({}), at())
        .await
        .expect("the prompt records");
    for event in mapped_turn() {
        recorder
            .record(env(event))
            .await
            .expect("every mapped row records");
    }
    recorder.finish().await.expect("the step closes");

    let row = sqlx::query(
        "SELECT quota, quota_at, probe FROM agent_box WHERE agent_id = $1 AND box_id = $2",
    )
    .bind(agent.as_uuid())
    .bind(on_box.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("read agent_box");
    let stored: Value = row
        .get::<Option<Value>, _>("quota")
        .expect("the latch wrote the column");
    let quota_at: DateTime<Utc> = row
        .get::<Option<DateTime<Utc>>, _>("quota_at")
        .expect("and stamped it");
    let probe: Option<Value> = row.get("probe");

    assert_eq!(
        probe.as_ref(),
        Some(&probe_snapshot()),
        "the narrow setter left the §4.6 snapshot byte-identical (plan D67), which is the other \
         side of the single-writer property D74 gave the two quota columns"
    );

    let quota = Quota::from_value(&stored).expect("the stored document parses as §7's seven keys");
    assert_eq!(
        quota
            .windows
            .iter()
            .map(|window| window.id.as_str())
            .collect::<Vec<_>>(),
        ["five_hour", "seven_day"],
        "both windows of the fixture's `_meta` blob, sorted by id"
    );
    assert_eq!(
        quota_at, quota.observed_at,
        "`agent_box.quota_at` mirrors the document's `observed_at` (§7)"
    );
    // The latched spend is the recorder's own running sum, so the column and criterion 7's figure
    // are one number rather than two (blueprint P-5).
    assert_eq!(
        quota.spend.session_micros,
        step_usage(&db.pool, chat.step_id)
            .await
            .get("cost_micros")
            .and_then(Value::as_i64),
        "and `spend.session_micros` is the same figure `run_step.usage` carries"
    );

    db.drop_db().await;
}

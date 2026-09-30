//! `Settings > Agents` registry writes against live Postgres (MOD-23 plan D239-D242, blueprint
//! §4.5), driven through `htui::store_worker::serve` over a `Backend::Online`.
//!
//! `tests/agent_settings.rs` proves the decisions over a `MemStore`. What only this file can prove
//! is that Postgres agrees: a compare-and-set token read back from `PgStore::agents` (microseconds,
//! MOD-40 F-17) round-trips into `upsert_agent`; the per-box switch survives an
//! `upsert_agent_box` that says `enabled: true`; and a taken name is the store's `UNIQUE`
//! sentence. Every read-back goes through `PgStore::agents`, never a `query!` macro, so `.sqlx`
//! does not move (blueprint F-5).
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite. Nothing here touches the disk
//! beyond a throwaway mirror, so it is not unix-only.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::agent_settings::{self, AgentDraft, AgentWrite};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{AgentBox, AgentId, AgentSummary, Billing, Transport};
use htui_core::store::WriteStore;
use htui_store::{Backend, CacheStore, PgStore, testkit};
use serde_json::json;

/// One case's world: the demo database, and `Backend::Online` over it and a throwaway mirror.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    backend: Backend,
}

impl Stack {
    /// The stack over a fresh demo database, or `None` (after `testkit::SKIP`) without a server.
    async fn new() -> Option<Self> {
        let db = testkit::demo_db().await?;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "agent-settings-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        Some(Self {
            db,
            _root: root,
            cache,
            backend,
        })
    }

    /// The registry as Postgres reads it now.
    async fn agents(&self) -> Vec<AgentSummary> {
        self.db.store.agents().await.expect("the registry reads")
    }

    /// Closes the mirror and drops the database: every case's last line.
    async fn drop_db(self) {
        self.cache.close().await;
        self.db.drop_db().await;
    }
}

/// A reply named by what it says, never by the registry it carries (`launch.env`, `R-SEC-2`).
fn describe(reply: &StoreReply) -> String {
    match reply {
        StoreReply::Failed { request, message } => format!("Failed {{ {request}: {message} }}"),
        StoreReply::AgentWritten { outcome, .. } => format!("AgentWritten {{ {outcome:?} }}"),
        other => format!("another reply ({:?})", std::mem::discriminant(other)),
    }
}

/// The registry and the outcome an `AgentWritten` carries, or a panic naming what came back.
#[track_caller]
fn written(reply: StoreReply) -> (Vec<AgentSummary>, AgentWrite) {
    match reply {
        StoreReply::AgentWritten { agents, outcome } => (agents, outcome),
        other => panic!("expected AgentWritten: {}", describe(&other)),
    }
}

/// The row with `id`, or a panic.
#[track_caller]
fn row(agents: &[AgentSummary], id: AgentId) -> &AgentSummary {
    agents
        .iter()
        .find(|summary| summary.agent.id == id)
        .unwrap_or_else(|| panic!("the registry lists {id}"))
}

/// The edit form's draft of `summary` with `models` replaced (and the default the first of them).
fn with_models(summary: &AgentSummary, models: &[&str]) -> AgentDraft {
    let models: Vec<String> = models.iter().map(|&model| model.to_owned()).collect();
    AgentDraft {
        default_model: models.first().cloned(),
        models,
        ..agent_settings::draft_of(&summary.agent)
    }
}

/// MOD-40 F-17: a token read from Postgres is the stored microsecond, so it applies; the reply's
/// re-read token applies again; the first token, spent, is `Stale`.
#[tokio::test]
async fn an_edit_round_trips_a_token_read_from_postgres() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let before = row(&stack.agents().await, ids::AGENT_CLAUDE).clone();

    let (agents, outcome) = written(
        serve(
            &stack.backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: before.agent.updated_at,
                draft: with_models(&before, &["a", "b"]),
            },
        )
        .await,
    );
    assert!(
        matches!(outcome, AgentWrite::Edited { id, .. } if id == ids::AGENT_CLAUDE),
        "a token read from Postgres applies: {outcome:?}"
    );
    let first = row(&agents, ids::AGENT_CLAUDE).clone();
    assert_eq!(first.agent.models, ["a", "b"]);
    assert!(
        first.agent.launch.get("env") == before.agent.launch.get("env")
            && first.agent.launch.get("discovery") == before.agent.launch.get("discovery"),
        "`env` and `discovery` are carried unchanged through Postgres"
    );

    let (agents, outcome) = written(
        serve(
            &stack.backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: first.agent.updated_at,
                draft: with_models(&first, &["c"]),
            },
        )
        .await,
    );
    assert!(
        matches!(outcome, AgentWrite::Edited { .. }),
        "the reply's own re-read token applies: {outcome:?}"
    );
    assert_eq!(row(&agents, ids::AGENT_CLAUDE).agent.models, ["c"]);

    let (agents, outcome) = written(
        serve(
            &stack.backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: before.agent.updated_at,
                draft: with_models(&before, &["d"]),
            },
        )
        .await,
    );
    assert_eq!(
        outcome,
        AgentWrite::Stale {
            id: ids::AGENT_CLAUDE
        }
    );
    assert_eq!(
        row(&agents, ids::AGENT_CLAUDE).agent.models,
        ["c"],
        "the spent token wrote nothing"
    );

    stack.drop_db().await;
}

/// Plan D242 on Postgres: the switch, then a probe's upsert that says `enabled: true`. The row
/// stays off, the switch stays on record, and the upsert still wrote its other columns.
#[tokio::test]
async fn a_switched_off_row_survives_an_upsert_that_says_enabled() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let box_id = stack.db.store.this_box();

    let (_, outcome) = written(
        serve(
            &stack.backend,
            &StoreRequest::SetAgentOnBox {
                agent_id: ids::AGENT_AGY,
                enabled: false,
            },
        )
        .await,
    );
    assert!(
        matches!(outcome, AgentWrite::Switched { enabled: false, .. }),
        "{outcome:?}"
    );

    let now = Utc::now();
    stack
        .db
        .store
        .upsert_agent_box(&AgentBox {
            agent_id: ids::AGENT_AGY,
            box_id,
            enabled: true,
            version: Some("9.9.9".to_owned()),
            path: None,
            probed_at: Some(now),
            quota: None,
            quota_at: None,
            updated_at: now,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        })
        .await
        .expect("the probe's upsert lands");

    let agents = stack.agents().await;
    let summary = row(&agents, ids::AGENT_AGY);
    assert!(summary.user_off, "the switch is still on record");
    let on_box = summary.on_box.as_ref().expect("the probe's row");
    assert!(!on_box.enabled, "no probe turns a switched-off row back on");
    assert_eq!(
        on_box.version.as_deref(),
        Some("9.9.9"),
        "the upsert wrote everything but `enabled`"
    );

    let (agents, outcome) = written(
        serve(
            &stack.backend,
            &StoreRequest::SetAgentOnBox {
                agent_id: ids::AGENT_AGY,
                enabled: true,
            },
        )
        .await,
    );
    assert!(
        matches!(outcome, AgentWrite::Switched { enabled: true, .. }),
        "{outcome:?}"
    );
    let summary = row(&agents, ids::AGENT_AGY);
    assert!(!summary.user_off);
    assert!(
        summary.on_box.as_ref().expect("the row").enabled,
        "switched on, a `ready` probe is enabled again"
    );

    stack.drop_db().await;
}

/// Plan D239 on Postgres: a create lands with the worker's id, and the same name again is the
/// store's `UNIQUE` sentence (`agent_name_key`), answered as `Failed`.
#[tokio::test]
async fn a_create_lands_on_postgres_and_a_taken_name_is_the_stores_sentence() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let draft = AgentDraft {
        transport: Transport::Cli,
        command: "/usr/bin/true".to_owned(),
        args: vec!["--flag".to_owned(), "a b".to_owned()],
        models: Vec::new(),
        default_model: Some("any".to_owned()),
        billing: Billing::PerToken,
        enabled: false,
    };
    let create = StoreRequest::CreateAgent {
        name: "agent-x".to_owned(),
        draft,
        settings_from: None,
    };

    let (agents, outcome) = written(serve(&stack.backend, &create).await);
    let AgentWrite::Created { id, name } = outcome else {
        panic!("a create answers Created: {outcome:?}");
    };
    assert_eq!(name, "agent-x");
    let created = &row(&agents, id).agent;
    assert_eq!(
        created.launch,
        json!({ "command": "/usr/bin/true", "args": ["--flag", "a b"], "env": {} })
    );
    assert_eq!(created.settings, json!({}));
    assert_eq!(created.transport, Transport::Cli);
    assert!(!created.enabled);

    match serve(&stack.backend, &create).await {
        StoreReply::Failed { request, message } => {
            assert_eq!(request, "create_agent");
            assert!(
                message.contains("agent_name_key"),
                "the store's sentence: {message}"
            );
        }
        other => panic!("a taken name is refused: {}", describe(&other)),
    }
    assert_eq!(
        stack.agents().await.len(),
        4,
        "the second create wrote nothing"
    );

    stack.drop_db().await;
}

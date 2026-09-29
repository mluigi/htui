//! `Settings > Agents` registry writes, from the worker side out (MOD-23 plan D239-D241, blueprint
//! D250, §4.5).
//!
//! Every case drives `htui::store_worker::serve` directly over a `Backend`, as
//! `tests/box_settings.rs` does: one request in, one reply out, no channels and no shell. They run
//! over `MemStore`, or over an offline `CacheStore` for the refusal case; the Postgres halves are
//! `tests/agent_settings_pg.rs`.
//!
//! No assertion message here prints a `launch` document or a whole reply: a reply carries the
//! registry, and `launch.env` is not for a log (`R-SEC-2`). [`describe`] names a reply by its
//! outcome instead.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::agent_settings::{self, AgentDraft, AgentWrite, REQUEST_NAMES};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{AgentId, AgentSummary, Billing, Transport};
use htui_core::store::{CasOutcome, MemStore, StoreError, WriteStore};
use htui_store::{Backend, CacheStore, PgStore, REGISTRY_ON_SERVER_ONLY};
use serde_json::json;

/// The demo world behind a memory backend: three registry rows and one box, which is this box.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// A valid draft with a literal command, so nothing about it needs a probe.
fn draft() -> AgentDraft {
    AgentDraft {
        transport: Transport::Acp,
        command: "/usr/bin/true".to_owned(),
        args: vec!["--flag".to_owned()],
        models: vec!["m1".to_owned(), "m2".to_owned()],
        default_model: Some("m1".to_owned()),
        billing: Billing::Subscription,
        enabled: true,
    }
}

/// A reply named by what it says, never by the registry it carries.
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

/// The request name and sentence a `Failed` carries, or a panic naming what came back.
#[track_caller]
fn refusal(reply: StoreReply) -> (&'static str, String) {
    match reply {
        StoreReply::Failed { request, message } => (request, message),
        other => panic!("expected a refusal: {}", describe(&other)),
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

/// The registry as the backend reads it now.
async fn registry(backend: &Backend) -> Vec<AgentSummary> {
    backend
        .agents()
        .await
        .expect("the memory store never fails")
}

/// Plan D239: the worker mints the id and the clock, `launch` is `{command, args, env: {}}`, and
/// `settings` is the source row's (OQ-5). The reply is the registry re-read, ordered by name.
#[tokio::test]
async fn create_agent_lands_with_a_minted_id_and_answers_created() {
    let backend = demo();
    let source = row(&registry(&backend).await, ids::AGENT_CLAUDE)
        .agent
        .settings
        .clone();

    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::CreateAgent {
                name: "agent-x".to_owned(),
                draft: draft(),
                settings_from: Some(ids::AGENT_CLAUDE),
            },
        )
        .await,
    );

    let AgentWrite::Created { id, name } = outcome else {
        panic!("a create answers Created: {outcome:?}");
    };
    assert_eq!(name, "agent-x");
    assert!(
        ![ids::AGENT_CLAUDE, ids::AGENT_AGY, ids::AGENT_CLAUDE_CLI].contains(&id),
        "the worker minted a fresh id"
    );
    assert_eq!(agents.len(), 4, "the re-read holds the new row");
    assert!(
        agents
            .windows(2)
            .all(|pair| pair[0].agent.name <= pair[1].agent.name),
        "the re-read is ordered by name"
    );
    let created = &row(&agents, id).agent;
    assert_eq!(created.name, "agent-x");
    assert_eq!(
        created.launch,
        json!({ "command": "/usr/bin/true", "args": ["--flag"], "env": {} }),
        "a blank launch: no discovery, no env"
    );
    assert_eq!(
        created.settings, source,
        "settings are the source row's (OQ-5)"
    );
    assert_eq!(created.transport, Transport::Acp);
    assert_eq!(created.models, draft().models);
    assert_eq!(created.default_model, draft().default_model);
    assert_eq!(created.billing, Billing::Subscription);
    assert!(created.enabled);
    assert_eq!(created.created_at, created.updated_at);
    assert_eq!(
        registry(&backend).await.len(),
        4,
        "the reply is the store's own read"
    );
}

/// OQ-5: with no source row the new row's `settings` is `{}`.
#[tokio::test]
async fn create_agent_without_a_source_starts_from_empty_settings() {
    let backend = demo();
    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::CreateAgent {
                name: "agent-x".to_owned(),
                draft: draft(),
                settings_from: None,
            },
        )
        .await,
    );
    let AgentWrite::Created { id, .. } = outcome else {
        panic!("a create answers Created: {outcome:?}");
    };
    assert_eq!(row(&agents, id).agent.settings, json!({}));
}

/// A source row gone by the time the worker reads is not a reason to refuse: the header named it,
/// and the new row starts from `{}`.
#[tokio::test]
async fn create_agent_from_a_vanished_source_starts_from_empty_settings() {
    let backend = demo();
    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::CreateAgent {
                name: "agent-x".to_owned(),
                draft: draft(),
                settings_from: Some(AgentId::new()),
            },
        )
        .await,
    );
    let AgentWrite::Created { id, .. } = outcome else {
        panic!("a create answers Created: {outcome:?}");
    };
    assert_eq!(row(&agents, id).agent.settings, json!({}));
}

/// The store's `UNIQUE` is the authority (plan D234): a taken name is `Failed` with the store's
/// sentence, and nothing is written.
#[tokio::test]
async fn create_agent_with_a_taken_name_is_failed_with_the_store_sentence() {
    let backend = demo();
    let taken = row(&registry(&backend).await, ids::AGENT_CLAUDE)
        .agent
        .name
        .clone();

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::CreateAgent {
                name: taken,
                draft: draft(),
                settings_from: None,
            },
        )
        .await,
    );

    assert_eq!(request, "create_agent");
    assert!(
        message.contains("agent_name_key"),
        "the store's sentence: {message}"
    );
    assert_eq!(registry(&backend).await.len(), 3, "nothing was written");
}

/// Blueprint D250: a name or a draft the section would refuse is refused by the worker with the
/// section's own sentence, byte for byte, and nothing is read or written.
#[tokio::test]
async fn create_agent_with_a_bad_name_is_failed_with_the_sections_sentence() {
    let backend = demo();

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::CreateAgent {
                name: "Bad Name".to_owned(),
                draft: draft(),
                settings_from: None,
            },
        )
        .await,
    );
    assert_eq!(request, "create_agent");
    assert_eq!(
        message,
        agent_settings::parse_name("Bad Name")
            .expect_err("the section refuses it")
            .to_string()
    );

    let blank = AgentDraft {
        command: "  ".to_owned(),
        ..draft()
    };
    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::CreateAgent {
                name: "agent-x".to_owned(),
                draft: blank.clone(),
                settings_from: None,
            },
        )
        .await,
    );
    assert_eq!(request, "create_agent");
    assert_eq!(
        message,
        agent_settings::check_draft(&blank)
            .expect_err("the section refuses it")
            .to_string()
    );
    assert!(message.starts_with("`command`:"), "{message}");
    assert_eq!(registry(&backend).await.len(), 3, "nothing was written");
}

/// Plan D235, D239: an edit is a compare-and-set that keeps `name`, `settings`, `created_at` and
/// every `launch` key but `command` and `args`, `env` and `discovery` included.
#[tokio::test]
async fn edit_agent_applies_and_keeps_name_env_discovery_and_settings() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    // An `env` placeholder, so the carry is observable. A placeholder, never a value.
    let mut seeded = row(&registry(&backend).await, ids::AGENT_CLAUDE)
        .agent
        .clone();
    seeded.launch["env"] = json!({ "TOKEN": "${claude_token}" });
    let applied = store
        .upsert_agent(&seeded, Some(seeded.updated_at))
        .await
        .expect("the memory store never fails");
    assert!(matches!(applied, CasOutcome::Applied(_)), "the seed edit");
    let before = row(&registry(&backend).await, ids::AGENT_CLAUDE)
        .agent
        .clone();
    assert!(
        before.launch.get("discovery").is_some(),
        "the demo row has a discovery block to keep"
    );

    let mut edit = agent_settings::draft_of(&before);
    edit.args.push("--verbose".to_owned());
    edit.models = vec!["opus".to_owned(), "sonnet".to_owned()];
    edit.default_model = Some("sonnet".to_owned());
    edit.billing = Billing::PerToken;
    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: before.updated_at,
                draft: edit.clone(),
            },
        )
        .await,
    );

    assert_eq!(
        outcome,
        AgentWrite::Edited {
            id: ids::AGENT_CLAUDE,
            name: before.name.clone(),
        }
    );
    let after = &row(&agents, ids::AGENT_CLAUDE).agent;
    assert_eq!(after.name, before.name);
    assert_eq!(after.settings, before.settings);
    assert_eq!(after.created_at, before.created_at);
    assert!(
        after.launch.get("env") == before.launch.get("env"),
        "`launch.env` is carried unchanged"
    );
    assert!(
        after.launch.get("discovery") == before.launch.get("discovery"),
        "`launch.discovery` is carried unchanged"
    );
    assert_eq!(after.launch["command"], json!(edit.command));
    assert_eq!(after.launch["args"], json!(edit.args));
    assert_eq!(after.models, edit.models);
    assert_eq!(after.default_model, edit.default_model);
    assert_eq!(after.billing, Billing::PerToken);
    assert!(after.updated_at > before.updated_at, "the store stamped it");
}

/// MOD-40 D5: a second edit on the first edit's token is `Stale` and writes nothing.
#[tokio::test]
async fn edit_agent_with_a_spent_token_answers_stale_and_writes_nothing() {
    let backend = demo();
    let before = row(&registry(&backend).await, ids::AGENT_CLAUDE)
        .agent
        .clone();

    let mut first = agent_settings::draft_of(&before);
    first.models = vec!["a".to_owned()];
    first.default_model = Some("a".to_owned());
    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: before.updated_at,
                draft: first,
            },
        )
        .await,
    );
    assert!(
        matches!(outcome, AgentWrite::Edited { .. }),
        "the first edit applies: {outcome:?}"
    );
    let edited = row(&agents, ids::AGENT_CLAUDE).agent.clone();

    let mut second = agent_settings::draft_of(&before);
    second.models = vec!["b".to_owned()];
    second.default_model = Some("b".to_owned());
    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: before.updated_at,
                draft: second,
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
    assert!(
        row(&agents, ids::AGENT_CLAUDE).agent == edited,
        "the spent token wrote nothing: the row is the first edit's"
    );
}

/// An id no read lists is `Gone`, and the reply still carries the registry.
#[tokio::test]
async fn edit_agent_on_an_unknown_id_answers_gone() {
    let backend = demo();
    let id = AgentId::new();
    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::EditAgent {
                agent_id: id,
                expected: Utc::now(),
                draft: draft(),
            },
        )
        .await,
    );
    assert_eq!(outcome, AgentWrite::Gone { id });
    assert_eq!(agents.len(), 3);
}

/// Plan D247: the worker runs the section's rules again; a draft they refuse is `Failed` with the
/// refused field's sentence, and the row is untouched.
#[tokio::test]
async fn edit_agent_with_an_invalid_draft_is_failed_by_field() {
    let backend = demo();
    let before = row(&registry(&backend).await, ids::AGENT_CLAUDE)
        .agent
        .clone();
    let bad = AgentDraft {
        models: vec!["a".to_owned()],
        default_model: Some("b".to_owned()),
        ..agent_settings::draft_of(&before)
    };

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::EditAgent {
                agent_id: ids::AGENT_CLAUDE,
                expected: before.updated_at,
                draft: bad.clone(),
            },
        )
        .await,
    );

    assert_eq!(request, "edit_agent");
    assert!(message.starts_with("`default model`:"), "{message}");
    assert_eq!(
        message,
        agent_settings::check_draft(&bad)
            .expect_err("the section refuses it")
            .to_string()
    );
    assert!(
        row(&registry(&backend).await, ids::AGENT_CLAUDE).agent == before,
        "nothing was written"
    );
}

/// Plan D242: off, then on. The demo holds no `agent_box` row, so the switch writes a bare one;
/// switching on with no probe document is `enabled`.
#[tokio::test]
async fn set_agent_on_box_off_then_on_answers_switched_and_the_summary_says_so() {
    let backend = demo();
    let name = row(&registry(&backend).await, ids::AGENT_AGY)
        .agent
        .name
        .clone();

    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::SetAgentOnBox {
                agent_id: ids::AGENT_AGY,
                enabled: false,
            },
        )
        .await,
    );
    assert_eq!(
        outcome,
        AgentWrite::Switched {
            id: ids::AGENT_AGY,
            name: name.clone(),
            enabled: false,
        }
    );
    let off = row(&agents, ids::AGENT_AGY);
    assert!(off.user_off, "the summary carries the switch");
    assert!(
        !off.on_box
            .as_ref()
            .expect("the switch wrote a bare row")
            .enabled
    );

    let (agents, outcome) = written(
        serve(
            &backend,
            &StoreRequest::SetAgentOnBox {
                agent_id: ids::AGENT_AGY,
                enabled: true,
            },
        )
        .await,
    );
    assert_eq!(
        outcome,
        AgentWrite::Switched {
            id: ids::AGENT_AGY,
            name,
            enabled: true,
        }
    );
    let on = row(&agents, ids::AGENT_AGY);
    assert!(!on.user_off);
    assert!(
        on.on_box.as_ref().expect("the row stays").enabled,
        "no probe document: the switch alone decides"
    );
}

/// Blueprint F-23: before this box is registered there is no `agent_box` row to switch, and the
/// refusal says so.
#[tokio::test]
async fn set_agent_on_box_before_registration_is_failed_naming_the_box() {
    let backend = Backend::memory(MemStore::new());
    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::SetAgentOnBox {
                agent_id: AgentId::new(),
                enabled: false,
            },
        )
        .await,
    );
    assert_eq!(request, "set_agent_on_box");
    assert_eq!(
        message,
        "this box is not registered yet; the per-box switch needs its agent_box row"
    );
}

/// MOD-25, plan D241: offline there is no writer, and all three are refused by their own names
/// with the registry sentence before any read (blueprint F-7: `contains`, because `Unreachable`'s
/// `Display` prefixes it).
#[tokio::test]
async fn offline_refuses_all_three_by_name_with_the_registry_sentence() {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(
        root.path(),
        "agent-settings-offline",
        PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache,
        since: Some(Utc::now()),
    };

    let requests = [
        StoreRequest::CreateAgent {
            name: "agent-x".to_owned(),
            draft: draft(),
            settings_from: None,
        },
        StoreRequest::EditAgent {
            agent_id: ids::AGENT_CLAUDE,
            expected: Utc::now(),
            draft: draft(),
        },
        StoreRequest::SetAgentOnBox {
            agent_id: ids::AGENT_CLAUDE,
            enabled: false,
        },
    ];
    for (request, name) in requests.into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(REGISTRY_ON_SERVER_ONLY),
            "`{name}` is refused with the registry sentence: {message}"
        );
    }
}

/// `agent_settings::serve` is reachable only through `try_serve`'s or-ed arm, so a request from
/// anywhere else is told which one it sent rather than panicking.
#[tokio::test]
async fn a_request_from_elsewhere_is_named_not_panicked() {
    let err = agent_settings::serve(&demo(), &StoreRequest::BoxInfo)
        .await
        .expect_err("not an agent registry request");
    match err {
        StoreError::Backend(message) => {
            assert!(message.contains("box_info"), "{message}");
        }
        other => panic!("expected a backend error: {other}"),
    }
}

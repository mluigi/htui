//! MOD-11 T7: `search_concepts` through `McpHost::client` (blueprint §10, §2.10, §2.11; plan
//! I-1, I-7, R-STO-8).
//!
//! The concept index is a [`ConceptSearch`] double that records every query it is asked and
//! answers what the case scripted. The host is `McpHost<Backend>` over a `MemStore::demo()`
//! (blueprint B-1): the tool never touches the store, but a session needs a writable backend to
//! open. The scope is built here, never from tool arguments (I-1).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use htui_core::fixtures::ids;
use htui_core::model::{ItemId, ProjectId, RunId, Status, StepId, Transport};
use htui_core::prompt::render::HostnameLine;
use htui_core::store::{MemStore, StepFence};
use htui_mcp::protocol::CallResult;
use htui_mcp::search::{ConceptHit, ConceptQuery, ConceptSearch, ConceptType, OwnerKind};
use htui_mcp::{ENV_TOKEN, McpClient, McpHost};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// A concept index that records its queries and answers a scripted outcome.
#[derive(Debug)]
struct Recording {
    /// Every query asked, in order.
    queries: Mutex<Vec<ConceptQuery>>,
    /// What every search answers.
    answer: Result<Vec<ConceptHit>, String>,
}

impl Recording {
    fn answering(answer: Result<Vec<ConceptHit>, String>) -> Arc<Self> {
        Arc::new(Self {
            queries: Mutex::new(Vec::new()),
            answer,
        })
    }

    fn empty() -> Arc<Self> {
        Self::answering(Ok(Vec::new()))
    }

    fn queries(&self) -> Vec<ConceptQuery> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ConceptSearch for Recording {
    fn search(
        &self,
        query: ConceptQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ConceptHit>, String>> + Send + '_>> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(query);
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}

/// A phase step's scope in `project`: an item, a kind, a lease.
fn phase_scope(project: ProjectId) -> ToolScope {
    ToolScope {
        run_id: RunId::new(),
        step_id: StepId::new(),
        project_id: project,
        item_id: Some(ItemId::new()),
        box_id: ids::BOX,
        user: ids::USER,
        fence: StepFence::Lease(uuid::Uuid::new_v4()),
        output_kind: Some("plan".to_owned()),
        hostname: HostnameLine::Omitted,
        command_queue: false,
        cwd: std::env::temp_dir(),
        transport: Transport::Acp,
    }
}

/// A fresh chat's scope (OQ-8): no item, no kind, no lease.
fn fresh_chat_scope() -> ToolScope {
    ToolScope {
        item_id: None,
        output_kind: None,
        fence: StepFence::Unleased,
        ..phase_scope(ids::PROJECT_HTUI)
    }
}

/// A host over the demo store, with `search` attached when given.
fn host(search: Option<Arc<Recording>>) -> McpHost<Backend> {
    let host = McpHost::new(Backend::memory(MemStore::demo())).expect("a host");
    match search {
        Some(search) => host.with_search(search),
        None => host,
    }
}

/// A live session on `scope` and its initialised client. The host and the lease must outlive the
/// client's calls: dropping either ends the session.
async fn open(host: &McpHost<Backend>, scope: ToolScope) -> (ToolLease, McpClient) {
    let lease = host.open(scope).expect("a lease");
    let mut client = host
        .client(&lease.spec.env[ENV_TOKEN])
        .expect("a live session");
    client.initialize().await.expect("initialize");
    (lease, client)
}

/// The successful call's JSON.
fn ok(result: &CallResult) -> Value {
    assert!(!result.is_error, "{}", result.text);
    serde_json::from_str(&result.text).expect("the result is JSON")
}

/// The refused call's one-line reason.
fn refused(result: &CallResult) -> &str {
    assert!(result.is_error, "expected a refusal, got {}", result.text);
    &result.text
}

fn requirement_hit() -> ConceptHit {
    ConceptHit {
        point_type: ConceptType::Requirement,
        owner_kind: OwnerKind::Requirement,
        key: "R-STO-8".to_owned(),
        document_kind: None,
        resolution: None,
        state: Some("active".to_owned()),
        score: 0.5,
        snippet: "A search is always scoped.".to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn search_is_scoped_to_the_scopes_project() {
    let index = Recording::empty();
    let host = host(Some(Arc::clone(&index)));
    let project = ProjectId::new();
    let (_lease, mut client) = open(&host, phase_scope(project)).await;

    let answer = ok(&client
        .call("search_concepts", json!({"query": "scoped search"}))
        .await
        .expect("a call"));
    assert_eq!(answer, json!({"hits": []}));

    let narrowed = ok(&client
        .call(
            "search_concepts",
            json!({
                "query": "decisions",
                "types": ["item", "document"],
                "statuses": ["closed"],
                "limit": 3
            }),
        )
        .await
        .expect("a call"));
    assert_eq!(narrowed, json!({"hits": []}));

    assert_eq!(
        index.queries(),
        [
            ConceptQuery {
                text: "scoped search".to_owned(),
                project,
                types: Vec::new(),
                statuses: Vec::new(),
                limit: 10,
            },
            ConceptQuery {
                text: "decisions".to_owned(),
                project,
                types: vec![ConceptType::Item, ConceptType::Document],
                statuses: vec![Status::Closed],
                limit: 3,
            },
        ],
        "the project is the scope's, the limit defaults to ten"
    );

    // I-1: the project never comes from the arguments.
    for naming in [
        json!({"query": "x", "project": ids::PROJECT_HTUI}),
        json!({"query": "x", "projects": [ids::PROJECT_HTUI]}),
    ] {
        let answer = client
            .call("search_concepts", naming)
            .await
            .expect("a call");
        assert!(
            refused(&answer).starts_with("invalid arguments: unknown field `project"),
            "{}",
            answer.text
        );
    }
    assert_eq!(index.queries().len(), 2, "a refused call never searches");
}

#[tokio::test]
async fn requirement_hits_carry_owner_kind_requirement() {
    let document = ConceptHit {
        point_type: ConceptType::Document,
        owner_kind: OwnerKind::Item,
        key: "HTUI-12".to_owned(),
        document_kind: Some("plan".to_owned()),
        resolution: Some("done".to_owned()),
        state: None,
        score: 2.0,
        snippet: "The plan.".to_owned(),
    };
    let index = Recording::answering(Ok(vec![document, requirement_hit()]));
    let host = host(Some(index));
    let (_lease, mut client) = open(&host, phase_scope(ids::PROJECT_HTUI)).await;

    let answer = ok(&client
        .call("search_concepts", json!({"query": "scoped"}))
        .await
        .expect("a call"));
    assert_eq!(
        answer,
        json!({"hits": [
            {
                "point_type": "document",
                "owner_kind": "item",
                "key": "HTUI-12",
                "document_kind": "plan",
                "resolution": "done",
                "score": 2.0,
                "snippet": "The plan."
            },
            {
                "point_type": "requirement",
                "owner_kind": "requirement",
                "key": "R-STO-8",
                "state": "active",
                "score": 0.5,
                "snippet": "A search is always scoped."
            }
        ]})
    );
}

#[tokio::test]
async fn an_unavailable_index_is_is_error_with_its_cause() {
    let cause = "Qdrant is unreachable at http://qdrant:6334";
    let index = Recording::answering(Err(cause.to_owned()));
    let host = host(Some(Arc::clone(&index)));
    let (_lease, mut client) = open(&host, phase_scope(ids::PROJECT_HTUI)).await;

    let answer = client
        .call("search_concepts", json!({"query": "anything"}))
        .await
        .expect("a call");
    assert_eq!(refused(&answer), format!("search unavailable: {cause}"));
    assert_eq!(index.queries().len(), 1);
}

#[tokio::test]
async fn unknown_types_are_refused() {
    let index = Recording::empty();
    let host = host(Some(Arc::clone(&index)));
    let (_lease, mut client) = open(&host, phase_scope(ids::PROJECT_HTUI)).await;

    let answer = client
        .call(
            "search_concepts",
            json!({"query": "anything", "types": ["item", "note"]}),
        )
        .await
        .expect("a call");
    assert_eq!(refused(&answer), "invalid arguments: unknown type note");

    let answer = client
        .call(
            "search_concepts",
            json!({"query": "anything", "statuses": ["shipped"]}),
        )
        .await
        .expect("a call");
    assert!(
        refused(&answer).starts_with("invalid arguments: unknown variant `shipped`"),
        "{}",
        answer.text
    );
    assert!(index.queries().is_empty(), "a refused call never searches");

    ok(&client
        .call(
            "search_concepts",
            json!({"query": "anything", "types": ["item", "document", "requirement"]}),
        )
        .await
        .expect("a call"));
    assert_eq!(
        index.queries()[0].types,
        ConceptType::ALL,
        "every wire name parses"
    );
}

#[tokio::test]
async fn a_limit_above_twenty_is_refused() {
    let index = Recording::empty();
    let host = host(Some(Arc::clone(&index)));
    let (_lease, mut client) = open(&host, phase_scope(ids::PROJECT_HTUI)).await;

    for limit in [21, 0] {
        let answer = client
            .call("search_concepts", json!({"query": "x", "limit": limit}))
            .await
            .expect("a call");
        assert_eq!(
            refused(&answer),
            format!("invalid arguments: limit {limit} is outside 1..=20")
        );
    }
    assert!(index.queries().is_empty(), "a refused call never searches");

    for limit in [1, 20] {
        ok(&client
            .call("search_concepts", json!({"query": "x", "limit": limit}))
            .await
            .expect("a call"));
    }
    let limits: Vec<u64> = index.queries().iter().map(|q| q.limit).collect();
    assert_eq!(limits, [1, 20]);
}

#[tokio::test]
async fn search_concepts_is_not_advertised_without_a_search_handle() {
    let host = host(None);
    for scope in [phase_scope(ids::PROJECT_HTUI), fresh_chat_scope()] {
        let (_lease, mut client) = open(&host, scope).await;
        let names = client.tool_names().await.expect("tools/list");
        assert!(
            !names.iter().any(|name| name == "search_concepts"),
            "{names:?}"
        );
        let answer = client
            .call("search_concepts", json!({"query": "anything"}))
            .await
            .expect("a call");
        assert_eq!(refused(&answer), "unknown tool: search_concepts");
    }
}

/// OQ-8: a fresh chat (`item_id NULL`, unleased) on a host with a concept index sees
/// `box_profile` and `search_concepts`, and nothing else; the `tools/list` entry is pinned.
#[tokio::test]
async fn a_fresh_chat_scope_sees_box_profile_and_search_concepts() {
    let host = host(Some(Recording::empty()));
    let (_lease, mut client) = open(&host, fresh_chat_scope()).await;

    let names = client.tool_names().await.expect("tools/list");
    assert_eq!(names, ["box_profile", "search_concepts"]);

    let listed = client
        .request("tools/list", json!({}))
        .await
        .expect("tools/list");
    insta::assert_snapshot!(
        serde_json::to_string_pretty(&listed["result"]["tools"]).expect("serialises")
    );
}

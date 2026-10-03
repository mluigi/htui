//! MOD-11 T4: the backlog write tools (`document_write`, `note_add`, `item_status`,
//! `item_link`) through `McpHost::client` (blueprint §7, §2.10; plan I-1, I-3..I-7).
//!
//! Every case hosts `McpHost<Backend>` over a `MemStore::demo()` (blueprint B-1) seeded with one
//! run, claimed by an owner and carrying one step, as the store conformance's `leased_step` does.
//! The session's scope is built here, never from tool arguments (I-1); its fence is the owner's
//! lease, so a stranger taking the run fences every write (I-3).

use chrono::{TimeDelta, Utc};
use htui_core::fixtures::ids;
use htui_core::model::{
    Claim, Document, DocumentId, GraphSnapshot, Isolation, ItemId, NewRun, NewRunStep, Note,
    NoteId, ProjectId, RunId, RunMode, SnapshotGraph, SnapshotSettings, StepId, Transport,
};
use htui_core::prompt::render::HostnameLine;
use htui_core::scrub::{MinimalScrubber, Scrubber, Unmasked};
use htui_core::store::{MemStore, ReadStore, StepFence, WriteStore};
use htui_mcp::protocol::CallResult;
use htui_mcp::{ENV_TOKEN, McpClient, McpHost};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// The secret the masking case's scrubber knows.
const SECRET: &str = "fake-secret-4c3b2a";

/// The phase output kind a document-writing scope carries.
const KIND: &str = "plan";

/// The smallest snapshot `create_run` accepts (the store conformance's `run_snapshot`).
fn snapshot() -> GraphSnapshot {
    GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: ids::GRAPH_HTUI_FEAT,
            name: "feature".to_owned(),
            is_override: false,
        },
        topology: "sha256:tools-backlog".to_owned(),
        mode: RunMode::Manual,
        phases: Vec::new(),
        settings: SnapshotSettings {
            default_isolation: Isolation::Worktree,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            max_fan_out: 4,
            max_agents_per_run: 8,
        },
        scope: None,
        personas: Vec::new(),
    }
}

/// A run of `item` in `project`, claimed by `owner` for five minutes, with one step.
#[derive(Debug)]
struct Leased {
    store: MemStore,
    project: ProjectId,
    item: ItemId,
    run: RunId,
    step: StepId,
    owner: Uuid,
}

impl Leased {
    async fn on(project: ProjectId, item: ItemId) -> Self {
        let store = MemStore::demo();
        let owner = Uuid::new_v4();
        let run = RunId::new();
        store
            .create_run(NewRun {
                id: run,
                project_id: project,
                item_id: item,
                mode: RunMode::Manual,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: snapshot(),
                repo_scope: Vec::new(),
                queued_at: Utc::now(),
            })
            .await
            .expect("the run is created");
        assert_eq!(
            store
                .claim_run(run, ids::BOX, owner, Utc::now(), TimeDelta::minutes(5))
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "the executor claims the run"
        );
        let step = store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: 0,
                phase_name: "implement".to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: Some("opus".to_owned()),
            })
            .await
            .expect("the step is created")
            .id;
        Self {
            store,
            project,
            item,
            run,
            step,
            owner,
        }
    }

    /// `htui` `ANA-2`, open: the item every case writes on unless it needs a link the demo has.
    async fn ana_2() -> Self {
        Self::on(ids::PROJECT_HTUI, ids::HTUI_ANA_2).await
    }

    /// The scope a phase step of this run gets: the item, the output kind, the owner's fence.
    fn scope(&self) -> ToolScope {
        ToolScope {
            run_id: self.run,
            step_id: self.step,
            project_id: self.project,
            item_id: Some(self.item),
            box_id: ids::BOX,
            user: ids::USER,
            fence: StepFence::Lease(self.owner),
            output_kind: Some(KIND.to_owned()),
            hostname: HostnameLine::Omitted,
            command_queue: false,
            cwd: std::env::temp_dir(),
            transport: Transport::Acp,
        }
    }

    /// A host over this store with the default scrubber.
    fn host(&self) -> McpHost<Backend> {
        McpHost::new(Backend::memory(self.store.clone())).expect("a host")
    }

    /// The run's lease lapses and a stranger takes it, as a sweep's adoption does.
    async fn take_the_lease(&self) {
        assert!(
            self.store
                .refresh_lease(self.run, self.owner, TimeDelta::zero())
                .await
                .expect("a refresh"),
            "the owner's lease lapses"
        );
        assert!(
            self.store
                .take_lease(self.run, ids::BOX, Uuid::new_v4(), TimeDelta::minutes(14))
                .await
                .expect("a take"),
            "a stranger takes the lapsed lease"
        );
    }

    async fn documents(&self) -> usize {
        self.store
            .documents(self.item)
            .await
            .expect("documents")
            .len()
    }

    async fn notes(&self) -> Vec<Note> {
        self.store.notes(self.item).await.expect("notes")
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

/// A scrubber that cannot mask anything it is given: every text is `Unmasked` (fail closed, I-5).
#[derive(Debug)]
struct Unmaskable;

impl Scrubber for Unmaskable {
    fn scrub(&self, _: &mut Value) -> Result<(), Unmasked> {
        Err(Unmasked {
            path: String::new(),
            rule: "test_rule",
        })
    }
}

// ---------------------------------------------------------------------------------------------
// document_write
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn document_write_writes_the_phase_kind_on_the_scope_item() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;

    let first = ok(&client
        .call("document_write", json!({"body": "the first plan"}))
        .await
        .expect("a call"));
    assert_eq!(first["kind"], KIND);
    assert_eq!(first["version"], 1);
    let second = ok(&client
        .call(
            "document_write",
            json!({"title": "The plan, again", "body": "the second plan"}),
        )
        .await
        .expect("a call"));
    assert_eq!(second["kind"], KIND);
    assert_eq!(second["version"], 2, "each call writes a new version");

    let read = |answer: &Value| {
        let id: DocumentId =
            serde_json::from_value(answer["document_id"].clone()).expect("a document id");
        let store = fx.store.clone();
        async move {
            store
                .document(id)
                .await
                .expect("a read")
                .expect("the document exists")
        }
    };
    let v1: Document = read(&first).await;
    assert_eq!(
        (v1.item_id, v1.kind.as_str(), v1.version, v1.title.as_str()),
        (fx.item, KIND, 1, KIND),
        "the title defaults to the kind"
    );
    assert_eq!(v1.body, "the first plan");
    assert_eq!(v1.produced_by_step_id, Some(fx.step));
    assert_eq!(v1.created_by, ids::USER);
    let v2 = read(&second).await;
    assert_eq!((v2.version, v2.title.as_str()), (2, "The plan, again"));
    assert_eq!(v2.produced_by_step_id, Some(fx.step));
}

#[tokio::test]
async fn document_write_is_not_advertised_without_an_item_or_a_kind() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let full = fx.scope();
    for (shape, on) in [
        (
            "no kind",
            ToolScope {
                output_kind: None,
                ..full.clone()
            },
        ),
        (
            "no item",
            ToolScope {
                item_id: None,
                ..full.clone()
            },
        ),
        (
            "neither",
            ToolScope {
                item_id: None,
                output_kind: None,
                ..full.clone()
            },
        ),
    ] {
        let (_lease, mut client) = open(&host, on).await;
        let names = client.tool_names().await.expect("tools/list");
        assert!(
            !names.iter().any(|name| name == "document_write"),
            "{shape}: {names:?}"
        );
        let answer = client
            .call("document_write", json!({"body": "x"}))
            .await
            .expect("a call");
        assert_eq!(refused(&answer), "unknown tool: document_write", "{shape}");
    }
    let (_lease, mut client) = open(&host, full).await;
    let names = client.tool_names().await.expect("tools/list");
    assert!(
        names.iter().any(|name| name == "document_write"),
        "{names:?}"
    );
    assert_eq!(fx.documents().await, 0, "nothing was written");
}

/// I-3: the session's fence is the owner's lease; once a stranger holds the run, nothing lands.
#[tokio::test]
async fn document_write_after_the_lease_moved_is_fenced() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let before = fx.documents().await;
    fx.take_the_lease().await;
    let answer = client
        .call("document_write", json!({"body": "too late"}))
        .await
        .expect("a call");
    assert_eq!(refused(&answer), "fenced: lease lost");
    assert_eq!(
        fx.documents().await,
        before,
        "the fenced write wrote no row"
    );
}

/// I-5: a known secret is masked before the row is written; a text the scrubber cannot mask is
/// refused by rule, never echoed, and writes nothing.
#[tokio::test]
async fn document_write_masks_a_known_secret_and_refuses_an_unmaskable_body() {
    let fx = Leased::ana_2().await;
    let host = fx
        .host()
        .with_scrubber(Arc::new(MinimalScrubber::new([SECRET.to_owned()])));
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let answer = ok(&client
        .call(
            "document_write",
            json!({"title": format!("about {SECRET}"), "body": format!("the key is {SECRET}.")}),
        )
        .await
        .expect("a call"));
    let id: DocumentId = serde_json::from_value(answer["document_id"].clone()).expect("an id");
    let written = fx
        .store
        .document(id)
        .await
        .expect("a read")
        .expect("written");
    assert!(!written.body.contains(SECRET), "{}", written.body);
    assert!(!written.title.contains(SECRET), "{}", written.title);
    assert_eq!(written.body, "the key is [REDACTED].");

    let before = fx.documents().await;
    let host = fx.host().with_scrubber(Arc::new(Unmaskable));
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let body = "nothing a scrubber could mask";
    let answer = client
        .call("document_write", json!({"body": body}))
        .await
        .expect("a call");
    assert_eq!(
        refused(&answer),
        "refused: the text matched credential rule test_rule; nothing was written"
    );
    assert!(!answer.text.contains(body));
    assert_eq!(fx.documents().await, before, "nothing was written");
}

// ---------------------------------------------------------------------------------------------
// note_add
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn note_add_writes_a_note_via_the_step() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let answer = ok(&client
        .call("note_add", json!({"body": "the tree was dirty"}))
        .await
        .expect("a call"));
    let id: NoteId = serde_json::from_value(answer["note_id"].clone()).expect("a note id");
    let notes = fx.notes().await;
    let note = notes
        .iter()
        .find(|note| note.id == id)
        .expect("the note is on the scope's item");
    assert_eq!(note.body, "the tree was dirty");
    assert_eq!(note.via_step_id, Some(fx.step));
    assert_eq!(note.created_by, ids::USER);
    assert_eq!(note.box_id, Some(ids::BOX));
}

#[tokio::test]
async fn note_add_over_16_kib_is_refused() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let before = fx.notes().await.len();
    let answer = client
        .call("note_add", json!({"body": "x".repeat(16_385)}))
        .await
        .expect("a call");
    assert_eq!(
        refused(&answer),
        "refused: note is 16385 bytes, the limit is 16384"
    );
    assert_eq!(fx.notes().await.len(), before, "nothing was written");
    ok(&client
        .call("note_add", json!({"body": "x".repeat(16_384)}))
        .await
        .expect("a call"));
    assert_eq!(fx.notes().await.len(), before + 1, "the limit itself fits");
}

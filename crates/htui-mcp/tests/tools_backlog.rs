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
    Claim, Document, DocumentId, GraphSnapshot, Isolation, ItemId, LinkKind, NewRun, NewRunStep,
    Note, NoteId, ProjectId, RunId, RunMode, SnapshotGraph, SnapshotSettings, StepId, Transport,
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

    /// Whether `item --kind--> to` is a live edge of the item's one-hop graph.
    async fn live_link(&self, to: ItemId, kind: LinkKind) -> bool {
        self.store
            .links(self.item, 1)
            .await
            .expect("links")
            .edges
            .iter()
            .any(|edge| {
                edge.from_item_id == self.item && edge.to_item_id == to && edge.kind == kind
            })
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

// ---------------------------------------------------------------------------------------------
// item_status
// ---------------------------------------------------------------------------------------------

/// I-4: a status request is a note through the step; the item's status and version stay put.
#[tokio::test]
async fn item_status_writes_a_note_and_never_moves_the_status() {
    let fx = Leased::ana_2().await;
    let before = fx
        .store
        .item(fx.item)
        .await
        .expect("a read")
        .expect("the item");
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;

    let done = ok(&client
        .call(
            "item_status",
            json!({"status": "done", "reason": "every test passes"}),
        )
        .await
        .expect("a call"));
    let closed = ok(&client
        .call(
            "item_status",
            json!({"status": "closed", "resolution": "superseded", "reason": "FEAT-1 covers it"}),
        )
        .await
        .expect("a call"));

    let notes = fx.notes().await;
    for (answer, body) in [
        (&done, "status request: done \u{2014} every test passes"),
        (
            &closed,
            "status request: closed (superseded) \u{2014} FEAT-1 covers it",
        ),
    ] {
        let id: NoteId = serde_json::from_value(answer["note_id"].clone()).expect("a note id");
        let note = notes
            .iter()
            .find(|note| note.id == id)
            .expect("the request is a note on the item");
        assert_eq!(note.body, body);
        assert_eq!(note.via_step_id, Some(fx.step));
    }
    let after = fx
        .store
        .item(fx.item)
        .await
        .expect("a read")
        .expect("the item");
    assert_eq!(
        (after.status, after.version, after.resolution),
        (before.status, before.version, before.resolution),
        "no agent moves a status"
    );
}

#[tokio::test]
async fn item_status_rejects_an_unknown_status_and_a_stray_resolution() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let before = fx.notes().await.len();

    let unknown = client
        .call(
            "item_status",
            json!({"status": "finished", "reason": "it is"}),
        )
        .await
        .expect("a call");
    assert!(
        refused(&unknown).starts_with("invalid arguments: unknown variant `finished`"),
        "{}",
        unknown.text
    );
    let stray = client
        .call(
            "item_status",
            json!({"status": "done", "resolution": "done", "reason": "it is"}),
        )
        .await
        .expect("a call");
    assert_eq!(
        refused(&stray),
        "refused: a resolution goes with closed only"
    );
    let bad_resolution = client
        .call(
            "item_status",
            json!({"status": "closed", "resolution": "abandoned", "reason": "it is"}),
        )
        .await
        .expect("a call");
    assert!(
        refused(&bad_resolution).starts_with("invalid arguments: unknown variant `abandoned`"),
        "{}",
        bad_resolution.text
    );
    assert_eq!(fx.notes().await.len(), before, "nothing was written");
}

// ---------------------------------------------------------------------------------------------
// item_link
// ---------------------------------------------------------------------------------------------

/// I-1: `to` is a key, resolved inside the scope's project: `htui`'s `FEAT-1`, not `agy`'s.
#[tokio::test]
async fn item_link_add_resolves_the_key_in_the_project() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let answer = ok(&client
        .call(
            "item_link",
            json!({"op": "add", "to": "FEAT-1", "kind": "relates"}),
        )
        .await
        .expect("a call"));
    assert_eq!(
        answer,
        json!({"from": "ANA-2", "to": "FEAT-1", "kind": "relates", "live": true})
    );
    assert!(fx.live_link(ids::HTUI_FEAT_1, LinkKind::Relates).await);
    assert!(!fx.live_link(ids::AGY_FEAT_1, LinkKind::Relates).await);
}

#[tokio::test]
async fn item_link_to_another_projects_key_is_out_of_scope() {
    // `agy` has `ANA-1`, `FEAT-1` and `FIX-1`; `FEAT-3` is `htui`'s only.
    let fx = Leased::on(ids::PROJECT_AGY, ids::AGY_FIX_1).await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let foreign_id = ids::HTUI_FEAT_3.to_string();
    for to in ["FEAT-3", foreign_id.as_str()] {
        for op in ["add", "remove"] {
            let answer = client
                .call("item_link", json!({"op": op, "to": to, "kind": "relates"}))
                .await
                .expect("a call");
            assert_eq!(
                refused(&answer),
                format!("out of scope: {to} is not an item of this project"),
                "{op}"
            );
        }
    }
    assert!(!fx.live_link(ids::HTUI_FEAT_3, LinkKind::Relates).await);
}

#[tokio::test]
async fn item_link_to_itself_is_refused() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let answer = client
        .call(
            "item_link",
            json!({"op": "add", "to": "ANA-2", "kind": "relates"}),
        )
        .await
        .expect("a call");
    assert!(
        refused(&answer).starts_with("refused: an item cannot link to itself"),
        "{}",
        answer.text
    );
    assert!(!fx.live_link(fx.item, LinkKind::Relates).await);
}

/// PRD OQ-4: the importer's `FIX-1 blocked_by ANA-1` is not this run's to withdraw.
#[tokio::test]
async fn item_link_remove_of_a_link_this_run_did_not_propose_is_not_yours() {
    let fx = Leased::on(ids::PROJECT_AGY, ids::AGY_FIX_1).await;
    assert!(fx.live_link(ids::AGY_ANA_1, LinkKind::BlockedBy).await);
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let answer = client
        .call(
            "item_link",
            json!({"op": "remove", "to": "ANA-1", "kind": "blocked_by"}),
        )
        .await
        .expect("a call");
    assert_eq!(
        refused(&answer),
        "not yours: FIX-1 blocked_by ANA-1 was not proposed by this run"
    );
    assert!(
        fx.live_link(ids::AGY_ANA_1, LinkKind::BlockedBy).await,
        "the link stays live"
    );
}

#[tokio::test]
async fn item_link_remove_of_its_own_proposal_tombstones_it() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let link = json!({"op": "add", "to": "ANA-1", "kind": "origin"});
    ok(&client.call("item_link", link).await.expect("a call"));
    assert!(fx.live_link(ids::HTUI_ANA_1, LinkKind::Origin).await);
    let answer = ok(&client
        .call(
            "item_link",
            json!({"op": "remove", "to": "ANA-1", "kind": "origin"}),
        )
        .await
        .expect("a call"));
    assert_eq!(
        answer,
        json!({"from": "ANA-2", "to": "ANA-1", "kind": "origin", "live": false})
    );
    assert!(
        !fx.live_link(ids::HTUI_ANA_1, LinkKind::Origin).await,
        "the link is tombstoned"
    );
}

// ---------------------------------------------------------------------------------------------
// Every backlog tool
// ---------------------------------------------------------------------------------------------

/// One valid call of each backlog tool, in table order.
fn backlog_calls() -> [(&'static str, Value); 4] {
    [
        ("document_write", json!({"body": "a plan"})),
        ("note_add", json!({"body": "a note"})),
        (
            "item_status",
            json!({"status": "done", "reason": "it works"}),
        ),
        (
            "item_link",
            json!({"op": "add", "to": "FEAT-1", "kind": "relates"}),
        ),
    ]
}

/// I-6: once the lease drops (or the host closes) every tool answers `session ended` and writes
/// nothing.
#[tokio::test]
async fn every_backlog_tool_after_the_session_ended_answers_session_ended() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (lease, mut dropped) = open(&host, fx.scope()).await;
    let (_closed_lease, mut closed) = open(&host, fx.scope()).await;
    let names = dropped.tool_names().await.expect("tools/list");
    for (name, _) in backlog_calls() {
        assert!(
            names.iter().any(|n| n == name),
            "{name} is offered: {names:?}"
        );
    }
    let (documents, notes) = (fx.documents().await, fx.notes().await.len());

    drop(lease);
    for (name, arguments) in backlog_calls() {
        let answer = dropped.call(name, arguments).await.expect("a call");
        assert_eq!(refused(&answer), "session ended", "{name} after the drop");
    }
    host.close();
    for (name, arguments) in backlog_calls() {
        let answer = closed.call(name, arguments).await.expect("a call");
        assert_eq!(refused(&answer), "session ended", "{name} after close");
    }
    assert_eq!(fx.documents().await, documents);
    assert_eq!(fx.notes().await.len(), notes);
    assert!(!fx.live_link(ids::HTUI_FEAT_1, LinkKind::Relates).await);
}

/// I-1: no backlog tool takes a run, step, project, item or box: the schema offers none and a
/// call naming one is refused before anything is written.
#[tokio::test]
async fn no_backlog_tool_accepts_a_run_project_or_item_id_argument() {
    let fx = Leased::ana_2().await;
    let host = fx.host();
    let (_lease, mut client) = open(&host, fx.scope()).await;
    let listed = client
        .request("tools/list", json!({}))
        .await
        .expect("tools/list");
    let foreign = [
        ("run_id", RunId::new().to_string()),
        ("step_id", StepId::new().to_string()),
        ("project_id", ids::PROJECT_AGY.to_string()),
        ("item_id", ids::HTUI_FEAT_1.to_string()),
        ("box_id", ids::BOX.to_string()),
    ];
    let (documents, notes) = (fx.documents().await, fx.notes().await.len());
    for (name, arguments) in backlog_calls() {
        let tool = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} is offered"));
        let schema = &tool["inputSchema"];
        assert_eq!(schema["additionalProperties"], false, "{name}");
        for (field, value) in &foreign {
            assert!(
                schema["properties"].get(field).is_none(),
                "{name} offers {field}"
            );
            let mut arguments = arguments.clone();
            arguments[field] = json!(value);
            let answer = client.call(name, arguments).await.expect("a call");
            assert!(
                refused(&answer)
                    .starts_with(&format!("invalid arguments: unknown field `{field}`")),
                "{name} with {field}: {}",
                answer.text
            );
        }
    }
    assert_eq!(fx.documents().await, documents);
    assert_eq!(fx.notes().await.len(), notes);
    assert!(!fx.live_link(ids::HTUI_FEAT_1, LinkKind::Relates).await);
}

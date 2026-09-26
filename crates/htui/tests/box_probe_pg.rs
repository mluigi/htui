//! MOD-7 milestone 1 against live Postgres (blueprint §7, T5): the PRD's metrics for the
//! registration probe, driven through the production `agent_worker::AgentRuntime` over a
//! `Backend::Online`.
//!
//! The unit cases in `agent_worker.rs` prove the same decisions over a `MemStore`. What only this
//! file can prove is that the writes they rely on are the ones **Postgres** accepts and reads
//! back: the probe columns, `box_tool`, `probe_spec_digest` under its `CHECK`, a reconnect that
//! leaves `last_probed_at` byte-equal, a version or stored-spec change that re-probes, and a
//! rename that keeps the row and its probe.
//!
//! Every case builds the same stack: a throwaway migrated database (`testkit::fresh_db`), a
//! throwaway mirror (`CacheStore`), `Backend::Online` over the two, and a `mock_keyring` guard.
//! One swap is what the store loop does after each `go_online`: `on_online`, then the box task
//! awaited to its end.
//!
//! **Fixture rule (blueprint H-7, F-T).** Nothing here probes this box: the runtime resolves
//! through an injected [`ProbeEnv`] whose `PATH` is a temporary `bin` directory of `sh` scripts
//! and whose `home` is `None`, the hardware is [`FixedHardware`], and every seeded registry row is
//! rewritten to a launch whose one tool cannot exist before any probe, so tier 1 answers `missing`
//! and nothing real is spawned.
//!
//! **Milestone 2 (blueprint §7, T5).** Four more cases prove the declared-tags and quirks
//! compare-and-set on Postgres, driven through `htui::store_worker::serve` with `Boxes` and
//! `EditBox` over the same `Stack`: an editor opened before a reconnect and a registration probe
//! still saves (neither touches `edit_version`); a second editor on a spent token is answered
//! `BoxesStale` and never overwrites the first; another box of this user is listed and editable;
//! and another user's box is neither listed nor editable.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite. The cases write `sh` scripts,
//! so they are unix-only, as the `agent_worker` registration cases are.
#![cfg(feature = "testkit")]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use htui::agent_worker::{AgentRuntime, BoxProbeReport};
use htui::box_settings::BoxesSnapshot;
use htui::store_worker::{Origin, ReplyEnvelope, StoreReply, StoreRequest, UNSOLICITED, serve};
use htui_agent::box_probe::hardware::{FixedHardware, Hardware};
use htui_agent::box_probe::spec::{SETTING_KEY, digest, seed};
use htui_agent::probe::{ProbeEnv, ProbeSnapshot, ProbeStatus, platform_key};
use htui_core::model::{BoxEdit, BoxId, BoxRow, UserId};
use htui_core::store::WriteStore as _;
use htui_store::identity::Identity;
use htui_store::{Backend, CacheStore, HTUI_VERSION, PgStore, Registration, testkit};
use serde_json::{Value, json};
use sqlx::postgres::PgPool;
use tokio::sync::mpsc;

/// How long one swap waits for the box task before it is aborted.
const PATIENCE: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------------------------
// The fake box (H-7, F-T)
// ---------------------------------------------------------------------------------------------

/// A box made of directories under `tmp`: `tmp/bin` is the whole `PATH`, `home` is `None`, and
/// versions are read.
fn fake_env(tmp: &Path) -> ProbeEnv {
    std::fs::create_dir_all(tmp.join("bin")).expect("the fixture bin");
    let mut vars = BTreeMap::new();
    vars.insert(
        "PATH".to_owned(),
        tmp.join("bin").to_string_lossy().into_owned(),
    );
    ProbeEnv {
        cwd: tmp.to_path_buf(),
        platform: platform_key(),
        home: None,
        vars,
        versions: true,
        version_timeout: Duration::from_secs(5),
    }
}

/// Writes an executable `sh` script `name` into `tmp/bin`.
///
/// Every script of a case is written before that case's first probe: a `fork` elsewhere while a
/// write handle is open makes `execve` answer `ETXTBSY`.
fn script(tmp: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(tmp.join("bin")).expect("the fixture bin");
    let path = tmp.join("bin").join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// The tools every case finds: `cargo` (the `rust` tag), `git`, and `terraform`, which only a
/// stored spec names, so the seed never reports it.
fn tool_scripts(tmp: &Path) {
    script(tmp, "git", "echo 'git version 2.43.0'");
    script(tmp, "cargo", "echo 'cargo 1.80.0 (376290515 2024-07-16)'");
    script(tmp, "terraform", "echo 'Terraform v1.9.0'");
}

/// Fixed facts: an AMD display device, so the seed names the GPU `amd` and derives `gpu`.
fn fake_hardware() -> Arc<FixedHardware> {
    Arc::new(FixedHardware(Hardware {
        os_version: "Test OS 1".to_owned(),
        cpu: "Test CPU".to_owned(),
        ram_mb: Some(2048),
        display_vendors: vec!["0x1002".to_owned()],
    }))
}

/// A stored overlay adding `terraform`.
fn terraform_spec() -> Value {
    json!({
        "tools": {
            "terraform": {
                "kind": "path",
                "names": ["terraform"],
                "version": { "args": ["version"], "pattern": "^Terraform v(\\S+)" }
            }
        }
    })
}

/// Every registry row the migrations seeded, rewritten to resolve nowhere (F-T).
async fn make_unresolvable(store: &PgStore) {
    let agents = store.agents().await.expect("read the registry");
    assert!(!agents.is_empty(), "the migrations seed the registry");
    for summary in agents {
        let mut agent = summary.agent;
        agent.launch = json!({
            "command": "${gone}",
            "args": [],
            "env": {},
            "discovery": {
                "tools": {
                    "gone": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] }
                },
                "handshake": true
            }
        });
        store.upsert_agent(&agent).await.expect("the row updates");
    }
}

// ---------------------------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------------------------

/// What `box` holds of one probe, as Postgres prints it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct ProbeColumns {
    os_version: String,
    cpu: String,
    ram_mb: Option<i32>,
    gpu_present: bool,
    gpu_vendor: Option<String>,
    htui_version: String,
    probed_tags: Vec<String>,
    last_probed_at: Option<String>,
    probe_spec_digest: Option<String>,
}

/// Everything one case holds: the database, the mirror (and the directory it lives in), the
/// keyring guard, the fake box, the runtime and its reply channel.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    _keyring: testkit::KeyringGuard,
    _tmp: tempfile::TempDir,
    backend: Backend,
    runtime: AgentRuntime,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    rx: mpsc::UnboundedReceiver<ReplyEnvelope>,
}

impl Stack {
    /// The stack over a fresh database, or `None` (after `testkit::SKIP`) without a server.
    async fn new() -> Option<Self> {
        let db = testkit::fresh_db().await?;
        make_unresolvable(&db.store).await;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "box-probe-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let keyring = testkit::mock_keyring().await;
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache,
        };
        let tmp = tempfile::tempdir().expect("temp box");
        tool_scripts(tmp.path());
        let runtime = AgentRuntime::production()
            .with_registration_probe()
            .with_probe_env(fake_env(tmp.path()), fake_hardware());
        let (tx, rx) = mpsc::unbounded_channel();
        Some(Self {
            db,
            _root: root,
            _keyring: keyring,
            _tmp: tmp,
            backend,
            runtime,
            tx,
            rx,
        })
    }

    /// One `Online` swap as the store loop makes it, and every reply it sent.
    async fn swap(&mut self) -> Vec<ReplyEnvelope> {
        self.runtime.on_online(&self.backend, &self.tx);
        self.runtime.finish_background(PATIENCE).await;
        let mut replies = Vec::new();
        while let Ok(reply) = self.rx.try_recv() {
            replies.push(reply);
        }
        replies
    }

    fn pool(&self) -> &PgPool {
        &self.db.pool
    }

    fn box_id(&self) -> BoxId {
        self.db.store.this_box()
    }

    /// This box's probe columns.
    async fn columns(&self) -> ProbeColumns {
        sqlx::query_as(
            "SELECT os_version, cpu, ram_mb, gpu_present, gpu_vendor, htui_version, probed_tags, \
                    last_probed_at::text AS last_probed_at, probe_spec_digest \
               FROM box WHERE id = $1",
        )
        .bind(self.box_id().as_uuid())
        .fetch_one(self.pool())
        .await
        .expect("read this box's probe columns")
    }

    /// This box's `box_tool` rows, `(name, version)` by name bytes.
    async fn tools(&self) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT name, version FROM box_tool WHERE box_id = $1 ORDER BY name COLLATE \"C\"",
        )
        .bind(self.box_id().as_uuid())
        .fetch_all(self.pool())
        .await
        .expect("read this box's tools")
    }

    /// How many `box` rows this user has: a registration that minted a second row shows here.
    async fn boxes_of_this_user(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE user_id = $1")
            .bind(self.db.store.this_user().as_uuid())
            .fetch_one(self.pool())
            .await
            .expect("count this user's boxes")
    }

    /// One request through the production store worker's `serve`, over this stack's backend.
    async fn serve(&self, request: StoreRequest) -> StoreReply {
        serve(&self.backend, &request).await
    }

    /// `box.edit_version` of `id`, by SQL.
    async fn edit_version(&self, id: BoxId) -> i32 {
        sqlx::query_scalar("SELECT edit_version FROM box WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(self.pool())
            .await
            .expect("read edit_version")
    }

    /// `box.declared_tags` and `box.quirks` of `id`, by SQL.
    async fn declared(&self, id: BoxId) -> (Vec<String>, String) {
        sqlx::query_as("SELECT declared_tags, quirks FROM box WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(self.pool())
            .await
            .expect("read declared_tags and quirks")
    }

    /// `box.updated_at` of this box, as Postgres prints it.
    async fn updated_at(&self) -> String {
        sqlx::query_scalar("SELECT updated_at::text FROM box WHERE id = $1")
            .bind(self.box_id().as_uuid())
            .fetch_one(self.pool())
            .await
            .expect("read updated_at")
    }

    /// A `box` row of `owner` named `hostname`, planted by SQL, never probed.
    async fn plant_box(&self, owner: UserId, hostname: &str) -> BoxId {
        let id = BoxId::new();
        sqlx::query(
            "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version) \
             VALUES ($1, $2, $3, 'linux', '', 'x86_64', '0.0.0')",
        )
        .bind(id.as_uuid())
        .bind(owner.as_uuid())
        .bind(hostname)
        .execute(self.pool())
        .await
        .expect("plant a box");
        id
    }

    /// Another `app_user`, planted by SQL.
    async fn plant_user(&self) -> UserId {
        let user = UserId::new();
        sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind("stranger")
            .execute(self.pool())
            .await
            .expect("plant another user");
        user
    }

    async fn drop_db(self) {
        self.db.drop_db().await;
    }
}

/// The one `BoxProbed` among `replies`, at `UNSOLICITED` and `Origin::App`.
fn the_report(replies: &[ReplyEnvelope]) -> BoxProbeReport {
    assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
    assert_eq!(replies[0].seq, UNSOLICITED);
    assert_eq!(replies[0].origin, Origin::App);
    let StoreReply::BoxProbed(report) = &replies[0].reply else {
        panic!("the box task answers BoxProbed: {:?}", replies[0].reply)
    };
    assert_eq!(report.box_failed, None, "the box half wrote: {report:?}");
    assert_eq!(report.agents_failed, None, "the agent half ran: {report:?}");
    report.clone()
}

/// The snapshot of a `Boxes` reply: a read, or an `EditBox` that applied.
fn applied(reply: StoreReply) -> BoxesSnapshot {
    let StoreReply::Boxes(snapshot) = reply else {
        panic!("the worker answers Boxes: {reply:?}")
    };
    *snapshot
}

/// The snapshot of a `BoxesStale` reply: an `EditBox` on a spent token or a box not found.
fn stale(reply: StoreReply) -> BoxesSnapshot {
    let StoreReply::BoxesStale(snapshot) = reply else {
        panic!("the worker answers BoxesStale: {reply:?}")
    };
    *snapshot
}

/// The row of `id` in `snapshot`, which must list it.
fn row_of(snapshot: &BoxesSnapshot, id: BoxId) -> &BoxRow {
    &snapshot
        .boxes
        .iter()
        .find(|record| record.row.id == id)
        .unwrap_or_else(|| panic!("the snapshot lists {id:?}: {snapshot:?}"))
        .row
}

/// An edit of the declared tags only.
fn tags(tags: &[&str]) -> BoxEdit {
    BoxEdit {
        declared_tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
        quirks: None,
    }
}

/// The pair of seeded tools every probe finds (`terraform` needs a stored spec).
fn seeded_tools() -> Vec<(String, String)> {
    vec![
        ("cargo".to_owned(), "1.80.0".to_owned()),
        ("git".to_owned(), "2.43.0".to_owned()),
    ]
}

// ---------------------------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------------------------

/// PRD metric: a box registered for the first time is probed at the first `Online` swap, and
/// Postgres holds the whole profile.
#[tokio::test]
async fn the_first_registration_probes_the_box() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    let before = stack.columns().await;
    assert_eq!(before.last_probed_at, None, "registration never probes");
    assert!(stack.tools().await.is_empty(), "nor writes box_tool");

    let report = the_report(&stack.swap().await);

    let after = stack.columns().await;
    assert_eq!(after.os_version, "Test OS 1");
    assert_eq!(after.cpu, "Test CPU");
    assert_eq!(after.ram_mb, Some(2048));
    assert!(after.gpu_present);
    assert_eq!(after.gpu_vendor.as_deref(), Some("amd"));
    assert_eq!(after.htui_version, HTUI_VERSION);
    assert!(after.last_probed_at.is_some(), "last_probed_at is stamped");
    assert_eq!(after.probed_tags, ["gpu", "rust"]);
    assert_eq!(after.probe_spec_digest, Some(digest(seed())));
    assert_eq!(stack.tools().await, seeded_tools());
    assert_eq!(report.tools, 2);
    assert_eq!(report.probed_tags, ["gpu", "rust"]);

    let agents = stack.db.store.agents().await.expect("read the registry");
    assert!(agents.iter().any(|summary| summary.agent.enabled));
    for summary in agents.iter().filter(|summary| summary.agent.enabled) {
        let status = summary
            .on_box
            .as_ref()
            .and_then(ProbeSnapshot::from_row)
            .map(|snapshot| snapshot.status);
        assert!(
            matches!(status, Some(ProbeStatus::Missing)),
            "{} was probed on this box: {status:?}",
            summary.agent.name
        );
    }

    stack.drop_db().await;
}

/// PRD metric: a reconnect at the same version and spec costs reads, not a probe.
#[tokio::test]
async fn a_reconnect_at_the_same_version_does_not_probe() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    the_report(&stack.swap().await);
    let first = stack.columns().await;
    let first_tools = stack.tools().await;

    let replies = stack.swap().await;

    assert!(
        replies.is_empty(),
        "a skipped probe sends nothing: {replies:?}"
    );
    assert_eq!(
        stack.columns().await,
        first,
        "last_probed_at and every probe column are byte-equal"
    );
    assert_eq!(stack.tools().await, first_tools);

    stack.drop_db().await;
}

/// PRD metric: a box last probed by another `htui` is probed again.
#[tokio::test]
async fn a_version_change_reprobes() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    the_report(&stack.swap().await);
    sqlx::query("UPDATE box SET htui_version = '0.0.0' WHERE id = $1")
        .bind(stack.box_id().as_uuid())
        .execute(stack.pool())
        .await
        .expect("stand in for an older htui");
    let older = stack.columns().await;

    the_report(&stack.swap().await);

    let after = stack.columns().await;
    assert_eq!(after.htui_version, HTUI_VERSION, "the probe rewrote it");
    assert_ne!(
        after.last_probed_at, older.last_probed_at,
        "the box was probed again"
    );
    assert_eq!(stack.tools().await, seeded_tools());

    stack.drop_db().await;
}

/// PRD D1-D5: a renamed box is the same row, and its probe survives the registration.
#[tokio::test]
async fn a_renamed_box_keeps_its_row_and_its_probe() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    the_report(&stack.swap().await);
    let probed = stack.columns().await;
    let tools = stack.tools().await;

    let answer = stack
        .db
        .store
        .register_box(
            &Identity {
                box_id: stack.box_id(),
                hostname: "renamed".to_owned(),
            },
            None,
        )
        .await
        .expect("the renamed box registers");

    assert!(
        matches!(
            answer,
            Registration::Known {
                renamed_from: Some(_)
            }
        ),
        "a rename is the same box: {answer:?}"
    );
    assert_eq!(stack.boxes_of_this_user().await, 1, "one row");
    let hostname: String = sqlx::query_scalar("SELECT hostname FROM box WHERE id = $1")
        .bind(stack.box_id().as_uuid())
        .fetch_one(stack.pool())
        .await
        .expect("read the hostname");
    assert_eq!(hostname, "renamed");
    assert_eq!(
        stack.columns().await,
        probed,
        "the probe columns are intact"
    );
    assert_eq!(stack.tools().await, tools, "and so is box_tool");

    stack.drop_db().await;
}

/// PRD metric, plan D17-D18: a stored spec that changes the effective spec re-probes, and the
/// tool it adds lands in `box_tool`.
#[tokio::test]
async fn a_stored_spec_change_reprobes() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    the_report(&stack.swap().await);
    let first = stack.columns().await;
    assert!(
        stack
            .tools()
            .await
            .iter()
            .all(|(name, _)| name != "terraform"),
        "the seed does not name terraform"
    );

    sqlx::query("INSERT INTO app_setting (key, value) VALUES ($1, $2)")
        .bind(SETTING_KEY)
        .bind(terraform_spec())
        .execute(stack.pool())
        .await
        .expect("store a spec");
    let report = the_report(&stack.swap().await);

    let after = stack.columns().await;
    assert_eq!(report.spec_error, None, "the stored spec is valid");
    assert_ne!(
        after.probe_spec_digest, first.probe_spec_digest,
        "the digest follows the spec"
    );
    assert_ne!(after.probe_spec_digest, Some(digest(seed())));
    assert_ne!(after.last_probed_at, first.last_probed_at, "probed again");
    assert!(
        stack
            .tools()
            .await
            .contains(&("terraform".to_owned(), "1.9.0".to_owned())),
        "the stored spec's tool is recorded"
    );

    stack.drop_db().await;
}

// ---------------------------------------------------------------------------------------------
// Milestone 2: the declared tags and quirks CAS (blueprint §7.2)
// ---------------------------------------------------------------------------------------------

/// PRD metric "Declared tags and quirks CAS", plan D39: an editor opened before a reconnect, a
/// rename and a registration probe still saves on its token, because neither writes
/// `edit_version`.
#[tokio::test]
async fn an_open_editor_survives_a_reconnect_and_a_registration_probe() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    the_report(&stack.swap().await);
    let this_box = stack.box_id();
    let opened = applied(stack.serve(StoreRequest::Boxes).await);
    assert_eq!(opened.this_box, Some(this_box));
    let token = row_of(&opened, this_box).edit_version;
    assert_eq!(token, 0, "a registered box starts at 0");
    let first_updated = stack.updated_at().await;
    let first = stack.columns().await;

    stack
        .db
        .store
        .register_box(
            &Identity {
                box_id: this_box,
                hostname: "reconnected".to_owned(),
            },
            None,
        )
        .await
        .expect("the reconnect registers");
    sqlx::query("INSERT INTO app_setting (key, value) VALUES ($1, $2)")
        .bind(SETTING_KEY)
        .bind(terraform_spec())
        .execute(stack.pool())
        .await
        .expect("store a spec");
    the_report(&stack.swap().await);

    let second = stack.columns().await;
    assert_ne!(
        stack.updated_at().await,
        first_updated,
        "the row was written"
    );
    assert_ne!(
        second.probe_spec_digest, first.probe_spec_digest,
        "the registration probe re-probed"
    );
    assert_eq!(
        stack.edit_version(this_box).await,
        token,
        "neither registration nor the probe writes edit_version"
    );

    let saved = applied(
        stack
            .serve(StoreRequest::EditBox {
                box_id: this_box,
                expected: token,
                edit: tags(&["gpu", "heavy_build"]),
            })
            .await,
    );

    let row = row_of(&saved, this_box);
    assert_eq!(row.declared_tags, ["gpu", "heavy_build"]);
    assert_eq!(row.edit_version, token + 1);
    assert_eq!(row.hostname, "reconnected");
    assert_eq!(
        stack.columns().await,
        second,
        "the probe columns are the second probe's"
    );
    assert_eq!(
        stack.declared(this_box).await,
        (
            vec!["gpu".to_owned(), "heavy_build".to_owned()],
            String::new()
        )
    );

    stack.drop_db().await;
}

/// PRD metric "Declared tags and quirks CAS": the second of two editors on one token is told, and
/// the first one's tags stay.
#[tokio::test]
async fn a_concurrent_edit_is_reported_and_never_overwritten() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let this_box = stack.box_id();
    let first_editor = applied(stack.serve(StoreRequest::Boxes).await);
    let second_editor = applied(stack.serve(StoreRequest::Boxes).await);
    let token = row_of(&first_editor, this_box).edit_version;
    assert_eq!(token, 0);
    assert_eq!(row_of(&second_editor, this_box).edit_version, token);

    let saved = applied(
        stack
            .serve(StoreRequest::EditBox {
                box_id: this_box,
                expected: token,
                edit: tags(&["gpu"]),
            })
            .await,
    );
    assert_eq!(row_of(&saved, this_box).declared_tags, ["gpu"]);

    let refused = stale(
        stack
            .serve(StoreRequest::EditBox {
                box_id: this_box,
                expected: token,
                edit: tags(&["vulkan"]),
            })
            .await,
    );

    let row = row_of(&refused, this_box);
    assert_eq!(row.declared_tags, ["gpu"], "the row as it is now");
    assert_eq!(row.edit_version, 1);
    assert_eq!(
        stack.declared(this_box).await.0,
        ["gpu"],
        "the second editor wrote nothing"
    );
    assert_eq!(stack.edit_version(this_box).await, 1);

    stack.drop_db().await;
}

/// PRD D47: every box of this user is listed, and one that is not this process's box is edited on
/// its own token without touching this one.
#[tokio::test]
async fn another_box_of_this_user_is_editable() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let this_box = stack.box_id();
    let second = stack.plant_box(stack.db.store.this_user(), "SECOND").await;
    let listed = applied(stack.serve(StoreRequest::Boxes).await);
    assert_eq!(listed.boxes.len(), 2, "this user's two boxes: {listed:?}");
    let token = row_of(&listed, second).edit_version;
    assert_eq!(token, 0);
    let this_before = stack.declared(this_box).await;
    let this_version = stack.edit_version(this_box).await;

    let saved = applied(
        stack
            .serve(StoreRequest::EditBox {
                box_id: second,
                expected: token,
                edit: BoxEdit {
                    declared_tags: None,
                    quirks: Some("x".to_owned()),
                },
            })
            .await,
    );

    assert_eq!(row_of(&saved, second).quirks, "x");
    assert_eq!(stack.declared(second).await, (Vec::new(), "x".to_owned()));
    assert_eq!(stack.edit_version(second).await, 1);
    assert_eq!(
        stack.declared(this_box).await,
        this_before,
        "this box is untouched"
    );
    assert_eq!(stack.edit_version(this_box).await, this_version);

    stack.drop_db().await;
}

/// PRD D47, T1's `NotFound { entity: "box" }`: another user's box is never listed, and an edit of
/// it is answered `BoxesStale` and writes nothing.
#[tokio::test]
async fn another_users_box_is_not_listed_and_not_editable() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let stranger = stack.plant_user().await;
    let foreign = stack.plant_box(stranger, "ELSEWHERE").await;
    let before = stack.declared(foreign).await;
    let version = stack.edit_version(foreign).await;

    let listed = applied(stack.serve(StoreRequest::Boxes).await);
    assert!(
        listed.boxes.iter().all(|record| record.row.id != foreign),
        "another user's box is not listed: {listed:?}"
    );

    let refused = stale(
        stack
            .serve(StoreRequest::EditBox {
                box_id: foreign,
                expected: version,
                edit: BoxEdit {
                    declared_tags: Some(vec!["gpu".to_owned()]),
                    quirks: Some("x".to_owned()),
                },
            })
            .await,
    );

    assert!(
        refused.boxes.iter().all(|record| record.row.id != foreign),
        "the stale snapshot does not list it either: {refused:?}"
    );
    assert_eq!(stack.declared(foreign).await, before, "nothing was written");
    assert_eq!(stack.edit_version(foreign).await, version);

    stack.drop_db().await;
}

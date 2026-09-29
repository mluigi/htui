//! MOD-39 T3: the Requirements tab's writes land on **Postgres**.
//!
//! `tests/requirements.rs` proves the tab over a `MemStore`. What only this file can prove is that
//! the requests the tab sends are ones `PgStore` answers the same way: a mint continues the area's
//! counter, an amend at the head writes the next version, an amend at an older one answers
//! `RequirementsStale` and writes nothing, the first gated write claims a project with no spec, and
//! a user who does not own a spec is refused (MOD-39 PRD D1, plan P5, blueprint §4.6).
//!
//! No harness: `store_worker::serve` over `Backend::Online { pg, cache }` is what the worker task
//! runs, and the tab's key handling is `requirements.rs`' concern. The case prints
//! `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics instead when `CI`
//! is set, like every other Postgres-backed suite.
#![cfg(feature = "testkit")]

use htui::requirements::{RequirementText, RequirementsSnapshot, not_the_maintainer};
use htui::store_worker::{self, StoreReply, StoreRequest};
use htui_core::fixtures::ids;
use htui_core::model::{Priority, ProjectId, RequirementFilter, Scope, UserId};
use htui_core::store::{CasOutcome, ReadStore as _, WriteStore as _};
use htui_store::{Backend, CacheStore, PgStore, testkit};

/// The database, the mirror (and the directory it lives in) and the backend over both.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    backend: Backend,
}

impl Stack {
    /// The stack over a seeded demo database, or `None` (after `testkit::SKIP`) without a server.
    async fn new(name: &str) -> Option<Self> {
        let db = testkit::demo_db().await?;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), name, PgStore::schema_version())
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

    /// The Platform workspace's scope: `htui` then `agy`.
    async fn platform(&self) -> Scope {
        let platform = self
            .db
            .store
            .workspaces()
            .await
            .expect("the server's workspaces")
            .into_iter()
            .find(|workspace| workspace.slug == "platform")
            .expect("the demo holds `platform`");
        Scope::from_workspace(&platform)
    }

    /// Closes the mirror and drops the database. The case calls this on its last line.
    async fn finish(self) {
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// The snapshot a tab write answered with; panics on anything else.
fn applied(reply: StoreReply) -> RequirementsSnapshot {
    match reply {
        StoreReply::Requirements(snapshot) => *snapshot,
        other => panic!("the write answered {other:?}"),
    }
}

/// The keys and versions of `project`'s requirements in `snapshot`.
fn versions(snapshot: &RequirementsSnapshot, project: ProjectId) -> Vec<(String, i32)> {
    snapshot
        .project(project)
        .expect("the project is in the snapshot")
        .requirements
        .iter()
        .map(|row| (row.key.clone(), row.version))
        .collect()
}

/// An amend of `R-ENT-1` at `expected`, decided by `ANA-2`.
fn amend(scope: &Scope, expected: i32) -> StoreRequest {
    StoreRequest::AmendRequirement {
        scope: scope.clone(),
        id: ids::REQ_ENT_1,
        expected_version: expected,
        body: RequirementText::new("Every item has a stable key of the form PREFIX-N."),
        rationale: RequirementText::new("Keys are what people type and search."),
        priority: Priority::Must,
        deciding: "ana-2".to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mint_amend_and_a_stale_amend_on_postgres() {
    let Some(stack) = Stack::new("requirements-pg-mint").await else {
        return;
    };
    let scope = stack.platform().await;
    let backend = &stack.backend;

    let mint = StoreRequest::MintRequirement {
        scope: scope.clone(),
        project: ids::PROJECT_HTUI,
        area: ids::AREA_ENT,
        body: RequirementText::new("Every item has a title."),
        rationale: RequirementText::new("Titles are what people read."),
        priority: Priority::Later,
    };
    let snapshot = applied(store_worker::serve(backend, &mint).await);
    let minted = snapshot
        .project(ids::PROJECT_HTUI)
        .expect("htui is in the snapshot")
        .requirements
        .iter()
        .find(|row| row.key == "R-ENT-3")
        .expect("the mint continues the fixture's counter");
    assert_eq!(minted.version, 1);
    assert_eq!(minted.priority, Priority::Later);
    assert_eq!(minted.created_by, stack.db.store.this_user());

    let snapshot = applied(store_worker::serve(backend, &amend(&scope, 2)).await);
    assert!(
        versions(&snapshot, ids::PROJECT_HTUI).contains(&("R-ENT-1".to_owned(), 3)),
        "the amend at the head writes v3"
    );

    match store_worker::serve(backend, &amend(&scope, 2)).await {
        StoreReply::RequirementsStale(snapshot) => assert!(
            versions(&snapshot, ids::PROJECT_HTUI).contains(&("R-ENT-1".to_owned(), 3)),
            "the stale answer shows the head as it is"
        ),
        other => panic!("an amend at v2 over v3 answered {other:?}"),
    }
    let head = stack
        .db
        .store
        .requirement(ids::REQ_ENT_1)
        .await
        .expect("the server's row")
        .expect("R-ENT-1 exists");
    assert_eq!(head.version, 3, "the stale amend wrote nothing");
    let revisions = stack
        .db
        .store
        .requirement_revisions(ids::REQ_ENT_1)
        .await
        .expect("the server's revisions")
        .expect("Postgres holds revisions");
    assert_eq!(revisions.len(), 3, "v1, v2 and the one amend that applied");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_write_claims_agy_and_a_non_owner_is_refused_on_postgres() {
    let Some(stack) = Stack::new("requirements-pg-gate").await else {
        return;
    };
    let scope = stack.platform().await;
    let backend = &stack.backend;
    let me = stack.db.store.this_user();

    // `agy` has no spec: the first gated write claims it for this user.
    let create = StoreRequest::CreateRequirementArea {
        scope: scope.clone(),
        project: ids::PROJECT_AGY,
        code: "DRV".to_owned(),
        title: "Driver".to_owned(),
    };
    let snapshot = applied(store_worker::serve(backend, &create).await);
    let agy = snapshot
        .project(ids::PROJECT_AGY)
        .expect("agy is in the snapshot");
    assert_eq!(agy.spec.as_ref().map(|spec| spec.owner_id), Some(me));
    assert!(agy.maintainer);
    assert_eq!(
        agy.areas
            .iter()
            .map(|area| (area.code.as_str(), area.position))
            .collect::<Vec<_>>(),
        vec![("DRV", 0)]
    );

    // `htui`'s spec moves to another user: this one is now read-only there.
    let stranger = UserId::new();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2)")
        .bind(stranger.as_uuid())
        .bind("stranger")
        .execute(&stack.db.pool)
        .await
        .expect("plant another user");
    let moved = stack
        .db
        .store
        .set_requirement_spec(ids::PROJECT_HTUI, Some(1), stranger, String::new())
        .await
        .expect("the spec write");
    assert!(matches!(moved, CasOutcome::Applied(_)), "{moved:?}");

    let before = stack
        .db
        .store
        .requirements(ids::PROJECT_HTUI, &RequirementFilter::default())
        .await
        .expect("the server's requirements");
    let mint = StoreRequest::MintRequirement {
        scope: scope.clone(),
        project: ids::PROJECT_HTUI,
        area: ids::AREA_ENT,
        body: RequirementText::new("Refused."),
        rationale: RequirementText::new(""),
        priority: Priority::Must,
    };
    match store_worker::serve(backend, &mint).await {
        StoreReply::Failed { request, message } => {
            assert_eq!(request, "mint_requirement");
            assert!(message.contains(&not_the_maintainer("htui")), "{message}");
        }
        other => panic!("a non-owner's mint answered {other:?}"),
    }
    let after = stack
        .db
        .store
        .requirements(ids::PROJECT_HTUI, &RequirementFilter::default())
        .await
        .expect("the server's requirements");
    assert_eq!(after, before, "nothing was written");
    match store_worker::serve(backend, &StoreRequest::Requirements(scope.clone())).await {
        StoreReply::Requirements(snapshot) => assert!(
            !snapshot
                .project(ids::PROJECT_HTUI)
                .expect("htui is in the snapshot")
                .maintainer,
            "the read says read-only too"
        ),
        other => panic!("the read answered {other:?}"),
    }

    stack.finish().await;
}

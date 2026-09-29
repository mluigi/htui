//! MOD-9 milestone 3, T5: a skill created, edited and attached in the TUI lands on **Postgres**.
//!
//! `tests/skills.rs` proves the Skills view over a `MemStore`. What only this file can prove is
//! that the three writes the view sends are ones `PgStore` accepts: the worker's `CreateSkill`
//! (the row and v1 in one transaction), `SaveSkillVersion` (the head compare-and-set) and
//! `SetSkillBinding` (the `expected: None` insert on the `NULLS NOT DISTINCT` key), each through
//! `Backend::writer()`, and that each reply is the snapshot the view lands its draft on.
//!
//! The stack is `templates_pg.rs`' (blueprint §6.7): a throwaway database with the demo world
//! (`testkit::demo_db`), a throwaway mirror, a `testkit::mock_keyring` guard holding the DSN, and a
//! [`Harness`] over `Backend::Online`. The Harness enters the Graphics workspace, whose one project
//! is `vulkan-tutorials`: the attachment written here is that project's.
//!
//! The case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite.
#![cfg(feature = "testkit")]

use htui::app::register_all;
use htui::testkit::Harness;
use htui_core::fixtures::ids;
use htui_core::model::Activation;
use htui_core::store::WriteStore as _;
use htui_store::{Backend, CacheStore, PgStore, secret, testkit};

/// The skill every step writes.
const NAME: &str = "pg-skill";

/// Everything the case holds: the database, the mirror (and the directory it lives in), the
/// keyring guard and the shell.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    _keyring: testkit::KeyringGuard,
    harness: Harness,
}

impl Stack {
    /// The stack over a seeded demo database, or `None` (after `testkit::SKIP`) without a server.
    async fn new() -> Option<Self> {
        let db = testkit::demo_db().await?;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "skills-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let keyring = testkit::mock_keyring().await;
        // An online box has its DSN stored; an empty keyring would send the shell to the DSN
        // field at start and the keys below would be typed into it (`templates_pg.rs`' reason).
        secret::set_dsn(&db.url).expect("the fake keyring accepts a write");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        let mut harness = Harness::over_backend(backend);
        register_all(harness.app());
        harness.drive().await;
        assert_eq!(
            harness.app().top_bar.store,
            "online",
            "the shell runs over the Postgres backend, not a memory one"
        );
        Some(Self {
            db,
            _root: root,
            cache,
            _keyring: keyring,
            harness,
        })
    }

    /// Closes the mirror and drops the database. The case calls this on its last line.
    async fn finish(self) {
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// Types `text` one key at a time: a space is `space`, a newline `enter`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        match c {
            ' ' => harness.key("space"),
            '\n' => harness.key("enter"),
            c => harness.key(&c.to_string()),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_skill_created_edited_and_attached_in_the_tui_lands_on_postgres() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    let harness = &mut stack.harness;
    harness.key("2");
    harness.settle().await;

    // Create: name, an empty description, then v1's body.
    harness.key("n");
    type_text(harness, NAME);
    harness.key("enter");
    harness.key("enter");
    type_text(harness, "First.");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains(&format!("created `{NAME}` v1")),
        "the create landed: {frame}"
    );

    // Edit into v2: the cursor is on the new skill, the editor opens on v1 at byte 0.
    harness.key("e");
    type_text(harness, "Second. ");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("saved v2"), "the append landed: {frame}");

    // Attach to `vulkan-tutorials` with the form's defaults.
    harness.key("a");
    harness.key("j");
    harness.key("enter");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("attached to vulkan-tutorials"),
        "the attachment landed: {frame}"
    );

    let store = &stack.db.store;
    let skill = store
        .skills()
        .await
        .expect("the server's library")
        .into_iter()
        .find(|skill| skill.name == NAME)
        .expect("the new skill is on the server");
    let bodies: Vec<String> = store
        .skill_versions(skill.id)
        .await
        .expect("the server's versions")
        .into_iter()
        .map(|row| row.body)
        .collect();
    assert_eq!(bodies, ["First.", "Second. First."]);
    let rows: Vec<_> = store
        .skill_bindings(Some(ids::PROJECT_VULKAN))
        .await
        .expect("the server's attachments")
        .into_iter()
        .filter(|row| row.skill_id == skill.id)
        .collect();
    let [row] = rows.as_slice() else {
        panic!("one attachment of {NAME} on vulkan-tutorials: {rows:?}");
    };
    assert_eq!(
        (
            row.phase_id,
            row.activation,
            row.pinned_version,
            row.position
        ),
        (None, Activation::Always, None, 0)
    );
    stack.finish().await;
}

//! MOD-9 milestone 1, T6: a template saved in the TUI lands on **Postgres** and reaches the preview
//! (the PRD's hypothesis, end to end).
//!
//! `tests/templates.rs` proves the Templates view over a `MemStore`. What only this file can prove
//! is that the save the view sends is one `PgStore` accepts: the worker's `SaveTemplate` goes
//! through `Backend::writer()` to `append_prompt_template`'s compare-and-set on the server, the new
//! head is v2 with the worker's `created_by`, the deferred preview (`htui::preview`) renders that
//! head, and an offline backend refuses the save with nothing written.
//!
//! Every case builds `runs_pg.rs`'s stack: a throwaway database with the demo world
//! (`testkit::demo_db`), a throwaway mirror (`CacheStore`), a `testkit::mock_keyring` guard (the
//! Harness answers `ConnectionInfo` at start, which over a non-`Memory` backend reaches the
//! keyring), `Backend::Online` over the two, and a [`Harness`] over that backend. Unlike
//! `runs_pg.rs`, which never types a key, the fake keyring holds the database's DSN, as an online
//! box's keyring does: an empty one sends the shell to the DSN field at start.
//!
//! The Harness enters the demo fixture's first workspace by name, `Graphics`, whose one project is
//! `vulkan-tutorials`: the template edited here is that project's `implement`, and the item
//! previewed is `vulkan-tutorials`' `FEAT-1`.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite.
#![cfg(feature = "testkit")]

use std::time::Duration;

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::store_worker::{self, Origin, RequestEnvelope, StoreReply, StoreRequest};
use htui::templates::TemplateBody;
use htui::testkit::Harness;
use htui::ui::tabs::SkillsTab;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{PromptTemplate, Scope};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore, secret, testkit};

/// The line a save adds as `implement`'s new first line: in v2, never in v1.
const MARKER: &str = "MOD9 MARKER LINE";

/// The template every case edits.
const NAME: &str = "implement";

/// Everything one case holds: the database, the mirror (and the directory it lives in), the
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
        let cache = CacheStore::open(root.path(), "templates-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let keyring = testkit::mock_keyring().await;
        // An online box has its DSN stored. With the fake keyring left empty, the first
        // `ConnectionInfo` reply redirects the shell to Settings > Connection (MOD-15 M6 D6) with
        // the DSN field open, and the keys below would be typed into it.
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

    /// The same `Backend::Online` the Harness holds: the server and the mirror, cloned.
    fn backend(&self) -> Backend {
        Backend::Online {
            pg: self.db.store.clone(),
            cache: self.cache.clone(),
        }
    }

    /// The Postgres head of `implement` in `vulkan-tutorials`, read straight from the server.
    async fn head(&self) -> Option<PromptTemplate> {
        self.db
            .store
            .prompt_template(ids::PROJECT_VULKAN, NAME, None)
            .await
            .unwrap_or_else(|err| panic!("the server read failed: {err}"))
    }

    /// Saves [`MARKER`] as a new first line of `implement` through the Templates view: `2`, `l`,
    /// select, `e`, type, `ctrl-s`, then serve what the save asked for.
    async fn save_marker(&mut self) {
        let harness = &mut self.harness;
        harness.key("2");
        harness.key("l");
        harness.settle().await;
        select(harness, NAME);
        harness.key("e");
        type_text(harness, MARKER);
        harness.key("enter");
        harness.key("ctrl-s");
        harness.settle().await;
    }

    /// Closes the mirror and drops the database. Every case calls this on its last line.
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

/// Moves the tree's cursor onto `name`: to the top, then down until the body pane is titled with
/// it.
fn select(harness: &mut Harness, name: &str) {
    for _ in 0..20 {
        harness.key("k");
    }
    let title = format!("\u{250c} {name} v");
    for _ in 0..20 {
        if harness.render().contains(&title) {
            return;
        }
        harness.key("j");
    }
    panic!("`{name}` is not in the tree:\n{}", harness.render());
}

/// The assembled preview of `vulkan-tutorials`' `FEAT-1` through `implement`, served the way the
/// shell serves it: deferred onto a task the agent runtime owns, over `backend`.
async fn preview_text(backend: &Backend, scope: &Scope) -> String {
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let envelope = RequestEnvelope {
        seq: 1,
        origin: Origin::Tab(SkillsTab::ID),
        request: StoreRequest::PromptPreview {
            item: ids::VULKAN_FEAT_1,
            template_name: Some(NAME.to_owned()),
            scope: scope.clone(),
        },
    };
    runtime.serve(backend, &tx, &envelope).await;
    runtime.finish_background(Duration::from_secs(30)).await;
    let answer = rx.try_recv().expect("the preview task answered");
    let StoreReply::PromptPreview(preview) = answer.reply else {
        panic!("not a preview: {:?}", answer.reply);
    };
    let template = preview.template.expect("the project has templates");
    assert_eq!(
        template.name, NAME,
        "the preview renders the named template"
    );
    preview
        .outcome
        .unwrap_or_else(|refusal| panic!("the preview assembled: {refusal}"))
        .text
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_through_the_worker_lands_as_version_two_on_postgres() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    let before = stack
        .head()
        .await
        .expect("the demo world seeds `implement`");
    assert_eq!(before.version, 1, "the seeded head");
    assert!(!before.body.contains(MARKER), "v1 never holds the marker");

    stack.save_marker().await;

    let after = stack.head().await.expect("the head is still there");
    assert_eq!(after.version, 2, "the save landed as v2 on the server");
    assert!(
        after.body.starts_with(&format!("{MARKER}\n")),
        "v2 holds the typed line first: {:?}",
        after.body
    );
    assert!(
        after.body.ends_with(&before.body),
        "v2 is v1 with the line put in front"
    );
    assert_eq!(
        after.created_by,
        stack.db.store.this_user(),
        "the worker fills `created_by` from the connected user"
    );
    assert_ne!(after.id, before.id, "a new row, not v1 rewritten");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preview_uses_the_new_head() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    let backend = stack.backend();
    let scope = stack.harness.app().scope.clone();

    let before = preview_text(&backend, &scope).await;
    assert!(
        !before.contains(MARKER),
        "the v1 preview has no marker:\n{before}"
    );

    stack.save_marker().await;
    assert_eq!(stack.head().await.map(|row| row.version), Some(2));

    let after = preview_text(&backend, &scope).await;
    assert!(
        after.contains(MARKER),
        "the preview renders the v2 head:\n{after}"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_save_is_refused_without_a_row() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    let scope = stack.harness.app().scope.clone();
    let offline = Backend::Offline {
        cache: stack.cache.clone(),
        since: None,
    };

    let reply = store_worker::serve(
        &offline,
        &StoreRequest::SaveTemplate {
            scope,
            project: ids::PROJECT_VULKAN,
            name: NAME.to_owned(),
            body: TemplateBody::new(format!("{MARKER}\n{{{{item}}}}\n")),
            expected: Some(1),
        },
    )
    .await;
    match reply {
        StoreReply::Failed { request, message } => {
            assert_eq!(request, "save_template");
            assert!(
                message.contains(DATABASE_UNREACHABLE),
                "the offline refusal names the unreachable database: {message}"
            );
        }
        other => panic!("an offline save is refused, not {other:?}"),
    }

    let head = stack.head().await.expect("the seeded head");
    assert_eq!(head.version, 1, "nothing reached the server");
    assert!(!head.body.contains(MARKER));
    stack.finish().await;
}

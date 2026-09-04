//! Shell-level tests for the workspace switcher and the migration prompt (blueprint E, MOD-6 E.5).
//!
//! Everything here goes through [`htui::testkit::Harness`] and the public `Overlay` contract: no
//! store handle, no database and no sleeps. The MOD-6 cases feed the shell a synthetic
//! `StoreReply::StoreState`, which is exactly what the store worker sends when it is offline or
//! when the schema has pending migrations, so the top bar and the prompt are tested without a
//! server.
#![cfg(feature = "testkit")]

use htui::app::{Action, register_all};
use htui::store_worker::{Origin, ReplyEnvelope, StoreReply};
use htui::testkit::Harness;
use htui::ui::overlay::workspace_switcher::WorkspaceSwitcher;

/// A `StoreState` reply addressed to the shell, as the fourth-tick refresh receives it.
fn store_state(label: &str, migrations_pending: Option<usize>) -> Action {
    Action::Reply(ReplyEnvelope {
        seq: 0,
        origin: Origin::App,
        reply: StoreReply::StoreState {
            label: label.to_owned(),
            migrations_pending,
        },
    })
}

/// The demo shell over a worker that reports this store state, settled and ready to render.
async fn shell_reporting(label: &str, migrations_pending: Option<usize>) -> Harness {
    let mut harness = Harness::demo().with_store_state(label, migrations_pending);
    register_all(harness.app());
    harness.settle().await;
    harness
}

/// The demo shell with the switcher open over it, settled and ready to render.
async fn open_over_demo() -> Harness {
    let mut harness = Harness::demo().with_overlay(Box::new(WorkspaceSwitcher::new()));
    register_all(harness.app());
    harness.settle().await;
    harness
}

/// The top bar, i.e. the first line of a rendered frame.
fn top_bar(frame: &str) -> String {
    frame.lines().next().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn the_switcher_lists_every_workspace_with_its_project_count() {
    let mut harness = open_over_demo().await;
    let frame = harness.render();

    assert!(frame.contains("Graphics"), "the first workspace is listed");
    assert!(frame.contains("Platform"), "the second workspace is listed");
    assert!(
        frame.contains("1 project") && frame.contains("2 projects"),
        "each row carries its project count: {frame}"
    );
    insta::assert_snapshot!("switcher_open", frame);
}

#[tokio::test]
async fn the_switcher_opens_on_the_workspace_the_shell_is_already_inside() {
    let mut harness = open_over_demo().await;
    let frame = harness.render();

    assert_eq!(
        harness.app().top_bar.workspace,
        "Graphics",
        "startup entered the first workspace"
    );
    assert!(
        frame.contains("> Graphics") && !frame.contains("> Platform"),
        "the cursor starts on the scope's own workspace: {frame}"
    );
}

#[tokio::test]
async fn j_and_k_move_the_cursor_and_stop_at_the_ends() {
    let mut harness = open_over_demo().await;

    harness.key("j");
    let down = harness.render();
    assert!(
        down.contains("> Platform") && !down.contains("> Graphics"),
        "`j` moves onto the second workspace: {down}"
    );

    // The list does not wrap: `j` at the bottom and `k` at the top are no-ops.
    harness.key("j");
    assert_eq!(harness.render(), down, "`j` stops at the last row");

    harness.key("k");
    harness.key("k");
    let up = harness.render();
    assert!(
        up.contains("> Graphics") && !up.contains("> Platform"),
        "`k` walks back to the first workspace and stops there: {up}"
    );
}

#[tokio::test]
async fn enter_switches_the_scope_and_the_top_bar_follows() {
    let mut harness = open_over_demo().await;
    assert_eq!(
        top_bar(&harness.render()),
        "Graphics · DESKTOP-HTUI · memory · 0 runs",
        "the fixture's only active run is in the other workspace"
    );

    harness.key("j");
    harness.key("enter");
    harness.settle().await;

    assert!(
        harness.app().overlays.is_empty(),
        "entering a workspace closes the switcher"
    );
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(
        harness.app().scope.project_ids.len(),
        2,
        "the scope carries both projects of `Platform`, ordered by position"
    );
    assert_eq!(
        harness
            .app()
            .projects
            .iter()
            .map(|p| p.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["htui", "agy"],
        "`workspace_project.position` decides the order"
    );

    let frame = harness.render();
    assert_eq!(
        top_bar(&frame),
        "Platform · DESKTOP-HTUI · memory · 1 run",
        "the top bar reads the new workspace and its active-run count"
    );
    insta::assert_snapshot!("after_switch", frame);
}

#[tokio::test]
async fn esc_closes_the_switcher_and_leaves_the_scope_alone() {
    let mut harness = open_over_demo().await;
    harness.key("j");
    harness.key("esc");
    harness.settle().await;

    assert!(
        harness.app().overlays.is_empty(),
        "`Esc` closes the overlay"
    );
    assert_eq!(
        harness.app().top_bar.workspace,
        "Graphics",
        "a cursor move that was never confirmed changes nothing"
    );
    assert!(
        !harness.render().contains("Workspaces"),
        "the box is gone from the frame"
    );
}

#[tokio::test]
async fn an_empty_store_shows_no_workspaces() {
    // `register_all` opens the switcher over a shell that never entered a workspace (blueprint D,
    // "Startup"), so this one does not push its own: two overlays sharing an id would put the
    // second one out of reach of every reply, which `OverlayStack::by_id_mut` addresses by id.
    let mut harness = Harness::empty();
    register_all(harness.app());
    harness.settle().await;
    let frame = harness.render();

    assert!(
        frame.contains("no workspaces"),
        "an empty store is an empty list, not a blank box: {frame}"
    );
    harness.key("enter");
    assert!(
        !harness.app().overlays.is_empty(),
        "`Enter` on an empty list does nothing"
    );
    insta::assert_snapshot!("switcher_empty", frame);
}

#[tokio::test]
async fn the_top_bar_shows_the_offline_age() {
    // What the worker reports once the first connect attempt has failed and three minutes passed.
    let mut harness = shell_reporting("offline · 3m", None).await;
    let frame = harness.render();

    assert_eq!(
        top_bar(&frame),
        "Graphics · DESKTOP-HTUI · offline · 3m · 0 runs",
        "the store field is `Backend::label()` verbatim (plan D11)"
    );
    assert!(
        harness.app().overlays.is_empty(),
        "an offline store is not a question to answer"
    );
    insta::assert_snapshot!("offline_label", frame);
}

#[tokio::test]
async fn a_pending_migration_opens_the_prompt() {
    let mut harness = shell_reporting("online", Some(3)).await;
    let frame = harness.render();

    assert!(
        frame.contains("3 schema migrations are pending"),
        "the prompt says what it is asking about: {frame}"
    );
    assert!(frame.contains("y apply"), "and how to answer it: {frame}");
    assert_eq!(
        harness.app().overlays.iter().count(),
        1,
        "exactly one prompt, opened by the shell (R-STO-5)"
    );
    insta::assert_snapshot!("migration_prompt", frame);
}

#[tokio::test]
async fn answering_n_does_not_reopen_the_prompt() {
    let mut harness = shell_reporting("online", Some(3)).await;
    harness.key("n");
    harness.settle().await;
    assert!(harness.app().overlays.is_empty(), "`n` closes the prompt");

    // The fourth-tick refresh reports the same count a second later.
    harness.app().update(store_state("online", Some(3)));
    harness.settle().await;
    assert!(
        harness.app().overlays.is_empty(),
        "one prompt per session: `n` is an answer, not a postponement"
    );
    assert!(!harness.render().contains("pending"));
}

#[tokio::test]
async fn answering_y_asks_the_store_to_apply_them() {
    let mut harness = shell_reporting("online", Some(3)).await;
    harness.key("y");
    // `settle` serves the queued `ApplyMigrations` through the harness's memory backend, which
    // answers `MigrationsApplied { applied: 0 }` — nothing to migrate in memory — and the shell
    // puts that on the status line. The request having been issued is what this asserts.
    harness.settle().await;

    assert!(
        harness.app().overlays.is_empty(),
        "`y` closes the prompt as well as sending the request"
    );
    assert_eq!(
        harness.app().status.as_deref(),
        Some("applied 0 migration(s)"),
        "the reply to `StoreRequest::ApplyMigrations` landed"
    );
}

#[tokio::test]
async fn esc_answers_the_prompt_like_n() {
    let mut harness = shell_reporting("online", Some(1)).await;
    assert!(
        harness.render().contains("1 schema migration is pending"),
        "one pending migration is never `1 migrations`"
    );

    harness.key("esc");
    harness.settle().await;
    assert!(
        harness.app().overlays.is_empty(),
        "`Esc` falls through to the wildcard overlay binding"
    );
}

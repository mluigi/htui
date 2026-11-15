//! The Workspaces tab (MOD-116 M1 T3, D7-D10), through a `Harness` over the demo world.
//!
//! The Harness enters the first workspace by name, `Graphics` (`vulkan-tutorials`, whose `RUN_8`
//! is `stopped`), at startup; `Platform` (`htui`, `agy`) has `RUN_2` queued, `RUN_4` running and
//! `RUN_5` parked, and no stop row. The tab reads the states from the shell's run counts
//! (`TopBarState::workspace_runs`), which the shell asks for once a second and on every scope
//! change; a case that writes to the store refreshes (four ticks) and settles to see them move.
//!
//! A direct write goes into the same `MemStore` through a clone (`MemStore` shares its state across
//! clones). [`demo`] gives both workspaces a folder row on the demo's box, so no frame prints this
//! machine's home directory (`<home>/workspaces/<slug>` is the lib test's, over a temp home).
#![cfg(feature = "testkit")]

use std::sync::{Arc, Mutex};

use chrono::Utc;
use htui::app::Action;
use htui::keys::KeyChord;
use htui::store_worker::StoreReply;
use htui::testkit::{Harness, SectionBench};
use htui::ui::state_file::UiStore;
use htui::ui::tabs::{Tab as _, WorkspacesTab};
use htui_core::fixtures::ids;
use htui_core::model::{
    DocumentId, NewOwnerDocument, NewProject, NewRepo, ProjectId, RepoBoxPath, RepoId, RunStatus,
    StopMode, WorkspaceBoxPath, WorkspaceProject,
};
use htui_core::store::{CasOutcome, MemStore, ReadStore as _, WriteStore as _};
use htui_store::{Backend, CacheStore, PgStore};

/// The demo world on the Workspaces tab alone, `width` x `height`.
async fn open_over(store: MemStore, width: u16, height: u16) -> Harness {
    let mut harness = Harness::over(store)
        .size(width, height)
        .with_tab(Box::new(WorkspacesTab::new()));
    harness.settle().await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    assert_eq!(harness.app().tabs.active_id(), Some(WorkspacesTab::ID));
    harness
}

/// The shell's once-a-second refresh, served: the counts are read and the tab asks again.
async fn refresh(harness: &mut Harness) {
    for _ in 0..4 {
        harness.app().update(Action::Tick);
    }
    harness.settle().await;
}

/// The frame's row that holds `needle`, trimmed of the frame's borders.
fn row_with(frame: &str, needle: &str) -> String {
    frame
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no row holds `{needle}`:\n{frame}"))
        .to_owned()
}

/// The list's group row of the workspace `name` (`▾ name` or, folded, `▸ name`).
fn group(frame: &str, name: &str) -> String {
    let (open, folded) = (format!("\u{25be} {name}"), format!("\u{25b8} {name}"));
    frame
        .lines()
        .find(|line| line.contains(&open) || line.contains(&folded))
        .unwrap_or_else(|| panic!("no group row for `{name}`:\n{frame}"))
        .to_owned()
}

/// The list's selected row (the cursor's `▌` in the list's first column).
fn selected(frame: &str) -> String {
    frame
        .lines()
        .find(|line| line.starts_with("\u{2502}\u{258c}"))
        .unwrap_or_else(|| panic!("no row is selected:\n{frame}"))
        .to_owned()
}

/// The frame's row after the one that holds `needle`.
fn row_after(frame: &str, needle: &str) -> String {
    let mut lines = frame.lines().skip_while(|line| !line.contains(needle));
    let _ = lines.next();
    lines
        .next()
        .unwrap_or_else(|| panic!("no row after `{needle}`:\n{frame}"))
        .to_owned()
}

/// The demo with a folder row on the demo's box for both workspaces.
async fn demo() -> MemStore {
    let store = MemStore::demo();
    for (workspace, folder) in [
        (ids::WORKSPACE_GRAPHICS, "/srv/graphics"),
        (ids::WORKSPACE_PLATFORM, "/srv/platform"),
    ] {
        store
            .upsert_workspace_box_path(&WorkspaceBoxPath {
                workspace_id: workspace,
                box_id: ids::BOX,
                folder_path: folder.to_owned(),
                updated_at: Utc::now(),
            })
            .await
            .expect("a folder row is written");
    }
    store
}

/// A `goals` document on `owner` with `body`.
async fn write_goals(store: &MemStore, owner: ProjectId, body: &str) {
    store
        .write_owner_document(NewOwnerDocument {
            id: DocumentId::new(),
            project_id: owner,
            kind: "goals".to_owned(),
            title: "Goals".to_owned(),
            body: body.to_owned(),
            created_by: ids::USER,
            created_at: Utc::now(),
        })
        .await
        .expect("a goals document is written");
}

/// D8: one group per workspace in name order with its state, then `overview` and its projects;
/// the open workspace's overview on the right.
#[tokio::test]
async fn the_list_groups_every_workspace_with_its_state_and_entries() {
    let mut harness = open_over(demo().await, 100, 30).await;
    let frame = harness.render();
    assert!(
        group(&frame, "Graphics").contains("Graphics  \u{25cf} open"),
        "{frame}"
    );
    assert!(
        group(&frame, "Platform").contains("Platform  \u{25d0} background \u{b7} 1 running"),
        "{frame}"
    );
    for entry in ["overview", "Vulkan Tutorials", "htui", "agy"] {
        assert!(frame.contains(entry), "{entry}:\n{frame}");
    }
    assert_eq!(harness.app().status, None);
    insta::assert_snapshot!("list_100", frame);
}

/// D2 through the tab: a stop on Platform with `RUN_4` still running reads `stopping…`; once it
/// is done the stop has settled, and its queued and parked runs are held.
#[tokio::test]
async fn a_stopping_and_a_stopped_workspace_read_so() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;

    store
        .stop_workspace(
            ids::WORKSPACE_PLATFORM,
            StopMode::Graceful,
            ids::USER,
            Utc::now(),
        )
        .await
        .expect("Platform stops");
    refresh(&mut harness).await;
    let frame = harness.render();
    assert!(
        group(&frame, "Platform").contains("\u{25cc} stopping\u{2026}"),
        "{frame}"
    );

    store
        .finish_run(ids::RUN_4, RunStatus::Done, None, Utc::now())
        .await
        .expect("RUN_4 finishes");
    refresh(&mut harness).await;
    let frame = harness.render();
    assert!(
        group(&frame, "Platform").contains("\u{25cb} stopped \u{b7} 2 held"),
        "{frame}"
    );
}

/// D8: the workspace overview: the state, the folder, the default target, the held runs, the
/// goals and one line per project.
#[tokio::test]
async fn the_workspace_overview_lists_state_folder_goals_projects_target_and_held() {
    let store = demo().await;
    write_goals(
        &store,
        ids::PROJECT_SCOPE_GRAPHICS,
        "Ship the renderer walkthroughs.\nKeep every chapter building.",
    )
    .await;
    let mut harness = open_over(store, 100, 30).await;
    let frame = harness.render();
    for needle in [
        "folder on this box",
        "default target      inherits the Settings",
        "default (this box)",
        "held runs           1",
        "Ship the renderer walkthroughs.",
        "Keep every chapter building.",
        "Vulkan Tutorials  no repos yet",
        "  2 items \u{b7} 0 running \u{b7} 0 waiting",
        "folder on this box  /srv/graphics",
    ] {
        assert!(frame.contains(needle), "{needle}:\n{frame}");
    }
    insta::assert_snapshot!("overview_100", frame);
}

/// D8: a project row shows its repos and where they are, its description and its items by
/// state.
#[tokio::test]
async fn a_project_row_shows_its_repos_description_and_item_counts() {
    let store = demo().await;
    // `htui` has no repo in the demo: a primary one checked out here, and one that is not.
    for (name, primary, path) in [
        ("core", true, Some("/srv/platform/htui")),
        ("docs", false, None),
    ] {
        let repo = store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: name.to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: primary,
            })
            .await
            .expect("a repo is created");
        if let Some(path) = path {
            store
                .upsert_repo_box_path(&RepoBoxPath {
                    repo_id: repo.id,
                    box_id: ids::BOX,
                    local_path: path.to_owned(),
                    updated_at: Utc::now(),
                })
                .await
                .expect("a checkout row is written");
        }
    }
    let mut harness = open_over(store, 100, 30).await;
    // Graphics, overview, Vulkan Tutorials, Platform, overview, htui.
    for _ in 0..5 {
        harness.key("j");
    }
    harness.settle().await;
    let frame = harness.render();
    for needle in [
        "repo core (primary)  /srv/platform/htui",
        "repo docs            not on this box",
        "The terminal UI and orchestrator",
        "Items",
        "needs you",
    ] {
        assert!(frame.contains(needle), "{needle}:\n{frame}");
    }
    insta::assert_snapshot!("project_100", frame);
}

/// D8: the detail pane carries one sub-tab chip, `overview`; MOD-117 and MOD-118 add theirs.
#[tokio::test]
async fn the_detail_carries_the_overview_chip_only() {
    let mut harness = open_over(demo().await, 100, 30).await;
    let frame = harness.render();
    let border = row_with(&frame, "\u{2500} Graphics");
    assert!(border.contains("overview"), "{border}");
    for other in ["components", "requirements", "memory"] {
        assert!(!border.contains(other), "{other}: {border}");
    }
}

/// D9: `o` opens the selected workspace through the shell's `SetScope`; the one left behind
/// reads its own state, and the tab stays in front.
#[tokio::test]
async fn o_opens_the_selected_workspace() {
    let mut harness = open_over(demo().await, 100, 30).await;
    // Graphics, overview, Vulkan Tutorials, Platform.
    for _ in 0..3 {
        harness.key("j");
    }
    harness.key("o");
    harness.settle().await;
    assert_eq!(harness.app().scope.workspace_id, ids::WORKSPACE_PLATFORM);
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(harness.app().tabs.active_id(), Some(WorkspacesTab::ID));
    let frame = harness.render();
    assert!(
        group(&frame, "Platform").contains("Platform  \u{25cf} open \u{b7} 1 running"),
        "{frame}"
    );
    assert!(
        group(&frame, "Graphics").contains("Graphics  \u{25cb} stopped \u{b7} 1 held"),
        "{frame}"
    );
    assert_eq!(harness.app().status, None);
}

/// D9: `o` on the open workspace enters it again, which reloads every tab (the waiting list is
/// dropped and asked for again), as the switcher's `Enter` on its current row did.
#[tokio::test]
async fn o_on_the_open_workspace_re_enters_it() {
    let mut harness = open_over(demo().await, 100, 30).await;
    assert!(
        harness.app().top_bar.waiting.is_some(),
        "the first read landed"
    );
    harness.key("o");
    assert_eq!(
        harness.app().top_bar.waiting,
        None,
        "the scope was entered again"
    );
    assert!(harness.queued() > 0, "and every read asked again");
    harness.settle().await;
    assert_eq!(harness.app().scope.workspace_id, ids::WORKSPACE_GRAPHICS);
    assert_eq!(harness.app().tabs.active_id(), Some(WorkspacesTab::ID));
    assert_eq!(harness.app().status, None);
}

/// D7: a change in the selected workspace's counts reads its overview again on the next refresh.
#[tokio::test]
async fn the_overview_is_read_again_when_its_counts_change() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    for _ in 0..3 {
        harness.key("j");
    }
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{25d0} background \u{b7} 1 running"),
        "{frame}"
    );
    assert!(
        row_after(&frame, "\u{2502}\u{2502}htui  ").contains("1 running"),
        "{frame}"
    );

    store
        .finish_run(ids::RUN_4, RunStatus::Done, None, Utc::now())
        .await
        .expect("RUN_4 finishes");
    refresh(&mut harness).await;
    refresh(&mut harness).await;
    let frame = harness.render();
    let detail = row_with(&frame, "\u{2502}\u{25d0} background");
    assert!(detail.contains("background \u{b7} 0 running"), "{frame}");
    assert!(
        row_after(&frame, "\u{2502}\u{2502}htui  ").contains("0 running"),
        "{frame}"
    );
}

/// D8: no workspace says so and names the next step, the key Settings › Hierarchy binds.
#[tokio::test]
async fn an_empty_store_says_how_to_create_a_workspace() {
    let mut harness = Harness::empty().with_tab(Box::new(WorkspacesTab::new()));
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("No workspaces yet."), "{frame}");
    assert!(
        frame.contains("`N` in Settings \u{203a} Hierarchy creates one"),
        "{frame}"
    );
    assert_eq!(harness.app().status, None);
}

/// D7, D8: offline the overview is refused; the detail pane says it needs the database and the
/// status line stays clear (the counts' refusal too). The open workspace still reads `open`.
#[tokio::test]
async fn a_refused_overview_says_it_needs_the_database_and_stays_off_the_status_line() {
    let _keyring = htui_store::testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "workspaces-offline", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    htui_store::testkit::seed_mirror(&cache, &htui_core::fixtures::demo_data())
        .await
        .expect("the mirror is seeded");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(WorkspacesTab::new()))
    .with_store_state("offline \u{b7} 0s", None);
    harness.settle().await;
    refresh(&mut harness).await;
    let frame = harness.render();
    assert!(frame.contains("the overview needs the database"), "{frame}");
    assert!(
        group(&frame, "Graphics").contains("\u{25cf} open"),
        "{frame}"
    );
    assert!(
        !group(&frame, "Platform").contains("background"),
        "no counts, no state: {frame}"
    );
    assert_eq!(harness.app().status, None);
}

/// D8 at 80 columns: both panes keep their content readable.
#[tokio::test]
async fn a_narrow_terminal_keeps_both_panes_readable() {
    let mut harness = open_over(demo().await, 80, 24).await;
    let frame = harness.render();
    assert!(frame.contains("Graphics"), "{frame}");
    assert!(frame.contains("Platform"), "{frame}");
    assert!(frame.contains("default target"), "{frame}");
    insta::assert_snapshot!("list_80", frame);
}

/// MOD-115 M2 D5 through the tab: `location` names the selected row; `restore` goes back to it,
/// unfolding its group, and is kept for a list that has not landed yet.
#[tokio::test]
async fn location_and_restore_return_to_the_entry() {
    let bench = SectionBench::new().await;
    let workspaces = MemStore::demo().workspaces().await.expect("a memory read");
    let key = |tab: &mut WorkspacesTab, chord: &str| {
        let event = KeyChord::parse(chord).expect("a chord").to_event();
        let _ = tab.on_key(event, &mut bench.ctx());
    };

    let mut tab = WorkspacesTab::new();
    tab.on_reply(
        &StoreReply::Workspaces(workspaces.clone()),
        &mut bench.ctx(),
    );
    assert_eq!(
        tab.location().cursor,
        Some(format!("workspace:{}", ids::WORKSPACE_GRAPHICS))
    );
    key(&mut tab, "j");
    key(&mut tab, "j");
    let loc = tab.location();
    assert_eq!(
        loc.cursor,
        Some(format!(
            "project:{}:{}",
            ids::WORKSPACE_GRAPHICS,
            ids::PROJECT_VULKAN
        ))
    );

    key(&mut tab, "k");
    key(&mut tab, "k");
    key(&mut tab, "enter");
    assert_eq!(
        tab.location().cursor,
        Some(format!("workspace:{}", ids::WORKSPACE_GRAPHICS))
    );
    assert!(tab.restore(&loc, &mut bench.ctx()), "the row is listed");
    assert_eq!(tab.location(), loc, "and its folded group opened");
    key(&mut tab, "k");
    assert_eq!(
        tab.location().cursor,
        Some(format!("overview:{}", ids::WORKSPACE_GRAPHICS)),
        "the group's rows are listed again"
    );

    let mut gone = loc.clone();
    gone.cursor = Some(format!(
        "workspace:{}",
        htui_core::model::WorkspaceId::new()
    ));
    assert!(!tab.restore(&gone, &mut bench.ctx()), "not listed");
    assert!(!tab.restore(&htui::ui::shell::TabLoc::default(), &mut bench.ctx()));

    let mut fresh = WorkspacesTab::new();
    assert!(fresh.restore(&loc, &mut bench.ctx()), "kept for the list");
    fresh.on_reply(&StoreReply::Workspaces(workspaces), &mut bench.ctx());
    assert_eq!(fresh.location(), loc);
}

/// D7: a target on the scope row is named in the overview instead of the inherited default.
#[tokio::test]
async fn a_scope_row_target_reads_as_set() {
    let store = MemStore::demo();
    let row = store
        .project(ids::PROJECT_SCOPE_GRAPHICS)
        .await
        .expect("a memory read")
        .expect("Graphics' scope row");
    let outcome = store
        .set_project_target(
            row.id,
            row.updated_at,
            Some(htui_core::seed::LOCAL_TARGET_ID),
        )
        .await
        .expect("the target is set");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
    let mut harness = open_over(store, 100, 30).await;
    let frame = harness.render();
    assert!(
        row_with(&frame, "default target").contains("default target      this box"),
        "{frame}"
    );
}

/// MOD-116 M1 R1 F3 (D7): a refused overview is not held: the next refresh asks again though no
/// count moved, and the answer replaces the refusal; neither reaches the status line.
#[tokio::test]
async fn a_refused_overview_is_asked_again_on_the_next_refresh() {
    let mut harness = open_over(demo().await, 100, 30).await;
    // `o` on the open workspace enters it again, and the activation reads the overview again.
    harness.key("o");
    harness.settle_refusing("workspace_overview", "gone").await;
    let frame = harness.render();
    assert!(
        frame.contains("the overview needs the database: gone"),
        "{frame}"
    );
    refresh(&mut harness).await;
    let frame = harness.render();
    assert!(!frame.contains("needs the database"), "{frame}");
    assert!(
        frame.contains("folder on this box  /srv/graphics"),
        "{frame}"
    );
    assert_eq!(harness.app().status, None);
}

/// MOD-116 M1 R1 F2 (D7): a project added after the overview was read shows the reading state,
/// not a blank pane, once the list holds it and until the overview read again lands; then its
/// own overview.
#[tokio::test]
async fn a_project_added_after_the_read_reads_then_shows_its_overview() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    let shaders = store
        .create_project(NewProject {
            id: ProjectId::new(),
            slug: "shaders".to_owned(),
            name: "Shaders".to_owned(),
            description: "Shader experiments".to_owned(),
            created_by: ids::USER,
        })
        .await
        .expect("a project is created");
    store
        .upsert_workspace_project(&WorkspaceProject {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_id: shaders.id,
            position: 10,
        })
        .await
        .expect("it joins Graphics");
    // `o` on the open workspace reads the list again, then the overview: the list lands alone.
    harness.key("o");
    assert!(harness.serve_next().await, "the list is served");
    // Graphics, overview, Vulkan Tutorials, Shaders.
    for _ in 0..3 {
        harness.key("j");
    }
    let frame = harness.render();
    assert!(selected(&frame).contains("Shaders"), "{frame}");
    assert!(frame.contains("reading the overview"), "{frame}");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("Shader experiments"), "{frame}");
    assert!(!frame.contains("reading the overview"), "{frame}");
}

/// MOD-116 M1 R1 F4 (MOD-115 M2 D1, D12): a `ui-state.toml` that failed to load is left as it is
/// by the startup scope, which is no change of the user's; an `o` onto another workspace saves.
#[tokio::test]
async fn startup_leaves_a_ui_state_that_failed_to_load_alone_and_o_saves() {
    let broken = "version = \"two\"\n";
    let text = Arc::new(Mutex::new(Some(broken.to_owned())));
    let ui = UiStore::Memory(Arc::clone(&text));
    assert!(ui.load().notice.is_some(), "the file does not load");
    let mut harness = Harness::over(demo().await)
        .size(100, 30)
        .with_ui_store(ui)
        .with_tab(Box::new(WorkspacesTab::new()));
    harness.settle().await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    assert_eq!(
        text.lock().expect("the lock").as_deref(),
        Some(broken),
        "the startup scope saved nothing"
    );
    // Graphics, overview, Vulkan Tutorials, Platform.
    for _ in 0..3 {
        harness.key("j");
    }
    harness.key("o");
    harness.settle().await;
    let saved = text
        .lock()
        .expect("the lock")
        .clone()
        .expect("the file is there");
    assert!(
        saved.contains(&format!("last = \"{}\"", ids::WORKSPACE_PLATFORM)),
        "{saved}"
    );
}

/// MOD-116 M1 R1 F6 (D8 over D5): offline, the counts held are the last ones read, not current:
/// the other workspaces read no state, the open one reads `● open`, and the header draws no
/// background chip; back online both return.
#[tokio::test]
async fn offline_the_last_counts_are_not_shown_as_current() {
    let mut harness = open_over(demo().await, 160, 30).await;
    let background = "+1 in background \u{25d0} 1";
    let frame = harness.render();
    assert!(
        group(&frame, "Platform").contains("\u{25d0} background \u{b7} 1 running"),
        "{frame}"
    );
    assert!(
        frame
            .lines()
            .next()
            .unwrap_or_default()
            .contains(background),
        "{frame}"
    );

    harness.app().top_bar.store = "offline \u{b7} 0s".to_owned();
    let frame = harness.render();
    let platform = group(&frame, "Platform");
    assert!(
        !platform.contains('\u{25d0}') && !platform.contains("running"),
        "{frame}"
    );
    assert!(
        group(&frame, "Graphics").contains("Graphics  \u{25cf} open"),
        "{frame}"
    );
    assert!(
        !frame
            .lines()
            .next()
            .unwrap_or_default()
            .contains("background"),
        "{frame}"
    );

    harness.app().top_bar.store = "memory".to_owned();
    let frame = harness.render();
    assert!(
        group(&frame, "Platform").contains("\u{25d0} background \u{b7} 1 running"),
        "{frame}"
    );
    assert!(
        frame
            .lines()
            .next()
            .unwrap_or_default()
            .contains(background),
        "{frame}"
    );
}

/// MOD-116 M1 R1 F7: the cursor stops at both ends (`k` on the first row, `j` past the last),
/// and `ctrl-j`, which no list motion binds, does not move it.
#[tokio::test]
async fn the_cursor_stops_at_both_ends_and_ctrl_j_does_not_move_it() {
    let mut harness = open_over(demo().await, 100, 30).await;
    harness.key("k");
    harness.settle().await;
    assert!(
        selected(&harness.render()).contains("\u{25be} Graphics"),
        "k on the first row stays"
    );
    // Seven rows: Graphics, overview, Vulkan Tutorials, Platform, overview, htui, agy.
    for _ in 0..10 {
        harness.key("j");
    }
    harness.settle().await;
    let frame = harness.render();
    assert!(selected(&frame).contains("agy"), "{frame}");
    harness.key("k");
    harness.key("ctrl-j");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        selected(&frame).contains("htui"),
        "ctrl-j moved nothing: {frame}"
    );
}

/// MOD-116 M1 R1 F7 (D8): `list.fold` on a child row folds its group (`▸`, its rows hidden) and
/// takes the cursor up to the group's row.
#[tokio::test]
async fn folding_from_a_child_row_hides_the_group_s_rows() {
    let mut harness = open_over(demo().await, 100, 30).await;
    // Graphics, overview, Vulkan Tutorials.
    harness.key("j");
    harness.key("j");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        selected(&frame).contains("\u{25b8} Graphics"),
        "the cursor is on the folded group: {frame}"
    );
    assert!(
        row_after(&frame, "\u{25b8} Graphics").contains("\u{25be} Platform"),
        "its rows are hidden: {frame}"
    );
    insta::assert_snapshot!("folded_100", frame);
}

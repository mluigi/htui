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
    DocumentId, NewOwnerDocument, NewProject, NewRepo, NewWorkspace, ProjectId, RepoBoxPath,
    RepoId, RunStatus, StopMode, WorkspaceBoxPath, WorkspaceId, WorkspaceProject,
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

/// D8: no workspace says so and names the next step, the key that creates one here (MOD-116 M3
/// D1).
#[tokio::test]
async fn an_empty_store_says_how_to_create_a_workspace() {
    let mut harness = Harness::empty().with_tab(Box::new(WorkspacesTab::new()));
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("No workspaces yet."), "{frame}");
    assert!(frame.contains("`N` creates one"), "{frame}");
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
    gone.cursor = Some(format!("workspace:{}", WorkspaceId::new()));
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

/// MOD-116 M2: the cursor onto Platform's group row (Graphics, overview, Vulkan Tutorials,
/// Platform), its overview read.
async fn onto_platform(harness: &mut Harness) {
    for _ in 0..3 {
        harness.key("j");
    }
    harness.settle().await;
    assert!(
        selected(&harness.render()).contains("\u{25be} Platform"),
        "the cursor is on Platform"
    );
}

/// The text of the kit box titled `title`: its content rows between its borders, each trimmed,
/// joined by one space, so a sentence the box wrapped reads whole again.
fn boxed(frame: &str, title: &str) -> String {
    let rows: Vec<Vec<char>> = frame.lines().map(|line| line.chars().collect()).collect();
    let top = rows
        .iter()
        .position(|row| row.iter().collect::<String>().contains(title))
        .unwrap_or_else(|| panic!("no box titled `{title}`:\n{frame}"));
    let at = rows[top]
        .iter()
        .collect::<String>()
        .find(title)
        .map(|byte| {
            frame.lines().nth(top).unwrap_or_default()[..byte]
                .chars()
                .count()
        })
        .unwrap_or_default();
    let left = rows[top][..at]
        .iter()
        .rposition(|cell| *cell == '\u{256d}')
        .unwrap_or_else(|| panic!("no corner left of `{title}`:\n{frame}"));
    let right = rows[top][at..]
        .iter()
        .position(|cell| *cell == '\u{256e}')
        .map(|offset| at + offset)
        .unwrap_or_else(|| panic!("no corner right of `{title}`:\n{frame}"));
    let mut words = Vec::new();
    for row in &rows[top + 1..] {
        if row.get(left) == Some(&'\u{2570}') {
            break;
        }
        let content: String = row
            .get(left + 1..right)
            .unwrap_or_default()
            .iter()
            .collect();
        words.extend(content.split_whitespace().map(str::to_owned));
    }
    words.join(" ")
}

/// `workspace`'s stop row's mode, if any.
async fn stop_of(store: &MemStore, workspace: WorkspaceId) -> Option<StopMode> {
    store
        .workspace_stop(workspace)
        .await
        .expect("the stop row is read")
        .map(|row| row.mode)
}

/// A stop of `mode` written on `workspace` straight into the store.
async fn stop_directly(store: &MemStore, workspace: WorkspaceId, mode: StopMode) {
    store
        .stop_workspace(workspace, mode, ids::USER, Utc::now())
        .await
        .expect("the stop is written");
}

/// The hint row of the focused pane (the list's bottom border).
fn hint_row(frame: &str) -> String {
    row_with(frame, "o open")
}

/// MOD-116 M2 D6: `s` on Platform asks to stop its runs gracefully, listing every live run with
/// what the stop does to it: the parity metric is `stop_report::report` over the same store.
#[tokio::test]
async fn s_asks_to_stop_the_runs_listing_each() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    onto_platform(&mut harness).await;
    harness.key("s");
    harness.settle().await;
    let frame = harness.render();
    let text = boxed(&frame, "Stop the runs in Platform?");
    assert!(
        text.starts_with(
            "Each agent step stops at its turn boundary, checks stop at once; nothing new starts \
             until it is resumed."
        ),
        "{text}"
    );
    let agents = store.agents().await.expect("the agents are read");
    let names = htui::stop_report::agent_names(&agents);
    let caps = htui::stop_report::caps_map(agents);
    let report = htui::stop_report::report(
        &store,
        &caps,
        Some(&names),
        ids::WORKSPACE_PLATFORM,
        "platform",
    )
    .await
    .expect("the report is read");
    let live: Vec<_> = report
        .lines
        .iter()
        .filter(|line| {
            matches!(
                line.status,
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval
            )
        })
        .collect();
    assert_eq!(live.len(), 3, "{report:?}");
    for line in live {
        assert!(
            text.contains(&format!("{}  {}", line.item, line.status_words()).replace("  ", " ")),
            "{} in {text}",
            line.item
        );
        for fate in line.under(StopMode::Graceful) {
            assert!(text.contains(&fate), "{fate} in {text}");
        }
    }
    assert_eq!(stop_of(&store, ids::WORKSPACE_PLATFORM).await, None);
    assert_eq!(harness.app().status, None);
    insta::assert_snapshot!("stop_question_100", frame);
}

/// D6: `y` writes a graceful stop as this user, and the state moves at once; `Enter` answers
/// the same.
#[tokio::test]
async fn confirming_writes_a_graceful_stop_and_the_state_moves() {
    for answer in ["y", "enter"] {
        let store = demo().await;
        let mut harness = open_over(store.clone(), 100, 30).await;
        onto_platform(&mut harness).await;
        harness.key("s");
        harness.settle().await;
        harness.key(answer);
        harness.settle().await;
        let row = store
            .workspace_stop(ids::WORKSPACE_PLATFORM)
            .await
            .expect("the stop row is read")
            .unwrap_or_else(|| panic!("{answer} wrote a stop"));
        assert_eq!(row.mode, StopMode::Graceful, "{answer}");
        assert_eq!(row.requested_by, ids::USER, "{answer}");
        let frame = harness.render();
        assert!(!frame.contains("Stop the runs in"), "{answer}: {frame}");
        assert!(
            group(&frame, "Platform").contains("\u{25cc} stopping\u{2026}"),
            "{answer}: {frame}"
        );
        assert_eq!(harness.app().status, None);
    }
}

/// D6: `Esc` closes the question and writes nothing.
#[tokio::test]
async fn esc_on_the_stop_question_writes_nothing() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    onto_platform(&mut harness).await;
    harness.key("s");
    harness.settle().await;
    harness.key("j");
    harness.settle().await;
    assert!(
        harness.render().contains("Stop the runs in Platform?"),
        "a list key does not close it"
    );
    harness.key("esc");
    harness.settle().await;
    let frame = harness.render();
    assert!(!frame.contains("Stop the runs in"), "{frame}");
    assert!(
        selected(&frame).contains("\u{25be} Platform"),
        "`j` under the question moved nothing: {frame}"
    );
    assert_eq!(stop_of(&store, ids::WORKSPACE_PLATFORM).await, None);
}

/// D6: `s` on a stopping workspace asks to force it; `Esc` keeps the graceful stop, `y` forces.
#[tokio::test]
async fn s_on_a_stopping_workspace_asks_to_force_and_esc_keeps_the_graceful_stop() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Graceful).await;
    refresh(&mut harness).await;
    onto_platform(&mut harness).await;
    harness.key("s");
    harness.settle().await;
    let frame = harness.render();
    let text = boxed(&frame, "Stop the runs in Platform now?");
    assert!(
        text.starts_with(
            "Every step still running is killed mid-turn and redoes its turn on resume."
        ),
        "{text}"
    );
    assert!(text.contains("keep the graceful stop") || frame.contains("keep the graceful stop"));
    insta::assert_snapshot!("force_question_100", frame);

    harness.key("esc");
    harness.settle().await;
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Graceful)
    );

    harness.key("s");
    harness.settle().await;
    harness.key("y");
    harness.settle().await;
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Forced)
    );
}

/// D6: `s` on Graphics, whose one run is stopped already, says nothing is running and writes no
/// stop row.
#[tokio::test]
async fn nothing_running_says_so_and_writes_nothing() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    harness.key("s");
    harness.settle().await;
    let frame = harness.render();
    assert_eq!(boxed(&frame, "Stop runs"), "Nothing is running in Graphics");
    insta::assert_snapshot!("nothing_running_100", frame);
    harness.key("y");
    harness.settle().await;
    assert!(!harness.render().contains("Nothing is running"));
    assert_eq!(stop_of(&store, ids::WORKSPACE_GRAPHICS).await, None);
}

/// D6: `s` on a forced stop says it is already stopped.
#[tokio::test]
async fn a_forced_workspace_is_already_stopped() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Forced).await;
    refresh(&mut harness).await;
    onto_platform(&mut harness).await;
    harness.key("s");
    harness.settle().await;
    let frame = harness.render();
    assert_eq!(
        boxed(&frame, "Stop runs"),
        "Platform is already stopped (forced)"
    );
    harness.key("esc");
    harness.settle().await;
    assert!(!harness.render().contains("already stopped"));
}

/// D6: `r` on a stopped workspace asks, listing every run with how it resumes; `y` deletes the
/// stop row and Platform reads background again.
#[tokio::test]
async fn r_asks_then_resumes() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Graceful).await;
    refresh(&mut harness).await;
    onto_platform(&mut harness).await;
    harness.key("r");
    harness.settle().await;
    let frame = harness.render();
    let text = boxed(&frame, "Resume Platform?");
    assert!(
        text.starts_with(
            "Its stopped runs resume on the boxes that ran them and its queued runs can start."
        ),
        "{text}"
    );
    assert!(
        text.contains("starts when the workspace is resumed"),
        "{text}"
    );
    insta::assert_snapshot!("resume_question_100", frame);
    harness.key("y");
    harness.settle().await;
    assert_eq!(stop_of(&store, ids::WORKSPACE_PLATFORM).await, None);
    let frame = harness.render();
    assert!(
        group(&frame, "Platform").contains("\u{25d0} background"),
        "{frame}"
    );
}

/// D6: `r` on a workspace with no stop row says so.
#[tokio::test]
async fn r_on_a_workspace_not_stopped_says_so() {
    let mut harness = open_over(demo().await, 100, 30).await;
    onto_platform(&mut harness).await;
    harness.key("r");
    harness.settle().await;
    assert_eq!(
        boxed(&harness.render(), "Resume runs"),
        "Platform is not stopped"
    );
}

/// D3/D6: a stop written by someone else while the question is up: `y` writes nothing, and the
/// box says so; the row is theirs.
#[tokio::test]
async fn a_stop_changed_while_asking_is_refused_and_said() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    onto_platform(&mut harness).await;
    harness.key("s");
    harness.settle().await;
    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Forced).await;
    harness.key("y");
    harness.settle().await;
    assert_eq!(
        boxed(&harness.render(), "Stop runs"),
        "The stop of Platform changed while asking; nothing was stopped. Press s again."
    );
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Forced)
    );
    assert_eq!(harness.app().status, None);
}

/// MOD-116 M2 R1 F6: a workspace deleted between `s` and its fresh overview is told as gone, in
/// the box; `r` the same; nothing is written and the status line stays clear.
#[tokio::test]
async fn s_or_r_on_a_workspace_deleted_meanwhile_says_it_no_longer_exists() {
    for (key, title) in [("s", "Stop runs"), ("r", "Resume runs")] {
        let store = demo().await;
        store
            .create_workspace(NewWorkspace {
                id: WorkspaceId::new(),
                slug: "scratch".to_owned(),
                name: "Scratch".to_owned(),
                description: String::new(),
                created_by: ids::USER,
            })
            .await
            .expect("a workspace is created");
        let mut harness = open_over(store.clone(), 100, 30).await;
        harness.key("G");
        harness.settle().await;
        let frame = harness.render();
        assert!(
            selected(&frame).contains("overview") && frame.contains("\u{256d}\u{2500} Scratch "),
            "{key}: the cursor is on Scratch's overview: {frame}"
        );
        let scratch = store
            .workspaces()
            .await
            .expect("the workspaces are read")
            .into_iter()
            .find(|summary| summary.slug == "scratch")
            .expect("Scratch is listed")
            .workspace_id;
        harness.key(key);
        store
            .delete_workspace(scratch)
            .await
            .expect("Scratch is deleted");
        harness.settle().await;
        assert_eq!(
            boxed(&harness.render(), title),
            "Scratch no longer exists",
            "{key}"
        );
        assert_eq!(harness.app().status, None, "{key}");
    }
}

/// D6: a stop whose overview read is refused (offline) says it needs the database, in the box,
/// not on the status line.
#[tokio::test]
async fn offline_the_stop_says_it_needs_the_database() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    onto_platform(&mut harness).await;
    harness.key("s");
    harness
        .settle_refusing("workspace_overview", "the database is unreachable")
        .await;
    assert_eq!(
        boxed(&harness.render(), "Stop runs"),
        "Stopping needs the database: the database is unreachable"
    );
    assert_eq!(harness.app().status, None);
    assert_eq!(stop_of(&store, ids::WORKSPACE_PLATFORM).await, None);
}

/// R-ORCH-16, D7: the open workspace stops too, and reads `open` with its stop after its counts.
#[tokio::test]
async fn the_open_workspace_stops_too_and_reads_open_stopping() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    onto_platform(&mut harness).await;
    harness.key("o");
    harness.settle().await;
    harness.key("s");
    harness.settle().await;
    harness.key("y");
    harness.settle().await;
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Graceful)
    );
    let frame = harness.render();
    assert!(
        group(&frame, "Platform")
            .contains("\u{25cf} open \u{b7} 1 running \u{b7} stopping\u{2026}"),
        "{frame}"
    );
    insta::assert_snapshot!("open_stopping_100", frame);
}

/// D5: the overview lists the workspace's runs and what the stop does to each.
#[tokio::test]
async fn the_overview_lists_the_runs_and_how_each_resumes() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Graceful).await;
    refresh(&mut harness).await;
    onto_platform(&mut harness).await;
    let frame = harness.render();
    assert!(frame.contains("\u{2502}Runs "), "{frame}");
    assert!(
        frame.contains("starts when the workspace is resumed"),
        "{frame}"
    );
    assert!(
        frame.contains("its gate can be answered once resumed"),
        "{frame}"
    );
    insta::assert_snapshot!("overview_runs_100", frame);
}

/// D8: the hint row offers what the selected workspace's state allows.
#[tokio::test]
async fn the_hints_follow_the_state() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    onto_platform(&mut harness).await;
    let hints = hint_row(&harness.render());
    assert!(
        hints.contains("s stop") && !hints.contains("r resume"),
        "{hints}"
    );

    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Graceful).await;
    refresh(&mut harness).await;
    let hints = hint_row(&harness.render());
    assert!(
        hints.contains("s force stop") && hints.contains("r resume"),
        "{hints}"
    );

    store
        .finish_run(ids::RUN_4, RunStatus::Done, None, Utc::now())
        .await
        .expect("RUN_4 finishes");
    refresh(&mut harness).await;
    let hints = hint_row(&harness.render());
    assert!(
        hints.contains("r resume") && !hints.contains("s "),
        "{hints}"
    );

    for _ in 0..3 {
        harness.key("k");
    }
    harness.settle().await;
    let frame = harness.render();
    assert!(selected(&frame).contains("\u{25be} Graphics"), "{frame}");
    let hints = hint_row(&frame);
    assert!(
        !hints.contains("s stop") && !hints.contains("r resume"),
        "{hints}"
    );
}

/// D7 (fact-check): offline a workspace that is not open shows no state in its detail, no Runs
/// section and M1's hints; back online they return.
#[tokio::test]
async fn offline_a_non_open_workspace_s_detail_shows_no_state() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    stop_directly(&store, ids::WORKSPACE_PLATFORM, StopMode::Graceful).await;
    refresh(&mut harness).await;
    onto_platform(&mut harness).await;
    let frame = harness.render();
    assert!(frame.contains("\u{2502}Runs "), "{frame}");
    assert!(frame.contains("stopping"), "{frame}");
    assert!(hint_row(&frame).contains("r resume"), "{frame}");

    harness.app().top_bar.store = "offline \u{b7} 0s".to_owned();
    let frame = harness.render();
    for absent in [
        "\u{25d0}",
        "\u{25cc}",
        "\u{25cb}",
        "background",
        "stopping",
        "\u{2502}Runs ",
    ] {
        assert!(!frame.contains(absent), "{absent}: {frame}");
    }
    assert!(frame.contains("folder on this box"), "{frame}");
    assert!(!hint_row(&frame).contains("r resume"), "{frame}");

    harness.app().top_bar.store = "memory".to_owned();
    let frame = harness.render();
    assert!(frame.contains("\u{2502}Runs "), "{frame}");
    assert!(frame.contains("stopping"), "{frame}");
    assert!(hint_row(&frame).contains("r resume"), "{frame}");
}

/// D7 (fact-check): offline the open workspace's detail reads `● open` alone: no stop
/// annotation, no Runs section.
#[tokio::test]
async fn offline_the_open_workspace_s_detail_reads_open_only() {
    let store = demo().await;
    let mut harness = open_over(store.clone(), 100, 30).await;
    stop_directly(&store, ids::WORKSPACE_GRAPHICS, StopMode::Graceful).await;
    // One refresh reads the counts, the next one sees the row moved and reads the overview.
    refresh(&mut harness).await;
    refresh(&mut harness).await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{2502}\u{25cf} open \u{b7} stopped \u{b7} 1 held"),
        "{frame}"
    );
    assert!(frame.contains("\u{2502}Runs "), "{frame}");

    harness.app().top_bar.store = "offline \u{b7} 0s".to_owned();
    let frame = harness.render();
    assert!(frame.contains("\u{2502}\u{25cf} open"), "{frame}");
    assert!(!frame.contains("stopped"), "{frame}");
    assert!(!frame.contains("\u{2502}Runs "), "{frame}");
}

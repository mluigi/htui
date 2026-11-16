//! Shell-level tests for moving between workspaces and the migration prompt (blueprint E, MOD-6
//! E.5). MOD-116 M1 replaced the workspace switcher with the Workspaces tab (`w`, then `o`).
//!
//! Everything here goes through [`htui::testkit::Harness`] and the public `Overlay` contract: no
//! store handle, no database and no sleeps. The MOD-6 cases feed the shell a synthetic
//! `StoreReply::StoreState`, which is exactly what the store worker sends when it is offline or
//! when the schema has pending migrations, so the top bar and the prompt are tested without a
//! server.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::store_worker::{Origin, ReplyEnvelope, StoreReply};
use htui::testkit::Harness;
use htui::ui::overlay::WaitingList;
use htui::ui::tabs::WorkspacesTab;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{ItemId, NewItem};
use htui_core::store::{MemStore, WriteStore as _};

/// A `StoreState` reply addressed to the shell, as the fourth-tick refresh receives it.
fn store_state(label: &str, migrations_pending: Option<usize>) -> Action {
    Action::Reply(ReplyEnvelope {
        seq: 0,
        origin: Origin::App,
        reply: StoreReply::StoreState {
            label: label.to_owned(),
            migrations_pending,
            below_target: None,
        },
    })
}

/// The demo shell over a worker that reports this store state, settled and ready to render.
async fn shell_reporting(label: &str, migrations_pending: Option<usize>) -> Harness {
    let mut harness = Harness::demo()
        .with_store_state(label, migrations_pending)
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness
}

/// The demo shell on the Workspaces tab (`w`), settled and ready to render.
async fn on_the_workspaces_tab() -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness.key("w");
    harness.drive_to_end().await;
    harness
}

/// The demo shell with the waiting list (a modal overlay) open over it.
async fn waiting_over_demo() -> Harness {
    let mut harness = Harness::demo()
        .with_overlay(Box::new(WaitingList::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness
}

/// The header, i.e. the first line of a rendered frame (MOD-115: it replaced the top bar and the
/// tab strip).
fn top_bar(frame: &str) -> String {
    frame.lines().next().unwrap_or_default().to_owned()
}

/// The header drawn 160 columns wide, then the harness back at the default 100x30. At 100 the
/// seven tab chips and `? keys` leave no room for the working count and the store label.
fn wide_header(harness: &mut Harness) -> String {
    harness.resize(160, 30);
    let header = top_bar(&harness.render());
    harness.resize(100, 30);
    header
}

/// MOD-116 M1 D9: `o` on a workspace's row of the Workspaces tab enters it, and the header and
/// the Backlog follow.
#[tokio::test]
async fn o_on_the_workspaces_tab_switches_the_scope_and_the_top_bar_follows() {
    let mut harness = on_the_workspaces_tab().await;
    let header = wide_header(&mut harness);
    assert!(
        header.starts_with("Graphics") && header.contains("0 working · 0 waiting · memory"),
        "the fixture's only active run is in the other workspace: {header}"
    );

    // Graphics, overview, Vulkan Tutorials, Platform.
    for _ in 0..3 {
        harness.key("j");
    }
    harness.key("o");
    harness.drive_to_end().await;

    assert!(harness.app().overlays.is_empty(), "no overlay is involved");
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

    // Back to the Backlog, which read its rows for the new scope.
    harness.key("2");
    harness.drive_to_end().await;
    let header = wide_header(&mut harness);
    let frame = harness.render();
    assert!(
        // MOD-144's demo: RUN_2 and RUN_4 work; FEAT-2 (blocked) and TOOL-1 (parked RUN_5) wait.
        header.starts_with("Platform") && header.contains("2 working · 2 waiting · memory"),
        "the header reads the new workspace and its active-run count: {header}"
    );
    insta::assert_snapshot!("after_switch", frame);
}

/// MOD-116 M1 D9: moving the cursor on the Workspaces tab and leaving it enters nothing.
#[tokio::test]
async fn leaving_the_workspaces_tab_without_o_leaves_the_scope_alone() {
    let mut harness = on_the_workspaces_tab().await;
    for _ in 0..3 {
        harness.key("j");
    }
    harness.key("2");
    harness.drive_to_end().await;

    assert_eq!(
        harness.app().top_bar.workspace,
        "Graphics",
        "a cursor move that was never confirmed changes nothing"
    );
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("backlog")
    );
}

#[tokio::test]
async fn the_top_bar_shows_the_offline_age() {
    // What the worker reports once the first connect attempt has failed and three minutes passed.
    let mut harness = shell_reporting("offline · 3m", None).await;
    let header = wide_header(&mut harness);
    let frame = harness.render();
    assert!(
        header.starts_with("Graphics") && header.contains("0 working · 0 waiting · offline · 3m"),
        "the store field is `Backend::label()` verbatim (plan D11): {header}"
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
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty(), "`n` closes the prompt");

    // The fourth-tick refresh reports the same count a second later.
    harness.app().update(store_state("online", Some(3)));
    harness.drive_to_end().await;
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
    harness.drive_to_end().await;

    assert!(
        harness.app().overlays.is_empty(),
        "`y` closes the prompt as well as sending the request"
    );
    assert_eq!(
        harness.app().status.as_deref(),
        Some("applied 0 migrations"),
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
    harness.drive_to_end().await;
    assert!(
        harness.app().overlays.is_empty(),
        "`Esc` is `[migration] no` (and `overlay.close` below it)"
    );
}

/// MOD-52: a modal overlay (the waiting list, since MOD-116 M1 retired the switcher) swallows
/// `q`, but `ctrl-c` quits through the overlay stack's quit layer: it opens the leave dialog
/// (MOD-115 M2 D6), and a second one leaves.
#[tokio::test]
async fn ctrl_c_quits_from_a_modal_overlay() {
    let mut harness = waiting_over_demo().await;
    harness.key("q");
    assert!(
        !harness.app().should_quit,
        "the modal overlay still swallows `q`"
    );
    assert_eq!(
        harness.app().overlays.iter().count(),
        1,
        "and opens nothing"
    );
    harness.key("ctrl-c");
    assert_eq!(
        harness.app().overlays.top().map(|overlay| overlay.id()),
        Some(htui::ui::overlay::LeaveDialog::ID),
        "`ctrl-c` asks first"
    );
    harness.key("ctrl-c");
    assert!(harness.app().should_quit, "`ctrl-c` quits from it");
}

/// The keys line the status row listed before MOD-115 (nothing draws it any more).
fn status_line(harness: &mut Harness) -> String {
    harness.app().keys_line()
}

/// MOD-67 M3 (D14): the prompt answers through `confirm.yes`/`confirm.no`, and chord equality
/// includes modifiers, so `ctrl-y` (an irreversible schema write nobody advertised) and `alt-n`
/// are swallowed by the modal prompt instead of answering it.
#[tokio::test]
async fn ctrl_y_and_alt_n_do_not_answer_the_migration_prompt() {
    let mut harness = shell_reporting("online", Some(3)).await;
    harness.key("ctrl-y");
    harness.key("alt-n");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().overlays.iter().count(),
        1,
        "the prompt is still up"
    );
    assert_eq!(
        harness.app().status,
        None,
        "nothing was applied: no `applied 0 migrations`"
    );
}

/// MOD-67 M3 (PA-1): the `[migration]` view defaults add `Y` and `N` to the shared `y` and `n`.
#[tokio::test]
async fn capital_y_applies_and_capital_n_declines() {
    let mut harness = shell_reporting("online", Some(3)).await;
    harness.key("Y");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty(), "`Y` closes the prompt");
    assert_eq!(
        harness.app().status.as_deref(),
        Some("applied 0 migrations"),
        "`Y` applies"
    );

    let mut harness = shell_reporting("online", Some(3)).await;
    harness.key("N");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty(), "`N` closes the prompt");
    assert_eq!(harness.app().status, None, "`N` applies nothing");
}

/// MOD-67 M3 (D10 over a D12 row): `[migration] yes` replaces the derived `y`/`Y` row, so the
/// old chords are inert and the hint names the new one.
#[tokio::test]
async fn a_migration_override_rebinds_yes() {
    let keys =
        htui::keys::load_str("version = 1\n[migration]\nyes = \"a\"\n").expect("the keys load");
    let mut harness = Harness::demo()
        .with_store_state("online", Some(3))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_keys(keys);
    register_all(harness.app());
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("a apply · n/Esc stay offline"), "{frame}");

    harness.key("y");
    harness.key("Y");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().overlays.iter().count(),
        1,
        "`y`/`Y` are inert"
    );
    assert_eq!(harness.app().status, None);

    harness.key("a");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty(), "`a` closes the prompt");
    assert_eq!(
        harness.app().status.as_deref(),
        Some("applied 0 migrations"),
        "`a` applies"
    );
}

/// MOD-67 M3 (D7, D8): over the prompt the status line and the `?` box render its stack; `Esc`
/// is the derived `[migration] no` row's (PA-1, `SHADOWING`), so it is listed there.
#[tokio::test]
async fn the_status_line_and_help_box_over_the_migration_prompt() {
    let mut harness = shell_reporting("online", Some(3)).await;
    assert_eq!(status_line(&mut harness), "Ctrl+c quit · ? help");
    harness.key("?");
    assert!(harness.app().help_visible);
    let frame = harness.render();
    assert!(frame.contains("Schema: y/Y yes · n/Esc/N no"), "{frame}");
    assert!(frame.contains("Global: Ctrl+c quit · ?/F1 help"), "{frame}");
    assert!(frame.contains("?/F1 closes this box"), "{frame}");
}

/// MOD-67 M3 (D7, D8): on the Workspaces tab the `?` box renders the tab's stack, one line per
/// layer, its own verb first (MOD-116 M1 D10).
#[tokio::test]
async fn the_help_box_on_the_workspaces_tab() {
    let mut harness = on_the_workspaces_tab().await;
    harness.key("?");
    assert!(harness.app().help_visible);
    let frame = harness.render();
    let headings = [
        "Workspaces: o open workspace",
        "List: ",
        "Pane: ",
        "Layout: ",
        "History: ",
        "Global: ",
    ];
    let rows: Vec<usize> = headings
        .iter()
        .map(|heading| {
            let found: Vec<usize> = frame
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains(heading))
                .map(|(row, _)| row)
                .collect();
            assert_eq!(found.len(), 1, "one `{heading}` line: {frame}");
            found[0]
        })
        .collect();
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "the tab's own verb first, then a line per layer in order: {frame}"
    );
    assert!(frame.contains("?/F1 closes this box"), "{frame}");
    assert!(!frame.contains("Backlog: "), "no other tab's line: {frame}");
}

/// MOD-116 M1 R1 F1 (D12): started in `run`'s order into a remembered workspace that is not the
/// first by name, `w` lands on the open workspace's row and overview, not the first one's (the
/// switcher's "opens on the workspace the shell is already inside").
#[tokio::test]
async fn w_lands_on_the_workspace_the_shell_reopened() {
    use htui::ui::state_file::{UiState, UiStore};

    let ui = UiStore::memory();
    ui.save(&UiState {
        last_workspace: Some(ids::WORKSPACE_PLATFORM),
        ..UiState::default()
    })
    .expect("a memory save");
    let mut harness = Harness::demo()
        .with_ui_store(ui)
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .registered_as_run();
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    harness.key("w");
    harness.drive_to_end().await;
    assert_eq!(harness.app().tabs.active_id(), Some(WorkspacesTab::ID));
    let frame = harness.render();
    let selected = frame
        .lines()
        .find(|line| line.starts_with("\u{2502}\u{258c}"))
        .unwrap_or_else(|| panic!("no row is selected: {frame}"));
    assert!(
        selected.contains("\u{258c}\u{25be} Platform"),
        "{selected}\n{frame}"
    );
    assert!(
        frame.contains("\u{256d}\u{2500} Platform \u{2500}"),
        "{frame}"
    );
}

/// MOD-116 M1 R1 F2 (D7): every activation reads the overview again: an item added while another
/// tab was in front is counted when `w` comes back, though no run count moved.
#[tokio::test]
async fn w_reads_the_overview_again() {
    let store = MemStore::demo();
    let mut harness =
        Harness::over(store.clone()).with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness.key("w");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("  2 items \u{b7} 0 running"), "{frame}");
    harness.key("2");
    harness.drive_to_end().await;
    assert_ne!(harness.app().tabs.active_id(), Some(WorkspacesTab::ID));

    store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_VULKAN,
            kind_id: ids::KIND_VULKAN_FEAT,
            title: "Add a shadow-mapping chapter".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            target_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("the mint lands");
    harness.key("w");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("  3 items \u{b7} 0 running"), "{frame}");
}

/// The hints of a 100x30 frame (MOD-67 M6 T2): MOD-115 draws them on the body's bottom border,
/// the last row, so this is that row with the rule and the corners trimmed off. Under the
/// Backlog's `ide_activity` default (MOD-122 M3 D11) the tab's border is the row above the
/// activity pane.
fn hint_row(frame: &str) -> &str {
    let lines: Vec<&str> = frame.lines().collect();
    lines
        .iter()
        .position(|line| line.starts_with("\u{256d}\u{2500} Activity"))
        .and_then(|at| at.checked_sub(1))
        .map_or_else(|| lines.last().copied(), |at| Some(lines[at]))
        .unwrap_or_default()
        .trim_matches([
            '\u{2514}', '\u{2518}', '\u{2500}', '\u{2570}', '\u{256f}', ' ',
        ])
}

/// The demo shell, every tab registered, settled on the Backlog.
async fn demo_shell(keys: Option<htui::keys::Keys>) -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    if let Some(keys) = keys {
        harness = harness.with_keys(keys);
    }
    register_all(harness.app());
    harness.drive_to_end().await;
    harness
}

/// MOD-67 M6 T2: the row above the status line holds the active tab's keys; on the Backlog's
/// Body pane, the list's own (the pane has none).
#[tokio::test]
async fn the_hint_row_shows_the_active_tabs_keys() {
    let mut harness = demo_shell(None).await;
    let frame = harness.render();
    assert_eq!(
        hint_row(&frame),
        "j/k move · h/l sub-tab · f filter · N new · e edit",
        "{frame}"
    );
}

/// MOD-67 M6 T2 (plan Q1 (a)): an overlay's box carries its own hint, so the row goes blank.
#[tokio::test]
async fn the_hint_row_is_blank_under_an_overlay() {
    let mut harness = demo_shell(None).await;
    harness.key("ctrl-w");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().overlays.iter().count(),
        1,
        "`ctrl-w` opened the waiting list"
    );
    let frame = harness.render();
    assert_eq!(hint_row(&frame), "", "{frame}");
}

/// MOD-67 M6 T2: the row is rendered through the tab's stack, so a rebound key shows its chord.
#[tokio::test]
async fn the_hint_row_follows_a_rebound_key() {
    let keys =
        htui::keys::load_str("version = 1\n[backlog]\nfilter = \"/\"\n").expect("the keys load");
    let mut harness = demo_shell(Some(keys)).await;
    let frame = harness.render();
    assert!(hint_row(&frame).contains("/ filter"), "{frame}");
    assert!(!hint_row(&frame).contains("f filter"), "{frame}");
}

/// MOD-115: the harness hides the about panel so the tabs keep the rows their tests were
/// written against; `F9` brings it up under the header, and again takes it away.
#[tokio::test]
async fn f9_toggles_the_about_panel_under_the_header() {
    let mut harness = shell_reporting("memory", None).await;
    let hidden = harness.render();
    assert!(!harness.app().about_visible, "the harness hides it");
    assert!(!hidden.contains("About"), "{hidden}");

    harness.key("f9");
    let shown = harness.render();
    assert!(harness.app().about_visible);
    let rows: Vec<&str> = shown.lines().collect();
    assert!(
        rows[1].contains("About"),
        "the panel opens right under the header: {shown}"
    );
    assert!(
        rows[2].contains("Your work items"),
        "the active tab's purpose leads: {shown}"
    );
    insta::assert_snapshot!("about_panel_on", shown);

    harness.key("f9");
    assert!(!harness.app().about_visible);
    assert!(!harness.render().contains("About"));
}

/// MOD-115 review: the header's key chips are labelled through the stack that holds the
/// keyboard, so `F9 about` is gone under an overlay that does not take it (the waiting list, the
/// migration prompt), `? keys` stays, and `F9 about` returns once the overlay closes.
#[tokio::test]
async fn the_header_chips_follow_the_overlay_stack() {
    let mut harness = waiting_over_demo().await;
    // MOD-122 D4: the sixth chip (`  ⁶Logs`, seven cells) left 100 columns no room for
    // `F9 about`, so the header was read seven columns wider; MOD-116 M1 D11's `  ¹Workspaces`
    // chip (13 cells) widens it again: the room it had before.
    harness.resize(120, 30);
    let header = top_bar(&harness.render());
    assert!(
        !header.contains("about") && !header.contains("F9") && header.ends_with(" ?  keys"),
        "{header}"
    );
    harness.key("esc");
    harness.drive_to_end().await;
    let header = top_bar(&harness.render());
    assert!(header.ends_with(" F9  about   ?  keys"), "{header}");

    let mut harness = shell_reporting("online", Some(3)).await;
    harness.resize(120, 30);
    let header = top_bar(&harness.render());
    assert!(
        !header.contains("about") && header.ends_with(" ?  keys"),
        "{header}"
    );
}

/// The about panel's actions row of a frame (header, top border, purpose, then this), without
/// the border glyphs.
fn about_actions(frame: &str) -> String {
    frame
        .lines()
        .nth(3)
        .unwrap_or_default()
        .trim_matches(['\u{2502}', ' '])
        .to_owned()
}

/// MOD-115 review: the about panel resolves its action labels against the tab's browse stack,
/// not the stack of the mode the tab is in, so the actions row is not blank under the filter
/// form (whose stack has no Backlog layer).
#[tokio::test]
async fn the_about_actions_survive_the_backlog_filter_form() {
    let mut harness = demo_shell(None).await;
    harness.key("f9");
    let browse = harness.render();
    assert!(
        about_actions(&browse).contains("filter"),
        "browsing: {browse}"
    );

    harness.key("f");
    let frame = harness.render();
    assert!(
        frame.contains("Filter"),
        "the filter form is open over the list: {frame}"
    );
    let actions = about_actions(&frame);
    assert!(!actions.is_empty(), "{frame}");
    assert!(
        actions.contains("new item") && actions.contains("queue"),
        "{frame}"
    );
}

/// MOD-115 review: a frame of ten rows has no room for the panel plus a useful body (the
/// header, four panel rows and at least six body rows), so the panel is left off; eleven rows
/// show it.
#[tokio::test]
async fn a_ten_row_frame_drops_the_about_panel() {
    for (height, shown) in [(10_u16, false), (11, true)] {
        let mut harness = Harness::demo()
            .size(100, height)
            .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
        register_all(harness.app());
        harness.drive_to_end().await;
        harness.key("f9");
        assert!(harness.app().about_visible);
        let frame = harness.render();
        assert_eq!(frame.contains("About"), shown, "{height} rows: {frame}");
    }
}

/// The demo shell with `keys` installed (the defaults when `None`), moved to the Platform
/// workspace and on the Requirements tab, as `tests/requirements.rs` opens it.
async fn requirements_shell(keys: Option<&str>) -> Harness {
    use htui::ui::tabs::RequirementsTab;

    let mut harness = Harness::over(MemStore::demo());
    if let Some(file) = keys {
        harness = harness.with_keys(htui::keys::load_str(file).expect("the keys load"));
    }
    register_all(harness.app());
    harness.settle().await;
    // MOD-116 M1: `w`, the cursor onto Platform's row (Graphics, overview, Vulkan Tutorials,
    // Platform), `o`; Requirements is tab 4.
    harness.key("w");
    harness.settle().await;
    for _ in 0..3 {
        harness.key("j");
    }
    harness.key("o");
    harness.settle().await;
    harness.key("4");
    harness.settle().await;
    assert_eq!(harness.app().tabs.active_id(), Some(RequirementsTab::ID));
    harness
}

/// Presses each key of `keys` in turn, settling after each.
async fn press_all(harness: &mut Harness, keys: &[&str]) {
    for key in keys {
        harness.key(key);
        harness.settle().await;
    }
}

/// Whether the view holding the keyboard captures text (a `[field]` layer in its stack).
fn captures(harness: &mut Harness) -> bool {
    harness
        .app()
        .active_stack()
        .is_some_and(htui::keys::Stack::captures_text)
}

/// MOD-115 M2 D7 (blueprint §3), T6R finding 3: the shell's `[field]` translation, end to end on
/// a real capturing screen (the Requirements filter). With `field.submit` and
/// `field.delete_back` rebound, the rebound chords edit and submit the filter exactly as the
/// canonical `Backspace` and `Enter` do with the defaults; `Esc` still cancels with
/// `field.cancel` moved to another chord (MD7), and so does that chord.
#[tokio::test]
async fn a_rebound_field_key_edits_and_submits_a_real_field_as_its_canonical_key_did() {
    const REBOUND: &str = "version = 2\n[field]\nsubmit = [\"ctrl-t\"]\ndelete_back = [\"f7\"]\ncancel = [\"ctrl-o\"]\n";

    let mut canonical = requirements_shell(None).await;
    let mut rebound = requirements_shell(Some(REBOUND)).await;
    press_all(&mut canonical, &["/"]).await;
    press_all(&mut rebound, &["/"]).await;
    assert!(
        captures(&mut canonical) && captures(&mut rebound),
        "the filter is open"
    );

    press_all(&mut canonical, &["S", "T", "O", "X", "backspace"]).await;
    press_all(&mut rebound, &["S", "T", "O", "X", "f7"]).await;
    // MOD-115 M2 T7: the filter's hint names `field.submit` through the keys (MD1), so the
    // rebound frame reads `Ctrl+t apply` where the canonical one reads `Enter apply`; M3 MD-A
    // (L-req) names `field.cancel` too, `Ctrl+o clear` for `Esc clear`: four rule cells shorter,
    // the rest the same frame.
    let open = rebound.render();
    assert!(
        open.contains("Ctrl+t apply"),
        "the hint names the rebound key: {open}"
    );
    assert_eq!(
        open.replace(
            "Ctrl+t apply \u{b7} Ctrl+o clear",
            "Enter apply \u{b7} Esc clear\u{2500}\u{2500}\u{2500}\u{2500}"
        ),
        canonical.render(),
        "`F7` deleted back as `Backspace` did"
    );
    press_all(&mut canonical, &["enter"]).await;
    press_all(&mut rebound, &["ctrl-t"]).await;
    let frame = rebound.render();
    assert_eq!(
        frame,
        canonical.render(),
        "`ctrl-t` submitted as `Enter` did"
    );
    assert!(!captures(&mut rebound), "the filter is submitted: {frame}");
    assert!(
        frame.contains("R-STO-1") && !frame.contains("R-ENT-2"),
        "the list is filtered on `STO`: {frame}"
    );

    // MD7: `Esc` is never configurable; the moved `field.cancel` chord cancels too.
    for cancel in ["esc", "ctrl-o"] {
        let mut canonical = requirements_shell(None).await;
        let mut rebound = requirements_shell(Some(REBOUND)).await;
        press_all(&mut canonical, &["/", "E", "N", "T", "esc"]).await;
        press_all(&mut rebound, &["/", "E", "N", "T", cancel]).await;
        assert!(!captures(&mut rebound), "`{cancel}` closed the filter");
        assert_eq!(
            rebound.render(),
            canonical.render(),
            "`{cancel}` cancelled as `Esc` does"
        );
    }
}

/// A list + detail tab that records the panes the shell gives it, for the layout cases (MOD-115
/// M2 T6): the production tabs adopt the layout in Task 7.
mod split {
    use std::sync::{Arc, Mutex};

    use crossterm::event::KeyEvent;
    use htui::app::{Ctx, Handled};
    use htui::keys::{Stack, views};
    use htui::store_worker::{StoreReply, StoreRequest};
    use htui::ui::shell::{PaneAreas, Panes, TabLayout};
    use htui::ui::tabs::{Tab, TabId};
    use htui_core::model::Scope;
    use ratatui::Frame;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Block, BorderType};

    /// The panes of every frame drawn, last last.
    pub type Drawn = Arc<Mutex<Vec<PaneAreas>>>;

    pub struct Split(pub Drawn);

    impl Tab for Split {
        fn id(&self) -> TabId {
            TabId("split")
        }
        fn title(&self) -> &str {
            "Split"
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
            let areas = ctx.split(area, TabLayout::DEFAULT);
            self.0.lock().expect("not poisoned").push(areas);
            for pane in [areas.list, Some(areas.detail), areas.activity]
                .into_iter()
                .flatten()
                .filter(|rect| !rect.is_empty())
            {
                frame.render_widget(Block::bordered().border_type(BorderType::Rounded), pane);
            }
        }
        fn key_stack(&self) -> Option<Stack<'static>> {
            Some(views::BACKLOG_BODY)
        }
        fn panes(&self) -> Panes {
            Panes::ListDetail(TabLayout::DEFAULT)
        }
    }
}

/// MOD-115 M2 D1, the plan's acceptance: the layout and the about panel survive a restart.
/// Change both, drop the shell, build another over the same UI-state store, and the new shell
/// draws the layout and shows the about panel as they were left.
#[tokio::test]
async fn the_layout_and_the_about_panel_survive_a_restart() {
    use htui::ui::shell::ListSize;
    use htui::ui::state_file::UiStore;

    let store = UiStore::memory();
    let drawn = split::Drawn::default();
    let mut first = Harness::demo()
        .with_ui_store(store.clone())
        .with_tab(Box::new(split::Split(drawn.clone())));
    first.settle().await;
    let _ = first.render();
    let before = *drawn.lock().expect("not poisoned").last().expect("drawn");
    first.key(">");
    first.key("f2");
    assert!(!first.app().about_visible);
    first.key("f9");
    assert!(first.app().about_visible);
    let _ = first.render();
    let left = *drawn.lock().expect("not poisoned").last().expect("drawn");
    assert_eq!(left.list, None, "f2 hid the list");
    drop(first);

    let redrawn = split::Drawn::default();
    let mut again = Harness::demo()
        .with_ui_store(store.clone())
        .with_tab(Box::new(split::Split(redrawn.clone())));
    again.settle().await;
    assert!(again.app().about_visible, "the about panel stays on");
    let _ = again.render();
    let restored = *redrawn.lock().expect("not poisoned").last().expect("drawn");
    assert_eq!(restored, left, "the layout is drawn as it was left");
    assert_ne!(restored, before);
    again.key("f2");
    let _ = again.render();
    let shown = *redrawn.lock().expect("not poisoned").last().expect("drawn");
    let list = shown.list.expect("f2 shows the list again");
    let body = list.width + shown.detail.width;
    assert_eq!(
        list.width,
        body * 55 / 100,
        "the grown list (55%, from `>`) came back with it"
    );
    let state = store.load().state;
    assert_eq!(
        state.layouts.get("split").map(|layout| layout.list),
        Some(ListSize::Percent(55))
    );
}

/// MOD-115 M2 RV2 M2: a restart with zoom on brings back the pane that was zoomed (the detail),
/// not the list: the new shell draws the detail over the whole body, as it was left.
#[tokio::test]
async fn a_restart_with_zoom_on_brings_the_zoomed_detail_back() {
    use htui::ui::state_file::UiStore;

    let store = UiStore::memory();
    let drawn = split::Drawn::default();
    let mut first = Harness::demo()
        .with_ui_store(store.clone())
        .with_tab(Box::new(split::Split(drawn.clone())));
    first.settle().await;
    let _ = first.render();
    first.key("tab");
    first.key("z");
    let _ = first.render();
    let left = *drawn.lock().expect("not poisoned").last().expect("drawn");
    assert_eq!(left.list, None, "the detail is zoomed");
    assert!(!left.detail.is_empty());
    drop(first);

    let redrawn = split::Drawn::default();
    let mut again = Harness::demo()
        .with_ui_store(store.clone())
        .with_tab(Box::new(split::Split(redrawn.clone())));
    again.settle().await;
    let _ = again.render();
    let restored = *redrawn.lock().expect("not poisoned").last().expect("drawn");
    assert_eq!(restored, left, "the zoomed detail comes back");
    again.key("z");
    let _ = again.render();
    let unzoomed = *redrawn.lock().expect("not poisoned").last().expect("drawn");
    assert!(unzoomed.list.is_some(), "unzoomed, the list is back");
}

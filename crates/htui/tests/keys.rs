//! The shell's key behaviour since MOD-67 M1 (`docs/ANA-26.md` §6.4, §6.5, §7.3): global and
//! overlay keys are named actions resolved through `htui::keys`, in D6's order.
//!
//! - `ctrl-c` quits before any overlay or view sees it (D6 step 0), so a browse mode whose `c`
//!   arm ignores modifiers (Connection, Qdrant) no longer eats it, and no modal overlay swallows it.
//! - `?` and `F1` toggle help (D12), over a modal overlay too, for a key the overlay passed (D6
//!   step 2). A field that types `?` keeps it as text, and `F1` still reaches help.
//! - A modal overlay swallows what it and the overlay stack passed (D6 step 3): under the switcher
//!   a digit or `w` reaches neither the tab strip nor a second switcher.
//! - `w`, `Ctrl+F` and `Ctrl+W` are offered by `register_all` (D5): without it they are inert and
//!   absent from the status line and the `?` box.
//! - The status line is byte-identical to the one the snapshots pin (D7); the `?` box follows D8.
//!
//! MOD-67 M3 (T1): the Settings tab cycles sections through `settings.next_section`/
//! `prev_section`, so `ctrl-l`/`ctrl-h` no longer cycle (defect 1); a view with a key stack drives
//! the status line, the `?` box and its closer (D7, D8), filtered in a capturing mode (D5); and
//! `Harness::with_keys` installs a rebound table. No real view has a stack yet: the doubles
//! `StackProbe` (a capturing Settings section) and `OverlayProbe` (a converted overlay) stand in.
//!
//! Every case goes through [`Harness`]; the ones that may read or store a DSN take
//! `common::mock_keyring()` as their first statement (`tests/connection.rs`'s header rule).
#![cfg(feature = "testkit")]

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent};
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, Ctx, Handled, TabAction, register_all};
use htui::keys::{KeyChord, Stack, load_str, views};
use htui::store_worker::{StoreReply, StoreRequest};
use htui::testkit::Harness;
use htui::ui::overlay::{
    ConceptsSearch, MigrationPrompt, Overlay, OverlayId, WaitingList, WorkspaceSwitcher,
};
use htui::ui::tabs::BacklogTab;
use htui::ui::tabs::settings::{
    ConnectionSection, QdrantSection, SectionId, SettingsSection, SettingsTab,
};
use htui_agent::registry::DriverFactory;
use htui_core::model::Scope;
use htui_store::testkit as common;
use htui_store::{Backend, CacheStore, PgStore, secret};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

/// A DSN that parses and names a port nothing listens on (`tests/connection.rs`'s).
const DEAD_DSN: &str = "postgres://htui:s3cret@127.0.0.1:1/htui?sslmode=disable";

/// The status line of the bare shell: today's, which 72 snapshots pin.
const BARE_STATUS: &str = "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help";

/// The status line with `register_all`, as a 100-column frame cuts it (34 snapshots pin it).
const FULL_STATUS_CUT: &str = "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · \
                               ? help · w workspaces · Ctrl+f find";

/// The `?` box's closing line.
const CLOSER: &str = "?/F1 closes this box";

/// The demo shell, settled: every view `register_all` registers when `register` is set, the
/// Backlog alone otherwise. The agent runtime keeps prompt previews off the status line
/// (`tests/backlog.rs`).
async fn shell(register: bool) -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    if register {
        register_all(harness.app());
    } else {
        harness = harness.with_tab(Box::new(BacklogTab::new()));
    }
    harness.drive_to_end().await;
    harness
}

/// The id of the overlay on top, if any.
fn top(harness: &mut Harness) -> Option<OverlayId> {
    harness.app().overlays.top().map(|overlay| overlay.id())
}

/// The last non-empty line of a frame: the status line.
fn status_line(frame: &str) -> &str {
    frame
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim_end()
}

#[tokio::test]
async fn ctrl_c_quits_from_connection_browse_with_a_stored_dsn() {
    let _keyring = common::mock_keyring().await;
    secret::set_dsn(DEAD_DSN).expect("the fake keyring writes");
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "keys-ctrl-c", PgStore::schema_version())
        .await
        .expect("a fresh mirror");

    let mut harness = Harness::over_backend(Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(BacklogTab::new()))
    .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
        ConnectionSection::new(),
    )])));
    harness.settle().await;
    harness.app().update(Action::Tab(TabAction::FocusSection(
        SettingsTab::ID,
        ConnectionSection::ID,
    )));
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("Rebuild cache") && !frame.contains("typed text is never shown"),
        "the section is in browse mode, no field open: {frame}"
    );

    harness.key("ctrl-c");
    assert!(
        harness.app().should_quit,
        "`ctrl-c` quits before the section sees it"
    );
    let frame = harness.render();
    assert!(
        !frame.contains("Remove the DSN from the keyring?"),
        "the modifier-blind `c` arm never ran: {frame}"
    );

    cache.close().await;
    drop(root);
}

#[tokio::test]
async fn ctrl_c_quits_from_qdrant_browse() {
    let mut harness = shell(true).await;
    harness.app().update(Action::Tab(TabAction::FocusSection(
        SettingsTab::ID,
        QdrantSection::ID,
    )));
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("r reload · j/k rows") && !frame.contains("Esc cancel"),
        "Qdrant is in browse mode, no field open: {frame}"
    );

    harness.key("ctrl-c");
    assert!(
        harness.app().should_quit,
        "`ctrl-c` quits before the section sees it"
    );
}

#[tokio::test]
async fn ctrl_c_quits_over_every_modal_overlay() {
    for (opener, expected) in [
        ("w", WorkspaceSwitcher::ID),
        ("ctrl-f", ConceptsSearch::ID),
        ("ctrl-w", WaitingList::ID),
    ] {
        let mut harness = shell(true).await;
        harness.key(opener);
        harness.drive_to_end().await;
        if expected == ConceptsSearch::ID {
            harness.key("a");
            harness.key("b");
        }
        assert_eq!(top(&mut harness), Some(expected), "`{opener}` opened it");
        harness.key("ctrl-c");
        assert!(
            harness.app().should_quit,
            "`ctrl-c` quits over {expected:?}"
        );
    }

    let mut harness = Harness::demo().with_store_state("online", Some(1));
    register_all(harness.app());
    harness.drive_to_end().await;
    assert_eq!(top(&mut harness), Some(MigrationPrompt::ID));
    harness.key("ctrl-c");
    assert!(
        harness.app().should_quit,
        "`ctrl-c` quits over the migration prompt"
    );
}

#[tokio::test]
async fn f1_opens_and_closes_help_on_the_backlog() {
    let mut harness = shell(true).await;
    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opens the box");
    harness.key("f1");
    assert!(!harness.app().help_visible, "`F1` closes it");
}

#[tokio::test]
async fn question_mark_and_f1_toggle_help_over_the_workspace_switcher() {
    let mut harness = shell(true).await;
    harness.key("w");
    harness.drive_to_end().await;
    assert_eq!(top(&mut harness), Some(WorkspaceSwitcher::ID));

    harness.key("?");
    assert!(
        harness.app().help_visible,
        "`?` opens the box over the switcher"
    );
    assert_eq!(
        top(&mut harness),
        Some(WorkspaceSwitcher::ID),
        "and leaves it up"
    );
    harness.key("?");
    assert!(!harness.app().help_visible, "`?` closes it");

    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opens it too");
    let frame = harness.render();
    assert!(frame.contains("Overlay: Esc close"), "{frame}");
    assert!(frame.contains(CLOSER), "{frame}");

    harness.key("esc");
    assert!(
        harness.app().overlays.is_empty(),
        "`Esc` still closes the switcher"
    );
}

#[tokio::test]
async fn the_modal_switcher_swallows_a_digit_and_w() {
    let mut harness = shell(true).await;
    let before = harness.app().tabs.active_id();
    assert!(before.is_some(), "a tab is active");
    harness.key("w");
    harness.drive_to_end().await;
    assert_eq!(top(&mut harness), Some(WorkspaceSwitcher::ID));

    for key in ["2", "w"] {
        harness.key(key);
        harness.drive_to_end().await;
        assert_eq!(
            harness.app().tabs.active_id(),
            before,
            "`{key}` never reached the tab strip"
        );
        let open: Vec<OverlayId> = harness.app().overlays.iter().map(|o| o.id()).collect();
        assert_eq!(
            open,
            [WorkspaceSwitcher::ID],
            "`{key}` left only the switcher up"
        );
    }
}

#[tokio::test]
async fn question_mark_still_toggles_help_without_register_all() {
    let mut harness = shell(false).await;
    harness.key("?");
    assert!(harness.app().help_visible, "`?` opened the box");
    harness.key("?");
    assert!(!harness.app().help_visible, "`?` closed the box");
}

#[tokio::test]
async fn a_question_mark_typed_into_the_concepts_search_is_text_and_f1_is_help() {
    let mut harness = shell(true).await;
    harness.key("ctrl-f");
    harness.drive_to_end().await;
    assert_eq!(top(&mut harness), Some(ConceptsSearch::ID));

    harness.key("?");
    assert!(!harness.app().help_visible, "the field took `?`");
    let frame = harness.render();
    assert!(frame.contains("query ?"), "`?` is query text: {frame}");

    harness.key("f1");
    assert!(
        harness.app().help_visible,
        "`F1` reaches help past the field"
    );
    assert_eq!(
        top(&mut harness),
        Some(ConceptsSearch::ID),
        "the search stays up"
    );
}

#[tokio::test]
async fn w_is_inert_without_register_all_and_opens_the_switcher_with_it() {
    let mut bare = shell(false).await;
    bare.key("w");
    bare.drive_to_end().await;
    assert!(
        bare.app().overlays.is_empty(),
        "nobody offered `workspaces`"
    );

    let mut full = shell(true).await;
    full.key("w");
    full.drive_to_end().await;
    assert_eq!(top(&mut full), Some(WorkspaceSwitcher::ID));
}

#[tokio::test]
async fn the_help_box_lists_ctrl_w_waiting_only_with_register_all() {
    let mut bare = shell(false).await;
    bare.key("?");
    let bare_frame = bare.render();

    let mut full = shell(true).await;
    full.key("?");
    let full_frame = full.render();

    for frame in [&bare_frame, &full_frame] {
        assert!(frame.contains(CLOSER), "{frame}");
        assert!(frame.contains("1-9 select tab"), "{frame}");
    }
    // The status line is cut before `Ctrl+w waiting` at 100 columns, so a match is the box's.
    assert!(!bare_frame.contains("Ctrl+w waiting"), "{bare_frame}");
    assert!(full_frame.contains("Ctrl+w waiting"), "{full_frame}");
}

#[tokio::test]
async fn the_status_line_is_todays_in_both_harnesses() {
    let mut bare = shell(false).await;
    assert_eq!(status_line(&bare.render()), BARE_STATUS);

    let mut full = shell(true).await;
    assert_eq!(status_line(&full.render()), FULL_STATUS_CUT);
}

/// A capturing Settings section on the agents form's stack: it types every printable chord and
/// passes what the stack lets through (MOD-67 M3 PA-5), as a converted form will.
struct StackProbe;

impl SettingsSection for StackProbe {
    fn id(&self) -> SectionId {
        SectionId("stack-probe")
    }
    fn title(&self) -> &str {
        "Probe"
    }
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }
    fn on_scope_change(&mut self, _scope: &Scope) {}
    fn captures_input(&self) -> bool {
        true
    }
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::AGENTS_FORM)
    }
    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        if chord.is_printable() || !views::AGENTS_FORM.passes(chord) {
            Handled::Consumed
        } else {
            Handled::Pass
        }
    }
    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
    fn render(&self, frame: &mut Frame<'_>, area: Rect, _ctx: &Ctx<'_>) {
        frame.render_widget(Paragraph::new("stack probe"), area);
    }
}

/// A modal overlay on the concepts search's stack that passes every key to the shell.
struct OverlayProbe;

impl Overlay for OverlayProbe {
    fn id(&self) -> OverlayId {
        OverlayId("overlay-probe")
    }
    fn title(&self) -> &str {
        "Probe"
    }
    fn is_modal(&self) -> bool {
        true
    }
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::CONCEPTS_QUERY)
    }
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }
    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
    fn render(&self, frame: &mut Frame<'_>, area: Rect, _ctx: &Ctx<'_>) {
        frame.render_widget(Paragraph::new("overlay probe"), area);
    }
}

/// D14 pin (ANA-26 §2.6 defect 1): section cycling compares whole chords, modifiers included.
#[tokio::test]
async fn ctrl_l_and_ctrl_h_do_not_cycle_sections() {
    let _keyring = common::mock_keyring().await;
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(ConnectionSection::new()),
        Box::new(QdrantSection::new()),
    ])));
    harness.drive_to_end().await;
    let on_connection = |frame: &str| frame.contains("Rebuild cache");
    let frame = harness.render();
    assert!(on_connection(&frame), "Connection is active: {frame}");

    for key in ["ctrl-l", "ctrl-h", "shift-right", "alt-["] {
        harness.key(key);
        harness.drive_to_end().await;
        let frame = harness.render();
        assert!(on_connection(&frame), "`{key}` did not cycle: {frame}");
    }

    harness.key("l");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(!on_connection(&frame), "`l` moved to Qdrant: {frame}");
    harness.key("h");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(on_connection(&frame), "`h` moved back: {frame}");
}

#[tokio::test]
async fn a_capturing_section_shows_the_filtered_status_line_and_box() {
    let mut harness =
        Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
            StackProbe,
        )])));
    harness.drive_to_end().await;
    assert_eq!(status_line(&harness.render()), "Ctrl+c quit · F1 help");

    harness.key("?");
    assert!(!harness.app().help_visible, "the probe typed `?`");
    harness.key("tab");
    assert!(!harness.app().help_visible);

    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` passes the field to help");
    let frame = harness.render();
    assert!(
        frame.contains("Agents: Tab/Down next field · Shift+Tab/Up previous field"),
        "{frame}"
    );
    assert!(frame.contains("Global: Ctrl+c quit · F1 help"), "{frame}");
    assert!(frame.contains("F1 closes this box"), "{frame}");
    assert!(!frame.contains(CLOSER), "{frame}");
    harness.key("f1");
    assert!(!harness.app().help_visible, "`F1` closes it");

    harness.key("ctrl-c");
    assert!(harness.app().should_quit, "`ctrl-c` still quits");
}

#[tokio::test]
async fn an_overlay_with_a_stack_drives_the_status_line_and_box() {
    let mut harness = Harness::demo()
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_overlay(Box::new(OverlayProbe));
    register_all(harness.app());
    harness.drive_to_end().await;
    assert_eq!(top(&mut harness), Some(OverlayId("overlay-probe")));
    assert_eq!(status_line(&harness.render()), "Ctrl+c quit · F1 help");

    harness.key("?");
    assert!(
        !harness.app().help_visible,
        "`?` is filtered out over the probe"
    );
    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opens the box");
    let frame = harness.render();
    assert!(frame.contains("Overlay: Esc close"), "{frame}");
    assert!(frame.contains("Global: Ctrl+c quit · F1 help"), "{frame}");
    assert!(frame.contains("F1 closes this box"), "{frame}");
    assert!(!frame.contains("Backlog:"), "no legacy tab line: {frame}");
    harness.key("f1");
    assert!(!harness.app().help_visible);

    harness.key("esc");
    assert!(harness.app().overlays.is_empty(), "`Esc` closes the probe");
    assert_eq!(status_line(&harness.render()), FULL_STATUS_CUT);
}

#[tokio::test]
async fn with_keys_installs_the_keys() {
    let keys = load_str("version = 1\n[global]\nquit = \"f10\"\n").expect("the keys load");
    let mut harness = Harness::demo()
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_tab(Box::new(BacklogTab::new()))
        .with_keys(keys);
    harness.drive_to_end().await;
    assert!(
        status_line(&harness.render()).starts_with("F10 quit · Tab next tab"),
        "{}",
        harness.render()
    );
    harness.key("q");
    assert!(!harness.app().should_quit, "`q` no longer quits");
    harness.key("f10");
    assert!(harness.app().should_quit, "`F10` quits");
}

/// A raw `Char('L')` with CONTROL, as a kitty-protocol terminal reports ctrl-shift-l (PA-6):
/// the chord is `ctrl-l`, which does not cycle either.
#[tokio::test]
async fn a_kitty_ctrl_capital_is_the_ctrl_chord() {
    let event = KeyEvent::new(KeyCode::Char('L'), crossterm::event::KeyModifiers::CONTROL);
    assert_eq!(
        KeyChord::from_event(event),
        KeyChord::parse("ctrl-l").expect("a chord")
    );

    let _keyring = common::mock_keyring().await;
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(ConnectionSection::new()),
        Box::new(QdrantSection::new()),
    ])));
    harness.drive_to_end().await;
    let on_connection = |frame: &str| frame.contains("Rebuild cache");
    let frame = harness.render();
    assert!(on_connection(&frame), "Connection is active: {frame}");

    harness.app().on_key(event);
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        on_connection(&frame),
        "a kitty ctrl-shift-l did not cycle: {frame}"
    );
}

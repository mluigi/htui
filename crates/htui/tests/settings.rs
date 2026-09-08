//! Settings tab tests: the section registry and MOD-2's agent section (`R-TUI-8`, plan T10).
//!
//! Everything runs through `Harness` and the tab's public surface, at the 100x30 frame the whole
//! snapshot suite is pinned to.
#![cfg(feature = "testkit")]

use htui::app::{Ctx, Emit, Handled, TopBarState};
use htui::keymap::Keymap;
use htui::store_worker::{Origin, StoreReply, StoreRequest};
use htui::testkit::Harness;
use htui::ui::Theme;
use htui::ui::tabs::settings::{AgentsSection, SectionId, SettingsSection, SettingsTab, message};
use htui_core::model::{Agent, AgentBox, AgentId, AgentSummary, Billing, BoxId, Scope, Transport};
use htui_core::store::{MemStore, WriteStore};
use ratatui::Frame;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use serde_json::{Value, json};

use crossterm::event::KeyEvent;

/// The scope of the demo fixture's first workspace: `Scope` has no `Default`, and a section that
/// ignores the scope should still be handed a real one.
async fn demo_scope() -> Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    Scope::from_workspace(workspaces.first().expect("the fixture has a workspace"))
}

/// Renders one section into a 100x30 buffer and returns it as text, the way `Harness::render`
/// does for a whole frame.
fn render_section(section: &dyn SettingsSection, ctx: &Ctx<'_>) -> String {
    let mut terminal = Terminal::with_options(
        TestBackend::new(100, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 100, 30)),
        },
    )
    .expect("a test terminal");
    terminal
        .draw(|frame| section.render(frame, frame.area(), ctx))
        .expect("the section draws");

    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A settled Settings tab over `store`, with the agent section registered.
async fn settings_over(store: MemStore) -> Harness {
    let mut harness =
        Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
            AgentsSection::new(),
        )])));
    harness.settle().await;
    harness
}

#[tokio::test]
async fn the_demo_registry_lists_both_agents() {
    let mut harness = settings_over(MemStore::demo()).await;
    let frame = harness.render();

    assert!(frame.contains("claude"), "the first seeded agent");
    assert!(frame.contains("agy"), "the second seeded agent");
    assert!(
        frame.contains("not probed"),
        "no probe has run in the fixture"
    );
    // Both seeds are `acp`/`subscription` since ANA-4 §5.3; the fixture is derived from them.
    assert!(frame.contains("acp"), "the transport column");
    assert!(frame.contains("subscription"), "the billing column");
    insta::assert_snapshot!("agents_demo", frame);
}

#[tokio::test]
async fn an_agent_no_fixture_contains_appears_from_its_row_alone() {
    // The same claim `R-AGT-5` makes about drivers, made about the UI: a registry row the
    // codebase does not know about renders with no code change. `Harness::over` is what lets the
    // store be written to before the shell reads it.
    let store = MemStore::demo();
    let mut extra: Agent = MemStore::demo()
        .agents()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .next()
        .expect("the fixture seeds at least one agent")
        .agent;
    extra.id = AgentId::new();
    extra.name = "kappa".to_owned();
    extra.default_model = Some("k1".to_owned());
    store.upsert_agent(&extra).await.expect("the row saves");

    let mut harness = settings_over(store).await;
    let frame = harness.render();

    assert!(frame.contains("kappa"), "the unknown agent is listed");
    assert!(frame.contains("claude"), "and the seeded ones still are");
    insta::assert_snapshot!("agents_unknown_row", frame);
}

#[tokio::test]
async fn an_empty_registry_says_so_rather_than_drawing_a_bare_table() {
    let mut harness = settings_over(MemStore::new()).await;
    let frame = harness.render();

    assert!(frame.contains("no agents registered"), "{frame}");
    insta::assert_snapshot!("agents_empty", frame);
}

#[tokio::test]
async fn a_failed_read_says_the_registry_needs_postgres() {
    // `agent` and `agent_box` are not mirrored (`docs/ANA-9.md` §4.4), so an offline backend
    // answers `Unreachable` and the section has nothing to fall back to. The reply is injected
    // rather than provoked because a `Backend::Memory` cannot be offline.
    let scope = demo_scope().await;
    let projects = Vec::new();
    let top_bar = TopBarState::default();
    let keymap = Keymap::default_global();
    let theme = Theme::default();
    let emit = Emit::default();
    let mut ctx = Ctx::new(
        &scope,
        &projects,
        &top_bar,
        &keymap,
        &theme,
        Origin::Tab(SettingsTab::ID),
        &emit,
    );

    let mut section = AgentsSection::new();
    section.on_reply(
        &StoreReply::Agents(
            MemStore::demo()
                .agents()
                .await
                .expect("the memory store never fails"),
        ),
        &mut ctx,
    );
    section.on_reply(
        &StoreReply::Failed {
            request: "agents",
            message: "the store is unreachable: agent registry is not mirrored".to_owned(),
        },
        &mut ctx,
    );

    let rendered = render_section(&section, &ctx);
    assert!(
        rendered.contains("agent registry needs Postgres"),
        "a failed read says why, and does not fall back to the rows it had: {rendered}"
    );
    assert!(
        !rendered.contains("claude"),
        "the stale rows are dropped, not left on screen as if they were current: {rendered}"
    );
}

#[tokio::test]
async fn the_section_asks_for_the_registry_once_and_unscoped() {
    let section = AgentsSection::new();
    let requests = section.wants_requests(&demo_scope().await);

    assert_eq!(requests.len(), 1);
    assert!(
        matches!(requests[0], StoreRequest::Agents),
        "the agent registry is a global table, so the read carries no scope"
    );
}

/// One registry row whose `agent_box` carries `probe`, for the column's own table (MOD-2 D54).
fn probed_row(
    name: &str,
    enabled: bool,
    version: Option<&str>,
    probe: Option<Value>,
) -> AgentSummary {
    let mut summary = AgentSummary {
        agent: Agent {
            id: AgentId::new(),
            name: name.to_owned(),
            transport: Transport::Acp,
            billing: Billing::Subscription,
            models: Vec::new(),
            default_model: None,
            launch: json!({}),
            settings: json!({}),
            enabled: true,
            created_at: htui_core::fixtures::demo_at(0, 0),
            updated_at: htui_core::fixtures::demo_at(0, 0),
        },
        on_box: None,
    };
    summary.on_box = Some(AgentBox {
        agent_id: summary.agent.id,
        box_id: BoxId::new(),
        enabled,
        version: version.map(str::to_owned),
        path: None,
        probed_at: Some(htui_core::fixtures::demo_at(0, 0)),
        quota: None,
        quota_at: None,
        updated_at: htui_core::fixtures::demo_at(0, 0),
        probe,
    });
    summary
}

/// The `on this box` column, one row per outcome of plan D50.
#[tokio::test]
async fn the_status_column_renders_each_probe_outcome() {
    let scope = demo_scope().await;
    let projects = Vec::new();
    let top_bar = TopBarState::default();
    let keymap = Keymap::default_global();
    let theme = Theme::default();
    let emit = Emit::default();
    let mut ctx = Ctx::new(
        &scope,
        &projects,
        &top_bar,
        &keymap,
        &theme,
        Origin::Tab(SettingsTab::ID),
        &emit,
    );

    let rows = vec![
        probed_row(
            "ready-on",
            true,
            Some("0.48.0"),
            Some(json!({ "status": "ready", "source": "probe" })),
        ),
        probed_row(
            "ready-off",
            false,
            Some("0.48.0"),
            Some(json!({ "status": "ready", "source": "probe" })),
        ),
        probed_row(
            "gone",
            false,
            None,
            Some(json!({ "status": "missing", "source": "probe" })),
        ),
        probed_row(
            "needs-auth",
            false,
            Some("1.1.26"),
            Some(json!({ "status": "unauthenticated", "source": "probe" })),
        ),
        probed_row(
            "broke",
            false,
            None,
            Some(json!({ "status": "failed", "source": "probe" })),
        ),
        // A row written before `0002` existed: no `probe` document at all, so the pre-probe rule
        // still decides what the column says.
        probed_row("legacy", true, Some("9.9.9"), None),
    ];

    let mut section = AgentsSection::new();
    section.on_reply(&StoreReply::Agents(rows), &mut ctx);
    let rendered = render_section(&section, &ctx);

    for (row, expected) in [
        ("ready-on", "0.48.0"),
        ("ready-off", "0.48.0 (off)"),
        ("gone", "missing"),
        ("needs-auth", "unauthenticated"),
        ("broke", "failed"),
        ("legacy", "9.9.9"),
    ] {
        let line = rendered
            .lines()
            .find(|line| line.starts_with(row))
            .unwrap_or_else(|| panic!("the `{row}` row is rendered:\n{rendered}"));
        assert!(
            line.ends_with(expected),
            "the `on this box` column of `{row}` reads `{expected}`: {line}"
        );
    }

    insta::assert_snapshot!("agents_probed", rendered);
}

/// A second section, so the strip has something to cycle between before MOD-7 and MOD-15 land.
#[derive(Debug, Default)]
struct ProbeSection;

impl SettingsSection for ProbeSection {
    fn id(&self) -> SectionId {
        SectionId("probe")
    }

    fn title(&self) -> &str {
        "Probe"
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
        message(frame, area, "box profile arrives with MOD-7", ctx.theme);
    }
}

#[tokio::test]
async fn h_and_l_move_between_sections() {
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(ProbeSection),
    ])));
    harness.settle().await;

    assert!(
        harness.render().contains("claude"),
        "Agents is active first"
    );

    harness.key("l");
    harness.settle().await;
    let probe = harness.render();
    assert!(probe.contains("box profile arrives with MOD-7"), "{probe}");

    harness.key("h");
    harness.settle().await;
    assert!(
        harness.render().contains("claude"),
        "`h` comes back to Agents"
    );
}

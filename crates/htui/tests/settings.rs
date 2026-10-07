//! Settings tab tests: the section registry and MOD-2's agent section (`R-TUI-8`, plan T10).
//!
//! Everything runs through `Harness` and the tab's public surface, at the 100x30 frame the whole
//! snapshot suite is pinned to.
#![cfg(feature = "testkit")]

use htui::agent_settings::{AgentDraft, AgentWrite};
use htui::app::{Action, Ctx, Handled};
use htui::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use htui::secrets_settings::{DEMO_SESSION, Redacted};
use htui::store_worker::{
    AuthFrame, InstallFrame, Origin, RequestEnvelope, StoreReply, StoreRequest,
};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{
    AgentsSection, BoxesSection, ConnectionSection, HierarchySection, KindsSection,
    PersonasSection, PromptSection, QdrantSection, SecretsSection, SectionId, SettingsSection,
    SettingsTab, message,
};
use htui::ui::text_field::PASTE_DOES_NOT_FIT;
use htui_agent::acp::Handshake;
use htui_agent::auth::loopback::{
    self, Advertised, DELIVERY_IN_FLIGHT, DeliverError, ListenerReply, NO_LOOPBACK_REDIRECT,
    PASTE_MAX, PasteError,
};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::install::InstallRecord;
use htui_agent::probe::{CredentialTier, ProbeSnapshot, ProbeSource, ProbeStatus, agent_box_row};
use htui_agent::{ArchiveFormat, InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::{Agent, AgentBox, AgentId, AgentSummary, Billing, BoxId, Scope, Transport};
use htui_core::store::{MemStore, WriteStore};
use htui_store::{Backend, Started};
use ratatui::Frame;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

/// The scope of the demo fixture's first workspace: `Scope` has no `Default`, and a section that
/// ignores the scope should still be handed a real one.
async fn demo_scope() -> Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    Scope::from_workspace(workspaces.first().expect("the fixture has a workspace"))
}

/// How wide this file draws a section: 100 columns, unbordered, the size the whole snapshot suite
/// is pinned to.
const SECTION_WIDE: u16 = 100;

/// How wide the *running app* draws it: the Settings pane's border costs two columns of the same
/// 100 (hazard H-12), and 98 is the width D76's ranking was decided at. The one case that has to be
/// asserted here rather than at 100 is [`the_on_box_column_holds_the_whole_unauthenticated_verdict`]
/// — the eighth column is the `Min` one, so it is the only column whose width differs between the
/// two, and a verdict that fitted at 100 and clipped at 98 would be a pass over a broken screen.
const SECTION_BORDERED: u16 = 98;

/// Draws one section into a `width`x30 buffer.
fn draw_section_at(section: &dyn SettingsSection, ctx: &Ctx<'_>, width: u16) -> Buffer {
    let mut terminal = Terminal::with_options(
        TestBackend::new(width, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, width, 30)),
        },
    )
    .expect("a test terminal");
    terminal
        .draw(|frame| section.render(frame, frame.area(), ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// Draws one section into a [`SECTION_WIDE`]x30 buffer, the size the whole snapshot suite is
/// pinned to.
fn draw_section(section: &dyn SettingsSection, ctx: &Ctx<'_>) -> Buffer {
    draw_section_at(section, ctx, SECTION_WIDE)
}

/// One drawn buffer as text, the way `Harness::render` returns a whole frame.
fn text_of(buffer: &Buffer) -> String {
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

/// Renders one section into a [`SECTION_WIDE`]x30 buffer and returns it as text.
fn render_section(section: &dyn SettingsSection, ctx: &Ctx<'_>) -> String {
    text_of(&draw_section(section, ctx))
}

/// Renders one section at a width of its own, for the case whose subject *is* the width.
fn render_section_at(section: &dyn SettingsSection, ctx: &Ctx<'_>, width: u16) -> String {
    text_of(&draw_section_at(section, ctx, width))
}

/// The lines a section drew in the theme's accent colour.
///
/// The row cursor is a *style*, not a character: nothing in the text says which row is
/// highlighted, so a test that only read [`render_section`] could not tell a moved cursor from a
/// stuck one.
fn accented_lines(section: &dyn SettingsSection, ctx: &Ctx<'_>) -> Vec<String> {
    let accent = Theme::default().accent.fg;
    let buffer = draw_section(section, ctx);
    (0..buffer.area.height)
        .filter(|y| buffer[(0, *y)].fg == accent.unwrap_or(Color::Reset))
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
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
async fn the_demo_registry_lists_every_seeded_agent() {
    let mut harness = settings_over(MemStore::demo()).await;
    let frame = harness.render();

    assert!(frame.contains("claude"), "the first seeded agent");
    assert!(frame.contains("agy"), "the second seeded agent");
    assert!(
        frame.contains("not probed"),
        "no probe has run in the fixture"
    );
    // Two seeds are `acp` and one is `cli` since MOD-2 D79 gave the CLI transport its own row
    // rather than hiding it inside `claude`'s; all three are `subscription`. The fixture is derived
    // from them, so both transports appear here without this test naming either agent.
    assert!(frame.contains("acp"), "the transport column");
    assert!(frame.contains("cli"), "and the degraded transport's row");
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
    store
        .upsert_agent(&extra, None)
        .await
        .expect("the row saves");

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
    let bench = SectionBench::new().await;
    let mut section = AgentsSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Agents(
            MemStore::demo()
                .agents()
                .await
                .expect("the memory store never fails"),
        ),
    );
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "agents",
            message: "the store is unreachable: agent registry is not mirrored".to_owned(),
        },
    );

    let rendered = render_section(&section, &bench.ctx());
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
        user_off: false,
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
    let bench = SectionBench::new().await;
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
    bench.reply(&mut section, &StoreReply::Agents(rows));
    let rendered = render_section(&section, &bench.ctx());

    for (row, expected) in [
        ("ready-on", "0.48.0"),
        ("ready-off", "0.48.0 (off)"),
        ("gone", "missing"),
        ("needs-auth", "unauthenticated"),
        ("broke", "failed"),
        ("legacy", "9.9.9"),
    ] {
        let line = row_line(&rendered, row);
        // Still through [`as_drawn`] after D76's third donor took the eighth column back to 15 at
        // the bordered 98 (17 at this render): every verdict here now fits whole, but the install
        // progress cells are longer than any width this section can hand out, so the helper stays
        // on the path that expectation is compared through.
        assert!(
            line.ends_with(&as_drawn(expected)),
            "the `on this box` column of `{row}` reads `{expected}`: {line}"
        );
    }

    insta::assert_snapshot!("agents_probed", rendered);
}

/// What the seven **fixed** columns cost together since D89: `transport` 9, `billing` 12, `models`
/// 6, `default` 21, `enabled` 7, `quota` 13, `on this box` 13.
///
/// Seven, not eight, because D89 made `name` the flexible one. Every other column sits at its own
/// longest string, which is the arithmetic behind "widening `name` is never free": there is nothing
/// in this figure that is not already being spent.
const FIXED_TOTAL: usize = 81;

/// The `column_spacing` this table draws: one column between each pair of eight, so seven.
const GAPS: usize = 7;

/// How wide the `name` column draws at a given section width — D89's reversal of D76's ranking.
///
/// `name` is `Constraint::Fill(1)` and therefore the column that absorbs the slack, so its width is
/// a **function of the render** rather than a constant, and so is the position of every column
/// after it. 10 at [`SECTION_BORDERED`], 12 at [`SECTION_WIDE`], 32 at a 120-column terminal, 112 at
/// 200. There is no cap and no tuned floor: "as wide as the terminal allows" is the whole rule.
///
/// It is also why [`row_line`] finds a row by the name **as drawn**: four of this file's made-up
/// row names are longer than the narrow widths (`needs-auth`, `spend-only`, `unparsable`,
/// `loginable`), and no registry name is.
const fn name_width(section: usize) -> usize {
    section - FIXED_TOTAL - GAPS
}

/// Where the `default` column starts: `name`'s own width, then `transport`'s 9, `billing`'s 12 and
/// `models`' 6, plus one space of `column_spacing` after each.
const fn default_at(section: usize) -> usize {
    name_width(section) + 1 + 9 + 1 + 12 + 1 + 6 + 1
}

/// How wide the `default` column is since D76: exactly `gemini-3.7-flash-high`, the longest id the
/// seeds carry. D89 did not touch it — it is D76's #1 and the one column D89's donor search was
/// forbidden to reach.
const DEFAULT_WIDTH: usize = 21;

/// Where the `quota` column starts: [`default_at`] plus `default`'s 21 and `enabled`'s 7, each with
/// its space.
const fn quota_at(section: usize) -> usize {
    default_at(section) + DEFAULT_WIDTH + 1 + 7 + 1
}

/// Where `on this box` starts: [`quota_at`] plus the quota column's own 13 (D76) and its space.
const fn on_box_at(section: usize) -> usize {
    quota_at(section) + 13 + 1
}

/// How wide `on this box` draws — **13 at every width since D89**, which is the price D89 states.
///
/// It was the `Min` column under D76 and grew with the terminal; it is fixed now, because D89 made
/// `name` the flexible one and `on this box` the donor that paid for it. So `unauthenticated` (15)
/// and `choose a method` (15) clip at *every* width rather than at the narrow one only, which is
/// what [`the_name_column_holds_the_whole_row_name_and_on_this_box_pays`] records.
const ON_BOX_WIDTH: usize = 13;

/// The `name` column at the width this file's snapshot suite renders in.
const NAME_WIDTH: usize = name_width(SECTION_WIDE as usize);

/// [`default_at`], [`quota_at`] and [`on_box_at`] at that same width, which is where all but one of
/// this file's cases look.
const DEFAULT_AT: usize = default_at(SECTION_WIDE as usize);
const QUOTA_AT: usize = quota_at(SECTION_WIDE as usize);
const ON_BOX_AT: usize = on_box_at(SECTION_WIDE as usize);

/// The eight headers, in order.
///
/// D76 made them the floor of the packing: the width for a whole model id came out of the other
/// seven columns, and a column shrunk past its own header would have bought that width with a word
/// nobody can read. Asserted as a set rather than as one pinned line so that a ninth column
/// (MOD-23's editor, MOD-12's caps section) fails this on the word it clipped.
const HEADERS: [&str; 8] = [
    "name",
    "transport",
    "billing",
    "models",
    "default",
    "enabled",
    "quota",
    "on this box",
];

/// `agy`'s seeded `default_model` (`crates/htui-core/seeds/agent_agy.json`), and the longest id in
/// the fixture at 21 characters.
const SEEDED_MODEL: &str = "gemini-3.7-flash-high";

/// One registry row whose `agent_box` carries a `quota` blob, for the quota column's own table
/// (MOD-2 D73).
///
/// Written onto a [`probed_row`] rather than through a parameter of it, so the twenty-odd cases
/// that only care about `probe` keep the four-argument helper they were written against.
fn quota_row(name: &str, quota: Option<Value>) -> AgentSummary {
    let mut summary = probed_row(
        name,
        true,
        Some("0.48.0"),
        Some(json!({ "status": "ready", "source": "probe" })),
    );
    summary
        .on_box
        .as_mut()
        .expect("`probed_row` always writes an `agent_box` row")
        .quota = quota;
    summary
}

/// The `quota` cell of one row, taken by character offset.
///
/// By offset and not by counting words, because a quota cell holds spaces of its own
/// (`62% to 09-08`), so nothing after it can be found by splitting a line on whitespace.
fn quota_cell(rendered: &str, name: &str) -> String {
    row_line(rendered, name)
        .chars()
        .skip(QUOTA_AT)
        .take(ON_BOX_AT - QUOTA_AT)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// One cell of a row rendered at a width of this file's own choosing, for the one case whose
/// subject *is* the width.
///
/// Needed since D89: `name` is the flexible column, so every column after it starts somewhere that
/// depends on the render, and the constants above are only true at [`SECTION_WIDE`].
fn cell_at(rendered: &str, name: &str, section: usize, from: usize, width: usize) -> String {
    let drawn = name.chars().take(name_width(section)).collect::<String>();
    rendered
        .lines()
        .find(|line| line.starts_with(&drawn))
        .unwrap_or_else(|| panic!("the `{name}` row is rendered:\n{rendered}"))
        .chars()
        .skip(from)
        .take(width)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The `default` cell of one row, taken by character offset for [`quota_cell`]'s reason: a model
/// id is one word, but the columns on either side of it are not, so the cell is found by where it
/// starts and not by counting.
fn default_cell(rendered: &str, name: &str) -> String {
    row_line(rendered, name)
        .chars()
        .skip(DEFAULT_AT)
        .take(DEFAULT_WIDTH)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The rendered line one row drew, by the name in its first column **as the column drew it**.
///
/// Clipped to [`NAME_WIDTH`] because four of this file's made-up row names are longer than that
/// (`needs-auth`, `spend-only`, `unparsable`, `loginable`) and no registry name is. A case keeps
/// naming its row the way it registered it; this is where the difference between the two is
/// absorbed, once, instead of in every call site.
fn row_line<'a>(rendered: &'a str, name: &str) -> &'a str {
    let drawn = name.chars().take(NAME_WIDTH).collect::<String>();
    rendered
        .lines()
        .find(|line| line.starts_with(&drawn))
        .unwrap_or_else(|| panic!("the `{name}` row is rendered:\n{rendered}"))
}

/// D76: the `default` column holds the whole `default_model` string, not a prefix of it.
///
/// `gemini-3.7-flash-high` is `agy`'s seeded default and 21 characters long; at `Length(9)` the
/// cell read `gemini-3.`, which names no model at all — the id *is* the coordinate
/// `docs/ANA-4.md` §4.4 selects a model by, so a truncated one is not a shorter answer but a wrong
/// one. The eight headers are asserted beside it because the 12 columns this needed came out of
/// the other seven, and the floor D76 set on that trade is that every header still reads.
#[tokio::test]
async fn the_default_column_holds_the_whole_model_id() {
    let bench = SectionBench::new().await;
    let mut row = quota_row("seeded", None);
    row.agent.default_model = Some(SEEDED_MODEL.to_owned());

    let mut section = AgentsSection::new();
    bench.reply(&mut section, &StoreReply::Agents(vec![row]));
    let rendered = render_section(&section, &bench.ctx());

    assert_eq!(
        default_cell(&rendered, "seeded"),
        SEEDED_MODEL,
        "the `default` column renders the id whole:\n{rendered}"
    );

    let header = rendered
        .lines()
        .next()
        .expect("the table draws a header row");
    for label in HEADERS {
        assert!(
            header.contains(label),
            "the `{label}` header is drawn whole: {header}"
        );
    }
}

/// D89's trade, both ends of it, at the [`SECTION_BORDERED`] width the running app draws in.
///
/// **This case inverts D76's.** It used to assert that `on this box` held `unauthenticated` whole,
/// which is what `name`'s 12 → 8 narrowing had bought. D89 reverses the ranking: `name` is first
/// now, `on this box` is the donor, and what this case pins is the new bargain rather than the old
/// one.
///
/// - `claude-cli` — the registry name MOD-2 milestone 8 adds, and the longest in the tree —
///   renders **whole** at 98. That is what `Constraint::Min(10)` is for, and 10 is not a round
///   number: it is that name's length, with nothing to spare.
/// - `unauthenticated` renders `unauthenticat`, and this case **records that as accepted**. It is
///   the stated price of D89 and not an accident. That verdict is where MOD-21's login flow starts
///   — `a` is offered on a row that says it — and the reason it is the affordable loss is that this
///   column carries status words, where `default` carries a selection coordinate and `quota`
///   carries an exhaustion warning. A truncated status word is still legible as itself.
/// - the model id still costs nothing, which is the half of D76 that D89 left standing.
///
/// Asserted on **one row and one render**, because the three claims are the ends of one trade: a
/// change that took width back off any of them would pass one of these and fail another, and this
/// is where that shows up as a test rather than as a screen.
#[tokio::test]
async fn the_name_column_holds_the_whole_row_name_and_on_this_box_pays() {
    let bench = SectionBench::new().await;
    let mut row = probed_row(
        "claude-cli",
        true,
        Some("1.1.26"),
        Some(json!({ "status": "unauthenticated", "source": "probe" })),
    );
    row.agent.default_model = Some(SEEDED_MODEL.to_owned());

    let mut section = AgentsSection::new();
    bench.reply(&mut section, &StoreReply::Agents(vec![row]));
    let rendered = render_section_at(&section, &bench.ctx(), SECTION_BORDERED);
    let bordered = SECTION_BORDERED as usize;

    assert_eq!(
        cell_at(&rendered, "claude-cli", bordered, 0, name_width(bordered)),
        "claude-cli",
        "the `name` column renders the whole row name at the width the app actually \
         draws:\n{rendered}"
    );
    assert_eq!(
        cell_at(
            &rendered,
            "claude-cli",
            bordered,
            on_box_at(bordered),
            ON_BOX_WIDTH
        ),
        "unauthenticat",
        "and `on this box` pays for it, which is D89's stated price and not a \
         regression:\n{rendered}"
    );
    assert_eq!(
        cell_at(
            &rendered,
            "claude-cli",
            bordered,
            default_at(bordered),
            DEFAULT_WIDTH
        ),
        SEEDED_MODEL,
        "the model id still costs nothing — D76's #1 survives D89 untouched:\n{rendered}"
    );
}

/// D89's other half: every character a wider terminal adds goes to `name` first, uncapped.
///
/// This is the behaviour the maintainer asked for — "wider, like 128", then "increase it to maximum
/// or like 256, there is no need to optimize the length" — and `Fill(1)` is that instruction with
/// no number in it at all.
///
/// The widths below are the measured resolution of ratatui 0.30.2's solver, and the case exists
/// because the obvious literal reading of the instruction does the opposite of what it sounds like:
/// `Constraint::Min(256)` resolves to `[91, 0, 0, 0, 0, 0, 0, 0]` at 98 — one column of names and
/// seven of nothing. A floor larger than the width does not widen a column, it starves its
/// neighbours. `Max(256)` and `Fill(1)` are identical at every width, so the code says `Fill(1)`.
#[tokio::test]
async fn the_name_column_absorbs_every_character_a_wider_terminal_adds() {
    let bench = SectionBench::new().await;
    let row = probed_row("claude-cli", true, Some("1.1.26"), None);
    let mut section = AgentsSection::new();
    bench.reply(&mut section, &StoreReply::Agents(vec![row]));

    for (width, expected) in [
        (98usize, 10usize),
        (100, 12),
        (120, 32),
        (160, 72),
        (200, 112),
    ] {
        assert_eq!(
            name_width(width),
            expected,
            "the arithmetic D89 was decided on: width minus the seven fixed columns and their \
             seven gaps, with no cap above it"
        );
        let rendered = render_section_at(
            &section,
            &bench.ctx(),
            u16::try_from(width).expect("a terminal width"),
        );
        assert!(
            rendered.lines().any(|line| line.starts_with("claude-cli")),
            "and the name draws whole at {width} columns:\n{rendered}"
        );
    }
}

/// The `quota` column, one row per shape plan D73 names (`R-TUI-8`, `docs/ANA-4.md` §7).
///
/// The four rows are the four answers this column has: the ANA-4 §7 document, whose tightest
/// window is the seven-day one; a source that reports no allowance at all and so carries only what
/// the session has spent; a box with a row but no blob; and a blob that parses as JSON and says
/// nothing this build understands — which is the `agy` fixture's shape, and is `—` rather than a
/// guess.
#[tokio::test]
async fn the_quota_column_renders_windows_spend_and_nothing() {
    let bench = SectionBench::new().await;
    let rows = vec![
        quota_row(
            "windows",
            Some(json!({
                "source": "acp_meta_rate_limit",
                "billing": "subscription",
                "status": "allowed",
                "exhausted": false,
                "windows": [
                    { "id": "five_hour", "utilization": 0.26, "resets_at": "2026-09-05T13:20:00Z" },
                    { "id": "seven_day", "utilization": 0.62, "resets_at": "2026-09-08T08:00:00Z" },
                ],
                "spend": { "session_micros": 394_692, "currency": "USD" },
                "observed_at": "2026-09-05T12:31:07Z",
            })),
        ),
        quota_row(
            "spend-only",
            Some(json!({
                "source": "none",
                "billing": "per_token",
                "status": "unknown",
                "exhausted": false,
                "windows": [],
                "spend": { "session_micros": 394_692, "currency": "USD" },
                "observed_at": "2026-09-05T12:31:07Z",
            })),
        ),
        quota_row("no-blob", None),
        quota_row("unparsable", Some(json!({ "remaining": 100 }))),
    ];

    let mut section = AgentsSection::new();
    bench.reply(&mut section, &StoreReply::Agents(rows));
    let rendered = render_section(&section, &bench.ctx());

    for (row, expected) in [
        // The reset without its clock time: D76 spent ` %H:%M` on the `default` column, and the
        // date is what a window's own doc comment calls the true half of the warning.
        ("windows", "62% to 09-08"),
        ("spend-only", "$0.39 spent"),
        ("no-blob", "\u{2014}"),
        ("unparsable", "\u{2014}"),
    ] {
        assert_eq!(
            quota_cell(&rendered, row),
            expected,
            "the `quota` column of `{row}` reads `{expected}`:\n{rendered}"
        );
    }

    insta::assert_snapshot!("agents_quota", rendered);
}

/// Review L-3: a window at `0.996` reads **`99%`**, not `100%`.
///
/// `{:.0}` rounded `0.995..0.999` up, and `100%` in this column is a claim about the *other* reader
/// of the same document: `quota::available` skips a row on `utilization >= 1.0` and this window is
/// below it, so the cell announced an exhausted allowance the predicate would still have selected —
/// two readers of one blob disagreeing, with the pessimistic one on screen.
///
/// The second row is the other half of the trade: `0.57` still reads `57%`, which a plain
/// `floor` of the product would render `56%` (`0.57 * 100.0` is `56.99999999999999289`). A window
/// that genuinely is full still reads `100%`, and the availability verdicts are asserted beside the
/// cells so a future change to either side fails here rather than on a screen.
#[tokio::test]
async fn a_window_short_of_full_is_not_rounded_up_to_a_full_one() {
    use htui_core::model::{Availability, quota};

    let window = |utilization: f64| {
        json!({
            "source": "acp_meta_rate_limit",
            "billing": "subscription",
            "status": "allowed",
            "exhausted": false,
            "windows": [
                { "id": "five_hour", "utilization": utilization,
                  "resets_at": "2026-09-08T08:00:00Z" },
            ],
            "spend": { "session_micros": 394_692, "currency": "USD" },
            "observed_at": "2026-09-05T12:31:07Z",
        })
    };

    let bench = SectionBench::new().await;
    let rows = vec![
        quota_row("nearly", Some(window(0.996))),
        quota_row("mid", Some(window(0.57))),
        quota_row("full", Some(window(1.0))),
    ];

    let mut section = AgentsSection::new();
    bench.reply(&mut section, &StoreReply::Agents(rows));
    let rendered = render_section(&section, &bench.ctx());

    for (row, utilization, expected) in [
        ("nearly", 0.996, "99% to 09-08"),
        ("mid", 0.57, "57% to 09-08"),
        ("full", 1.0, "100% to 09-08"),
    ] {
        assert_eq!(
            quota_cell(&rendered, row),
            expected,
            "the `quota` column of `{row}` reads `{expected}`:\n{rendered}"
        );
        let full = matches!(
            quota::available(Some(&window(utilization)), None, None),
            Availability::Skip(_)
        );
        assert_eq!(
            full,
            expected.starts_with("100%"),
            "the cell and the predicate agree about `{row}`: only a full window reads `100%`"
        );
    }
}

/// `docs/ANA-4.md` §7 requires the refresh limit be "stated in the UI rather than implied": a
/// probe handshake reports no allowance, so `r` re-probes every row and moves this column on none
/// of them. The hint line is where the section says so, because it is where every other key it
/// binds is written (MOD-20 D19).
#[tokio::test]
async fn the_idle_hint_says_r_cannot_refresh_quota() {
    let bench = SectionBench::new().await;
    let section = section_over(&bench, vec![registry_row("declared", true)]);

    let rendered = render_section(&section, &bench.ctx());
    let hint = rendered
        .lines()
        .last()
        .expect("the section always draws a hint line");
    assert!(
        hint.contains("r cannot refresh"),
        "the idle hint says `r` cannot refresh quota: `{hint}`"
    );
}

/// A second section, so the strip has something to cycle between whichever product sections are
/// registered.
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

/// A section that is taking typed text: every key is its own, `l` included (D2).
///
/// The counter is what makes the difference visible in a frame — a section that merely swallowed
/// `l` would render the same thing whether the tab cycled to it or not.
#[derive(Debug, Default)]
struct CapturingProbe {
    seen: Vec<KeyCode>,
    /// Every bracketed paste it was handed, whole (MOD-22 review M-1).
    pasted: Vec<String>,
}

impl SettingsSection for CapturingProbe {
    fn id(&self) -> SectionId {
        SectionId("capturing")
    }

    fn title(&self) -> &str {
        "Capturing"
    }

    fn captures_input(&self) -> bool {
        true
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.seen.push(key.code);
        Handled::Consumed
    }

    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        self.pasted.push(text.to_owned());
        Handled::Consumed
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pasted = self.pasted.join("|").replace('\n', "\\n");
        message(
            frame,
            area,
            &format!("seen {} pasted [{pasted}]", self.seen.len()),
            ctx.theme,
        );
    }
}

#[tokio::test]
async fn a_capturing_section_receives_l_and_a_plain_one_cycles() {
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(CapturingProbe::default()),
    ])));
    harness.settle().await;

    // Agents keeps the default `captures_input() == false`, so the tab still takes `l`.
    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("seen 0"),
        "`l` cycled into Capturing: {frame}"
    );

    // From here `l` is a letter, not a cycle: the strip must not move.
    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("seen 1"),
        "the capturing section received `l`: {frame}"
    );
    assert!(
        !frame.contains("claude"),
        "and the strip did not cycle back to Agents: {frame}"
    );

    // `h`, `[`, `]` and the arrows are the tab's the same way `l` is.
    harness.key("h");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("seen 2"), "`h` reached it too: {frame}");

    // A `Consumed` from a capturing section swallows the globals, which is the point of the gate
    // while a slug is half typed.
    harness.key("q");
    harness.settle().await;
    assert!(
        !harness.app().should_quit,
        "`q` is a letter while a section is taking text"
    );
}

/// MOD-22 review M-1: a bracketed paste while nothing captures input is dropped whole. Before the
/// terminal's paste mode it was replayed as keys, so the `2` of a pasted redirect switched tabs, a
/// `/` opened an unmasked filter and the code after it was echoed there in clear; as one
/// `Event::Paste` it reaches no keymap at all.
#[tokio::test]
async fn a_bracketed_paste_with_nothing_capturing_input_does_nothing() {
    let mut harness = Harness::demo()
        .with_tab(Box::new(SettingsTab::with_sections(vec![
            Box::new(AgentsSection::new()),
            Box::new(CapturingProbe::default()),
        ])))
        .with_tab(Box::new(htui::ui::tabs::RequirementsTab::new()));
    harness.settle().await;
    let before = harness.render();

    harness.paste(&format!("2/?code={CODE}&state={STATE}\nq"));
    harness.settle().await;
    let after = harness.render();
    assert_eq!(after, before, "no tab, section or state moved");
    assert!(!after.contains(CODE), "and nothing drew the pasted text");
    assert!(!harness.app().should_quit, "its `q` quit nothing");
}

/// MOD-22 review M-1: a section that is taking text gets the paste whole, as one event — the
/// tab's `l` and the shell's `q` inside it are characters, not a cycle and a quit.
#[tokio::test]
async fn a_bracketed_paste_reaches_a_capturing_section_whole() {
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(CapturingProbe::default()),
    ])));
    harness.settle().await;
    harness.key("l");
    harness.settle().await;

    harness.paste("l2q\n");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("seen 0 pasted [l2q\\n]"),
        "one paste, whole, and no key: {frame}"
    );
    assert!(!harness.app().should_quit);
}

/// MOD-22 review M-1: a paste into an open unmasked field lands there as typed text would, and
/// its trailing newline does not submit the form.
#[tokio::test]
async fn a_bracketed_paste_fills_an_unmasked_form_field() {
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(ProbeSection),
    ])));
    harness.settle().await;
    harness.key("n");
    harness.paste("pasted-agent\n");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("pasted-agent"), "the paste landed: {frame}");

    // Still open: `l` is a letter, not a cycle to the next section.
    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("pasted-agentl"),
        "the form is still open: {frame}"
    );
}

/// Review round 2, test gap: a modal overlay over an open field takes a paste the way it takes a
/// key it has no use for — it swallows it. The paste is dropped, never reaches the field under
/// the overlay, and nothing renders it, before or after the overlay closes.
#[tokio::test]
async fn a_bracketed_paste_under_a_modal_overlay_reaches_no_field() {
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(ProbeSection),
    ])));
    harness.settle().await;
    harness.key("n");
    harness.settle().await;
    let form = harness.render();
    harness
        .app()
        .push_overlay(Box::new(htui::ui::overlay::WorkspaceSwitcher::new()));
    harness.settle().await;
    assert_ne!(harness.render(), form, "the overlay is up");

    harness.paste("pasted-under-overlay");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("pasted-under-overlay"),
        "the overlay swallowed it: {frame}"
    );

    harness.key("esc");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("pasted-under-overlay"),
        "and the field under it never got it: {frame}"
    );
    for key in ["z", "q", "x"] {
        harness.key(key);
    }
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("zqx"),
        "the form is still open under where the overlay was, and empty before: {frame}"
    );
}

/// The strip has to fit the frame it is drawn in (D4; PRD risk "strip overflow at 100 columns").
///
/// `render_strip` draws `format!(" {title} ")` per registered section, so the joined width is
/// `Σ (title chars + 2)`. The pin is over the sections the product registers, not over a test
/// fixture: the moment a section makes the strip 101 columns wide this fails, and that is the one
/// warning a snapshot of a clipped strip could not give.
///
/// The seventh section is MOD-7 milestone 2's `Boxes` (D47), the eighth MOD-26 milestone 2's
/// `Personas` (D22) and the ninth MOD-10 milestone 4's `Secrets` (D1), each appended last as
/// `register_all` appends it. The nine (`Agents`, `Hierarchy`, `Kinds`, `Prompt`, `Connection`,
/// `Qdrant`, `Boxes`, `Personas`, `Secrets`) cost 80 of the 100 columns, so the pin is re-run
/// rather than relaxed — if a tenth ever does not fit, the fix is the strip's, not a shorter
/// title.
#[test]
fn the_section_strip_fits_the_frame() {
    let sections: Vec<Box<dyn SettingsSection>> = vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
        Box::new(PromptSection::new()),
        Box::new(ConnectionSection::new()),
        Box::new(QdrantSection::new()),
        Box::new(BoxesSection::new()),
        Box::new(PersonasSection::new()),
        Box::new(SecretsSection::new()),
    ];
    assert_eq!(sections.len(), 9);
    let width: usize = sections
        .iter()
        .map(|section| section.title().chars().count() + 2)
        .sum();
    assert!(
        width <= usize::from(SECTION_WIDE),
        "the section strip is {width} columns and the frame is {SECTION_WIDE}"
    );
}

// -------------------------------------------------------------------------------------------
// MOD-20 T8: the row cursor, `i`, and the install pane (blueprint B.12)
// -------------------------------------------------------------------------------------------

/// One registry row, with or without a declared install source.
///
/// The id is made up and so is the tool: `R-AGT-5` says no vendor is named in `src/`, and a test
/// that spelled one would be asserting the seeds rather than the section.
fn registry_row(name: &str, declares: bool) -> AgentSummary {
    let mut discovery = json!({
        "tools": {
            "demo_server": {
                "kind": "glob",
                "patterns": ["%HTUI_AGENTS_ROOT%/demo-acp/*/demo_server"],
            },
        },
        "handshake": true,
    });
    if declares {
        discovery["install"] =
            json!({ "source": "acp_registry", "id": "demo-acp", "tool": "demo_server" });
    }
    let mut summary = probed_row(name, true, None, None);
    summary.on_box = None;
    summary.agent.launch = json!({
        "command": "${demo_server}",
        "args": [],
        "env": {},
        "discovery": discovery,
    });
    summary
}

/// The plan the consent pane draws, for the row `agent_id` names.
///
/// Hand-built rather than produced by `plan()`: this file's subject is what the section does with
/// a plan, and a pre-flight in front of every case would put a registry read on the path of a
/// test about rendering. It is exactly the value `StoreReply::Install(Plan)` carries.
fn demo_plan(agent_id: AgentId, agent_name: &str, sha256: Option<&str>) -> InstallPlan {
    InstallPlan {
        agent_id,
        agent_name: agent_name.to_owned(),
        tool: "demo_server".to_owned(),
        registry_id: "demo-acp".to_owned(),
        registry_name: "Demo".to_owned(),
        version: "1.2.3".to_owned(),
        platform: "linux-x86_64".to_owned(),
        archive_url: "http://127.0.0.1:1/archive.zip".to_owned(),
        format: ArchiveFormat::Zip,
        content_length: Some(4 * 1024 * 1024),
        sha256: sha256.map(str::to_owned),
        cmd: "./demo_server".to_owned(),
        args: Vec::new(),
        env: BTreeMap::new(),
        license: Some("proprietary".to_owned()),
        license_url: Some("http://127.0.0.1:1/terms".to_owned()),
        root: PathBuf::from("/tmp/htui-agents"),
        install_dir: PathBuf::from("/tmp/htui-agents/demo-acp/1.2.3"),
        existing_versions: vec!["1.2.2".to_owned()],
        available_bytes: Some(64 * 1024 * 1024),
        need_bytes: Some(16 * 1024 * 1024),
        args_differ: false,
        consent: None,
        recorded: None,
        registry_cached_age_secs: None,
        planned_at: htui_core::fixtures::demo_at(0, 0),
    }
}

/// A section holding `rows`, with the cursor where a fresh registry read leaves it.
/// A pre-flight is two requests and usually gone before a key lands, but a plan task that ends
/// without a frame would strand the section in `Planning` with no way out but a restart — `i` and
/// `r` are both refused there and `Esc` belongs to `Manual`. So `x` cancels a pre-flight too
/// (review finding, MOD-20 T8). The runtime serves the cancel, and a task already swept answers
/// `Failed { "install_cancel" }`, which lands on `Idle` either way.
#[tokio::test]
async fn x_cancels_a_pre_flight_that_has_not_answered() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("declared", true)]);

    assert_eq!(bench.key(&mut section, "i"), Handled::Consumed);
    let asked = bench.drained();
    assert!(
        asked
            .iter()
            .any(|action| matches!(action, Action::Store(StoreRequest::InstallPlan { .. }))),
        "`i` asks for a plan: {asked:?}"
    );

    assert_eq!(
        bench.key(&mut section, "x"),
        Handled::Consumed,
        "`x` is bound while planning, not only while downloading"
    );
    let asked = bench.drained();
    assert!(
        asked
            .iter()
            .any(|action| matches!(action, Action::Store(StoreRequest::InstallCancel))),
        "and it asks the runtime to drop the pre-flight: {asked:?}"
    );
    assert!(
        render_section(&section, &bench.ctx()).contains("x cancel install"),
        "the hint offers the key it binds"
    );
}

fn section_over(bench: &SectionBench, rows: Vec<AgentSummary>) -> AgentsSection {
    let mut section = AgentsSection::new();
    bench.reply(&mut section, &StoreReply::Agents(rows));
    section
}

/// The `on this box` cell of one row, which is the last column of its line.
///
/// By character offset since D73's `quota` column landed in front of it: that cell holds spaces of
/// its own, so the columns after it can no longer be counted in words.
fn on_box_cell(rendered: &str, name: &str) -> String {
    row_line(rendered, name)
        .chars()
        .skip(ON_BOX_AT)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// One `on this box` expectation as the eighth column can actually draw it.
///
/// [`ON_BOX_WIDTH`] characters, at **every** render width since D89 made this the donor column and
/// `name` the flexible one. That is a wider net than it used to be: under D76 this column grew with
/// the terminal and only the bordered 98 clipped, so an expectation written whole passed at 100 and
/// the one case that mattered was asserted at 98. Now `unauthenticated` and `choose a method` (15
/// each) clip everywhere, and [`the_name_column_holds_the_whole_row_name_and_on_this_box_pays`] is
/// where that is recorded as the price rather than as damage. The install progress cells
/// (`downloading 12.0 MB`, 19) were already over the line before D76 and no width was ever spent on
/// them.
///
/// So the expectations stay written **whole**, because they are what the section computed and a
/// test with `unauthenticat` inlined in it would read as a bug rather than as a packing decision;
/// this clips them the way the frame does.
fn as_drawn(expected: &str) -> String {
    expected
        .chars()
        .take(ON_BOX_WIDTH)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Plan D19's cursor: `j`/`k` only, no wrap at either end.
///
/// Where the cursor is, is asserted two ways because it has two jobs: it is the accented line, and
/// it is the row `i` acts on. A test that only read the highlight would pass on a section that
/// installs the wrong row.
#[tokio::test]
async fn j_and_k_move_the_row_cursor_and_stop_at_both_ends() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![
            registry_row("alpha", false),
            registry_row("beta", false),
            registry_row("gamma", false),
        ],
    );

    assert!(
        accented_lines(&section, &bench.ctx())[0].starts_with("alpha"),
        "a fresh registry read leaves the cursor on the first row"
    );

    bench.key(&mut section, "j");
    assert!(accented_lines(&section, &bench.ctx())[0].starts_with("beta"));

    // Three more `j` on a three-row table: the cursor stops on the last row rather than wrapping
    // round to the first, because a wrap makes `i` install a row the user did not aim at.
    for _ in 0..3 {
        bench.key(&mut section, "j");
    }
    assert!(accented_lines(&section, &bench.ctx())[0].starts_with("gamma"));
    let _ = bench.drained();
    bench.key(&mut section, "i");
    assert_eq!(
        bench.errors(),
        vec!["nothing declares how to install `gamma`".to_owned()],
        "`i` acts on the row the cursor is on"
    );

    for _ in 0..5 {
        bench.key(&mut section, "k");
    }
    assert!(accented_lines(&section, &bench.ctx())[0].starts_with("alpha"));
    let _ = bench.drained();
    bench.key(&mut section, "i");
    assert_eq!(
        bench.errors(),
        vec!["nothing declares how to install `alpha`".to_owned()],
    );
}

/// `i` on a row that declares no source is refused **by the row**: the answer is in the document
/// the section is already holding, so no request leaves and no byte is fetched.
#[tokio::test]
async fn i_on_a_row_without_an_install_block_is_refused_by_name_and_sends_nothing() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("undeclared", false)]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "i"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        emitted
            .iter()
            .filter_map(|action| match action {
                Action::Error(message) => Some(message.clone()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["nothing declares how to install `undeclared`".to_owned()],
    );
    assert!(
        !emitted
            .iter()
            .any(|action| matches!(action, Action::Store(_))),
        "the refusal is parsed from the row in hand, not asked of the store: {emitted:?}"
    );
    assert!(
        !render_section(&section, &bench.ctx()).contains("install demo-acp"),
        "and no consent pane opens"
    );
}

/// `i` on a row that does declare one asks for the pre-flight and **nothing else**: `R-AGT-10`'s
/// rule is that the user is told what will be fetched before anything is.
#[tokio::test]
async fn i_asks_for_a_plan_and_never_for_a_confirm() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "i"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(emitted.len(), 1, "one request, no status line: {emitted:?}");
    assert!(
        matches!(
            &emitted[0],
            Action::Store(StoreRequest::InstallPlan { agent_id: asked }) if *asked == agent_id
        ),
        "`i` pre-flights the highlighted row: {emitted:?}"
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "declared"),
        "planning\u{2026}",
        "and the cell says the pre-flight is running"
    );
}

/// `i` while a probe is running is refused by name: the rows on screen do not answer the question
/// the user just asked, so an install planned against them would be planned against stale facts.
#[tokio::test]
async fn i_while_a_probe_runs_is_refused() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("declared", true)]);
    bench.key(&mut section, "r");
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "i"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        emitted
            .iter()
            .filter_map(|action| match action {
                Action::Error(message) => Some(message.clone()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["a probe is running".to_owned()],
    );
    assert!(
        !emitted
            .iter()
            .any(|action| matches!(action, Action::Store(_))),
        "and nothing is asked of the store: {emitted:?}"
    );
}

/// Hazard H-10 at the section: a second `i` while a pre-flight or an install runs is refused, and
/// so is `r` — a probe spawns a process per agent and the install is about to spawn one itself.
#[tokio::test]
async fn i_and_r_are_both_refused_while_an_install_is_in_flight() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("declared", true)]);
    bench.key(&mut section, "i");
    let _ = bench.drained();

    bench.key(&mut section, "i");
    assert_eq!(
        bench.errors(),
        vec!["an install is already running".to_owned()]
    );
    bench.key(&mut section, "r");
    assert_eq!(
        bench.errors(),
        vec!["an install is running; probe afterwards".to_owned()]
    );
}

/// Plan D19's local modality: with a plan on screen the section consumes every key it is offered,
/// so a `j` or an `r` cannot move the ground under a consent the user has not answered yet.
///
/// The tab's own `h`/`l`/`[`/`]`/arrows never reach here — `SettingsTab::on_key` takes them first —
/// which is what keeps a pending plan from trapping the user in the section.
#[tokio::test]
async fn a_pending_plan_swallows_every_key_but_y_n_and_esc() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row, registry_row("second", true)]);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(demo_plan(
            agent_id, "declared", None,
        )))),
    );
    let _ = bench.drained();

    for chord in ["j", "k", "r", "i", "x"] {
        assert_eq!(
            bench.key(&mut section, chord),
            Handled::Consumed,
            "`{chord}` is this section's own key, so the pane swallows it"
        );
    }
    // `App::on_key` offers the active tab a key **before** the `Tab` and `Global` keymaps and
    // returns as soon as the tab says `Consumed` (`app/state.rs:380-421`). A pane that consumed
    // everything would therefore kill `q`, `?`, `Tab` and the digit tab-switches while it is open,
    // and the user could not even quit. The pane is modal over the table below it, not over the app.
    for chord in ["q", "g", "?"] {
        assert_eq!(
            bench.key(&mut section, chord),
            Handled::Pass,
            "`{chord}` belongs to the global table and must still reach it"
        );
    }
    assert!(
        bench.drained().is_empty(),
        "and none of them asks anything of the store"
    );
    assert!(
        accented_lines(&section, &bench.ctx())[0].starts_with("declared"),
        "the cursor did not move under the pane"
    );

    assert_eq!(bench.key(&mut section, "n"), Handled::Consumed);
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        !rendered.contains("install Demo 1.2.3"),
        "`n` closes the pane: {rendered}"
    );
    assert!(
        rendered.contains("install declined"),
        "and says so on the hint line: {rendered}"
    );
    assert!(
        bench.drained().is_empty(),
        "a declined plan is not confirmed"
    );
}

/// `y` is the only key that starts a download, and it carries the plan back unchanged: plan D12's
/// rule that what the user said yes to is what is installed.
#[tokio::test]
async fn y_confirms_exactly_the_plan_that_was_shown() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    let plan = demo_plan(agent_id, "declared", Some(&"ab".repeat(32)));
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(plan.clone()))),
    );
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "y"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(emitted.len(), 1, "{emitted:?}");
    let Action::Store(StoreRequest::InstallConfirm { plan: confirmed }) = &emitted[0] else {
        panic!("`y` confirms: {emitted:?}")
    };
    assert_eq!(**confirmed, plan, "byte for byte the plan that was drawn");
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "declared"),
        "planning\u{2026}",
        "the cell moves to the install's own first phase"
    );
}

/// The consent pane, every line of plan D13 in order — and both wordings of the digest line, since
/// eight of the registry's forty entries publish no `sha256` and the honest sentence for those is
/// what the user is agreeing to.
#[tokio::test]
async fn the_consent_pane_renders_every_line_and_both_digest_wordings() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);

    let published = demo_plan(agent_id, "declared", Some(&"ab".repeat(32)));
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(published.clone()))),
    );
    let rendered = render_section(&section, &bench.ctx());
    for line in published.consent_lines() {
        assert!(
            rendered.contains(line.trim_end()),
            "the pane draws `{line}`:\n{rendered}"
        );
    }
    assert!(
        rendered.contains("sha256 published: verified before unpacking"),
        "{rendered}"
    );
    assert!(
        rendered.contains("y install \u{b7} n cancel"),
        "the hint line stands in for a help entry: {rendered}"
    );

    let unpublished = demo_plan(agent_id, "declared", None);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(unpublished))),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains(
            "none published: htui cannot verify this download and will record what it receives"
        ),
        "{rendered}"
    );
}

/// The progress cell, one phase at a time (plan D19).
///
/// The `unpacking` case is the one a live run found: `unpack` counts *completed entries*, so a
/// single-entry archive reports `done = 0` for the whole unpack and then jumps to `done == total`.
/// A percentage computed from that reads `0%` for the entire phase, which is why a zero numerator
/// renders the phase word alone.
#[tokio::test]
async fn the_progress_cell_renders_each_phase_and_never_a_misleading_zero() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(demo_plan(
            agent_id, "declared", None,
        )))),
    );
    bench.key(&mut section, "y");
    let _ = bench.drained();

    for (phase, done, total, expected) in [
        (InstallPhase::Planning, 0, None, "planning\u{2026}"),
        (
            InstallPhase::Downloading,
            0,
            Some(100),
            "downloading\u{2026}",
        ),
        (InstallPhase::Downloading, 42, Some(100), "downloading 42%"),
        (
            InstallPhase::Downloading,
            12 * 1024 * 1024,
            None,
            "downloading 12.0 MB",
        ),
        (InstallPhase::Verifying, 0, None, "verifying\u{2026}"),
        (InstallPhase::Unpacking, 0, Some(4096), "unpacking\u{2026}"),
        (InstallPhase::Unpacking, 4096, Some(4096), "unpacking 100%"),
        (InstallPhase::Probing, 0, None, "probing\u{2026}"),
    ] {
        bench.reply(
            &mut section,
            &StoreReply::Install(InstallFrame::Progress { phase, done, total }),
        );
        let rendered = render_section(&section, &bench.ctx());
        // Through [`as_drawn`]: three of the eight phase cells are longer than the 13 columns
        // `on this box` draws in since D76, and what this case is about — that a zero numerator
        // renders the phase word instead of `0%` — is decided in the first twelve of them.
        assert_eq!(
            on_box_cell(&rendered, "declared"),
            as_drawn(expected),
            "{phase} {done}/{total:?}:\n{rendered}"
        );
        assert!(
            rendered.contains("x cancel install"),
            "the hint line offers the way out: {rendered}"
        );
    }
}

/// `x` asks the runtime to stop, and the cell says so at once rather than after the install
/// notices: a cancel the user cannot see is a cancel they press twice.
#[tokio::test]
async fn x_while_an_install_runs_asks_for_a_cancel_and_the_cell_says_so() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(demo_plan(
            agent_id, "declared", None,
        )))),
    );
    bench.key(&mut section, "y");
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "x"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        matches!(&emitted[..], [Action::Store(StoreRequest::InstallCancel)]),
        "{emitted:?}"
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "declared"),
        "cancelling\u{2026}"
    );

    bench.reply(&mut section, &StoreReply::Install(InstallFrame::Cancelled));
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "declared"),
        "not probed",
        "a cancelled install leaves the cell where it was"
    );
    assert!(rendered.contains("install cancelled"), "{rendered}");
}

/// Plan D20's fallback: a failure the user can route around is rendered as the steps, derived
/// entirely from the plan and the helper, and `Esc` puts them away.
#[tokio::test]
async fn a_failed_install_with_manual_steps_renders_them_under_the_table() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    let plan = demo_plan(agent_id, "declared", None);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(plan.clone()))),
    );
    bench.key(&mut section, "y");
    let _ = bench.drained();

    let steps: ManualSteps = plan.manual_steps("http://127.0.0.1:1");
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Failed {
            message: "the registry could not be reached".to_owned(),
            manual: Some(Box::new(steps.clone())),
        }),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("the registry could not be reached"),
        "{rendered}"
    );
    for line in steps.lines() {
        assert!(rendered.contains(line.trim_end()), "`{line}`:\n{rendered}");
    }
    assert!(rendered.contains("Esc close"), "{rendered}");
    assert_eq!(
        on_box_cell(&rendered, "declared"),
        "not probed",
        "and the row is back to what the probe last said about it"
    );

    assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);
    assert!(
        !render_section(&section, &bench.ctx()).contains("make ./demo_server executable"),
        "`Esc` closes the steps"
    );
}

/// A failure the user cannot route around is one sentence on the hint line, not a pane.
///
/// The sentence is shorter than MOD-20 wrote it: the idle hint gained `a authenticate` (MOD-21
/// D20); since MOD-23 D245 the keys and the notice are two lines, so the notice no longer shares
/// the keys' row. What this case is about is that a failure with no steps *is* a notice, and a
/// fixture long enough to be clipped at the frame's edge would be asserting the width rather than
/// the routing.
#[tokio::test]
async fn a_failure_without_manual_steps_is_a_notice() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(demo_plan(
            agent_id, "declared", None,
        )))),
    );
    bench.key(&mut section, "y");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Failed {
            message: "the archive is refused: an entry escapes".to_owned(),
            manual: None,
        }),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("the archive is refused: an entry escapes"),
        "{rendered}"
    );
    assert!(
        rendered.contains("i install"),
        "and the section is idle again: {rendered}"
    );
}

/// `R-AGT-6` at the section: `Done` is not a status, it is the cue to read the registry again.
/// There is no "installed" state anywhere in this file — the probe's row is what the cell shows.
#[tokio::test]
async fn a_done_frame_reads_the_registry_again_rather_than_claiming_success() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(demo_plan(
            agent_id, "declared", None,
        )))),
    );
    bench.key(&mut section, "y");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Done(Box::new(InstallOutcome::Installed {
            record: demo_record(),
            version: "1.2.3".to_owned(),
            dir: PathBuf::from("/tmp/htui-agents/demo-acp/1.2.3"),
            row: probed_box(agent_id),
            status: ProbeStatus::Ready,
            removed_versions: vec!["1.2.2".to_owned()],
            digest_changed: false,
        }))),
    );
    let emitted = bench.drained();
    assert!(
        matches!(&emitted[..], [Action::Store(StoreRequest::Agents)]),
        "the probe is the authority, so the section re-reads what it wrote: {emitted:?}"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("installed demo-acp 1.2.3"), "{rendered}");
    assert_eq!(
        on_box_cell(&rendered, "declared"),
        "not probed",
        "the cell shows the row it has, not a claim the install made"
    );

    // What the fresh read brings back is what the cell then says, whatever the verdict was.
    let mut probed = row;
    probed.on_box = Some(probed_box(agent_id));
    bench.reply(&mut section, &StoreReply::Agents(vec![probed]));
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "declared"),
        "9.9.9"
    );
}

/// Hazard H-17: a `StoreReply::Agents` from a scope change mid-download must not close the pane.
///
/// The module doc's warning is about `probing`, and it stays true of `probing`. The install state
/// deliberately does not join it: a probe that loses its in-flight flag re-renders one column, an
/// install that loses its state leaves a running download with no way to cancel it.
#[tokio::test]
async fn an_agents_reply_does_not_clear_an_install_in_flight() {
    let bench = SectionBench::new().await;
    let row = registry_row("declared", true);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Plan(Box::new(demo_plan(
            agent_id, "declared", None,
        )))),
    );
    bench.key(&mut section, "y");
    bench.reply(
        &mut section,
        &StoreReply::Install(InstallFrame::Progress {
            phase: InstallPhase::Downloading,
            done: 42,
            total: Some(100),
        }),
    );
    let _ = bench.drained();

    bench.reply(&mut section, &StoreReply::Agents(vec![row]));
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "declared"),
        as_drawn("downloading 42%"),
        "the download is still running and still says so: {rendered}"
    );
    assert!(
        rendered.contains("x cancel install"),
        "and can still be stopped: {rendered}"
    );
}

/// A `Failed` reply naming one of the three install requests is the shell's message to render, not
/// the section's: `App::update` has already put it on the status line, so all that is left here is
/// to stop saying an install is running.
#[tokio::test]
async fn a_refused_install_request_leaves_the_section_idle() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("declared", true)]);
    bench.key(&mut section, "i");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "install_plan",
            message: "this runtime has no installer".to_owned(),
        },
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "declared"),
        "not probed"
    );
    bench.key(&mut section, "i");
    let emitted = bench.drained();
    assert!(
        matches!(
            &emitted[..],
            [Action::Store(StoreRequest::InstallPlan { .. })]
        ),
        "a refused pre-flight does not lock the section out of a second try: {emitted:?}"
    );
}

/// One manifest record, for an outcome a test hands the section.
fn demo_record() -> InstallRecord {
    InstallRecord {
        sha256: "ab".repeat(32),
        published: true,
        archive: "http://127.0.0.1:1/archive.zip".to_owned(),
        platform: "linux-x86_64".to_owned(),
        installed_at: htui_core::fixtures::demo_at(0, 0),
    }
}

/// The `agent_box` row a probe would have written for `agent_id`.
fn probed_box(agent_id: AgentId) -> AgentBox {
    AgentBox {
        agent_id,
        box_id: BoxId::new(),
        enabled: true,
        version: Some("9.9.9".to_owned()),
        path: None,
        probed_at: Some(htui_core::fixtures::demo_at(0, 0)),
        quota: None,
        quota_at: None,
        updated_at: htui_core::fixtures::demo_at(0, 0),
        probe: Some(json!({ "status": "ready", "source": "probe" })),
    }
}

// -------------------------------------------------------------------------------------------
// MOD-21 T7: `a`, the chooser and the login stream (plan D20, blueprint B.10)
// -------------------------------------------------------------------------------------------

/// The method id the chooser is fed with, and the one `Enter` sends back.
///
/// Made up, like every other id in this file (`R-AGT-5`): the chooser is fed by the agent's own
/// live `initialize` answer, so a case only ever needs *an* id and never a real one.
const METHOD: &str = "m-one";

/// A second one, so `j` has somewhere to go.
const OTHER_METHOD: &str = "m-two";

/// A registry row a login may be offered on: `acp`, probed, and advertising `auth_methods`.
///
/// Built through [`ProbeSnapshot`] rather than as hand-written JSON because the section reads it
/// back through `ProbeSnapshot::from_row`: a case that spelled the document by hand would pass on
/// a section that read a field the probe does not write.
fn login_row(name: &str, status: ProbeStatus, auth_methods: &[&str]) -> AgentSummary {
    let snapshot = ProbeSnapshot {
        transport: Transport::Acp,
        resolved: None,
        tools: BTreeMap::new(),
        handshake: Some(Handshake {
            at: htui_core::fixtures::demo_at(0, 0),
            protocol_version: 1,
            agent_name: Some(name.to_owned()),
            agent_version: Some("0.0.0".to_owned()),
            capabilities: json!({}),
            auth_methods: auth_methods.iter().map(|id| (*id).to_owned()).collect(),
        }),
        credential: Some(CredentialTier::Absent),
        status,
        stderr_tail: None,
        source: ProbeSource::Probe,
        manual: BTreeMap::new(),
    };
    probed_row(name, true, Some("1.1.1"), Some(snapshot.to_value()))
}

/// The error texts of one drain, without emptying the queue a second time.
fn errors_of(emitted: &[Action]) -> Vec<String> {
    emitted
        .iter()
        .filter_map(|action| match action {
            Action::Error(message) => Some(message.clone()),
            _ => None,
        })
        .collect()
}

/// Whether a drain asked the store for anything at all.
fn asked_anything(emitted: &[Action]) -> bool {
    emitted
        .iter()
        .any(|action| matches!(action, Action::Store(_)))
}

/// The method list a live flow answers `AuthStart` with.
fn methods_frame(logout: bool, hidden: usize) -> StoreReply {
    StoreReply::Auth(AuthFrame::Methods {
        methods: vec![
            AuthMethodInfo {
                id: METHOD.to_owned(),
                name: "One".to_owned(),
                description: Some("the first way in".to_owned()),
            },
            AuthMethodInfo {
                id: OTHER_METHOD.to_owned(),
                name: "Two".to_owned(),
                description: None,
            },
        ],
        logout,
        hidden: (0..hidden)
            .map(|n| AuthMethodInfo {
                id: format!("t-{n}"),
                name: format!("Terminal {n}"),
                description: None,
            })
            .collect(),
    })
}

/// A section with one row a login is offered on, already past `a` and past the method list.
fn chooser_over(bench: &SectionBench, logout: bool, hidden: usize) -> AgentsSection {
    let mut section = section_over(
        bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD, OTHER_METHOD],
        )],
    );
    bench.key(&mut section, "a");
    bench.reply(&mut section, &methods_frame(logout, hidden));
    let _ = bench.drained();
    section
}

/// D10's predicate, read off the row before a request is spent: a `cli` row has no `authenticate`
/// call at all, and the refusal names the row and its transport.
#[tokio::test]
async fn a_on_a_cli_row_is_refused_by_name_and_sends_nothing() {
    let bench = SectionBench::new().await;
    let mut row = login_row("cli-row", ProbeStatus::Unauthenticated, &[METHOD]);
    row.agent.transport = Transport::Cli;
    let mut section = section_over(&bench, vec![row]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["`cli-row` is a `cli` agent; it has no `authenticate` call".to_owned()]
    );
    assert!(
        !asked_anything(&emitted),
        "the predicate is read from the row in hand: {emitted:?}"
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "cli-row"),
        as_drawn("unauthenticated"),
        "and the row is exactly where it was"
    );
}

/// D7's stated cost: the *snapshot* decides whether `a` is offered, so a row nobody has probed has
/// nothing to offer it from.
#[tokio::test]
async fn a_on_an_unprobed_row_says_probe_first() {
    let bench = SectionBench::new().await;
    let unprobed = probed_row("never-probed", true, None, None);
    let mut section = section_over(&bench, vec![unprobed]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["probe `never-probed` first".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");
}

/// The same sentence for a row whose probe said something a login cannot start from: `missing` and
/// `failed` are boxes that could not run the adapter at all, and a spawn would only say so again.
#[tokio::test]
async fn a_on_a_row_the_probe_could_not_run_says_probe_first() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![login_row("broke", ProbeStatus::Failed, &[METHOD])],
    );
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec!["probe `broke` first".to_owned()]);
    assert!(!asked_anything(&emitted), "{emitted:?}");
}

/// A probed row whose agent demands nothing is a row with no method to choose from, and the
/// refusal says so in the row's own name rather than spawning to find out.
#[tokio::test]
async fn a_on_a_row_with_no_auth_methods_is_refused_by_name() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![login_row("open-door", ProbeStatus::Ready, &[])],
    );
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["`open-door` advertises no authentication methods".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");
}

/// The three in-flight refusals and the empty table, each by its own sentence and each before a
/// request is spent.
#[tokio::test]
async fn a_while_something_else_is_in_flight_is_refused_and_sends_nothing() {
    let bench = SectionBench::new().await;
    let mut empty = AgentsSection::new();
    assert_eq!(bench.key(&mut empty, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["no agent row is selected".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");

    // A probe in flight: the rows on screen do not answer the question the user just asked.
    let mut probing = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD],
        )],
    );
    bench.key(&mut probing, "r");
    let _ = bench.drained();
    bench.key(&mut probing, "a");
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec!["a probe is running".to_owned()]);
    assert!(!asked_anything(&emitted), "{emitted:?}");

    // An install in flight: both end by writing the same `agent_box` row (D19).
    let mut installing = section_over(&bench, vec![registry_row("declared", true)]);
    bench.key(&mut installing, "i");
    let _ = bench.drained();
    bench.key(&mut installing, "a");
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["an install is running".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");

    // A login in flight: the runtime allows one, and a second `a` is refused here rather than
    // there so the first flow's pane is never replaced by a refusal.
    let mut logging_in = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD],
        )],
    );
    bench.key(&mut logging_in, "a");
    let _ = bench.drained();
    bench.key(&mut logging_in, "a");
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["a login is already running".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");
}

/// `a` on a row the probe left `unauthenticated` asks for the flow and says so in the cell.
#[tokio::test]
async fn a_on_an_unauthenticated_row_sends_auth_start_and_the_cell_reads_starting() {
    let bench = SectionBench::new().await;
    let row = login_row("loginable", ProbeStatus::Unauthenticated, &[METHOD]);
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(emitted.len(), 1, "one request, no status line: {emitted:?}");
    assert!(
        matches!(
            &emitted[0],
            Action::Store(StoreRequest::AuthStart { agent_id: asked }) if *asked == agent_id
        ),
        "`a` logs the highlighted row in: {emitted:?}"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(on_box_cell(&rendered, "loginable"), "starting\u{2026}");
    assert!(
        rendered.contains("o open link \u{b7} p paste redirect \u{b7} x cancel"),
        "and the hint offers the keys a live flow binds: {rendered}"
    );
}

/// `a` is offered on a `ready` row too: a box that is logged in is exactly the box that can only
/// log *out*, and the chooser is where a logout lives (D20).
#[tokio::test]
async fn a_on_a_ready_row_that_advertises_methods_is_offered() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![login_row("logged-in", ProbeStatus::Ready, &[METHOD])],
    );
    let _ = bench.drained();

    bench.key(&mut section, "a");
    let emitted = bench.drained();
    assert!(
        matches!(
            &emitted[..],
            [Action::Store(StoreRequest::AuthStart { .. })]
        ),
        "{emitted:?}"
    );
}

/// The chooser is the agent's own words: every method's name and description, a logout row when it
/// advertised one, and one dim line for what `htui` cannot offer (D4, D7).
#[tokio::test]
async fn a_methods_frame_renders_the_chooser_with_names_descriptions_logout_and_the_hidden_count() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, true, 2);

    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("One \u{2014} the first way in"),
        "a method with a description reads as one line: {rendered}"
    );
    assert!(
        rendered.contains("Two"),
        "and one without still reads as its name: {rendered}"
    );
    assert!(
        !rendered.contains(METHOD),
        "the id is what is sent, never what is shown: {rendered}"
    );
    assert!(
        rendered.contains("log out"),
        "the agent advertised a logout verb: {rendered}"
    );
    assert!(
        rendered.contains("2 method(s) need a terminal htui does not provide"),
        "and the terminal-typed ones are named as missing, not offered: {rendered}"
    );
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        as_drawn("choose a method"),
        "the cell says what the pane is waiting for"
    );
    assert!(
        rendered.contains("j/k choose \u{b7} Enter select \u{b7} Esc cancel"),
        "and the hint says which keys answer it: {rendered}"
    );

    // Nothing was advertised, nothing is offered: no logout row and no hidden line.
    let bare = chooser_over(&bench, false, 0);
    let rendered = render_section(&bare, &bench.ctx());
    assert!(!rendered.contains("log out"), "{rendered}");
    assert!(!rendered.contains("need a terminal"), "{rendered}");
    let _ = &mut section;
}

/// `j`/`k` move over the methods and the logout row, and `Enter` sends what the cursor is on.
///
/// `Enter` rather than a digit: the digits are the shell's tab switches (MOD-20's review finding),
/// and a pane that bound them would take the user's way out of the section with it.
#[tokio::test]
async fn j_k_and_enter_choose_and_send_the_choice() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, true, 0);

    assert!(
        accented_lines(&section, &bench.ctx())
            .iter()
            .any(|line| line.starts_with("One")),
        "the cursor starts on the agent's first method"
    );
    assert_eq!(bench.key(&mut section, "j"), Handled::Consumed);
    assert!(
        accented_lines(&section, &bench.ctx())
            .iter()
            .any(|line| line.starts_with("Two")),
        "`j` moves down the live list"
    );
    assert_eq!(bench.key(&mut section, "k"), Handled::Consumed);
    assert!(
        accented_lines(&section, &bench.ctx())
            .iter()
            .any(|line| line.starts_with("One")),
        "`k` moves back"
    );

    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        matches!(
            &emitted[..],
            [Action::Store(StoreRequest::AuthChoose {
                choice: AuthChoice::Method(id)
            })] if id == METHOD
        ),
        "`Enter` sends the id the agent gave, for the row the cursor is on: {emitted:?}"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(on_box_cell(&rendered, "loginable"), "logging in\u{2026}");
    assert!(
        !rendered.contains("the first way in"),
        "the chooser is gone: {rendered}"
    );

    // The last row is the logout the agent advertised, and it is a different call.
    let mut section = chooser_over(&bench, true, 0);
    for _ in 0..5 {
        bench.key(&mut section, "j");
    }
    assert!(
        accented_lines(&section, &bench.ctx())
            .iter()
            .any(|line| line.starts_with("log out")),
        "the cursor stops on the last row rather than wrapping"
    );
    bench.key(&mut section, "Enter");
    let emitted = bench.drained();
    assert!(
        matches!(
            &emitted[..],
            [Action::Store(StoreRequest::AuthChoose {
                choice: AuthChoice::Logout
            })]
        ),
        "{emitted:?}"
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "loginable"),
        "logging out\u{2026}"
    );
}

/// `Esc` (and `n`) in the chooser stop the flow rather than closing the pane behind its back: the
/// adapter is already spawned and only the runtime can kill it.
#[tokio::test]
async fn esc_in_the_chooser_cancels() {
    let bench = SectionBench::new().await;
    for chord in ["Esc", "n"] {
        let mut section = chooser_over(&bench, true, 0);
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        let emitted = bench.drained();
        assert!(
            matches!(&emitted[..], [Action::Store(StoreRequest::AuthCancel)]),
            "`{chord}` asks the runtime to stop the flow: {emitted:?}"
        );
        let rendered = render_section(&section, &bench.ctx());
        assert_eq!(
            on_box_cell(&rendered, "loginable"),
            "cancelling\u{2026}",
            "and the cell says so until the flow's own last frame: {rendered}"
        );
        assert!(
            !rendered.contains("the first way in"),
            "the chooser is closed: {rendered}"
        );
    }
}

/// The MOD-20 review rule, applied to the second pane this section grew: the digits are tab
/// switches and `q`/`?` are global, so a chooser that consumed them would take the user's way out
/// of the application with it.
#[tokio::test]
async fn digits_and_q_pass_through_the_chooser() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, true, 1);

    for chord in ["1", "2", "9", "q", "?", "g"] {
        assert_eq!(
            bench.key(&mut section, chord),
            Handled::Pass,
            "`{chord}` belongs to the global table and must still reach it"
        );
    }
    assert!(
        bench.drained().is_empty(),
        "and none of them asks anything of the store"
    );
    assert!(
        render_section(&section, &bench.ctx()).contains("the first way in"),
        "the chooser is still up"
    );
}

/// The stream pane: the last six stderr lines as the adapter wrote them, and the link it printed.
#[tokio::test]
async fn line_and_url_frames_render_the_last_six_lines_and_the_link() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    for n in 0..8 {
        bench.reply(
            &mut section,
            &StoreReply::Auth(AuthFrame::Line(format!("line {n}"))),
        );
    }
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Url("https://h.invalid/o?a=1".to_owned())),
    );

    let rendered = render_section(&section, &bench.ctx());
    for gone in ["line 0", "line 1"] {
        assert!(
            !rendered.contains(gone),
            "only the last six lines are on screen: {rendered}"
        );
    }
    for kept in ["line 2", "line 3", "line 4", "line 5", "line 6", "line 7"] {
        assert!(
            rendered.contains(kept),
            "{kept} is one of the last six: {rendered}"
        );
    }
    assert!(
        rendered.contains("link: https://h.invalid/o?a=1"),
        "and the link the adapter printed is on screen to open: {rendered}"
    );
}

/// `o` opens the link the pane is showing, and says so when there is none: `htui` never invents a
/// URL, it forwards the one the adapter wrote.
#[tokio::test]
async fn o_sends_auth_open_with_the_last_url_and_is_refused_without_one() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "o"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec!["no link yet".to_owned()]);
    assert!(!asked_anything(&emitted), "{emitted:?}");

    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Url("https://h.invalid/first".to_owned())),
    );
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Url("https://h.invalid/second".to_owned())),
    );
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "o"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        matches!(
            &emitted[..],
            [Action::Store(StoreRequest::AuthOpen { url })] if url == "https://h.invalid/second"
        ),
        "the newest link is the one the pane is showing: {emitted:?}"
    );

    // The opener answers at its own `seq` and says only that it was spawned (D17).
    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Opened));
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("link opened"), "{rendered}");
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        "logging in\u{2026}",
        "and the flow is untouched by it"
    );
}

/// `x` stops a live login, and a refused `auth_open` does **not** stop it (hazard H-22): the two
/// `Failed`s in this codebase mean different things and the section renders them differently.
#[tokio::test]
async fn x_sends_auth_cancel_and_the_cell_reads_cancelling() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "x"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        matches!(&emitted[..], [Action::Store(StoreRequest::AuthCancel)]),
        "{emitted:?}"
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "loginable"),
        "cancelling\u{2026}"
    );

    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Cancelled));
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("login cancelled"), "{rendered}");
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        as_drawn("unauthenticated"),
        "and the row goes back to what the probe last said about it: {rendered}"
    );
}

/// A refused `auth_open` leaves the running flow exactly where it was (hazard H-22).
#[tokio::test]
async fn a_refused_open_keeps_the_pane_and_a_refused_start_clears_it() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "auth_open",
            message: "only http and https links are opened".to_owned(),
        },
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "loginable"),
        "logging in\u{2026}",
        "a refused open is one request, not the end of the flow"
    );

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "auth_choose",
            message: "no login is running".to_owned(),
        },
    );
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "loginable"),
        as_drawn("unauthenticated"),
        "a refused request of the flow itself leaves a state a second `a` can start from"
    );
}

/// Review L-3: a method list already in flight when `x` was pressed does not undo the cancel.
///
/// `begin_auth_cancel` from `Starting` synthesises a `Running { cancelling: true }` at the very
/// address the `AuthStart` is streaming to, so a `Methods` frame the adapter had already sent
/// arrives afterwards and — matching only on "is a flow running?" — replaced it with a chooser. The
/// cell then read `choose a method` for a login already on its way out, and offered the user a list
/// whose `Enter` would go into a flow that was being killed.
#[tokio::test]
async fn a_methods_frame_does_not_undo_a_cancel_pressed_while_starting() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD, OTHER_METHOD],
        )],
    );
    bench.key(&mut section, "a");
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "x"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        matches!(&emitted[..], [Action::Store(StoreRequest::AuthCancel)]),
        "`x` while starting asks the runtime to stop the flow: {emitted:?}"
    );

    // The frame that was already on its way when the key landed.
    bench.reply(&mut section, &methods_frame(true, 0));
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        "cancelling\u{2026}",
        "the cancel stands: {rendered}"
    );
    assert!(
        !rendered.contains("the first way in"),
        "and no chooser is offered for a flow that is being killed: {rendered}"
    );

    // The flow's own last frame is still what clears the state.
    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Cancelled));
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "loginable"),
        as_drawn("unauthenticated")
    );
}

/// Review L-4: the one refused `auth_choose` that leaves the login running.
///
/// `no login is running` and `this login has ended` both mean the flow is gone, and `Idle` is the
/// state a second `a` starts from. `a method was already chosen` means the opposite — the runtime
/// took the first choice and the adapter is live — and clearing the pane on it would leave a spawned
/// child, an open loopback listener and no `x` on screen to stop either.
#[tokio::test]
async fn a_second_choice_refused_by_a_live_flow_keeps_the_pane_and_the_others_clear_it() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "auth_choose",
            message: htui::agent_worker::AUTH_ALREADY_CHOSEN.to_owned(),
        },
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        "logging in\u{2026}",
        "the flow the runtime refused a second choice for is still running: {rendered}"
    );
    assert!(
        rendered.contains("x cancel"),
        "and the key that stops it is still on screen: {rendered}"
    );

    // The two that do mean the flow is gone still clear it.
    for message in ["no login is running", "this login has ended"] {
        let mut section = chooser_over(&bench, false, 0);
        bench.key(&mut section, "Enter");
        let _ = bench.drained();
        bench.reply(
            &mut section,
            &StoreReply::Failed {
                request: "auth_choose",
                message: message.to_owned(),
            },
        );
        assert_eq!(
            on_box_cell(&render_section(&section, &bench.ctx()), "loginable"),
            as_drawn("unauthenticated"),
            "`{message}` leaves a state a second `a` can start from"
        );
    }
}

/// `R-AGT-6` at the section: `Done` is not a status, it is the cue to read the registry again.
#[tokio::test]
async fn done_clears_the_state_notes_the_status_and_re_reads_the_registry() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, true, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Done {
            call: AuthCall::Authenticate(METHOD.to_owned()),
            status: ProbeStatus::Ready,
        }),
    );
    let emitted = bench.drained();
    assert!(
        matches!(&emitted[..], [Action::Store(StoreRequest::Agents)]),
        "the probe is the authority, so the section re-reads what the flow wrote: {emitted:?}"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("logged in: ready"), "{rendered}");
    assert!(
        rendered.contains("a authenticate"),
        "and the section is idle again: {rendered}"
    );

    // A logout is the same stream and the other sentence.
    let mut section = chooser_over(&bench, true, 0);
    for _ in 0..5 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "Enter");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Done {
            call: AuthCall::Logout,
            status: ProbeStatus::Unauthenticated,
        }),
    );
    assert!(
        render_section(&section, &bench.ctx()).contains("logged out: unauthenticated"),
        "a logout says which way it went"
    );
}

/// D5: the agent's own sentence, verbatim. It is the one line that says what the user has to do.
#[tokio::test]
async fn refused_shows_the_agents_sentence() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Refused {
            message: "the FIXTURE_KEY variable must be set where this server is launched from"
                .to_owned(),
        }),
    );
    let emitted = bench.drained();
    assert!(
        !asked_anything(&emitted),
        "a refusal wrote nothing, so there is nothing to re-read: {emitted:?}"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("FIXTURE_KEY variable must be set"),
        "the agent's own words: {rendered}"
    );
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        as_drawn("unauthenticated"),
        "and the flow is over"
    );
}

/// Hazard H-7: a `StoreReply::Agents` from a scope change mid-login must not clear the flow.
///
/// The one the browser round trip makes expensive: the adapter is spawned, a human is part-way
/// through an OAuth page, and a section that reset here would leave that child with no key on
/// screen to cancel it with.
#[tokio::test]
async fn an_agents_reply_during_a_login_does_not_clear_the_state() {
    let bench = SectionBench::new().await;
    let row = login_row("loginable", ProbeStatus::Unauthenticated, &[METHOD]);
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.key(&mut section, "a");
    bench.reply(&mut section, &methods_frame(false, 0));
    bench.key(&mut section, "Enter");
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Url("https://h.invalid/o".to_owned())),
    );
    let _ = bench.drained();

    bench.reply(&mut section, &StoreReply::Agents(vec![row]));
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        "logging in\u{2026}",
        "the flow is still running and still says so: {rendered}"
    );
    assert!(
        rendered.contains("link: https://h.invalid/o"),
        "its link is still on screen: {rendered}"
    );
    assert!(
        rendered.contains("o open link \u{b7} p paste redirect \u{b7} x cancel"),
        "and it can still be stopped: {rendered}"
    );
}

/// `r` and `i` while a login runs: the flow ends by re-probing and writing the same `agent_box`
/// row, and either of the other two would be racing it for that row (D19).
#[tokio::test]
async fn r_and_i_are_refused_while_a_login_runs() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD],
        )],
    );
    bench.key(&mut section, "a");
    let _ = bench.drained();

    for (chord, expected) in [
        ("r", "a login is running; probe afterwards"),
        ("i", "a login is running; install afterwards"),
    ] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        let emitted = bench.drained();
        assert_eq!(errors_of(&emitted), vec![expected.to_owned()]);
        assert!(!asked_anything(&emitted), "{emitted:?}");
    }

    // And from inside the chooser, where the same two keys are the pane's to swallow.
    bench.reply(&mut section, &methods_frame(false, 0));
    let _ = bench.drained();
    for (chord, expected) in [
        ("r", "a login is running; probe afterwards"),
        ("i", "a login is running; install afterwards"),
    ] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        assert_eq!(errors_of(&bench.drained()), vec![expected.to_owned()]);
    }
}

/// The two frames a flow can die with, each on the hint line and each leaving the section idle.
#[tokio::test]
async fn a_failed_or_idle_flow_leaves_a_notice_and_an_idle_section() {
    let bench = SectionBench::new().await;
    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Failed {
            message: "`/bin/sh`: No such file or directory".to_owned(),
        }),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("No such file or directory"), "{rendered}");
    assert!(rendered.contains("a authenticate"), "{rendered}");

    let mut section = chooser_over(&bench, false, 0);
    bench.key(&mut section, "Enter");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Idle {
            after: std::time::Duration::from_secs(600),
        }),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("no activity for 600s; login cancelled"),
        "the notice says what happened rather than implying the user did it: {rendered}"
    );
    assert!(rendered.contains("a authenticate"), "{rendered}");
}

// -------------------------------------------------------------------------------------------
// MOD-22 T3: p, the paste field and the delivery (plan D270-D272; blueprint §4)
// -------------------------------------------------------------------------------------------

/// The authorization code every paste here carries: a sentinel, so that its **absence** can be
/// asserted from every render and every emitted request (the credential rule, plan D273). No
/// assertion message in this block prints a pasted text or a request that carries one.
const CODE: &str = "CODE-SENTINEL-4f1c";

/// The `state` the link advertises and a good paste repeats.
const STATE: &str = "STATE-SENTINEL-9a2e";

/// A link advertising a loopback redirect. The port is fixed so the snapshot is deterministic;
/// nothing in this block connects to it.
const LOOPBACK_LINK: &str =
    "https://h.invalid/o?redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F&state=STATE-SENTINEL-9a2e";

/// The address the browser could not open: the advertised redirect, a code and the link's state.
const PASTE: &str = "http://127.0.0.1:39879/?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e";

/// The pane's line while a redirect is advertised and nothing is open (D270), written out as the
/// pin.
const REDIRECT_LINE: &str =
    "redirect: 127.0.0.1:39879 \u{b7} p pastes the address if the browser cannot reach it";

/// The prompt over the open field (D270).
const PASTE_PROMPT: &str = "paste the address the browser could not open (127.0.0.1:39879):";

/// The pane's line while a delivery is unanswered (D270).
const DELIVERING: &str = "delivering to 127.0.0.1:39879\u{2026}";

/// What `p` says while the login is being cancelled (blueprint D282). The section's constant is
/// private, so the pin is spelled here.
const PASTE_CANCELLING: &str = "this login is being cancelled; there is nothing to paste into";

/// The keys line while a login is spawning or running (D270).
const HINT_AUTH_RUNNING: &str = "o open link \u{b7} p paste redirect \u{b7} x cancel";

/// The keys line while the paste field is open (D270).
const HINT_PASTING: &str = "Enter sends \u{b7} Esc cancels";

/// A section past `a`, the method list and `Enter`, whose flow has printed `link`.
fn running_over(bench: &SectionBench, link: &str) -> AgentsSection {
    let mut section = chooser_over(bench, false, 0);
    bench.key(&mut section, "Enter");
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Url(link.to_owned())),
    );
    let _ = bench.drained();
    section
}

/// The same, with the paste field open over [`LOOPBACK_LINK`].
fn pasting_over(bench: &SectionBench) -> AgentsSection {
    let mut section = running_over(bench, LOOPBACK_LINK);
    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let _ = bench.drained();
    section
}

/// The same, with [`PASTE`] sent and its answer outstanding.
fn delivering_over(bench: &SectionBench) -> AgentsSection {
    let mut section = pasting_over(bench);
    typed(bench, &mut section, PASTE);
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    let _ = bench.drained();
    section
}

/// The one line of a render the field is drawn on, found by its `› ` prefix.
fn field_line(rendered: &str) -> Option<&str> {
    rendered.lines().find(|line| line.starts_with("\u{203a} "))
}

/// D270, D282: `p` opens a masked field, and from then on the section's letters, the shell's `q`,
/// `?` and digits and the tab's `h`/`l` are all characters. Only a `CONTROL` chord gets out.
#[tokio::test]
async fn p_with_a_loopback_redirect_opens_a_masked_field_that_captures_input() {
    let bench = SectionBench::new().await;
    let mut section = running_over(&bench, LOOPBACK_LINK);
    assert!(
        !section.captures_input(),
        "a running login alone captures nothing"
    );

    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        emitted.is_empty(),
        "opening the field asks nothing: {emitted:?}"
    );
    assert!(
        section.captures_input(),
        "the open field takes `h` and `l` from the tab"
    );

    for chord in ["q", "h", "l", "x", "o", "?", "1"] {
        assert_eq!(
            bench.key(&mut section, chord),
            Handled::Consumed,
            "`{chord}` is typed, not acted on"
        );
        let emitted = bench.drained();
        assert!(emitted.is_empty(), "`{chord}` asked nothing: {emitted:?}");
    }
    let rendered = render_section(&section, &bench.ctx());
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert_eq!(
        field.matches('\u{2022}').count(),
        7,
        "every key landed in the field: {rendered}"
    );
    assert!(field.contains("(7)"), "{rendered}");

    assert_eq!(
        bench.key(&mut section, "ctrl-c"),
        Handled::Pass,
        "a CONTROL chord still reaches the shell, so ctrl-c quits"
    );
    assert!(bench.drained().is_empty());
    assert!(section.captures_input(), "and the field is still open");
}

/// D282: with no loopback redirect advertised there is nothing to paste into, and `p` says so by
/// name, in `Running` and in `Starting` alike.
#[tokio::test]
async fn p_without_a_loopback_redirect_is_refused_by_name_and_opens_nothing() {
    let bench = SectionBench::new().await;
    let mut section = running_over(&bench, "https://h.invalid/o");
    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec![NO_LOOPBACK_REDIRECT.to_owned()]);
    assert!(!asked_anything(&emitted), "{emitted:?}");
    assert!(!section.captures_input());

    // `a` pressed, the method list not answered yet.
    let mut section = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD],
        )],
    );
    bench.key(&mut section, "a");
    let _ = bench.drained();
    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec![NO_LOOPBACK_REDIRECT.to_owned()]);
    assert!(!asked_anything(&emitted), "{emitted:?}");
    assert!(!section.captures_input());
}

/// `p` is `o`'s kind of key: bound while a login is in flight and a free letter otherwise.
#[tokio::test]
async fn p_is_not_bound_outside_a_running_login() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD],
        )],
    );
    let _ = bench.drained();
    assert_eq!(bench.key(&mut section, "p"), Handled::Pass);
    let emitted = bench.drained();
    assert!(emitted.is_empty(), "{emitted:?}");
    assert!(!section.captures_input());
}

/// D282: a login on its way out has nothing to paste into, and a second paste while the first is
/// unanswered would make the first's answer stale (plan D263), so both are refused by name.
#[tokio::test]
async fn p_while_cancelling_or_delivering_is_refused_by_name() {
    let bench = SectionBench::new().await;
    let mut section = running_over(&bench, LOOPBACK_LINK);
    bench.key(&mut section, "x");
    let _ = bench.drained();
    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec![PASTE_CANCELLING.to_owned()]);
    assert!(!asked_anything(&emitted), "{emitted:?}");
    assert!(!section.captures_input());

    let mut section = pasting_over(&bench);
    typed(&bench, &mut section, PASTE);
    bench.key(&mut section, "Enter");
    let sent = requests_of(&bench);
    assert_eq!(sent.len(), 1, "one request for one paste");
    assert!(
        matches!(&sent[0], StoreRequest::AuthDeliver { .. }),
        "and it is the delivery"
    );
    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec![DELIVERY_IN_FLIGHT.to_owned()]);
    assert!(
        !asked_anything(&emitted),
        "still exactly one AuthDeliver: {emitted:?}"
    );
    assert!(!section.captures_input());
}

/// D272: the field is masked. What is drawn is one dot per character and the count, and the prompt
/// names the advertised redirect rather than anything that was pasted.
#[tokio::test]
async fn the_field_draws_dots_and_a_count_and_never_the_text() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    typed(&bench, &mut section, PASTE);
    assert!(bench.drained().is_empty(), "typing asks nothing");

    let rendered = render_section(&section, &bench.ctx());
    let count = format!("({})", PASTE.chars().count());
    assert_eq!(count, "(73)");
    assert!(!rendered.contains(CODE), "the code is never drawn");
    assert!(
        !rendered.contains("127.0.0.1:39879/?"),
        "nor any of the pasted address"
    );
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert!(field.contains(&count), "{rendered}");
    assert!(field.contains('\u{2022}'), "{rendered}");
    assert!(rendered.contains(PASTE_PROMPT), "{rendered}");
    assert!(
        !rendered.contains(REDIRECT_LINE),
        "the prompt replaces the redirect line: {rendered}"
    );
}

/// D270: `Enter` sends the paste once, as a `RedirectUrl` that `validate` accepts, and closes the
/// field; the pane says where it is delivering to and nothing else.
#[tokio::test]
async fn enter_with_a_valid_paste_sends_auth_deliver_and_closes_the_field() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    typed(&bench, &mut section, PASTE);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    let emitted = bench.drained();
    let printed = format!("{emitted:?}");
    assert!(
        !printed.contains(CODE),
        "the request's Debug is redacted: {printed}"
    );
    let [Action::Store(StoreRequest::AuthDeliver { url })] = &emitted[..] else {
        panic!("exactly one AuthDeliver: {printed}");
    };
    let advertised =
        Advertised::from_auth_url(LOOPBACK_LINK).expect("the link advertises a loopback redirect");
    assert!(
        loopback::validate(url, &advertised).is_ok(),
        "what was sent is the paste the worker will accept"
    );

    let rendered = render_section(&section, &bench.ctx());
    assert!(!rendered.contains(CODE), "the code is never drawn");
    assert!(rendered.contains(DELIVERING), "{rendered}");
    assert!(!rendered.contains(PASTE_PROMPT), "{rendered}");
    assert!(!section.captures_input(), "the field closed with the send");
}

/// D270, review L-8: the pane's courtesy check reads the host and the port only. A paste for
/// another port or host is refused with `validate`'s own sentence, sends nothing, and leaves a
/// fresh empty field for the re-paste.
#[tokio::test]
async fn enter_with_a_wrong_port_or_host_is_refused_locally_and_sends_nothing() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    let cases = [
        (
            format!("http://127.0.0.1:39880/?code={CODE}&state={STATE}"),
            PasteError::WrongPort {
                pasted: 39880,
                advertised: 39879,
            },
        ),
        (
            format!("127.0.0.1/?code={CODE}&state={STATE}"),
            PasteError::WrongPort {
                pasted: 80,
                advertised: 39879,
            },
        ),
        (
            format!("http://192.168.1.5:39879/?code={CODE}&state={STATE}"),
            PasteError::WrongHost {
                advertised: "127.0.0.1:39879".to_owned(),
            },
        ),
    ];
    for (paste, refusal) in cases {
        typed(&bench, &mut section, &paste);
        assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
        let emitted = bench.drained();
        assert_eq!(errors_of(&emitted), vec![refusal.to_string()]);
        assert!(!asked_anything(&emitted), "{refusal:?} sends nothing");
        assert!(
            section.captures_input(),
            "{refusal:?}: the field is still open"
        );
        let rendered = render_section(&section, &bench.ctx());
        assert!(!rendered.contains(CODE), "the code is never drawn");
        let field =
            field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
        assert!(field.contains("(0)"), "{refusal:?}: and empty: {rendered}");
    }
}

/// Review L-8: everything past the host and the port is the worker's to judge — it runs the full
/// `validate` and is the authority — so a stale state or a missing code is sent, and its refusal
/// comes back as a failed `auth_deliver` that leaves the login and `p` as they were.
#[tokio::test]
async fn a_paste_for_the_right_host_and_port_is_sent_for_the_worker_to_judge() {
    let bench = SectionBench::new().await;
    for (paste, refusal) in [
        (
            format!("http://127.0.0.1:39879/?code={CODE}&state=STATE-EARLIER-0000"),
            PasteError::StaleState,
        ),
        (
            format!("http://127.0.0.1:39879/?state={STATE}"),
            PasteError::MissingCode,
        ),
    ] {
        let mut section = pasting_over(&bench);
        typed(&bench, &mut section, &paste);
        assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
        let emitted = bench.drained();
        assert!(errors_of(&emitted).is_empty(), "{refusal:?}: {emitted:?}");
        assert!(
            matches!(
                &emitted[..],
                [Action::Store(StoreRequest::AuthDeliver { .. })]
            ),
            "{refusal:?} is sent: {emitted:?}"
        );
        assert!(!section.captures_input(), "the field closed with the send");

        bench.reply(
            &mut section,
            &StoreReply::Failed {
                request: "auth_deliver",
                message: refusal.to_string(),
            },
        );
        // Review R2-L1: the worker's refusal reopens the field for the re-paste.
        let rendered = render_section(&section, &bench.ctx());
        assert!(rendered.contains(PASTE_PROMPT), "{rendered}");
        assert!(!rendered.contains(DELIVERING), "{rendered}");
        assert!(section.captures_input(), "the field is open again");
    }
}

/// MOD-22 review M-1: a bracketed paste into the open field lands whole, masked — dots and a
/// count, never a byte of it — and `Enter` sends it as typed text would be sent.
#[tokio::test]
async fn a_bracketed_paste_lands_in_the_open_paste_field_as_dots() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    assert_eq!(
        bench.paste(&mut section, &format!("{PASTE}\n")),
        Handled::Consumed
    );
    assert!(bench.drained().is_empty(), "a paste asks nothing");
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        !rendered.contains(CODE),
        "the code is never drawn: {rendered}"
    );
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert!(
        field.contains(&format!("({})", PASTE.chars().count())),
        "every pasted character counted, the newline dropped: {field}"
    );
    assert!(
        field
            .chars()
            .all(|c| "\u{203a}\u{2022}\u{2026} ()0123456789".contains(c)),
        "nothing but dots and the count: {field}"
    );

    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    let emitted = bench.drained();
    let [Action::Store(StoreRequest::AuthDeliver { url })] = &emitted[..] else {
        panic!("exactly one AuthDeliver");
    };
    let advertised = Advertised::from_auth_url(LOOPBACK_LINK).expect("a loopback redirect");
    assert!(loopback::validate(url, &advertised).is_ok());
}

/// MOD-22 review M-1, R2-L3: a paste that alone is past `PASTE_MAX` is refused whole by
/// `validate`'s own `TooLong`; one that is short enough itself but does not fit beside what was
/// already typed is refused with `PASTE_DOES_NOT_FIT`. Either way nothing goes in and the buffer
/// never reallocates.
#[tokio::test]
async fn a_bracketed_paste_that_does_not_fit_is_refused_by_the_right_sentence() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    assert_eq!(
        bench.paste(&mut section, &"a".repeat(PASTE_MAX + 1)),
        Handled::Consumed
    );
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec![PasteError::TooLong.to_string()]);
    let rendered = render_section(&section, &bench.ctx());
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert!(field.contains("(0)"), "nothing went in: {field}");

    typed(&bench, &mut section, "abc");
    let _ = bench.drained();
    assert_eq!(
        bench.paste(&mut section, &"a".repeat(PASTE_MAX - 1)),
        Handled::Consumed
    );
    let emitted = bench.drained();
    assert_eq!(errors_of(&emitted), vec![PASTE_DOES_NOT_FIT.to_owned()]);
    let rendered = render_section(&section, &bench.ctx());
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert!(field.contains("(3)"), "only what was typed: {field}");
}

/// Review R2-L3: a paste made before `p`, while the login is running with a redirect and nothing
/// is in flight, opens the masked field and lands in it — through the Settings tab too, which
/// otherwise offers a paste only to a section that is taking text.
#[tokio::test]
async fn a_bracketed_paste_before_p_opens_the_field_and_lands_in_it() {
    let bench = SectionBench::new().await;
    let mut section = running_over(&bench, LOOPBACK_LINK);
    assert!(!section.captures_input());
    assert_eq!(bench.paste(&mut section, PASTE), Handled::Consumed);
    assert!(bench.drained().is_empty(), "no refusal, no request");
    assert!(section.captures_input(), "the field is open");
    let rendered = render_section(&section, &bench.ctx());
    assert!(!rendered.contains(CODE), "{rendered}");
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert!(
        field.contains(&format!("({})", PASTE.chars().count())),
        "the paste is in it: {field}"
    );
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    assert!(matches!(
        &requests_of(&bench)[..],
        [StoreRequest::AuthDeliver { .. }]
    ));

    // Through the tab.
    let mut tab = SettingsTab::with_sections(vec![Box::new(running_over(&bench, LOOPBACK_LINK))]);
    assert_eq!(
        htui::ui::tabs::Tab::on_paste(&mut tab, PASTE, &mut bench.ctx()),
        Handled::Consumed,
        "the tab offers it to the running login"
    );
    assert!(bench.drained().is_empty());
}

/// Review R2-L3: with no redirect advertised, a delivery in flight or the login being cancelled,
/// a paste before `p` is dropped with the sentence `p` itself would give, and opens nothing.
#[tokio::test]
async fn a_bracketed_paste_before_p_is_refused_as_p_would_be() {
    let bench = SectionBench::new().await;
    let mut no_redirect = running_over(&bench, "https://h.invalid/o");
    let mut delivering = delivering_over(&bench);
    let mut cancelling = running_over(&bench, LOOPBACK_LINK);
    bench.key(&mut cancelling, "x");
    let _ = bench.drained();
    for (section, refusal) in [
        (&mut no_redirect, NO_LOOPBACK_REDIRECT),
        (&mut delivering, DELIVERY_IN_FLIGHT),
        (&mut cancelling, PASTE_CANCELLING),
    ] {
        assert_eq!(bench.paste(section, PASTE), Handled::Consumed);
        let emitted = bench.drained();
        assert_eq!(errors_of(&emitted), vec![refusal.to_owned()]);
        assert!(!asked_anything(&emitted), "{emitted:?}");
        assert!(!section.captures_input(), "{refusal}: nothing opened");
    }
}

/// D270: `Esc` closes the field and nothing else — the login, its link and its keys stay.
#[tokio::test]
async fn esc_closes_the_field() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    typed(&bench, &mut section, "http://127");

    assert_eq!(bench.key(&mut section, "Esc"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(!asked_anything(&emitted), "{emitted:?}");
    assert!(!section.captures_input());
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains(REDIRECT_LINE), "{rendered}");
    assert!(!rendered.contains(PASTE_PROMPT), "{rendered}");
    assert_eq!(on_box_cell(&rendered, "loginable"), "logging in\u{2026}");

    assert_eq!(bench.key(&mut section, "x"), Handled::Consumed);
    assert!(
        matches!(&requests_of(&bench)[..], [StoreRequest::AuthCancel]),
        "the login is still running and `x` still stops it"
    );
}

/// D268, D270: what the listener said lands on the note line, and the login it was said to keeps
/// running: the verdict is still the probe's (`R-AGT-6`).
#[tokio::test]
async fn a_delivered_frame_notes_what_the_listener_said_and_the_login_keeps_running() {
    let bench = SectionBench::new().await;
    let mut section = delivering_over(&bench);
    let reply = ListenerReply {
        target: "127.0.0.1:39879".to_owned(),
        status: 200,
        reason: "OK".to_owned(),
        said: Some("signed in".to_owned()),
        location_host: None,
    };

    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Delivered(reply.clone())),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        rendered.lines().last(),
        Some(reply.summary().as_str()),
        "{rendered}"
    );
    assert_eq!(on_box_cell(&rendered, "loginable"), "logging in\u{2026}");
    assert!(rendered.contains(REDIRECT_LINE), "{rendered}");
    assert!(!rendered.contains(DELIVERING), "{rendered}");

    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(emitted.is_empty(), "p opens again: {emitted:?}");
    assert!(section.captures_input());
}

/// D270: a refused or failed delivery is one request answered no, as a refused `auth_open` is. The
/// login, its link and its redirect stay, and `p` works again.
#[tokio::test]
async fn a_refused_auth_deliver_keeps_the_login_pane_and_its_link() {
    let bench = SectionBench::new().await;
    let mut section = delivering_over(&bench);

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "auth_deliver",
            message: DeliverError::NothingListening {
                target: "127.0.0.1:39879".to_owned(),
            }
            .to_string(),
        },
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains(&format!("link: {LOOPBACK_LINK}")),
        "{rendered}"
    );
    assert_eq!(on_box_cell(&rendered, "loginable"), "logging in\u{2026}");
    // Review R2-L1: the field is back for the re-paste; `Esc` closes it onto the redirect line,
    // and `p` opens it again.
    assert!(section.captures_input(), "{rendered}");
    assert_eq!(bench.key(&mut section, "Esc"), Handled::Consumed);
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains(REDIRECT_LINE), "{rendered}");

    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(emitted.is_empty(), "p opens again: {emitted:?}");
    assert!(section.captures_input());
}

/// Review R2-L1: the pane checks host and port only, so a stale state, a missing code or a wrong
/// path is refused by the worker after the field has closed. That refusal reopens a fresh, empty,
/// masked field for the re-paste — the sentence itself is the shell's, from the same reply — as
/// long as the login is still running with a redirect and is not being cancelled.
#[tokio::test]
async fn a_refused_delivery_reopens_an_empty_masked_field() {
    let bench = SectionBench::new().await;
    for refusal in [
        PasteError::StaleState.to_string(),
        PasteError::MissingCode.to_string(),
        DeliverError::NothingListening {
            target: "127.0.0.1:39879".to_owned(),
        }
        .to_string(),
    ] {
        let mut section = delivering_over(&bench);
        assert!(!section.captures_input());
        bench.reply(
            &mut section,
            &StoreReply::Failed {
                request: "auth_deliver",
                message: refusal.clone(),
            },
        );
        assert!(
            section.captures_input(),
            "{refusal}: the field is open again"
        );
        let rendered = render_section(&section, &bench.ctx());
        assert!(rendered.contains(PASTE_PROMPT), "{refusal}: {rendered}");
        assert!(!rendered.contains(DELIVERING), "{refusal}: {rendered}");
        let field =
            field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
        assert!(field.contains("(0)"), "{refusal}: fresh and empty: {field}");
        assert!(bench.drained().is_empty(), "{refusal}: and it asks nothing");
    }

    // A login on its way out has nothing to paste into.
    let mut section = delivering_over(&bench);
    bench.key(&mut section, "x");
    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Cancelling));
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "auth_deliver",
            message: PasteError::StaleState.to_string(),
        },
    );
    assert!(!section.captures_input(), "cancelling: no field");
}

/// The refusal of an `auth_deliver` that means no login is held at all clears the pane, as it does
/// for `auth_choose` (review L-4's rule): there is no login left to paste into or to cancel.
#[tokio::test]
async fn a_deliver_refused_because_no_login_is_running_clears_the_pane() {
    let bench = SectionBench::new().await;
    let mut section = delivering_over(&bench);
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "auth_deliver",
            message: "no login is running".to_owned(),
        },
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        as_drawn("unauthenticated"),
        "a state a second `a` can start from"
    );
    assert!(!rendered.contains(DELIVERING), "{rendered}");
    assert!(!rendered.contains("link: "), "{rendered}");
}

/// Review L-2: `this login has ended` is a delivery the flow's end overtook. The flow is on its way
/// out — draining, re-probing — and the worker guarantees its own terminal frame follows, so the
/// pane only stops waiting for the delivery and stays until that frame: `cancelling…` after an `x`,
/// `logging in…` otherwise. An `a` meanwhile is the pane's own "already running" rather than an
/// `AuthStart` the worker would refuse while the old flow holds its claim, and that would make the
/// flow's own last frame stale.
#[tokio::test]
async fn a_deliver_overtaken_by_the_logins_end_waits_for_the_flows_last_frame() {
    let bench = SectionBench::new().await;
    let ended = || StoreReply::Failed {
        request: "auth_deliver",
        message: "this login has ended".to_owned(),
    };

    // `x` during a delivery.
    let mut section = delivering_over(&bench);
    assert_eq!(bench.key(&mut section, "x"), Handled::Consumed);
    let _ = bench.drained();
    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Cancelling));
    bench.reply(&mut section, &ended());
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        "cancelling\u{2026}",
        "the pane is still the flow's until its last frame: {rendered}"
    );
    assert!(!rendered.contains(DELIVERING), "{rendered}");

    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["a login is already running".to_owned()]
    );
    assert!(!asked_anything(&emitted), "no AuthStart: {emitted:?}");

    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Cancelled));
    let _ = bench.drained();
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "loginable"),
        as_drawn("unauthenticated")
    );
    assert_eq!(bench.key(&mut section, "a"), Handled::Consumed);
    assert!(
        matches!(&requests_of(&bench)[..], [StoreRequest::AuthStart { .. }]),
        "after the flow's own last frame, `a` starts the next login"
    );

    // No `x`: the login completed under the delivery, and its re-probe is still to come.
    let mut section = delivering_over(&bench);
    bench.reply(&mut section, &ended());
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(on_box_cell(&rendered, "loginable"), "logging in\u{2026}");
    assert!(!rendered.contains(DELIVERING), "{rendered}");
    assert!(rendered.contains(REDIRECT_LINE), "{rendered}");
    bench.reply(
        &mut section,
        &StoreReply::Auth(AuthFrame::Done {
            call: AuthCall::Authenticate(METHOD.to_owned()),
            status: ProbeStatus::Ready,
        }),
    );
    assert!(
        matches!(&requests_of(&bench)[..], [StoreRequest::Agents]),
        "the flow's own Done re-reads the row"
    );
}

/// D282: the runtime acknowledging a cancel closes an open field; there is nothing left to send it
/// to.
#[tokio::test]
async fn a_cancelling_frame_closes_an_open_field() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    typed(&bench, &mut section, "http://127");

    bench.reply(&mut section, &StoreReply::Auth(AuthFrame::Cancelling));
    assert!(!section.captures_input());
    let rendered = render_section(&section, &bench.ctx());
    assert!(!rendered.contains(PASTE_PROMPT), "{rendered}");
    assert_eq!(on_box_cell(&rendered, "loginable"), "cancelling\u{2026}");
}

/// D270: every way a flow ends drops the field with the state, so its buffer is wiped and the
/// section's keys are its own again.
#[tokio::test]
async fn a_flow_that_ends_while_the_field_is_open_closes_it_and_releases_input() {
    let bench = SectionBench::new().await;
    let ends = [
        AuthFrame::Done {
            call: AuthCall::Authenticate(METHOD.to_owned()),
            status: ProbeStatus::Ready,
        },
        AuthFrame::Refused {
            message: "set the key first".to_owned(),
        },
        AuthFrame::Cancelled,
        AuthFrame::Idle {
            after: std::time::Duration::from_secs(600),
        },
        AuthFrame::Failed {
            message: "the adapter exited".to_owned(),
        },
    ];
    for end in ends {
        let mut section = pasting_over(&bench);
        typed(&bench, &mut section, "http://127");
        bench.reply(&mut section, &StoreReply::Auth(end.clone()));
        let _ = bench.drained();
        assert!(!section.captures_input(), "{end:?} releases input");
        let rendered = render_section(&section, &bench.ctx());
        assert!(!rendered.contains(PASTE_PROMPT), "{end:?}: {rendered}");
        assert!(field_line(&rendered).is_none(), "{end:?}: {rendered}");
    }
}

/// D270: the running hint names `p`, and the open field's hint names the two keys it answers.
#[tokio::test]
async fn the_running_hint_offers_p_and_the_pasting_hint_offers_enter_and_esc() {
    let bench = SectionBench::new().await;
    let mut section = running_over(&bench, LOOPBACK_LINK);
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains(HINT_AUTH_RUNNING), "{rendered}");

    bench.key(&mut section, "p");
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains(HINT_PASTING), "{rendered}");
    assert!(!rendered.contains(HINT_AUTH_RUNNING), "{rendered}");
}

/// The login pane with the paste field open, half a paste in: the link, the prompt naming the
/// advertised redirect, and dots with a count — never a character that was typed.
#[tokio::test]
async fn the_paste_field_renders_under_the_link() {
    let bench = SectionBench::new().await;
    let mut section = pasting_over(&bench);
    typed(&bench, &mut section, "http://127.0.0.1:39879/?code=");
    let rendered = render_section(&section, &bench.ctx());
    assert!(!rendered.contains(CODE), "the code is never drawn");
    assert!(
        !rendered.contains("39879/?code="),
        "nor anything that was typed"
    );
    let field = field_line(&rendered).unwrap_or_else(|| panic!("the field is drawn: {rendered}"));
    assert_eq!(field.matches('\u{2022}').count(), 29, "{rendered}");
    assert!(field.contains("(29)"), "{rendered}");
    insta::assert_snapshot!("agents_paste_redirect", rendered);
}

// -------------------------------------------------------------------------------------------
// MOD-23 T3: the registry editor (plan D230-D232, D244, D245; blueprint §5)
// -------------------------------------------------------------------------------------------

/// The label column of the form: `default model` and `enabled (y/n)` are both 13 wide (D231).
const LABEL_WIDTH: usize = 13;

/// The idle keys line (D245), written out rather than read from the section: this is the pin.
const IDLE_KEYS: &str = "j/k select \u{b7} n new \u{b7} e edit \u{b7} m paths \u{b7} t this box \u{b7} r probe \u{b7} i install \u{b7} a authenticate";

/// The idle note line, the one limit of this section that is not a key (MOD-2 D73).
const IDLE_NOTE: &str = "quota latches per chat, r cannot refresh it";

/// What an edit that changed nothing says (blueprint §5.1).
const UNCHANGED: &str = "nothing changed; nothing was written";

/// `settings/mod.rs`'s `CHANGED_ELSEWHERE`, which is `pub(crate)`: the section's own sentence for
/// a spent token under an open editor.
const CHANGED_ELSEWHERE: &str = "changed elsewhere since you opened it \u{2014} reloaded; Enter retries against the current row";

/// `settings/mod.rs`'s `DELETED_ELSEWHERE`, for the same reason.
const DELETED_ELSEWHERE: &str = "deleted elsewhere \u{2014} the editor was closed";

/// Review M-1: a `Stale` whose re-read changed fields the user had changed too names them.
const CHANGED_ON_BOTH_SIDES: &str = "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: command, models";

/// A valid create form, in `FIELD_LABELS` order: name, transport, command, args, models, default
/// model, billing, enabled.
const VALID_CREATE: [&str; 8] = [
    "gamma",
    "acp",
    "/opt/demo/bin/agent",
    "--flag 'a b'",
    "m1, m2",
    "m2",
    "subscription",
    "y",
];

/// Types `text` into a section one character at a time, as a terminal delivers it.
fn typed(bench: &SectionBench, section: &mut AgentsSection, text: &str) {
    for c in text.chars() {
        let mut ctx = bench.ctx();
        section.on_key(KeyEvent::from(KeyCode::Char(c)), &mut ctx);
    }
}

/// Presses `chord` `times` times.
fn pressed(bench: &SectionBench, section: &mut AgentsSection, chord: &str, times: usize) {
    for _ in 0..times {
        bench.key(section, chord);
    }
}

/// Fills an open form from its focused field onwards: each value replaces the field's text, and
/// `Tab` moves on after every value but the last, so focus ends on the last field filled.
fn filled(bench: &SectionBench, section: &mut AgentsSection, values: &[&str]) {
    for (index, value) in values.iter().enumerate() {
        pressed(bench, section, "backspace", 64);
        typed(bench, section, value);
        if index + 1 < values.len() {
            bench.key(section, "tab");
        }
    }
}

/// The text of one form field as drawn, or `None` when the form has no such field.
fn field_of(rendered: &str, label: &str) -> Option<String> {
    let prefix = format!("{label:<width$}: ", width = LABEL_WIDTH);
    rendered
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(|rest| rest.trim_end().to_owned())
}

/// Whether the field labelled `label` holds the focus: its label is the accented one.
fn focused_on(section: &AgentsSection, bench: &SectionBench, label: &str) -> bool {
    let prefix = format!("{label:<width$}: ", width = LABEL_WIDTH);
    accented_lines(section, &bench.ctx())
        .iter()
        // Trimmed: a field holding only spaces draws its label and nothing after the colon.
        .any(|line| line.starts_with(prefix.trim_end()))
}

/// The section's last line: the note (D245).
fn note_line(rendered: &str) -> String {
    rendered
        .lines()
        .last()
        .expect("the section always draws a note line")
        .to_owned()
}

/// A registry row carrying a model list, a default and a stamp of its own, so an edit has
/// something to prefill and a token to send.
fn editable_row(name: &str) -> AgentSummary {
    let mut row = registry_row(name, false);
    row.agent.models = vec!["m1".to_owned(), "m2".to_owned()];
    row.agent.default_model = Some("m1".to_owned());
    row.agent.launch["args"] = json!(["--flag", "a b"]);
    row.agent.updated_at = htui_core::fixtures::demo_at(1, 0);
    row
}

/// The `AgentWritten` reply a write is answered with.
fn written(agents: Vec<AgentSummary>, outcome: AgentWrite) -> StoreReply {
    StoreReply::AgentWritten { agents, outcome }
}

/// `n` opens the create form, and from then on the section takes every printable key (F-11):
/// `l` and `h` are letters, not section cycling, and so is `q`. A `CONTROL` chord still passes, so
/// `ctrl-c` quits.
#[tokio::test]
async fn n_opens_the_create_form_and_captures_input() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![registry_row("alpha", false), registry_row("beta", false)],
    );
    let _ = bench.drained();
    assert!(!section.captures_input(), "browsing captures nothing");

    assert_eq!(bench.key(&mut section, "n"), Handled::Consumed);
    assert!(
        section.captures_input(),
        "an open form takes every printable key"
    );
    for chord in ["l", "h", "q"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed, "{chord}");
    }
    assert_eq!(
        bench.key(&mut section, "ctrl-c"),
        Handled::Pass,
        "a CONTROL chord passes, so the shell still quits"
    );

    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        field_of(&rendered, "name").as_deref(),
        Some("lhq"),
        "{rendered}"
    );
    assert_eq!(field_of(&rendered, "transport").as_deref(), Some("acp"));
    assert_eq!(
        field_of(&rendered, "billing").as_deref(),
        Some("subscription")
    );
    assert_eq!(field_of(&rendered, "enabled (y/n)").as_deref(), Some("y"));
    assert!(
        rendered.contains("new agent \u{b7} settings from alpha"),
        "the header names the highlighted row the settings come from (OQ-5): {rendered}"
    );
    assert!(
        requests_of(&bench).is_empty(),
        "opening a form asks for nothing"
    );
}

/// F-11 through the whole tab: with the form open, `SettingsTab` hands `l` and `h` to the section
/// instead of cycling, and `q` does not quit. Closing the form gives the tab its keys back.
#[tokio::test]
async fn the_settings_tab_gives_an_open_form_every_letter() {
    let mut harness = Harness::demo().with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(ProbeSection),
    ])));
    harness.settle().await;

    harness.key("n");
    harness.key("l");
    harness.key("h");
    harness.key("q");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("box profile arrives with MOD-7"),
        "`l` did not cycle to the next section: {frame}"
    );
    assert!(
        frame.contains("lhq"),
        "the letters landed in the form: {frame}"
    );
    assert!(!harness.app().should_quit, "`q` is a letter in the form");

    harness.key("esc");
    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("box profile arrives with MOD-7"),
        "with the form closed `l` cycles again: {frame}"
    );
}

/// `Enter` sends exactly one `CreateAgent`, with the parsed draft and the highlighted row as the
/// settings source; a second `Enter` before the reply is refused (F-20).
#[tokio::test]
async fn enter_on_the_create_form_sends_create_agent_with_the_parsed_draft_and_the_source_row() {
    let bench = SectionBench::new().await;
    let beta = registry_row("beta", false);
    let beta_id = beta.agent.id;
    let mut section = section_over(&bench, vec![registry_row("alpha", false), beta]);
    bench.key(&mut section, "j");
    bench.key(&mut section, "n");
    filled(&bench, &mut section, &VALID_CREATE);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "enter"), Handled::Consumed);
    let requests = requests_of(&bench);
    assert_eq!(requests.len(), 1, "{requests:?}");
    match &requests[0] {
        StoreRequest::CreateAgent {
            name,
            draft,
            settings_from,
        } => {
            assert_eq!(name, "gamma");
            assert_eq!(
                *draft,
                AgentDraft {
                    transport: Transport::Acp,
                    command: "/opt/demo/bin/agent".to_owned(),
                    args: vec!["--flag".to_owned(), "a b".to_owned()],
                    models: vec!["m1".to_owned(), "m2".to_owned()],
                    default_model: Some("m2".to_owned()),
                    billing: Billing::Subscription,
                    enabled: true,
                }
            );
            assert_eq!(*settings_from, Some(beta_id), "the highlighted row");
        }
        other => panic!("expected CreateAgent, got {other:?}"),
    }

    assert_eq!(bench.key(&mut section, "enter"), Handled::Consumed);
    let emitted = bench.drained();
    assert!(
        !asked_anything(&emitted),
        "one write at a time: {emitted:?}"
    );
    assert_eq!(
        errors_of(&emitted),
        vec!["`create_agent` is still in flight".to_owned()]
    );
}

/// Every draft rule refuses locally, by field, before a request is spent: the note line starts
/// with the field and the focus moves to it.
#[tokio::test]
async fn a_bad_field_is_refused_locally_by_name_and_sends_nothing() {
    let bench = SectionBench::new().await;
    for (index, bad, field) in [
        (0, "Bad Name", "name"),
        (0, "alpha", "name"),
        (1, "ssh", "transport"),
        (2, "  ", "command"),
        (3, "'open", "args"),
        (4, "m1, m1", "models"),
        (5, "m9", "default model"),
        (6, "free", "billing"),
        (7, "maybe", "enabled (y/n)"),
    ] {
        let mut section = section_over(&bench, vec![registry_row("alpha", false)]);
        bench.key(&mut section, "n");
        let mut values = VALID_CREATE;
        values[index] = bad;
        filled(&bench, &mut section, &values);
        let _ = bench.drained();

        assert_eq!(bench.key(&mut section, "enter"), Handled::Consumed);
        let emitted = bench.drained();
        assert!(
            !asked_anything(&emitted),
            "`{bad}` sends nothing: {emitted:?}"
        );
        let rendered = render_section(&section, &bench.ctx());
        let note = note_line(&rendered);
        assert!(
            note.starts_with(&format!("`{field}`: ")),
            "`{bad}` is refused by `{field}`: `{note}`"
        );
        assert!(
            section.captures_input(),
            "the form stays open over its text"
        );
        assert!(
            focused_on(&section, &bench, field),
            "the focus moves to `{field}`:\n{rendered}"
        );
    }
}

/// `e` prefills the highlighted row: no `name` field, `args` as shell words, `env` nowhere on
/// screen. `Enter` after one edit sends the row's own token.
#[tokio::test]
async fn e_prefills_the_highlighted_row_without_a_name_field_and_sends_its_token() {
    let bench = SectionBench::new().await;
    let mut row = editable_row("alpha");
    row.agent.launch["env"] = json!({ "TOKEN": "s3cret-value" });
    let (agent_id, expected) = (row.agent.id, row.agent.updated_at);
    let mut section = section_over(&bench, vec![row]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    assert!(section.captures_input());
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        field_of(&rendered, "name"),
        None,
        "a name is never edited (D233)"
    );
    for (label, text) in [
        ("transport", "acp"),
        ("command", "${demo_server}"),
        ("args", "--flag 'a b'"),
        ("models", "m1, m2"),
        ("default model", "m1"),
        ("billing", "subscription"),
        ("enabled (y/n)", "y"),
    ] {
        assert_eq!(
            field_of(&rendered, label).as_deref(),
            Some(text),
            "`{label}`:\n{rendered}"
        );
    }
    assert!(rendered.contains("edit alpha"), "{rendered}");
    assert!(
        !rendered.contains("s3cret-value") && !rendered.contains("TOKEN"),
        "`env` is never shown (D235):\n{rendered}"
    );

    pressed(&bench, &mut section, "tab", 3);
    typed(&bench, &mut section, ", m3");
    bench.key(&mut section, "enter");
    let requests = requests_of(&bench);
    assert_eq!(requests.len(), 1, "{requests:?}");
    match &requests[0] {
        StoreRequest::EditAgent {
            agent_id: asked,
            expected: token,
            draft,
        } => {
            assert_eq!(*asked, agent_id);
            assert_eq!(*token, expected, "the token the registry read answered");
            assert_eq!(draft.models, ["m1", "m2", "m3"]);
            assert_eq!(draft.args, ["--flag", "a b"], "the prefill round-trips");
        }
        other => panic!("expected EditAgent, got {other:?}"),
    }
}

/// Review L-4: `e` over a row whose stored model name holds a comma opens nothing and says why:
/// the form's comma-separated `models` would split the name on save.
#[tokio::test]
async fn e_refuses_a_row_whose_model_name_holds_a_comma() {
    let bench = SectionBench::new().await;
    let mut row = editable_row("alpha");
    row.agent.models = vec!["m1".to_owned(), "vendor,model".to_owned()];
    let mut section = section_over(&bench, vec![row]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["`models`: a stored model name contains a comma; edit it with SQL".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");
    assert!(!section.captures_input(), "no form opened");
}

/// Plan D239's "unchanged closes": `e` then `Enter` writes nothing.
#[tokio::test]
async fn an_unchanged_edit_closes_without_a_write() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![editable_row("alpha")]);
    bench.key(&mut section, "e");
    let _ = bench.drained();

    bench.key(&mut section, "enter");
    assert!(requests_of(&bench).is_empty(), "nothing to write");
    assert!(!section.captures_input(), "the form closed");
    assert_eq!(
        note_line(&render_section(&section, &bench.ctx())),
        UNCHANGED
    );
}

/// `Edited` closes the form; the notice adds the re-probe clause only when `transport`, `command`
/// or `args` changed (D246).
#[tokio::test]
async fn agent_written_edited_closes_the_form_and_says_re_probe_when_launch_changed() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row.clone()]);

    bench.key(&mut section, "e");
    pressed(&bench, &mut section, "tab", 2);
    typed(&bench, &mut section, " --more");
    bench.key(&mut section, "enter");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &written(
            vec![row.clone()],
            AgentWrite::Edited {
                id: agent_id,
                name: "alpha".to_owned(),
            },
        ),
    );
    assert!(!section.captures_input(), "`Edited` closes the form");
    let note = note_line(&render_section(&section, &bench.ctx()));
    assert!(note.contains("saved `alpha`"), "{note}");
    assert!(
        note.contains("r probes now"),
        "an args edit re-probes: {note}"
    );

    bench.key(&mut section, "e");
    pressed(&bench, &mut section, "tab", 3);
    typed(&bench, &mut section, ", mz");
    bench.key(&mut section, "enter");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &written(
            vec![row],
            AgentWrite::Edited {
                id: agent_id,
                name: "alpha".to_owned(),
            },
        ),
    );
    let note = note_line(&render_section(&section, &bench.ctx()));
    assert!(note.contains("saved `alpha`"), "{note}");
    assert!(
        !note.contains("r probes now"),
        "a models edit changes nothing a probe checked: {note}"
    );
}

/// `Created` closes the form and moves the cursor onto the new row by id, wherever the re-read
/// sorted it (F-14).
#[tokio::test]
async fn agent_written_created_selects_the_new_row() {
    let bench = SectionBench::new().await;
    let (alpha, gamma) = (registry_row("alpha", false), registry_row("gamma", false));
    let mut section = section_over(&bench, vec![alpha.clone(), gamma.clone()]);
    bench.key(&mut section, "n");
    let mut values = VALID_CREATE;
    values[0] = "beta";
    filled(&bench, &mut section, &values);
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let beta = registry_row("beta", false);
    let beta_id = beta.agent.id;
    bench.reply(
        &mut section,
        &written(
            vec![alpha, beta, gamma],
            AgentWrite::Created {
                id: beta_id,
                name: "beta".to_owned(),
            },
        ),
    );
    assert!(!section.captures_input(), "`Created` closes the form");
    assert!(
        accented_lines(&section, &bench.ctx())[0].starts_with("beta"),
        "the cursor is on the new row"
    );
    let note = note_line(&render_section(&section, &bench.ctx()));
    assert!(note.contains("created `beta`"), "{note}");
    assert!(
        !note.contains("settings.cli"),
        "an acp row needs no cli block: {note}"
    );
}

/// R-9 (F-13): a new `cli` row with no `settings.cli` block cannot chat yet, and the notice says so
/// from the row's data alone.
#[tokio::test]
async fn a_created_cli_row_without_a_cli_block_says_so() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("alpha", false)]);
    bench.key(&mut section, "n");
    let mut values = VALID_CREATE;
    values[1] = "cli";
    filled(&bench, &mut section, &values);
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let mut gamma = registry_row("gamma", false);
    gamma.agent.transport = Transport::Cli;
    let gamma_id = gamma.agent.id;
    bench.reply(
        &mut section,
        &written(
            vec![registry_row("alpha", false), gamma],
            AgentWrite::Created {
                id: gamma_id,
                name: "gamma".to_owned(),
            },
        ),
    );
    let note = note_line(&render_section_at(&section, &bench.ctx(), 200));
    assert!(note.contains("created `gamma`"), "{note}");
    assert!(
        note.contains("a `cli` row needs a settings.cli block to chat"),
        "{note}"
    );
}

/// `Stale` keeps the form and its text, takes the new token, and says so; the next `Enter` sends
/// the new token.
#[tokio::test]
async fn agent_written_stale_keeps_the_text_and_takes_the_new_token() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.key(&mut section, "e");
    pressed(&bench, &mut section, "tab", 3);
    typed(&bench, &mut section, ", m9");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let mut moved = row;
    moved.agent.updated_at = htui_core::fixtures::demo_at(2, 0);
    bench.reply(
        &mut section,
        &written(vec![moved], AgentWrite::Stale { id: agent_id }),
    );
    assert!(section.captures_input(), "the form stays open");
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        field_of(&rendered, "models").as_deref(),
        Some("m1, m2, m9"),
        "the typed text is kept"
    );
    assert_eq!(note_line(&rendered), CHANGED_ELSEWHERE);

    bench.key(&mut section, "enter");
    let requests = requests_of(&bench);
    assert!(
        matches!(
            requests.as_slice(),
            [StoreRequest::EditAgent { expected, .. }]
                if *expected == htui_core::fixtures::demo_at(2, 0)
        ),
        "the retry carries the new token: {requests:?}"
    );
}

/// Review M-1: a `Stale` reply rebases every field the user left alone onto the re-read row, so
/// the retry carries another writer's change instead of silently reverting it; the fields the user
/// changed keep their text.
#[tokio::test]
async fn agent_written_stale_rebases_untouched_fields() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.key(&mut section, "e");
    pressed(&bench, &mut section, "tab", 3);
    typed(&bench, &mut section, ", m9");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let mut moved = row;
    moved.agent.updated_at = htui_core::fixtures::demo_at(2, 0);
    moved.agent.launch["command"] = json!("/opt/remote/bin/agent");
    bench.reply(
        &mut section,
        &written(vec![moved], AgentWrite::Stale { id: agent_id }),
    );
    assert!(section.captures_input(), "the form stays open");
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        field_of(&rendered, "command").as_deref(),
        Some("/opt/remote/bin/agent"),
        "an untouched field takes the other writer's value:\n{rendered}"
    );
    assert_eq!(
        field_of(&rendered, "models").as_deref(),
        Some("m1, m2, m9"),
        "the user's edit is kept"
    );
    assert_eq!(
        note_line(&rendered),
        CHANGED_ELSEWHERE,
        "nothing clashed, so the plain sentence"
    );

    bench.key(&mut section, "enter");
    let requests = requests_of(&bench);
    match requests.as_slice() {
        [
            StoreRequest::EditAgent {
                expected, draft, ..
            },
        ] => {
            assert_eq!(*expected, htui_core::fixtures::demo_at(2, 0));
            assert_eq!(
                draft.command, "/opt/remote/bin/agent",
                "the retry carries the other writer's command"
            );
            assert_eq!(draft.models, ["m1", "m2", "m9"], "and the user's models");
            assert_eq!(draft.args, ["--flag", "a b"]);
        }
        other => panic!("expected one EditAgent, got {other:?}"),
    }
}

/// Review M-1: a field changed on both sides keeps the user's text, and the notice names it (and
/// only by label, never by value); a field only the other writer changed is rebased silently.
#[tokio::test]
async fn agent_written_stale_names_the_fields_changed_on_both_sides() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.key(&mut section, "e");
    bench.key(&mut section, "tab");
    filled(&bench, &mut section, &["/opt/mine/bin/agent"]);
    pressed(&bench, &mut section, "tab", 2);
    typed(&bench, &mut section, ", m9");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let mut moved = row;
    moved.agent.updated_at = htui_core::fixtures::demo_at(2, 0);
    moved.agent.launch["command"] = json!("/opt/theirs/bin/agent");
    moved.agent.launch["args"] = json!(["--theirs"]);
    moved.agent.models = vec!["m1".to_owned(), "m7".to_owned()];
    bench.reply(
        &mut section,
        &written(vec![moved], AgentWrite::Stale { id: agent_id }),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        field_of(&rendered, "command").as_deref(),
        Some("/opt/mine/bin/agent")
    );
    assert_eq!(field_of(&rendered, "models").as_deref(), Some("m1, m2, m9"));
    assert_eq!(
        field_of(&rendered, "args").as_deref(),
        Some("--theirs"),
        "a field only the other writer changed is rebased"
    );
    let note = note_line(&rendered);
    assert_eq!(note, CHANGED_ON_BOTH_SIDES, "{rendered}");
    assert!(
        !note.contains("/opt/") && !note.contains("m7") && !note.contains("m9"),
        "labels only, never a value: {note}"
    );
    assert!(
        SECTION_BORDERED as usize >= CHANGED_ON_BOTH_SIDES.chars().count(),
        "the notice fits the bordered pane"
    );
}

/// Opens the edit form over `row`, fills its seven fields with `mine`, sends it, and answers it
/// `Stale` with `theirs` as the re-read: the note line the rebase leaves.
fn stale_note(
    bench: &SectionBench,
    row: &AgentSummary,
    mine: &[&str; 7],
    theirs: AgentSummary,
) -> String {
    let mut section = section_over(bench, vec![row.clone()]);
    bench.key(&mut section, "e");
    filled(bench, &mut section, mine);
    bench.key(&mut section, "enter");
    assert!(
        matches!(
            requests_of(bench).as_slice(),
            [StoreRequest::EditAgent { .. }]
        ),
        "the edit is sent"
    );
    bench.reply(
        &mut section,
        &written(vec![theirs], AgentWrite::Stale { id: row.agent.id }),
    );
    note_line(&render_section_at(&section, &bench.ctx(), 200))
}

/// MOD-23 re-review Low-2: three clashing fields whose labels do not all fit are listed as many
/// as fit, then counted, and the notice fits the bordered 98.
#[tokio::test]
async fn a_stale_with_three_clashes_counts_what_does_not_fit() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let mut theirs = row.clone();
    theirs.agent.updated_at = htui_core::fixtures::demo_at(2, 0);
    theirs.agent.launch["command"] = json!("/opt/theirs/bin/agent");
    theirs.agent.models = vec!["m1".to_owned(), "m7".to_owned()];
    theirs.agent.default_model = Some("m7".to_owned());
    let note = stale_note(
        &bench,
        &row,
        &[
            "acp",
            "/opt/mine/bin/agent",
            "--flag 'a b'",
            "m1, m2, m9",
            "m2",
            "subscription",
            "y",
        ],
        theirs,
    );
    assert_eq!(
        note,
        "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: command, \
         models +1 more"
    );
    assert!(note.chars().count() <= SECTION_BORDERED as usize, "{note}");
}

/// MOD-23 re-review Low-2: all seven fields clashing still fit, with the count of the rest.
#[tokio::test]
async fn a_stale_with_all_seven_clashes_fits_and_counts_the_rest() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let mut theirs = row.clone();
    theirs.agent.updated_at = htui_core::fixtures::demo_at(2, 0);
    theirs.agent.transport = Transport::Cli;
    theirs.agent.launch["command"] = json!("/opt/theirs/bin/agent");
    theirs.agent.launch["args"] = json!(["--theirs"]);
    theirs.agent.models = vec!["m1".to_owned(), "m7".to_owned()];
    theirs.agent.default_model = Some("m7".to_owned());
    theirs.agent.billing = Billing::PerToken;
    theirs.agent.enabled = false;
    // Each of the user's texts differs from the prefill and from the re-read's, `CLI`, `PER_TOKEN`
    // and `N` included: the comparison is of the text, as the form holds it.
    let note = stale_note(
        &bench,
        &row,
        &[
            "CLI",
            "/opt/mine/bin/agent",
            "--mine",
            "m1, m2, m9",
            "m9",
            "PER_TOKEN",
            "N",
        ],
        theirs,
    );
    assert_eq!(
        note,
        "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: \
         transport, command +5 more"
    );
    assert!(note.chars().count() <= SECTION_BORDERED as usize, "{note}");
}

/// `Gone` closes the form with the section's deleted sentence.
#[tokio::test]
async fn agent_written_gone_closes_with_deleted_elsewhere() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let agent_id = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "e");
    pressed(&bench, &mut section, "tab", 3);
    typed(&bench, &mut section, ", m9");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &written(Vec::new(), AgentWrite::Gone { id: agent_id }),
    );
    assert!(!section.captures_input(), "the form closed");
    assert_eq!(
        note_line(&render_section(&section, &bench.ctx())),
        DELETED_ELSEWHERE
    );
}

/// A `Failed` named in `REQUEST_NAMES` keeps the form open with the worker's sentence and clears
/// the in-flight guard, so a second `Enter` sends.
#[tokio::test]
async fn a_refused_write_keeps_the_form_open_with_the_sentence() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("alpha", false)]);
    bench.key(&mut section, "n");
    filled(&bench, &mut section, &VALID_CREATE);
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let message = "constraint violated: agent_name_key";
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "create_agent",
            message: message.to_owned(),
        },
    );
    assert!(
        section.captures_input(),
        "the form stays open over its text"
    );
    assert!(
        note_line(&render_section(&section, &bench.ctx())).contains(message),
        "the worker's sentence is on the note line"
    );

    bench.key(&mut section, "enter");
    let requests = requests_of(&bench);
    assert!(
        matches!(requests.as_slice(), [StoreRequest::CreateAgent { .. }]),
        "the guard is cleared and `Enter` retries: {requests:?}"
    );
}

/// `settings/mod.rs`'s `CHANGED_ELSEWHERE_CLOSED`: a spent token with no form open to retry from.
const CHANGED_ELSEWHERE_CLOSED: &str =
    "changed elsewhere; nothing was written \u{2014} reopen the editor and retry";

/// Review L-5 (a): `Esc` while an edit is in flight closes the form but not the guard, and each
/// answer then lands on the closed-form branch: `Edited` says it saved, `Stale` says nothing was
/// written and how to retry, and `Gone` says the row went. Each answer clears the guard.
#[tokio::test]
async fn esc_during_an_edit_then_each_answer_lands_on_the_closed_form() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let agent_id = row.agent.id;
    for (outcome, rows, expected) in [
        (
            AgentWrite::Edited {
                id: agent_id,
                name: "alpha".to_owned(),
            },
            vec![row.clone()],
            "saved `alpha`",
        ),
        (
            AgentWrite::Stale { id: agent_id },
            vec![row.clone()],
            CHANGED_ELSEWHERE_CLOSED,
        ),
        (
            AgentWrite::Gone { id: agent_id },
            Vec::new(),
            "deleted elsewhere; nothing was written",
        ),
    ] {
        let mut section = section_over(&bench, vec![row.clone()]);
        bench.key(&mut section, "e");
        pressed(&bench, &mut section, "tab", 2);
        typed(&bench, &mut section, " --more");
        bench.key(&mut section, "enter");
        assert!(
            matches!(
                requests_of(&bench).as_slice(),
                [StoreRequest::EditAgent { .. }]
            ),
            "the edit is in flight"
        );

        assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);
        assert!(!section.captures_input(), "`Esc` closed the form");
        bench.key(&mut section, "e");
        assert_eq!(
            errors_of(&bench.drained()),
            vec!["`edit_agent` is still in flight".to_owned()],
            "the guard outlives the form"
        );

        bench.reply(&mut section, &written(rows, outcome.clone()));
        assert!(!section.captures_input(), "{outcome:?} opens nothing");
        assert_eq!(
            note_line(&render_section(&section, &bench.ctx())),
            expected,
            "{outcome:?}"
        );
        // `n` needs no row, so it answers the guard alone even after `Gone` emptied the table.
        bench.key(&mut section, "n");
        let emitted = bench.drained();
        assert!(
            errors_of(&emitted).is_empty() && section.captures_input(),
            "{outcome:?} cleared the guard: {emitted:?}"
        );
    }
}

/// Review L-5 (b): only a registry write's own `Failed` clears the guard; a refusal of any other
/// request the tab hands the section leaves the write in flight.
#[tokio::test]
async fn a_failed_for_another_request_does_not_clear_the_write_guard() {
    let bench = SectionBench::new().await;
    for other in [
        "probe_agents",
        "install_plan",
        "auth_start",
        "auth_open",
        "box_edit",
    ] {
        let mut section = section_over(&bench, vec![registry_row("alpha", false)]);
        bench.key(&mut section, "t");
        let _ = bench.drained();

        bench.reply(
            &mut section,
            &StoreReply::Failed {
                request: other,
                message: "refused".to_owned(),
            },
        );
        bench.key(&mut section, "t");
        let emitted = bench.drained();
        assert_eq!(
            errors_of(&emitted),
            vec!["`set_agent_on_box` is still in flight".to_owned()],
            "`{other}` is not this write's answer"
        );
        assert!(!asked_anything(&emitted), "`{other}`: {emitted:?}");
    }
}

/// D240: a plain `Agents` reply (activation, a probe, a login's re-read) replaces the rows and
/// never closes the form or moves its token.
#[tokio::test]
async fn an_agents_reply_does_not_close_the_form_or_move_its_token() {
    let bench = SectionBench::new().await;
    let row = editable_row("alpha");
    let opened_at = row.agent.updated_at;
    let mut section = section_over(&bench, vec![row.clone()]);
    bench.key(&mut section, "e");
    pressed(&bench, &mut section, "tab", 3);
    typed(&bench, &mut section, ", m9");

    let mut moved = row;
    moved.agent.updated_at = htui_core::fixtures::demo_at(2, 0);
    bench.reply(&mut section, &StoreReply::Agents(vec![moved]));
    assert!(section.captures_input(), "the form is still open");

    let _ = bench.drained();
    bench.key(&mut section, "enter");
    let requests = requests_of(&bench);
    assert!(
        matches!(
            requests.as_slice(),
            [StoreRequest::EditAgent { expected, .. }] if *expected == opened_at
        ),
        "the token is still the one the form opened on: {requests:?}"
    );
}

/// `t` flips this box's switch: a row that is on is sent `enabled: false`, a switched-off row
/// `enabled: true`. The reply's notice names the row.
#[tokio::test]
async fn t_sends_set_agent_on_box_with_the_inverse_of_the_switch() {
    let bench = SectionBench::new().await;
    let alpha = registry_row("alpha", false);
    let mut beta = registry_row("beta", false);
    beta.user_off = true;
    let (alpha_id, beta_id) = (alpha.agent.id, beta.agent.id);
    let mut section = section_over(&bench, vec![alpha.clone(), beta.clone()]);
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "t"), Handled::Consumed);
    let requests = requests_of(&bench);
    assert!(
        matches!(
            requests.as_slice(),
            [StoreRequest::SetAgentOnBox { agent_id, enabled: false }] if *agent_id == alpha_id
        ),
        "{requests:?}"
    );
    let mut off = alpha;
    off.user_off = true;
    bench.reply(
        &mut section,
        &written(
            vec![off, beta],
            AgentWrite::Switched {
                id: alpha_id,
                name: "alpha".to_owned(),
                enabled: false,
            },
        ),
    );
    let note = note_line(&render_section(&section, &bench.ctx()));
    assert!(note.contains("`alpha` switched off on this box"), "{note}");

    bench.key(&mut section, "j");
    bench.key(&mut section, "t");
    let requests = requests_of(&bench);
    assert!(
        matches!(
            requests.as_slice(),
            [StoreRequest::SetAgentOnBox { agent_id, enabled: true }] if *agent_id == beta_id
        ),
        "{requests:?}"
    );
    // A row a probe has answered on this box: its verdict is what decides now.
    let mut probed_beta = probed_row(
        "beta",
        true,
        Some("0.48.0"),
        Some(json!({ "status": "ready", "source": "probe" })),
    );
    probed_beta.agent.id = beta_id;
    if let Some(on_box) = probed_beta.on_box.as_mut() {
        on_box.agent_id = beta_id;
    }
    bench.reply(
        &mut section,
        &written(
            vec![probed_beta],
            AgentWrite::Switched {
                id: beta_id,
                name: "beta".to_owned(),
                enabled: true,
            },
        ),
    );
    let note = note_line(&render_section(&section, &bench.ctx()));
    assert!(
        note.contains("`beta` switched on; the probe's verdict decides"),
        "{note}"
    );
}

/// Review L-2: switching on a row no probe has answered on this box says so and offers `r`, rather
/// than promising a verdict that does not exist: no `agent_box` row, a bare one the switch wrote,
/// and a row the re-read lacks all read the same.
#[tokio::test]
async fn switching_on_an_unprobed_row_says_not_probed_yet() {
    let bench = SectionBench::new().await;
    let absent = registry_row("absent", false);
    let mut bare = probed_row("bare", true, None, None);
    if let Some(on_box) = bare.on_box.as_mut() {
        on_box.probed_at = None;
    }
    for (rows, id, name) in [
        (vec![absent.clone()], absent.agent.id, "absent"),
        (vec![bare.clone()], bare.agent.id, "bare"),
        (Vec::new(), absent.agent.id, "absent"),
    ] {
        let mut section = section_over(&bench, rows.clone());
        let _ = bench.drained();
        bench.reply(
            &mut section,
            &written(
                rows,
                AgentWrite::Switched {
                    id,
                    name: name.to_owned(),
                    enabled: true,
                },
            ),
        );
        assert_eq!(
            note_line(&render_section(&section, &bench.ctx())),
            format!("`{name}` switched on; not probed yet \u{b7} r probes")
        );
    }
}

/// D232: `n`, `e` and `t` are refused by one sentence while a probe, an install or a login runs.
#[tokio::test]
async fn n_e_and_t_are_refused_while_a_probe_an_install_or_a_login_runs() {
    let bench = SectionBench::new().await;
    let probing = {
        let mut section = section_over(&bench, vec![registry_row("declared", true)]);
        bench.key(&mut section, "r");
        (section, "a probe is running; edit afterwards")
    };
    let installing = {
        let mut section = section_over(&bench, vec![registry_row("declared", true)]);
        bench.key(&mut section, "i");
        (section, "an install is running; edit afterwards")
    };
    let logging_in = {
        let mut section = section_over(
            &bench,
            vec![login_row(
                "loginable",
                ProbeStatus::Unauthenticated,
                &[METHOD],
            )],
        );
        bench.key(&mut section, "a");
        (section, "a login is running; edit afterwards")
    };
    let _ = bench.drained();

    for (mut section, expected) in [probing, installing, logging_in] {
        for chord in ["n", "e", "t"] {
            assert_eq!(bench.key(&mut section, chord), Handled::Consumed, "{chord}");
            let emitted = bench.drained();
            assert_eq!(errors_of(&emitted), vec![expected.to_owned()], "{chord}");
            assert!(!asked_anything(&emitted), "{chord}: {emitted:?}");
            assert!(!section.captures_input(), "{chord} opened nothing");
        }
    }
}

/// D232 and F-20: while a registry write is in flight, `r`, `i` and `a` are refused, and so are
/// `n`, `e` and `t`; the guard clears on the write's own `Failed`.
#[tokio::test]
async fn r_i_and_a_are_refused_while_a_registry_write_is_in_flight() {
    let bench = SectionBench::new().await;
    let mut section = section_over(&bench, vec![registry_row("declared", true)]);
    bench.key(&mut section, "t");
    let _ = bench.drained();

    for chord in ["r", "i", "a", "n", "e", "t"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed, "{chord}");
        let emitted = bench.drained();
        assert_eq!(
            errors_of(&emitted),
            vec!["`set_agent_on_box` is still in flight".to_owned()],
            "{chord}"
        );
        assert!(!asked_anything(&emitted), "{chord}: {emitted:?}");
    }

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_agent_on_box",
            message: "this box is not registered yet".to_owned(),
        },
    );
    bench.key(&mut section, "t");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::SetAgentOnBox { .. }]
        ),
        "the write's own `Failed` clears the guard"
    );
}

/// D244: a row the human switched off on this box reads `switched off`, ahead of the probe's words,
/// and the 12 characters fit the 13-wide column at the bordered width.
#[tokio::test]
async fn a_switched_off_row_reads_switched_off() {
    let bench = SectionBench::new().await;
    let mut row = probed_row(
        "vetoed",
        false,
        Some("0.48.0"),
        Some(json!({ "status": "ready", "source": "probe" })),
    );
    row.user_off = true;
    let section = section_over(&bench, vec![row]);
    let rendered = render_section_at(&section, &bench.ctx(), SECTION_BORDERED);
    let bordered = SECTION_BORDERED as usize;
    assert_eq!(
        cell_at(
            &rendered,
            "vetoed",
            bordered,
            on_box_at(bordered),
            ON_BOX_WIDTH
        ),
        "switched off",
        "{rendered}"
    );
}

/// D244: a row the switch inserted, never probed, reads `not probed` like an absent one.
#[tokio::test]
async fn a_bare_row_reads_not_probed() {
    let bench = SectionBench::new().await;
    let mut row = probed_row("bare", true, None, None);
    if let Some(on_box) = row.on_box.as_mut() {
        on_box.probed_at = None;
    }
    let section = section_over(&bench, vec![row]);
    assert_eq!(
        on_box_cell(&render_section(&section, &bench.ctx()), "bare"),
        "not probed"
    );
}

/// D245: the keys and the note are two lines, the note last (which is what
/// [`the_idle_hint_says_r_cannot_refresh_quota`] reads).
#[tokio::test]
async fn the_idle_keys_and_the_quota_note_are_two_lines() {
    let bench = SectionBench::new().await;
    let section = section_over(&bench, vec![registry_row("declared", true)]);
    let rendered = render_section(&section, &bench.ctx());
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines[lines.len() - 2], IDLE_KEYS, "{rendered}");
    assert_eq!(lines[lines.len() - 1], IDLE_NOTE, "{rendered}");
}

/// The create form under the table (D230, D231).
#[tokio::test]
async fn the_create_form_renders_under_the_table() {
    let bench = SectionBench::new().await;
    let mut section = section_over(
        &bench,
        vec![registry_row("alpha", false), registry_row("beta", false)],
    );
    bench.key(&mut section, "n");
    typed(&bench, &mut section, "gamma");
    let rendered = render_section(&section, &bench.ctx());
    insta::assert_snapshot!("agents_create_form", rendered);
}

/// The edit form over a row carrying the second seed's model list and default (F-24): the long
/// `models` line is clipped with a leading `…` to its tail, and the 21-character default is whole.
#[tokio::test]
async fn the_edit_form_renders_the_rows_own_values() {
    let bench = SectionBench::new().await;
    let seeded = htui_core::model::agent::seed_rows(htui_core::fixtures::demo_at(0, 0))
        .into_iter()
        .max_by_key(|agent| agent.models.len())
        .expect("the seeds carry a model list");
    let mut row = registry_row("agent-b", false);
    row.agent.models = seeded.models;
    row.agent.default_model = seeded.default_model;
    row.agent.launch["args"] = json!(["--flag", "a b"]);
    let mut section = section_over(&bench, vec![registry_row("agent-a", false), row]);
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        field_of(&rendered, "default model").as_deref(),
        Some(SEEDED_MODEL),
        "{rendered}"
    );
    assert!(
        field_of(&rendered, "models").is_some_and(|models| models.starts_with('\u{2026}')),
        "{rendered}"
    );
    insta::assert_snapshot!("agents_edit_form", rendered);
}

/// A switched-off row in the table, beside one that is on (D244).
#[tokio::test]
async fn a_switched_off_row_renders_in_the_table() {
    let bench = SectionBench::new().await;
    let on = probed_row(
        "running",
        true,
        Some("0.48.0"),
        Some(json!({ "status": "ready", "source": "probe" })),
    );
    let mut off = probed_row(
        "vetoed",
        false,
        Some("0.48.0"),
        Some(json!({ "status": "ready", "source": "probe" })),
    );
    off.user_off = true;
    let section = section_over(&bench, vec![on, off]);
    let rendered = render_section(&section, &bench.ctx());
    insta::assert_snapshot!("agents_switched_off", rendered);
}

// -------------------------------------------------------------------------------------------
// MOD-66 T4: m and the tool-paths form (plan D10, D11; blueprint §5)
// -------------------------------------------------------------------------------------------

/// `m` on a row whose launch declares no tool (D10).
const LITERAL_LAUNCH: &str = "this row's launch is literal; e edits its command";

/// `m` on a row whose launch does not parse (B14).
const LAUNCH_UNREADABLE: &str = "this row's launch does not parse; nothing declares a tool";

/// The idle note with a manual row in the table (blueprint §5, the maintainer's amendment).
const IDLE_NOTE_MANUAL: &str = "quota latches per chat, r cannot refresh it \u{b7} * manual path";

/// The tool-paths form's header for a row named `paths`.
const PATHS_HEADER: &str = "tool paths for paths \u{b7} empty = no manual path";

/// An absolute path under the temp directory. Built rather than written out, because `/opt/x` is
/// not absolute on Windows (blueprint H-17). Never created: the form checks the shape, and whether
/// it is a file is the worker's check.
fn tool_path(leaf: &str) -> String {
    std::env::temp_dir().join(leaf).display().to_string()
}

/// The snapshot a probe wrote over `stored`, with `0.48.0` as the handshake's version.
fn paths_snapshot(
    stored: &[(&str, &str)],
    source: ProbeSource,
    status: ProbeStatus,
) -> ProbeSnapshot {
    ProbeSnapshot {
        transport: Transport::Acp,
        resolved: None,
        tools: BTreeMap::new(),
        handshake: Some(Handshake {
            at: htui_core::fixtures::demo_at(0, 0),
            protocol_version: 1,
            agent_name: None,
            agent_version: Some("0.48.0".to_owned()),
            capabilities: json!({}),
            auth_methods: Vec::new(),
        }),
        credential: None,
        status,
        stderr_tail: None,
        source,
        manual: stored
            .iter()
            .map(|(tool, path)| ((*tool).to_owned(), (*path).to_owned()))
            .collect(),
    }
}

/// [`paths_row`] with the probe's verdict chosen.
fn paths_row_with(
    name: &str,
    tools: &[&str],
    stored: &[(&str, &str)],
    source: ProbeSource,
    status: ProbeStatus,
) -> AgentSummary {
    let mut summary = registry_row(name, false);
    let declared: serde_json::Map<String, Value> = tools
        .iter()
        .map(|tool| {
            (
                (*tool).to_owned(),
                json!({ "kind": "path", "names": [tool] }),
            )
        })
        .collect();
    summary.agent.launch = json!({
        "command": format!("${{{}}}", tools.first().copied().unwrap_or("none")),
        "args": [],
        "env": {},
        "discovery": { "tools": declared, "handshake": true },
    });
    // Through `agent_box_row`, as `login_row` goes through `ProbeSnapshot`: the section reads the
    // map back with `ProbeSnapshot::from_row`, and a hand-written document might not parse.
    let snapshot = paths_snapshot(stored, source, status);
    summary.on_box = Some(agent_box_row(
        &summary.agent,
        BoxId::new(),
        &snapshot,
        htui_core::fixtures::demo_at(0, 0),
    ));
    summary
}

/// A registry row declaring one `path` tool per name in `tools`, whose `agent_box` row a probe
/// wrote `ready` over the manual map `stored`.
fn paths_row(
    name: &str,
    tools: &[&str],
    stored: &[(&str, &str)],
    source: ProbeSource,
) -> AgentSummary {
    paths_row_with(name, tools, stored, source, ProbeStatus::Ready)
}

/// The text of one tool-paths field as drawn, its label padded to `width` (blueprint H-9: the
/// form sizes its label column to its own tools, so [`field_of`]'s 13 does not apply). An empty
/// field draws its label and nothing after the colon, and the line is trimmed.
fn paths_field_of(rendered: &str, label: &str, width: usize) -> Option<String> {
    let prefix = format!("{label:<width$}:");
    rendered
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(|rest| rest.trim().to_owned())
}

/// Whether the tool-paths field labelled `label` holds the focus: its label is the accented one.
fn paths_focused_on(section: &AgentsSection, bench: &SectionBench, label: &str) -> bool {
    accented_lines(section, &bench.ctx())
        .iter()
        .any(|line| line.starts_with(label))
}

/// The one `SetToolPaths` a drain asked for, or a panic naming what was asked instead.
fn tool_paths_request(bench: &SectionBench) -> (AgentId, BTreeMap<String, String>) {
    match requests_of(bench).as_slice() {
        [StoreRequest::SetToolPaths { agent_id, paths }] => (*agent_id, paths.clone()),
        other => panic!("expected exactly one set_tool_paths: {other:?}"),
    }
}

/// `m` opens the form under the table: one field per tool the row declares, in name order, each
/// prefilled from the stored map (an empty field is "no manual path"). From then on the section
/// takes every printable key, so `l` and `h` are letters of a path, not section cycling (H-12).
#[tokio::test]
async fn m_opens_the_tool_paths_form_with_one_field_per_declared_tool() {
    let bench = SectionBench::new().await;
    let alpha = tool_path("htui-mod66-alpha");
    let row = paths_row(
        "paths",
        &["beta_tool", "alpha_tool"],
        &[("alpha_tool", alpha.as_str())],
        ProbeSource::Manual,
    );
    let mut section = section_over(&bench, vec![row]);
    let _ = bench.drained();
    assert!(!section.captures_input(), "browsing captures nothing");

    assert_eq!(bench.key(&mut section, "m"), Handled::Consumed);
    assert!(
        section.captures_input(),
        "an open tool-paths form takes every printable key"
    );
    assert!(
        requests_of(&bench).is_empty(),
        "opening a form asks for nothing"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains(PATHS_HEADER), "{rendered}");
    assert_eq!(
        paths_field_of(&rendered, "alpha_tool", 10),
        Some(alpha.clone()),
        "prefilled from the stored map: {rendered}"
    );
    assert_eq!(
        paths_field_of(&rendered, "beta_tool", 10),
        Some(String::new()),
        "a tool with no manual path is an empty field: {rendered}"
    );
    let alpha_at = rendered.find("alpha_tool").expect("alpha_tool is drawn");
    let beta_at = rendered.find("beta_tool ").expect("beta_tool is drawn");
    assert!(alpha_at < beta_at, "fields in name order: {rendered}");
    assert!(
        paths_focused_on(&section, &bench, "alpha_tool"),
        "the first field holds the focus"
    );
    // The last drawn line: the note line under it is empty while a form is open, and `lines`
    // does not yield a trailing empty line.
    assert_eq!(
        rendered.lines().last(),
        Some("Tab next field \u{b7} Enter saves \u{b7} Esc cancels"),
        "the form's keys line: {rendered}"
    );

    for chord in ["l", "h"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed, "{chord}");
    }
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        paths_field_of(&rendered, "alpha_tool", 10),
        Some(format!("{alpha}lh")),
        "`l` and `h` landed in the focused field: {rendered}"
    );

    assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);
    assert!(!section.captures_input(), "`Esc` closes the form");
    assert!(requests_of(&bench).is_empty(), "and writes nothing");
}

/// D10, B14: `m` on a row whose launch declares no tool says how that row is edited instead, and
/// one whose launch does not parse says so; an empty table has no row. Nothing opens, nothing is
/// asked.
#[tokio::test]
async fn m_on_a_literal_row_is_refused() {
    let bench = SectionBench::new().await;
    let mut literal = paths_row("literal", &["alpha_tool"], &[], ProbeSource::Probe);
    literal.agent.launch = json!({ "command": "/bin/x", "args": [], "env": {} });
    let mut no_tools = paths_row("no-tools", &["alpha_tool"], &[], ProbeSource::Probe);
    no_tools.agent.launch["discovery"]["tools"] = json!({});
    let mut broken = paths_row("broken", &["alpha_tool"], &[], ProbeSource::Probe);
    broken.agent.launch = json!("nonsense");

    for (rows, expected) in [
        (vec![literal], LITERAL_LAUNCH),
        (vec![no_tools], LITERAL_LAUNCH),
        (vec![broken], LAUNCH_UNREADABLE),
        (Vec::new(), "no agent row is selected"),
    ] {
        let mut section = section_over(&bench, rows);
        let _ = bench.drained();
        assert_eq!(
            bench.key(&mut section, "m"),
            Handled::Consumed,
            "{expected}"
        );
        let emitted = bench.drained();
        assert_eq!(errors_of(&emitted), vec![expected.to_owned()]);
        assert!(!asked_anything(&emitted), "{expected}: {emitted:?}");
        assert!(!section.captures_input(), "{expected}: nothing opened");
    }
}

/// D10: `m` is refused as `n`, `e` and `t` are, while a registry write, a probe, an install or a
/// login is in flight, and a probe's own `Failed` frees it again.
#[tokio::test]
async fn m_is_refused_while_a_write_or_a_probe_is_in_flight() {
    let bench = SectionBench::new().await;
    let row = || paths_row("paths", &["alpha_tool"], &[], ProbeSource::Probe);

    let mut writing = section_over(&bench, vec![row()]);
    bench.key(&mut writing, "t");
    let mut probing = section_over(&bench, vec![row()]);
    bench.key(&mut probing, "r");
    let mut installing = section_over(&bench, vec![registry_row("declared", true)]);
    bench.key(&mut installing, "i");
    let mut logging_in = section_over(
        &bench,
        vec![login_row(
            "loginable",
            ProbeStatus::Unauthenticated,
            &[METHOD],
        )],
    );
    bench.key(&mut logging_in, "a");
    let _ = bench.drained();

    for (section, expected) in [
        (&mut writing, "`set_agent_on_box` is still in flight"),
        (&mut probing, "a probe is running; edit afterwards"),
        (&mut installing, "an install is running; edit afterwards"),
        (&mut logging_in, "a login is running; edit afterwards"),
    ] {
        assert_eq!(bench.key(section, "m"), Handled::Consumed, "{expected}");
        let emitted = bench.drained();
        assert_eq!(errors_of(&emitted), vec![expected.to_owned()]);
        assert!(!asked_anything(&emitted), "{expected}: {emitted:?}");
        assert!(!section.captures_input(), "{expected}: nothing opened");
    }

    bench.reply(
        &mut probing,
        &StoreReply::Failed {
            request: "probe_agents",
            message: "refused".to_owned(),
        },
    );
    bench.key(&mut probing, "m");
    assert!(bench.errors().is_empty(), "the probe is over");
    assert!(probing.captures_input(), "and `m` opens the form");
}

/// D10: `Enter` sends one `SetToolPaths` carrying every non-empty field, trimmed. The form stays
/// open until the reply, and a second `Enter` meanwhile is refused by the guard (F-20).
#[tokio::test]
async fn enter_sends_set_tool_paths_with_trimmed_non_empty_entries() {
    let bench = SectionBench::new().await;
    let (alpha, beta) = (tool_path("htui-mod66-alpha"), tool_path("htui-mod66-beta"));
    let row = paths_row(
        "paths",
        &["alpha_tool", "beta_tool", "gamma_tool"],
        &[("alpha_tool", alpha.as_str())],
        ProbeSource::Manual,
    );
    let agent = row.agent.id;
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    bench.key(&mut section, "tab");
    typed(&bench, &mut section, &format!("  {beta}  "));
    bench.key(&mut section, "tab");
    typed(&bench, &mut section, "   ");
    let _ = bench.drained();

    bench.key(&mut section, "enter");
    let (agent_id, paths) = tool_paths_request(&bench);
    assert_eq!(agent_id, agent);
    assert_eq!(
        paths,
        BTreeMap::from([
            ("alpha_tool".to_owned(), alpha),
            ("beta_tool".to_owned(), beta),
        ]),
        "trimmed, and a blank field is no manual path"
    );
    assert!(
        section.captures_input(),
        "the form stays open until the reply"
    );

    bench.key(&mut section, "enter");
    let emitted = bench.drained();
    assert_eq!(
        errors_of(&emitted),
        vec!["`set_tool_paths` is still in flight".to_owned()]
    );
    assert!(!asked_anything(&emitted), "{emitted:?}");
}

/// D9: an empty map is "clear every manual path", and the form sends it.
#[tokio::test]
async fn clearing_every_field_sends_an_empty_map() {
    let bench = SectionBench::new().await;
    let alpha = tool_path("htui-mod66-alpha");
    let row = paths_row(
        "paths",
        &["alpha_tool"],
        &[("alpha_tool", alpha.as_str())],
        ProbeSource::Manual,
    );
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    // As many as the prefill has characters: a fixed count leaves `/` behind under a long
    // `$TMPDIR` (review L2).
    pressed(&bench, &mut section, "backspace", alpha.chars().count());
    let _ = bench.drained();

    bench.key(&mut section, "enter");
    let (_, paths) = tool_paths_request(&bench);
    assert!(paths.is_empty(), "{paths:?}");
}

/// D10: an unchanged map closes the form with `UNCHANGED` and writes nothing. A field holding
/// only spaces is still "no manual path".
#[tokio::test]
async fn an_unchanged_map_closes_with_unchanged() {
    let bench = SectionBench::new().await;
    let alpha = tool_path("htui-mod66-alpha");
    let row = paths_row(
        "paths",
        &["alpha_tool", "beta_tool"],
        &[("alpha_tool", alpha.as_str())],
        ProbeSource::Manual,
    );
    for blank in ["", "   "] {
        let mut section = section_over(&bench, vec![row.clone()]);
        bench.key(&mut section, "m");
        bench.key(&mut section, "tab");
        typed(&bench, &mut section, blank);
        let _ = bench.drained();

        bench.key(&mut section, "enter");
        assert!(!section.captures_input(), "{blank:?}: the form closed");
        assert_eq!(
            note_line(&render_section(&section, &bench.ctx())),
            UNCHANGED,
            "{blank:?}"
        );
        assert!(requests_of(&bench).is_empty(), "{blank:?}: nothing written");
    }
}

/// D10, B10: a relative path is refused here by `parse_tool_path`'s own sentence, naming the
/// tool; the focus moves to that field and nothing is sent.
#[tokio::test]
async fn a_relative_path_is_refused_locally_naming_the_tool() {
    let bench = SectionBench::new().await;
    let row = paths_row(
        "paths",
        &["alpha_tool", "beta_tool"],
        &[],
        ProbeSource::Probe,
    );
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    bench.key(&mut section, "tab");
    typed(&bench, &mut section, "bin/x");
    bench.key(&mut section, "tab");
    assert!(paths_focused_on(&section, &bench, "alpha_tool"));
    let _ = bench.drained();

    bench.key(&mut section, "enter");
    assert!(requests_of(&bench).is_empty(), "nothing is sent");
    assert!(section.captures_input(), "the form stays open");
    assert_eq!(
        note_line(&render_section(&section, &bench.ctx())),
        "`beta_tool`: the path must be absolute"
    );
    assert!(
        paths_focused_on(&section, &bench, "beta_tool"),
        "the focus moved to the refused field"
    );
}

/// D10, B16, B18: `AgentWritten::ToolPaths` for the form's own row closes it and notes the probe's
/// word; one for another row leaves it open.
#[tokio::test]
async fn agent_written_tool_paths_closes_the_form_and_notes_the_status() {
    let bench = SectionBench::new().await;
    let row = paths_row("paths", &["alpha_tool"], &[], ProbeSource::Probe);
    let other = paths_row("other", &["alpha_tool"], &[], ProbeSource::Probe);
    let (id, other_id) = (row.agent.id, other.agent.id);
    let rows = vec![other, row];
    let mut section = section_over(&bench, rows.clone());
    bench.key(&mut section, "j");
    bench.key(&mut section, "m");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &written(
            rows.clone(),
            AgentWrite::ToolPaths {
                id: other_id,
                name: "other".to_owned(),
                status: ProbeStatus::Missing,
            },
        ),
    );
    assert!(
        section.captures_input(),
        "another row's answer leaves this form open (B18)"
    );

    bench.reply(
        &mut section,
        &written(
            rows,
            AgentWrite::ToolPaths {
                id,
                name: "paths".to_owned(),
                status: ProbeStatus::Ready,
            },
        ),
    );
    assert!(!section.captures_input(), "its own answer closes it");
    assert_eq!(
        note_line(&render_section(&section, &bench.ctx())),
        "tool paths saved for `paths` \u{b7} this box: ready"
    );
}

/// H-6: a refused `set_tool_paths` clears the guard though it is not in `REQUEST_NAMES`; the form
/// stays open over its text with the worker's sentence, and the next `Enter` sends again.
#[tokio::test]
async fn a_failed_set_tool_paths_clears_busy_and_keeps_the_form() {
    let bench = SectionBench::new().await;
    let beta = tool_path("htui-mod66-beta");
    let row = paths_row(
        "paths",
        &["alpha_tool", "beta_tool"],
        &[],
        ProbeSource::Probe,
    );
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    bench.key(&mut section, "tab");
    typed(&bench, &mut section, &beta);
    bench.key(&mut section, "enter");
    let _ = tool_paths_request(&bench);

    let message = format!("`beta_tool`: `{beta}` is not a file on this box");
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_tool_paths",
            message: message.clone(),
        },
    );
    assert!(section.captures_input(), "the form stays open");
    assert_eq!(note_line(&render_section(&section, &bench.ctx())), message);

    bench.key(&mut section, "enter");
    let (_, paths) = tool_paths_request(&bench);
    assert_eq!(
        paths,
        BTreeMap::from([("beta_tool".to_owned(), beta)]),
        "the guard is cleared and `Enter` retries"
    );
}

/// H-12: a bracketed paste lands in the focused field of an open tool-paths form.
#[tokio::test]
async fn a_paste_lands_in_the_focused_tool_path() {
    let bench = SectionBench::new().await;
    let beta = tool_path("htui-mod66-beta");
    let row = paths_row(
        "paths",
        &["alpha_tool", "beta_tool"],
        &[],
        ProbeSource::Probe,
    );
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    bench.key(&mut section, "tab");
    assert_eq!(bench.paste(&mut section, &beta), Handled::Consumed);
    let _ = bench.drained();

    bench.key(&mut section, "enter");
    let (_, paths) = tool_paths_request(&bench);
    assert_eq!(paths, BTreeMap::from([("beta_tool".to_owned(), beta)]));
}

/// D10, B17, and the maintainer's amendment to blueprint §5: a row whose `probe.source` is
/// `manual` ends its verdict in `*`, whatever the verdict is, and the idle note says what the
/// star means only while such a row is listed. A switched-off row reads `switched off`, unmarked.
/// The note counts only a star a cell can show (review N1): not a switched-off row's, and none
/// while every cell reads `probing…`.
#[tokio::test]
async fn a_manual_row_reads_manual_in_the_on_this_box_cell() {
    let bench = SectionBench::new().await;
    let alpha = tool_path("htui-mod66-alpha");
    let stored = [("alpha_tool", alpha.as_str())];
    let tools = ["alpha_tool"];
    let manual_ready = paths_row("m-ready", &tools, &stored, ProbeSource::Manual);
    let manual_missing = paths_row_with(
        "m-missing",
        &tools,
        &stored,
        ProbeSource::Manual,
        ProbeStatus::Missing,
    );
    let probe_ready = paths_row("p-ready", &tools, &[], ProbeSource::Probe);
    let mut manual_off = paths_row("m-off", &tools, &stored, ProbeSource::Manual);
    manual_off.user_off = true;

    let section = section_over(
        &bench,
        vec![
            manual_ready.clone(),
            manual_missing,
            probe_ready.clone(),
            manual_off.clone(),
        ],
    );
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(on_box_cell(&rendered, "m-ready"), "0.48.0*", "{rendered}");
    assert_eq!(
        on_box_cell(&rendered, "m-missing"),
        "missing*",
        "{rendered}"
    );
    assert_eq!(on_box_cell(&rendered, "p-ready"), "0.48.0", "{rendered}");
    assert_eq!(
        on_box_cell(&rendered, "m-off"),
        "switched off",
        "{rendered}"
    );
    assert_eq!(note_line(&rendered), IDLE_NOTE_MANUAL, "{rendered}");
    assert!(IDLE_NOTE_MANUAL.chars().count() <= SECTION_BORDERED as usize);

    let section = section_over(&bench, vec![probe_ready.clone()]);
    assert_eq!(
        note_line(&render_section(&section, &bench.ctx())),
        IDLE_NOTE,
        "no manual row, no star to explain"
    );

    let section = section_over(&bench, vec![manual_off, probe_ready]);
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        note_line(&rendered),
        IDLE_NOTE,
        "a switched-off manual row shows no star: {rendered}"
    );

    let mut section = section_over(&bench, vec![manual_ready]);
    bench.key(&mut section, "r");
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        on_box_cell(&rendered, "m-ready"),
        "probing\u{2026}",
        "{rendered}"
    );
    assert_eq!(
        note_line(&rendered),
        IDLE_NOTE,
        "no star while probing: {rendered}"
    );
}

/// B8: the label column is the longest tool's name, capped at a third of the width; a longer name
/// is cut to one less than the column and ends in `…`.
#[tokio::test]
async fn the_tool_paths_form_sizes_its_labels_to_the_longest_tool() {
    let bench = SectionBench::new().await;
    let row = paths_row(
        "paths",
        &["demo_agent_server", "x"],
        &[],
        ProbeSource::Probe,
    );
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    let rendered = render_section(&section, &bench.ctx());
    assert_eq!(
        paths_field_of(&rendered, "demo_agent_server", 17),
        Some(String::new()),
        "{rendered}"
    );
    assert_eq!(
        paths_field_of(&rendered, "x", 17),
        Some(String::new()),
        "`x` is padded to the longest tool: {rendered}"
    );

    let long = format!("tool_{}", "n".repeat(35));
    assert_eq!(long.chars().count(), 40);
    let row = paths_row("paths", &[long.as_str(), "x"], &[], ProbeSource::Probe);
    let mut section = section_over(&bench, vec![row]);
    bench.key(&mut section, "m");
    let rendered = render_section(&section, &bench.ctx());
    let cut = format!("{}\u{2026}", &long[..32]);
    assert_eq!(cut.chars().count(), usize::from(SECTION_WIDE) / 3);
    assert_eq!(
        paths_field_of(&rendered, &cut, 33),
        Some(String::new()),
        "{rendered}"
    );
    assert_eq!(
        paths_field_of(&rendered, "x", 33),
        Some(String::new()),
        "{rendered}"
    );
}

/// The tool-paths form under the table (D10), on the second of two rows.
#[tokio::test]
async fn the_tool_paths_form_renders_under_the_table() {
    let bench = SectionBench::new().await;
    let alpha = tool_path("htui-mod66-alpha");
    let mut section = section_over(
        &bench,
        vec![
            paths_row(
                "manual",
                &["alpha_tool"],
                &[("alpha_tool", alpha.as_str())],
                ProbeSource::Manual,
            ),
            paths_row(
                "paths",
                &["alpha_tool", "demo_agent_server"],
                &[],
                ProbeSource::Probe,
            ),
        ],
    );
    bench.key(&mut section, "j");
    bench.key(&mut section, "m");
    let rendered = render_section(&section, &bench.ctx());
    insta::assert_snapshot!("agents_tool_paths_form", rendered);
}

/// A Qdrant snapshot with the URL stored and no key.
fn qdrant_stored() -> QdrantSnapshot {
    QdrantSnapshot {
        url_state: QdrantState::Stored,
        key_state: QdrantState::NotStored,
        url_summary: Some("https://qdrant.example:6334".to_owned()),
    }
}

/// The store requests a section sent since the queue was last drained.
fn requests_of(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// MOD-63: the hints advertise `r reload`, so `r` re-reads, and the unavailable state (whose only
/// hint is `r reload`) recovers through it without leaving the section.
#[tokio::test]
async fn qdrant_r_re_reads_and_recovers_the_unavailable_state() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "r"), Handled::Consumed);
    assert!(
        matches!(requests_of(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "one read from the browse state"
    );

    let mut section = QdrantSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "qdrant_info",
            message: "the keyring is locked".to_owned(),
        },
    );
    let _ = bench.drained();
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("r reload"), "{rendered}");
    assert!(!rendered.contains("e edit"), "{rendered}");

    assert_eq!(bench.key(&mut section, "r"), Handled::Consumed);
    assert!(
        matches!(requests_of(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "one read from the unavailable state"
    );
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("e edit"),
        "the fresh snapshot ends the outage: {rendered}"
    );
}

/// A write remembers its name until the reply, so its snapshot says what it did and its failure
/// lands on the section.
#[tokio::test]
async fn a_qdrant_write_says_what_it_did_or_why_it_failed() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    let _ = bench.drained();

    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::ClearQdrantSettings]
        ),
        "y sends the clear"
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("clear_qdrant_settings in flight"),
        "{rendered}"
    );
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(QdrantSnapshot {
            url_state: QdrantState::NotStored,
            key_state: QdrantState::NotStored,
            url_summary: None,
        }),
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("the Qdrant settings are gone from the keyring"),
        "{rendered}"
    );

    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "clear_qdrant_settings",
            message: "the keyring refused the delete".to_owned(),
        },
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("the keyring refused the delete"),
        "{rendered}"
    );
    assert!(!rendered.contains("in flight"), "{rendered}");
}

/// One write at a time: while one is out, `e` and `c` are refused by name, and `r` is not refused
/// because it is how a lost reply recovers.
#[tokio::test]
async fn a_qdrant_write_in_flight_refuses_e_and_c_but_not_r() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = bench.drained();

    for chord in ["e", "c"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        assert!(requests_of(&bench).is_empty(), "`{chord}` sends nothing");
        assert!(!section.captures_input(), "`{chord}` opens nothing");
        let rendered = render_section(&section, &bench.ctx());
        assert!(
            rendered.contains("`clear_qdrant_settings` is still in flight"),
            "`{chord}` is refused by name: {rendered}"
        );
    }

    assert_eq!(bench.key(&mut section, "r"), Handled::Consumed);
    assert!(
        matches!(requests_of(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "one read, with a write still out"
    );
}

/// MOD-10 M4 R1 L-5: Enter on an empty key field, or one holding only spaces, leaves the stored key
/// alone and says so, as the URL row does; removing the key stays on `c`, behind its question.
#[tokio::test]
async fn an_empty_qdrant_key_submit_sends_nothing() {
    let bench = SectionBench::new().await;
    for typed in ["", "   "] {
        let mut section = QdrantSection::new();
        bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
        let _ = bench.drained();

        bench.key(&mut section, "j");
        assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
        if !typed.is_empty() {
            assert_eq!(bench.paste(&mut section, typed), Handled::Consumed);
        }
        assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);

        assert!(
            requests_of(&bench).is_empty(),
            "{typed:?} sends no key write"
        );
        assert!(!section.captures_input(), "{typed:?} closes the field");
        let rendered = render_section(&section, &bench.ctx());
        assert!(
            rendered.contains("nothing typed; the stored API key is unchanged"),
            "{typed:?}: {rendered}"
        );
    }
}

/// A reload sent just before a write lands first; it answers the reload, so the section never
/// says the settings were cleared before the clear has run, and the clear's refusal still lands.
#[tokio::test]
async fn a_qdrant_reload_just_before_a_write_is_not_taken_as_its_answer() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    bench.key(&mut section, "r");
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::QdrantInfo, StoreRequest::ClearQdrantSettings]
        ),
        "the read, then the clear"
    );

    // The read's pre-clear snapshot comes back first.
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        !rendered.contains("the Qdrant settings are gone from the keyring"),
        "the read is not the clear's answer: {rendered}"
    );
    assert!(
        rendered.contains("clear_qdrant_settings in flight"),
        "the clear is still out: {rendered}"
    );

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "clear_qdrant_settings",
            message: "the keyring refused the delete".to_owned(),
        },
    );
    let rendered = render_section(&section, &bench.ctx());
    assert!(
        rendered.contains("the keyring refused the delete"),
        "the refusal lands on the section: {rendered}"
    );
    assert!(
        !rendered.contains("the Qdrant settings are gone from the keyring"),
        "{rendered}"
    );
}

/// The backend a `--demo` shell (and the harness) runs on: `Memory`, which has no keyring.
fn qdrant_demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The demo answer to a Qdrant read: both rows `NotApplicable` and no URL summary.
#[track_caller]
fn assert_qdrant_not_applicable(reply: StoreReply, path: &str) {
    match reply {
        StoreReply::Qdrant(snapshot) => {
            assert_eq!(snapshot.url_state, QdrantState::NotApplicable, "{path}");
            assert_eq!(snapshot.key_state, QdrantState::NotApplicable, "{path}");
            assert_eq!(snapshot.url_summary, None, "{path}");
        }
        other => panic!("{path}: expected a not-applicable Qdrant snapshot, got {other:?}"),
    }
}

/// A demo write's refusal: `Failed`, named by the request, saying a demo has no keyring.
#[track_caller]
fn assert_qdrant_demo_refusal(reply: StoreReply, name: &str, path: &str) {
    match reply {
        StoreReply::Failed { request, message } => {
            assert_eq!(request, name, "{path}");
            assert_eq!(message, DEMO_SESSION, "{path}");
        }
        other => panic!("{path}: expected `{name}` refused, got {other:?}"),
    }
}

/// One request through a spawned store loop over the demo backend: the path `--demo` takes
/// (CLEAN-8 #9, blueprint A-2), where `serve` is the harness's.
async fn through_the_demo_loop(request: StoreRequest) -> StoreReply {
    let (req_tx, req_rx) = tokio::sync::mpsc::unbounded_channel();
    let (rep_tx, mut rep_rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = htui::store_worker::spawn(Started::detached(qdrant_demo()), req_rx, rep_tx);
    req_tx
        .send(RequestEnvelope {
            seq: 1,
            origin: Origin::App,
            request,
        })
        .expect("the worker is alive");
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), rep_rx.recv())
        .await
        .expect("the loop answers within five seconds")
        .expect("the loop answers");
    assert_eq!(envelope.seq, 1, "the answer goes to the request's address");
    drop(req_tx);
    worker.await.expect("the worker stops with its channel");
    envelope.reply
}

/// CLEAN-8 #9: a demo session's Qdrant read consults no keyring, through `serve` (the harness)
/// and through the spawned loop (`--demo`) alike. No keyring guard on purpose: a `Memory` backend
/// must not read a keyring, and a guard would hide one.
#[tokio::test]
async fn qdrant_demo_reads_no_keyring() {
    assert_qdrant_not_applicable(
        htui::store_worker::serve(&qdrant_demo(), &StoreRequest::QdrantInfo).await,
        "serve",
    );
    assert_qdrant_not_applicable(
        through_the_demo_loop(StoreRequest::QdrantInfo).await,
        "the spawned loop",
    );
}

/// CLEAN-8 #9: a demo session refuses every Qdrant keyring write before the keyring is reached,
/// with the Secrets section's sentence. No guard on purpose, as above.
#[tokio::test]
async fn qdrant_demo_refuses_every_keyring_write() {
    for request in [
        StoreRequest::SetQdrantUrl("https://q.example:6334".to_owned()),
        StoreRequest::SetQdrantApiKey(Redacted::new("qk-typed-123".to_owned())),
        StoreRequest::ClearQdrantSettings,
    ] {
        let name = request.name();
        assert_qdrant_demo_refusal(
            htui::store_worker::serve(&qdrant_demo(), &request).await,
            name,
            "serve",
        );
    }
    assert_qdrant_demo_refusal(
        through_the_demo_loop(StoreRequest::ClearQdrantSettings).await,
        "clear_qdrant_settings",
        "the spawned loop",
    );
}

/// CLEAN-8 #9: a demo snapshot opens no editor, shows both rows as not applicable, refuses `e` and
/// `c` with the demo sentence and sends nothing, and `r` still re-reads.
#[tokio::test]
async fn qdrant_demo_section_offers_no_edit() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(QdrantSnapshot {
            url_state: QdrantState::NotApplicable,
            key_state: QdrantState::NotApplicable,
            url_summary: None,
        }),
    );
    let _ = bench.drained();
    assert!(!section.captures_input(), "a demo snapshot opens no editor");
    let rendered = render_section(&section, &bench.ctx());
    assert!(rendered.contains("n/a in a demo session"), "{rendered}");

    for chord in ["e", "c"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        assert!(requests_of(&bench).is_empty(), "`{chord}` sends nothing");
        assert!(!section.captures_input(), "`{chord}` opens nothing");
        let rendered = render_section(&section, &bench.ctx());
        assert!(
            rendered.contains(DEMO_SESSION),
            "`{chord}` is refused with the demo sentence: {rendered}"
        );
    }

    assert_eq!(bench.key(&mut section, "r"), Handled::Consumed);
    assert!(
        matches!(requests_of(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "`r` still re-reads in a demo"
    );
}

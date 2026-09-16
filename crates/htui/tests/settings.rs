//! Settings tab tests: the section registry and MOD-2's agent section (`R-TUI-8`, plan T10).
//!
//! Everything runs through `Harness` and the tab's public surface, at the 100x30 frame the whole
//! snapshot suite is pinned to.
#![cfg(feature = "testkit")]

use htui::app::{Action, Ctx, Handled};
use htui::store_worker::{AuthFrame, InstallFrame, StoreReply, StoreRequest};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{
    AgentsSection, HierarchySection, SectionId, SettingsSection, SettingsTab, message,
};
use htui_agent::acp::Handshake;
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::install::InstallRecord;
use htui_agent::probe::{CredentialTier, ProbeSnapshot, ProbeSource, ProbeStatus};
use htui_agent::{ArchiveFormat, InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::{Agent, AgentBox, AgentId, AgentSummary, Billing, BoxId, Scope, Transport};
use htui_core::store::{MemStore, WriteStore};
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

/// A section that is taking typed text: every key is its own, `l` included (D2).
///
/// The counter is what makes the difference visible in a frame — a section that merely swallowed
/// `l` would render the same thing whether the tab cycled to it or not.
#[derive(Debug, Default)]
struct CapturingProbe {
    seen: Vec<KeyCode>,
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

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        message(frame, area, &format!("seen {}", self.seen.len()), ctx.theme);
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

/// The strip has to fit the frame it is drawn in (D4; PRD risk "strip overflow at 100 columns").
///
/// `render_strip` draws `format!(" {title} ")` per registered section, so the joined width is
/// `Σ (title chars + 2)`. The pin is over the sections the product registers, not over a test
/// fixture: the moment a fifth section makes the strip 101 columns wide this fails, and that is
/// the one warning a snapshot of a clipped strip could not give.
#[test]
fn the_section_strip_fits_the_frame() {
    let sections: Vec<Box<dyn SettingsSection>> = vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
    ];
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
/// D20) and the hint row is one line of the 100-column frame, so a notice and the keys share it.
/// What this case is about is that a failure with no steps *is* a notice, and a fixture long enough
/// to be clipped at the frame's edge would be asserting the width rather than the routing.
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
        rendered.contains("o open link \u{b7} x cancel"),
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
        rendered.contains("o open link \u{b7} x cancel"),
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

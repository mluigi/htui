//! The Agents table (MOD-115 M3 D7): its rows and cursor, the `on this box` and `quota` cells,
//! and the column constants they read.
//!
//! Since MOD-115 M3 it draws through the kit: a header in `theme.title`, then one `kit::list`
//! row per agent, the cursor's in the gutter and the band, every cell fitted to its column
//! ([`columns`]). The enum cells are `kit::label` words (MOD-83), never the wire values.

use chrono::{DateTime, Utc};
use htui_agent::probe::ProbeStatus;
use htui_core::model::{AgentBox, AgentSummary};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::app::Ctx;
use crate::ui::kit::label;
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::{Theme, cells};

use super::AgentsSection;

/// What the `on this box` column reads while this box has no `agent_box` row for the agent.
pub(super) const NOT_PROBED: &str = "not probed";

/// What the `on this box` column reads while a probe this section asked for is in flight.
pub(super) const PROBING: &str = "probing\u{2026}";

/// What replaces an absent `default_model`, and a quota this build has nothing to say about.
pub(super) const NONE: &str = "\u{2014}";

/// The `utilization` at which a window **is** full: `docs/ANA-4.md` §7's own threshold, and the
/// one [`available`](htui_core::model::quota::available) skips a row on (`>= 1.0`).
pub(super) const FULL: f64 = 1.0;

/// The highest percentage a window below [`FULL`] may render as, so the cell never claims an
/// allowance is exhausted while the predicate would still select it (review L-3).
pub(super) const NEARLY_FULL: f64 = 99.0;

/// What the `on this box` column reads for a row the human switched off on this box (MOD-23 D244).
/// Twelve characters, inside the 13-wide column.
pub(super) const SWITCHED_OFF: &str = "switched off";

/// What the `on this box` cell appends when `probe.source` is `manual` (MOD-66 D10, the
/// maintainer's answer to blueprint F-3). One character, so the 13-wide column keeps it beside
/// every verdict but `⚠ needs login`, which fills that column: then the column takes a cell from
/// `name` while `name` has one past [`NAME_MIN`], and where it has none the verdict is drawn as
/// its icon and the mark (`⚠*`, RV2 L5) while the idle note has room to say its word, else it
/// gives up its last letters rather than the mark (`⚠ needs log…*`); a version gives up its
/// last cells ([`fit_on_box`]). Either way the idle note's [`MANUAL_NOTE`] never names a mark the
/// table does not draw (repair 2, D-18).
pub(super) const MANUAL_SUFFIX: &str = "*";

/// What the idle note adds while any row's `probe.source` is `manual`: the key to
/// [`MANUAL_SUFFIX`] (MOD-66, the amendment to blueprint §5). 59 cells with
/// [`quota_note`](super::quota_note) and the default probe key, 75 with the [`note_word`] of a
/// `⚠*` cell after it.
pub(super) const MANUAL_NOTE: &str = " \u{b7} * manual path";

/// The `enabled` cell of a row the registry switches on, and of one it switches off: the form
/// toggle's glyphs and words (`kit::form`'s `Value::Toggle`).
const ON: &str = "\u{25cf} on";
/// See [`ON`].
const OFF: &str = "\u{25cb} off";

/// The narrowest the `name` column gets while a column is left to give way (MOD-81): 10 cells,
/// so `claude-cli`, the longest seeded name, is drawn whole.
pub(super) const NAME_MIN: u16 = 10;

/// One column of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Column {
    /// The flexible one: every cell the others leave (MOD-2 D89).
    Name,
    /// `label::transport`, 9 for its header.
    Transport,
    /// `label::billing`, 12 for `subscription`.
    Billing,
    /// How many models the row lists, 6 for its header.
    Models,
    /// The model id `docs/ANA-4.md` §4.4 selects by, 21 for `gemini-3.7-flash-high` (D76's #1).
    Default,
    /// The registry switch, `● on`/`○ off`: 6, one past `○ off`. Its header, `enabled` (the
    /// form's own word for the flag), is 7 and starts in the blank gap before the column
    /// ([`header_cells`]).
    Enabled,
    /// The tightest window, 13 for `100% to 09-08`.
    Quota,
    /// This box's verdict, 13 for `⚠ needs login` (D-18: never the donor).
    OnBox,
}

impl Column {
    /// Every column, in table order.
    pub(super) const ALL: [Self; 8] = [
        Self::Name,
        Self::Transport,
        Self::Billing,
        Self::Models,
        Self::Default,
        Self::Enabled,
        Self::Quota,
        Self::OnBox,
    ];

    /// The columns that give way below the width every column needs, in the order they go
    /// (MOD-81, the maintainer's answer of 2026-10-21): the count first, then the billing, then
    /// the transport. `default` and `quota` never do.
    const GIVE_WAY: [Self; 3] = [Self::Models, Self::Billing, Self::Transport];

    /// The header.
    ///
    /// `enabled` is one cell wider than its column since MOD-115 M3: `kit::list`'s gutter took one
    /// cell of the 98, and with every other column at its own longest string and `on this box`
    /// not the donor (D-18), the one cell `name` needed for its 10 came from this column, whose
    /// cells read `● on`/`○ off` in 5. The header keeps the word the create and edit forms use
    /// (`enabled (y/n)`) and borrows the gap before it ([`header_cells`]); `active` would claim the
    /// agent is live on this box, which a row switched off here contradicts in its next cell.
    pub(super) const fn header(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Transport => "transport",
            Self::Billing => "billing",
            Self::Models => "models",
            Self::Default => "default",
            Self::Enabled => "enabled",
            Self::Quota => "quota",
            Self::OnBox => "on this box",
        }
    }

    /// The fixed width, `None` for [`Column::Name`], which takes the rest.
    pub(super) const fn width(self) -> Option<u16> {
        match self {
            Self::Name => None,
            Self::Transport => Some(9),
            Self::Billing => Some(12),
            Self::Models | Self::Enabled => Some(6),
            Self::Default => Some(21),
            Self::Quota | Self::OnBox => Some(13),
        }
    }
}

/// The columns a row `width` cells wide holds, each with its width, one cell between each pair.
///
/// Every column while `name` keeps [`NAME_MIN`]; below that, [`Column::GIVE_WAY`] in order until
/// it does (MOD-81). `name` then takes every cell left, so a wider terminal still goes to it
/// first (D89); with all three gone it takes what is left, even under the minimum.
#[must_use]
pub(super) fn columns(width: u16) -> Vec<(Column, u16)> {
    let mut shown = Column::ALL.to_vec();
    for column in Column::GIVE_WAY {
        if name_room(width, &shown) >= NAME_MIN {
            break;
        }
        shown.retain(|shown| *shown != column);
    }
    let name = name_room(width, &shown);
    shown
        .into_iter()
        .map(|column| (column, column.width().unwrap_or(name)))
        .collect()
}

/// What `shown`'s fixed columns and the cells between them leave of `width` for `name`.
fn name_room(width: u16, shown: &[Column]) -> u16 {
    let fixed: u16 = shown.iter().filter_map(|column| column.width()).sum();
    let gaps = u16::try_from(shown.len().saturating_sub(1)).unwrap_or(u16::MAX);
    width.saturating_sub(fixed.saturating_add(gaps))
}

/// Lends `on this box` one of `name`'s cells while `name` has one past [`NAME_MIN`]: for a manual
/// row whose verdict fills the column, so `⚠ needs login*` draws whole (D-18, repair 2). A table
/// with no such row keeps D89's rule, every spare cell to `name`.
fn lend_on_box_a_cell(columns: &mut [(Column, u16)]) {
    let Some(name) = columns
        .iter()
        .position(|(column, width)| *column == Column::Name && *width > NAME_MIN)
    else {
        return;
    };
    let Some(on_box) = columns
        .iter()
        .position(|(column, _)| *column == Column::OnBox)
    else {
        return;
    };
    columns[name].1 -= 1;
    columns[on_box].1 += 1;
}

/// The header cells of `columns`: each header in its column, but for one a cell wider than its
/// column (`enabled` over the 6-cell [`Column::Enabled`]), which starts one cell early, in the gap
/// before it, when the header before it leaves its own last cell blank. The cells after it keep
/// their places, so every header still starts over its column or the gap just before it.
fn header_cells(columns: &[(Column, u16)]) -> Vec<(&'static str, u16)> {
    let mut headers: Vec<(&'static str, u16)> = columns
        .iter()
        .map(|&(column, width)| (column.header(), width))
        .collect();
    for index in 1..headers.len() {
        let (before, width_before) = headers[index - 1];
        let (header, width) = headers[index];
        if cells::cell_width(header) == usize::from(width) + 1
            && cells::cell_width(before) < usize::from(width_before)
        {
            headers[index - 1].1 -= 1;
            headers[index].1 += 1;
        }
    }
    headers
}

/// What one row's `on this box` cell says before it is fitted to its column
/// ([`AgentsSection::on_box_parts`]): the cell without its [`MANUAL_SUFFIX`], its style, whether
/// the suffix follows, and the probe verdict it reads, when it is one.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct OnBox {
    /// The cell's words.
    pub(super) text: String,
    /// Its style.
    pub(super) style: Style,
    /// A `manual` row's verdict: [`MANUAL_SUFFIX`] follows.
    pub(super) starred: bool,
    /// The verdict `text` says (`⚠ needs login`), `None` for a version or any other cell.
    pub(super) status: Option<ProbeStatus>,
}

/// An `on this box` cell fitted to its `width`, and whether it was drawn as its icon alone. A
/// `starred` (manual) verdict keeps its [`MANUAL_SUFFIX`], since the idle note's [`MANUAL_NOTE`]
/// promises the mark: a status verdict that does not fit beside it is its icon alone (`⚠*`, the
/// maintainer's answer, MOD-115 M3 RV2 L5) when `icon` says the idle note has room for its word,
/// else it gives up its last letters (`⚠ needs log…*`, RV2 repair 1), so the word is never off
/// the screen; a version gives up its last cells. Every other cell is `table_line`'s to fit.
#[must_use]
fn fit_on_box(cell: &OnBox, width: u16, icon: bool) -> (String, bool) {
    if !cell.starred {
        return (cell.text.clone(), false);
    }
    let star = cells::cell_width(MANUAL_SUFFIX);
    let width = usize::from(width);
    if cells::cell_width(&cell.text) + star <= width || width <= star {
        return (format!("{}{MANUAL_SUFFIX}", cell.text), false);
    }
    if icon && let Some(status) = cell.status {
        let glyph = label::probe(status).glyph;
        if cells::cell_width(glyph) + star <= width {
            return (format!("{glyph}{MANUAL_SUFFIX}"), true);
        }
    }
    let cut = cells::fit(&cell.text, width - star);
    (format!("{cut}{MANUAL_SUFFIX}"), false)
}

/// One table line: each cell fitted to its column (`cells::fit`: padded, or cut with `…`), one
/// blank between each pair.
fn table_line(cells: impl IntoIterator<Item = (String, u16, Style)>) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (text, width, style)) in cells.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(cells::fit(&text, usize::from(width)), style));
    }
    Line::from(spans)
}

/// A probe verdict as the section says it (MOD-83, D-18): its `kit::label` glyph and word.
#[must_use]
pub(super) fn verdict(status: ProbeStatus) -> String {
    let mark = label::probe(status);
    format!("{} {}", mark.glyph, mark.word)
}

/// What the idle note adds for a verdict the table drew as its icon alone (RV2 L5): ` · ` and
/// its [`verdict`].
#[must_use]
pub(super) fn note_word(status: ProbeStatus) -> String {
    format!(" \u{b7} {}", verdict(status))
}

impl AgentsSection {
    /// The highlighted row, or `None` while the table is empty.
    pub(super) fn selected(&self) -> Option<&AgentSummary> {
        self.cursor.and_then(|index| self.agents.get(index))
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, deliberately: `i` installs whatever the cursor is on, and a cursor that jumped
    /// from the last row to the first would make a held `j` install a row the user never aimed at.
    pub(super) fn move_cursor(&mut self, down: bool) {
        let Some(last) = self.agents.len().checked_sub(1) else {
            self.cursor = None;
            return;
        };
        let current = self.cursor.unwrap_or(0).min(last);
        let next = if down {
            current.saturating_add(1).min(last)
        } else {
            current.saturating_sub(1)
        };
        self.cursor = Some(next);
    }

    /// Puts the cursor back inside the table after a read replaced the rows.
    ///
    /// A fresh read selects the first row rather than nothing, so `i` has something to act on the
    /// moment the section is looked at.
    pub(super) fn clamp_cursor(&mut self) {
        self.cursor = self
            .agents
            .len()
            .checked_sub(1)
            .map(|last| self.cursor.unwrap_or(0).min(last));
    }

    /// What the `on this box` column says about one row, in the order plan D54 sets, and the
    /// style it says it in.
    ///
    /// An install this section is running wins over everything, because it is the one thing on
    /// screen the user is waiting on. Then a probe in flight, because none of the rows on screen
    /// answer the question they just asked. Then the per-box switch (MOD-23 D244): a row the human
    /// switched off reads `switched off` ahead of the probe's words, because the switch decides
    /// selection before they do. Then a box with no `agent_box` row at all, or a bare one the
    /// switch wrote that no probe has filled (`probed_at` and `probe` both absent), which is the
    /// same fact. Then the snapshot's own verdict when it is not `ready` — `missing`,
    /// `unauthenticated` and `failed` are the three facts a version string cannot express (plan
    /// D50), each drawn as its `kit::label` mark in its tone (`✗ missing`, `⚠ needs login`,
    /// `✗ failed`). Everything left is a row that works, or one written before the `probe` column
    /// existed, and both are answered by the version.
    ///
    /// A `manual` row's verdict ends in `*`, whatever it is (MOD-66 D10, B17): the status word or
    /// the version, never the install, login, probing, switched-off or not-probed cells, which are
    /// not the snapshot's verdict. Answered in parts ([`OnBox`]), so the table keeps the mark
    /// when it fits the cell to its column ([`fit_on_box`]).
    fn on_box_parts(&self, summary: &AgentSummary, theme: &Theme) -> OnBox {
        let plain = |text: String| OnBox {
            text,
            style: theme.dim,
            starred: false,
            status: None,
        };
        if let Some(cell) = self.install_cell(summary.agent.id) {
            return plain(cell);
        }
        // Beside the install and for its reason: a login this section is running is the one thing
        // on screen the user is waiting on, and it outlasts every other answer here.
        if let Some(cell) = self.auth_cell(summary.agent.id) {
            return plain(cell);
        }
        if self.probing {
            return plain(PROBING.to_owned());
        }
        if summary.user_off {
            return plain(SWITCHED_OFF.to_owned());
        }
        let Some(row) = summary.on_box.as_ref() else {
            return plain(NOT_PROBED.to_owned());
        };
        if row.probed_at.is_none() && row.probe.is_none() {
            return plain(NOT_PROBED.to_owned());
        }
        let status = row
            .probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(Value::as_str);
        // Read by key, as `status` always was: the three verdicts a version cannot express.
        let failed = [
            ProbeStatus::Missing,
            ProbeStatus::Unauthenticated,
            ProbeStatus::Failed,
        ]
        .into_iter()
        .find(|verdict| status == Some(verdict.as_str()));
        let (text, style) = if let Some(status) = failed {
            (
                verdict(status),
                label::tone_style(label::probe(status).tone, theme),
            )
        } else {
            let version = row.version.as_deref().unwrap_or(NONE);
            let cell = if row.enabled {
                version.to_owned()
            } else {
                format!("{version} (off)")
            };
            (cell, theme.dim)
        };
        OnBox {
            text,
            style,
            starred: is_manual(row),
            status: failed,
        }
    }

    /// Whether any listed row's cell shows `*`: the idle note then says what it means (MOD-66,
    /// the amendment to blueprint §5). Only a star [`on_box_parts`](Self::on_box_parts) can draw
    /// counts (review N1): none while `probing` covers every cell, and none on a row switched off
    /// here, whose cell reads `switched off`. The install and login cells need no test: the note
    /// is the idle one, so neither is running.
    pub(super) fn lists_a_manual_row(&self) -> bool {
        !self.probing
            && self
                .agents
                .iter()
                .any(|summary| !summary.user_off && summary.on_box.as_ref().is_some_and(is_manual))
    }

    /// One agent's row: its cells in `columns`' order, `name` in the base role (dim for a row
    /// the registry switches off) and the rest dim, but for the verdict's own tone.
    /// `on_box` is the row's [`on_box_parts`](Self::on_box_parts), worked out once per row
    /// ([`render_table`](Self::render_table), RV2 L3); `icon` says whether a verdict that does not
    /// fit may be drawn as its icon alone ([`fit_on_box`]).
    fn row(
        summary: &AgentSummary,
        on_box: &OnBox,
        columns: &[(Column, u16)],
        theme: &Theme,
        icon: bool,
    ) -> Line<'static> {
        let agent = &summary.agent;
        let style = if agent.enabled { theme.base } else { theme.dim };
        table_line(columns.iter().map(|&(column, width)| {
            let (text, style) = match column {
                Column::Name => (agent.name.clone(), style),
                Column::Transport => (label::transport(agent.transport).to_owned(), theme.dim),
                Column::Billing => (label::billing(agent.billing).to_owned(), theme.dim),
                Column::Models => (agent.models.len().to_string(), theme.dim),
                Column::Default => (
                    agent
                        .default_model
                        .clone()
                        .unwrap_or_else(|| NONE.to_owned()),
                    theme.dim,
                ),
                Column::Enabled => {
                    if agent.enabled {
                        (ON.to_owned(), style)
                    } else {
                        (OFF.to_owned(), style)
                    }
                }
                Column::Quota => (quota_cell(summary), theme.dim),
                Column::OnBox => (fit_on_box(on_box, width, icon).0, on_box.style),
            };
            (text, width, style)
        }))
    }

    /// The table (MOD-115 M3): the header in `theme.title` behind the gutter's blank, then the
    /// rows through `kit::list`, the cursor's in the gutter (`ctx.accent()`) and the band, scrolled
    /// the least that keeps it in view. The rows are rebuilt every frame and the offset is
    /// computed from 0 each time, as the `TableState` clone it replaces did, so the frame never
    /// depends on how often it was drawn.
    ///
    /// A manual verdict that does not fit is drawn as its icon alone (`⚠*`) only while the
    /// [`note_word`]s of all such verdicts fit in `room`, the cells the idle note leaves (0 while a
    /// notice holds the line or the section is not idle): else every one keeps its cut word
    /// (RV2 repair 1).
    ///
    /// Answers what it drew ([`Drawn`]), for the pane's scrollbar and the idle note.
    pub(super) fn render_table(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &Ctx<'_>,
        room: usize,
    ) -> Drawn {
        let theme = ctx.theme;
        if area.width < 2 || area.height == 0 {
            return Drawn {
                total: self.agents.len(),
                ..Drawn::default()
            };
        }
        let width = area.width - 1;
        let mut columns = columns(width);
        // Once per row (RV2 L3): the column's width and the row's cell both read it.
        let on_box: Vec<OnBox> = self
            .agents
            .iter()
            .map(|summary| self.on_box_parts(summary, theme))
            .collect();
        if on_box.iter().any(|cell| {
            cell.starred
                && cells::cell_width(&cell.text) + cells::cell_width(MANUAL_SUFFIX)
                    > Column::OnBox.width().map_or(0, usize::from)
        }) {
            lend_on_box_a_cell(&mut columns);
        }
        let header = table_line(
            header_cells(&columns)
                .into_iter()
                .map(|(header, width)| (header.to_owned(), width, theme.title)),
        );
        frame
            .buffer_mut()
            .set_line(area.x + 1, area.y, &header, width);

        let body = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };
        let mut icons = Vec::new();
        if let Some(width) = columns
            .iter()
            .find_map(|&(column, width)| (column == Column::OnBox).then_some(width))
        {
            for cell in &on_box {
                if let Some(status) = cell.status
                    && fit_on_box(cell, width, true).1
                    && !icons.contains(&status)
                {
                    icons.push(status);
                }
            }
        }
        let said: usize = icons
            .iter()
            .map(|status| cells::cell_width(&note_word(*status)))
            .sum();
        let icon = said <= room;
        if !icon {
            icons.clear();
        }
        let rows: Vec<Row<'static>> = self
            .agents
            .iter()
            .zip(&on_box)
            .map(|(summary, on_box)| Row {
                line: Self::row(summary, on_box, &columns, theme, icon),
                kind: RowKind::Item,
            })
            .collect();
        let offset = self.cursor.map_or(0, |cursor| {
            list::scroll_into_view(cursor, 0, usize::from(body.height), rows.len())
        });
        list::render(frame, body, &rows, self.cursor, offset, ctx.accent(), theme);
        Drawn {
            total: rows.len(),
            offset,
            body,
            icons,
        }
    }
}

/// What [`AgentsSection::render_table`] drew: its rows in all, the first one shown, the rect
/// they were drawn into (under the header), and the verdicts drawn as their icon alone, whose
/// words the idle note says (RV2 L5).
#[derive(Debug, Clone, Default)]
pub(super) struct Drawn {
    /// The table's rows.
    pub(super) total: usize,
    /// The first row shown.
    pub(super) offset: usize,
    /// The rows' rect, under the header.
    pub(super) body: Rect,
    /// The verdicts drawn as `⚠*`, each once.
    pub(super) icons: Vec<ProbeStatus>,
}

/// Whether an `agent_box` row's `probe.source` is `manual` (MOD-66 D10). Read by key, as
/// [`AgentsSection::on_box_parts`] reads `status`, so a snapshot this build cannot parse whole is
/// still marked.
pub(super) fn is_manual(row: &AgentBox) -> bool {
    row.probe
        .as_ref()
        .and_then(|probe| probe.get("source"))
        .and_then(Value::as_str)
        == Some("manual")
}

/// The `quota` column of one row (`R-TUI-8`, MOD-2 D73).
///
/// Read off `agent_box.quota` the way [`AgentsSection::on_box_parts`] reads `probe.status`: **by
/// key**, off a [`Value`]. That is what keeps this column free of the driver crate, and it is also
/// `R-AGT-5` — nothing here is keyed on an agent's name, only on what its row says.
///
/// Three answers and a fallback, in the order `docs/ANA-4.md` §7 describes the document: the
/// tightest window as a percentage with its reset when the blob has windows; the session spend
/// when it has only that (a `per_token` row has no allowance to be a fraction of, so what has been
/// spent is the only true thing to say); and [`NONE`] for a box with no `agent_box` row, a row with
/// no blob, or a blob this build does not understand. A vendor shape nobody has written a mapper
/// for is the last of those, and it renders as nothing rather than as a guess.
///
/// Refreshing is deliberately not offered: a probe handshake reports no allowance, so `r` cannot
/// (§7), and [`quota_note`] says so.
pub(super) fn quota_cell(summary: &AgentSummary) -> String {
    let Some(quota) = summary.on_box.as_ref().and_then(|row| row.quota.as_ref()) else {
        return NONE.to_owned();
    };
    if let Some((utilization, window)) = tightest_window(quota) {
        // **A window that is not full never reads `100%`** (review L-3). `{:.0}` alone rounds
        // `0.995..0.999` up to `100%`, and a cell saying `100%` says
        // [`available`](htui_core::model::quota::available) would skip this row — which it would
        // not, because that predicate's rule is `utilization >= 1.0` and this window is not full.
        // The two have to agree, and the honest side of the disagreement is the one that does not
        // announce an exhausted allowance the operator still has room in.
        //
        // A **cap** rather than the review's `floor` of the product, because a bare floor trades
        // this error for the one the rounding format was there to avoid: `0.57 * 100.0` is
        // `56.99999999999999289` in binary floating point, so `floor` reports a fifty-seven-percent
        // window as `56%`. Capping at 99 below the threshold is the same answer as `floor` for
        // every value the finding is about — `0.996` reads `99%` either way — and leaves every
        // other window reading as written.
        let scaled = utilization * 100.0;
        let percentage = if utilization < FULL {
            format!("{:.0}%", scaled.min(NEARLY_FULL))
        } else {
            format!("{scaled:.0}%")
        };
        return match reset_of(window) {
            Some(resets) => format!("{percentage} to {resets}"),
            None => percentage,
        };
    }
    quota
        .get("spend")
        .and_then(|spend| spend.get("session_micros"))
        .and_then(Value::as_i64)
        // The `usage_line` cast precedent (`chat/transcript.rs`): micros are money, and a figure
        // this column has room for is nowhere near `f64`'s exact range.
        .map_or_else(
            || NONE.to_owned(),
            |micros| format!("${:.2} spent", micros as f64 / 1_000_000.0),
        )
}

/// The window a row is closest to the end of: the highest `utilization`, and the lowest `id` of
/// the ones that tie.
///
/// The tie is broken on the `id` rather than on the array's own order so that two sources which
/// serialise the same allowance in a different order render the same cell. A window carrying no
/// numeric `utilization` is not one this column can render, so it is not a candidate at all —
/// which is also why a blob with `"windows": []` falls through to the spend.
pub(super) fn tightest_window(quota: &Value) -> Option<(f64, &Value)> {
    quota
        .get("windows")?
        .as_array()?
        .iter()
        .filter_map(|window| {
            window
                .get("utilization")
                .and_then(Value::as_f64)
                .map(|utilization| (utilization, window))
        })
        .reduce(|tightest, next| {
            let (utilization, window) = next;
            if utilization > tightest.0
                || (utilization == tightest.0 && window_id(window) < window_id(tightest.1))
            {
                next
            } else {
                tightest
            }
        })
}

/// A window's `id`, or the empty string for one that carries none — enough to break a tie with.
pub(super) fn window_id(window: &Value) -> &str {
    window.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// A window's `resets_at` as the column says it: `%m-%d`, in UTC.
///
/// The date alone since MOD-2 D76, which ranked the reset's clock time below the whole
/// `default_model` string and above nothing else: the six characters of ` %H:%M` are half of what
/// bought `default` its 21, so `62% to 09-08 08:00` now reads `62% to 09-08`. The year was already
/// left off, and for the same kind of reason — a reset a year out is not what this column warns
/// about.
///
/// Dropped by **format** and not by letting the 13-wide column clip it, because the clip is wrong
/// where it matters most: `100% to 09-08 08:00` cut at 13 reads `100% to 09-0`, a broken date on
/// the exhausted window this column exists for, while the formatted `100% to 09-08` is 13 exactly
/// and fits whole.
///
/// Hazard H-18: this parse runs on the UI task and the string comes from an agent, so
/// [`DateTime::parse_from_rfc3339`] is taken through `ok()`. A malformed one renders the
/// percentage alone, which is still the true half of the answer, and never brings a frame down.
pub(super) fn reset_of(window: &Value) -> Option<String> {
    let resets_at = window.get("resets_at")?.as_str()?;
    DateTime::parse_from_rfc3339(resets_at)
        .ok()
        .map(|resets_at| resets_at.with_timezone(&Utc).format("%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-81 (MOD-115 M3, the maintainer's answer of 2026-10-21): over every width from 60 to
    /// 160 the columns fit, `default`, `quota` and `on this box` keep their widths, and `models`,
    /// `billing` and `transport` give way in that order and only while `name` would be under 10.
    #[test]
    fn the_columns_give_way_in_order_and_keep_the_name_at_ten() {
        for section in 60..=160_u16 {
            // `kit::list`'s gutter takes one cell of the section.
            let width = section - 1;
            let shown = columns(width);
            let of = |wanted: Column| shown.iter().find(|(column, _)| *column == wanted);
            let name = of(Column::Name).map_or(0, |(_, width)| *width);
            let used: u16 =
                shown.iter().map(|(_, width)| width).sum::<u16>() + shown.len() as u16 - 1;
            for kept in [
                Column::Default,
                Column::Quota,
                Column::OnBox,
                Column::Enabled,
            ] {
                assert_eq!(
                    of(kept).map(|(_, width)| *width),
                    kept.width(),
                    "{section}: {kept:?} stays whole"
                );
            }
            let dropped: Vec<Column> = Column::GIVE_WAY
                .into_iter()
                .filter(|column| of(*column).is_none())
                .collect();
            assert_eq!(
                dropped.as_slice(),
                &Column::GIVE_WAY[..dropped.len()],
                "{section}: they give way in order"
            );
            if dropped.len() < Column::GIVE_WAY.len() {
                assert!(
                    name >= NAME_MIN,
                    "{section}: `name` keeps {NAME_MIN}: {shown:?}"
                );
            }
            if let Some(last) = dropped.last() {
                // The last one to go was needed: with it back, `name` would be under 10.
                let mut back: Vec<Column> = shown.iter().map(|(column, _)| *column).collect();
                back.push(*last);
                assert!(
                    name_room(width, &back) < NAME_MIN,
                    "{section}: {last:?} went early"
                );
            }
            if name > 0 {
                assert_eq!(
                    used, width,
                    "{section}: `name` takes every cell left: {shown:?}"
                );
            }
        }
    }

    /// The two widths the snapshots pin: every column at the bordered 98 (`name` 10), and at 78
    /// (an 80-column frame) `models` and `billing` gone, `name` 10 again.
    #[test]
    fn the_snapshot_widths_pack_as_pinned() {
        let all = columns(97);
        assert_eq!(all.len(), 8);
        assert_eq!(all[0], (Column::Name, NAME_MIN));
        let narrow = columns(77);
        assert_eq!(
            narrow
                .iter()
                .map(|(column, _)| column.header())
                .collect::<Vec<_>>(),
            [
                "name",
                "transport",
                "default",
                "enabled",
                "quota",
                "on this box"
            ]
        );
        assert_eq!(narrow[0], (Column::Name, NAME_MIN));
    }

    /// Repair 2 (D-18): a manual verdict keeps its `*` at any width; an unmarked cell is left to
    /// `table_line`. RV2 L5 (the maintainer's answer): a status verdict that does not fit beside
    /// its `*` is its icon alone (`⚠*`), never a cut word, and says so; a version that does not
    /// fit still gives up its last cells.
    #[test]
    fn a_manual_verdict_keeps_its_star() {
        let status = |status: ProbeStatus| OnBox {
            text: verdict(status),
            style: Style::new(),
            starred: true,
            status: Some(status),
        };
        let login = status(ProbeStatus::Unauthenticated);
        assert_eq!(
            fit_on_box(&login, 14, true),
            ("\u{26a0} needs login*".to_owned(), false)
        );
        assert_eq!(fit_on_box(&login, 13, true), ("\u{26a0}*".to_owned(), true));
        // RV2 repair 1: with no room in the note for its word, the cell keeps the word, cut.
        assert_eq!(
            fit_on_box(&login, 13, false),
            ("\u{26a0} needs log\u{2026}*".to_owned(), false)
        );
        let version = |text: &str| OnBox {
            text: text.to_owned(),
            style: Style::new(),
            starred: true,
            status: None,
        };
        assert_eq!(
            fit_on_box(&version("0.48.0"), 13, true),
            ("0.48.0*".to_owned(), false)
        );
        assert_eq!(
            fit_on_box(&version("0.48.0-nightly-2026"), 13, true),
            ("0.48.0-nigh\u{2026}*".to_owned(), false)
        );
        let unmarked = OnBox {
            starred: false,
            ..login
        };
        assert_eq!(
            fit_on_box(&unmarked, 13, true),
            (unmarked.text.clone(), false)
        );
    }

    /// Repair 2 (finding 4): `enabled` starts in the gap before its column, taking the cell the
    /// blank end of `default`'s header leaves; every header after it keeps its place.
    #[test]
    fn the_enabled_header_borrows_the_gap_before_it() {
        let shown = columns(97);
        let headers = header_cells(&shown);
        for (index, ((column, width), (header, drawn))) in shown.iter().zip(&headers).enumerate() {
            assert_eq!(*header, column.header());
            let expected = match column {
                Column::Default => width - 1,
                Column::Enabled => width + 1,
                _ => *width,
            };
            assert_eq!(*drawn, expected, "{index}: {column:?}");
        }
    }

    /// D-18: every verdict the column can show fits its 13 cells, `⚠ needs login` exactly.
    #[test]
    fn every_verdict_fits_the_on_this_box_column() {
        for status in ProbeStatus::ALL {
            assert!(cells::cell_width(&verdict(*status)) <= 13, "{status:?}");
        }
        assert_eq!(
            verdict(ProbeStatus::Unauthenticated),
            "\u{26a0} needs login"
        );
    }
}

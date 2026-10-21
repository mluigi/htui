//! The Agents table (MOD-115 M3 D7): its rows and cursor, the `on this box` and `quota` cells,
//! and the column constants they read.

use chrono::{DateTime, Utc};
use htui_core::model::{AgentBox, AgentSummary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Row, Table};
use serde_json::Value;

use crate::app::Ctx;

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
/// maintainer's answer to blueprint F-3). One character, so the 13-wide column keeps it on every
/// verdict but `unauthenticated`, which that column already clips.
pub(super) const MANUAL_SUFFIX: &str = "*";

/// What the idle note adds while any row's `probe.source` is `manual`: the key to
/// [`MANUAL_SUFFIX`] (MOD-66, the amendment to blueprint §5). 60 of the 98 columns with
/// [`quota_note`].
pub(super) const MANUAL_NOTE: &str = " \u{b7} * manual path";

impl AgentsSection {
    /// The highlighted row, or `None` while the table is empty.
    pub(super) fn selected(&self) -> Option<&AgentSummary> {
        self.cursor
            .selected()
            .and_then(|index| self.agents.get(index))
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, deliberately: `i` installs whatever the cursor is on, and a cursor that jumped
    /// from the last row to the first would make a held `j` install a row the user never aimed at.
    pub(super) fn move_cursor(&mut self, down: bool) {
        let Some(last) = self.agents.len().checked_sub(1) else {
            self.cursor.select(None);
            return;
        };
        let current = self.cursor.selected().unwrap_or(0).min(last);
        let next = if down {
            current.saturating_add(1).min(last)
        } else {
            current.saturating_sub(1)
        };
        self.cursor.select(Some(next));
    }

    /// Puts the cursor back inside the table after a read replaced the rows.
    ///
    /// A fresh read selects the first row rather than nothing, so `i` has something to act on the
    /// moment the section is looked at.
    pub(super) fn clamp_cursor(&mut self) {
        match self.agents.len().checked_sub(1) {
            Some(last) => self
                .cursor
                .select(Some(self.cursor.selected().unwrap_or(0).min(last))),
            None => self.cursor.select(None),
        }
    }

    /// What the `on this box` column says about one row, in the order plan D54 sets.
    ///
    /// An install this section is running wins over everything, because it is the one thing on
    /// screen the user is waiting on. Then a probe in flight, because none of the rows on screen
    /// answer the question they just asked. Then the per-box switch (MOD-23 D244): a row the human
    /// switched off reads `switched off` ahead of the probe's words, because the switch decides
    /// selection before they do. Then a box with no `agent_box` row at all, or a bare one the
    /// switch wrote that no probe has filled (`probed_at` and `probe` both absent), which is the
    /// same fact. Then the snapshot's own verdict when it is not `ready` — `missing`,
    /// `unauthenticated` and `failed` are the three facts a version string cannot express (plan
    /// D50). Everything left is a row that works, or one written before the `probe` column
    /// existed, and both are answered by the version.
    ///
    /// A `manual` row's verdict ends in `*`, whatever it is (MOD-66 D10, B17): the status word or
    /// the version, never the install, login, probing, switched-off or not-probed cells, which are
    /// not the snapshot's verdict.
    pub(super) fn on_box_cell(&self, summary: &AgentSummary) -> String {
        if let Some(cell) = self.install_cell(summary.agent.id) {
            return cell;
        }
        // Beside the install and for its reason: a login this section is running is the one thing
        // on screen the user is waiting on, and it outlasts every other answer here.
        if let Some(cell) = self.auth_cell(summary.agent.id) {
            return cell;
        }
        if self.probing {
            return PROBING.to_owned();
        }
        if summary.user_off {
            return SWITCHED_OFF.to_owned();
        }
        let Some(row) = summary.on_box.as_ref() else {
            return NOT_PROBED.to_owned();
        };
        if row.probed_at.is_none() && row.probe.is_none() {
            return NOT_PROBED.to_owned();
        }
        let status = row
            .probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(Value::as_str);
        let cell = if let Some(status @ ("missing" | "unauthenticated" | "failed")) = status {
            status.to_owned()
        } else {
            let version = row.version.as_deref().unwrap_or(NONE);
            if row.enabled {
                version.to_owned()
            } else {
                format!("{version} (off)")
            }
        };
        if is_manual(row) {
            format!("{cell}{MANUAL_SUFFIX}")
        } else {
            cell
        }
    }

    /// Whether any listed row's cell shows `*`: the idle note then says what it means (MOD-66,
    /// the amendment to blueprint §5). Only a star [`on_box_cell`](Self::on_box_cell) can draw
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

    /// The eight-column table, with the cursor row accented.
    pub(super) fn render_table(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let rows = self.agents.iter().map(|summary| {
            let agent = &summary.agent;
            let quota = quota_cell(summary);
            let on_box = self.on_box_cell(summary);
            let style = if agent.enabled {
                ctx.theme.base
            } else {
                ctx.theme.dim
            };
            Row::new(vec![
                Cell::from(Line::styled(agent.name.clone(), style)),
                Cell::from(Line::styled(agent.transport.as_str(), ctx.theme.dim)),
                Cell::from(Line::styled(agent.billing.as_str(), ctx.theme.dim)),
                Cell::from(Line::styled(agent.models.len().to_string(), ctx.theme.dim)),
                Cell::from(Line::styled(
                    agent
                        .default_model
                        .clone()
                        .unwrap_or_else(|| NONE.to_owned()),
                    ctx.theme.dim,
                )),
                Cell::from(Line::styled(
                    if agent.enabled { "yes" } else { "no" },
                    style,
                )),
                Cell::from(Line::styled(quota, ctx.theme.dim)),
                Cell::from(Line::styled(on_box, ctx.theme.dim)),
            ])
        });

        let header = Row::new(vec![
            Cell::from("name"),
            Cell::from("transport"),
            Cell::from("billing"),
            Cell::from("models"),
            Cell::from("default"),
            Cell::from("enabled"),
            Cell::from("quota"),
            Cell::from("on this box"),
        ])
        .style(ctx.theme.title);

        // MOD-2 D89's packing, which **reverses D76's ranking**: `name` takes the slack, and
        // `on this box` is the donor that pays for it.
        //
        // **`name` has no width of its own to tune, and that is the decision.** `Fill(1)` takes
        // every column the seven fixed ones leave, so the answer to "how wide should a name be" is
        // "as wide as the terminal allows" rather than a constant somebody chose. D76 had it as
        // `Length(8)` — a number picked to fit the longest name *then in the tree*, which is a
        // constant that goes stale the next time a registry row is added, and did: `claude-cli` is
        // 10.
        //
        // The instruction that produced this was "wider, like 128", and then "increase it to
        // maximum or like 256, there is no need to optimize the length". Both are satisfied here,
        // and the measured reason a literal 256 is *not* what the code says is worth recording,
        // because it is counter-intuitive: as a `Min` it does not widen this column, it **deletes
        // the other seven**. `Min(256)` and `Min(128)` both resolve to `[91, 0, 0, 0, 0, 0, 0, 0]`
        // at the bordered 98 — one column of names and seven of nothing — and `Min(128)` is still
        // `[128, 4, 3, 4, 3, 4, 3, 4]` at 160. Measured against ratatui 0.30.2's solver, not
        // reasoned about. As a `Max`, 256 is a no-op: `Max(256)` and `Fill(1)` resolve identically
        // at every width, so `Fill(1)` is the same behaviour written without a number that invites
        // a reader to think it means something.
        //
        // What `Fill(1)` gives at each width, which is the whole of the intent: 10 at the bordered
        // 98, 12 at 100, 32 at 120, 72 at 160, 112 at 200. **Every character a wider terminal adds
        // goes here first**, without a cap of any kind.
        //
        // Below ~98 the column yields rather than starving its neighbours, and can reach 0. That is
        // the one thing a `Min(10)` floor would have bought, and it is not bought here: at those
        // widths the table is already past the point of being readable — `default` gets 6 columns
        // for a 21-character model id — so a floor would preserve the *name* of a row whose every
        // other cell had become nonsense. The section is drawn inside a pane the app does not offer
        // below a usable size.
        //
        // The seven fixed columns each sit at their own longest string, which is why they are the
        // ones that do not move: `transport` (9), `models` (6) and `enabled` (7) at their headers,
        // `billing` (12) at `subscription`, `default` (21) at `gemini-3.7-flash-high` — the model id
        // `docs/ANA-4.md` §4.4 selects by, and D76's #1, which D89 leaves untouched — `quota` (13)
        // at `100% to 09-08`, the exhausted window that column exists to warn about, and
        // `on this box` (13), the donor.
        //
        // **The donor's price is stated rather than discovered.** At 13, its two longest cells clip:
        // `unauthenticated` (15) renders `unauthenticat` and `choose a method` (15) renders
        // `choose a meth`. Both are MOD-21's login flow written into a cell and the loss is real.
        // It is the affordable one because this column carries *status words*, where `default`
        // carries a selection coordinate and `quota` carries an exhaustion warning, and a truncated
        // status word is still legible as itself. D76 had already ranked its slack last for that
        // reason. The `a` key is still offered on the row.
        //
        // A number to be sceptical of if you are reading this to add a ninth column: the 98 that
        // every "at the bordered width" claim above refers to is **the test harness**, not a
        // screen. `testkit.rs`'s `DEFAULT_SIZE` is 100x30 and the pane's border costs two of it.
        // Nobody runs `htui` at 98 columns. It is the width the snapshots are pinned to, so it is
        // the width a regression is *caught* at, which is worth something — but it is not the width
        // the packing should be designed for, and D76's ranking was tighter than it needed to be
        // because it treated a fixture constant as a constraint.
        let table = Table::new(
            rows,
            [
                Constraint::Fill(1),
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(21),
                Constraint::Length(7),
                Constraint::Length(13),
                Constraint::Length(13),
            ],
        )
        .header(header)
        .row_highlight_style(ctx.theme.accent);

        // The cursor is cloned because `render` takes `&self`: ratatui writes the scroll offset it
        // computed back into the state, and a view that mutated during a draw would be a view
        // whose frame depends on how often it was drawn.
        frame.render_stateful_widget(table, area, &mut self.cursor.clone());
    }
}

/// Whether an `agent_box` row's `probe.source` is `manual` (MOD-66 D10). Read by key, as
/// [`AgentsSection::on_box_cell`] reads `status`, so a snapshot this build cannot parse whole is
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
/// Read off `agent_box.quota` the way [`AgentsSection::on_box_cell`] reads `probe.status`: **by
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

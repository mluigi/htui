//! The rounded pane: title, sub-tab chips, bottom-border hints, focus and a scrollbar (MOD-115).
//!
//! A focused pane's border takes the tab accent, any other the faint border role. The title sits
//! on the top border after the corner, the sub-tab chips flush with the right corner, the hints
//! on the bottom border two cells in; none of them ever overwrites a corner. When the pane is
//! narrow the title is clipped first, and gives way entirely once any chip is cut off (MOD-115
//! M3 D-10). The strip then keeps the active chip and marks the chips cut off with `‹`/`›`
//! ([`chip_strip`]), and the hints drop whole keys from their end, ending in ` …`
//! ([`fit_hints`], MOD-115 M4 T3): never half a key. A pane that asks for its title first
//! ([`Pane::title_first`], M4 C5) keeps the title and windows the chips in what is left
//! ([`chip_strip_after`]).
//!
//! [`Pane::border`] fixes the border's style whatever the focus, and [`Pane::render_into`] draws
//! on a bare buffer, for a pane drawn inside another widget (MOD-115 M4 D-16).
//!
//! A view that needs a number only its content knows (a scrollbar's total) draws its content
//! into [`Pane::inner`] first and the pane after it: the frame touches border cells only
//! (MOD-115 M3 C3). [`Pane::scrollbar_span`] keeps the scrollbar beside some rows only.
//!
//! Every style is a [`Theme`] role, so the monochrome theme keeps focus visible: a focused border
//! is bold, an unfocused one dim.
//!
//! [`paint_bottom_hints`] puts a pane's hints on its bottom rule (left) and [`paint_bottom_note`]
//! a hidden pane's note (right): the note is the shell's (R-TUI-12), painted on the bottom rule
//! of the pane a tab drew, after the tab drew it (MOD-115 M2 D3).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use crate::app::Ctx;
use crate::ui::shell::{PaneAreas, PaneId};
use crate::ui::{Theme, cells};

/// The scrollbar's thumb: heavy, so it reads over the light vertical border it replaces.
const THUMB: &str = "\u{2503}";

/// Whether a border cell is a horizontal rule, the only cell [`paint_bottom_hints`] may overwrite.
fn is_rule(symbol: &str) -> bool {
    matches!(symbol, "\u{2500}" | "\u{2501}")
}

/// Whether a pane has focus: the focused border takes the tab accent, the other the faint one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The pane keys go to.
    Focused,
    /// Any other pane.
    Unfocused,
}

/// A pane under construction; [`Pane::render`] draws it.
#[derive(Debug, Clone)]
pub struct Pane<'a> {
    title: &'a str,
    sub_tabs: &'a [&'a str],
    active: usize,
    hints: Option<Line<'a>>,
    note: Option<Line<'a>>,
    focus: Focus,
    accent: Option<Style>,
    scrollbar: Option<(usize, usize)>,
    span: Option<Rect>,
    title_first: bool,
    border: Option<Style>,
}

impl<'a> Pane<'a> {
    /// A pane titled `title`, unfocused, with no chips, hints or scrollbar.
    #[must_use]
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            sub_tabs: &[],
            active: 0,
            hints: None,
            note: None,
            focus: Focus::Unfocused,
            accent: None,
            scrollbar: None,
            span: None,
            title_first: false,
            border: None,
        }
    }

    /// The title keeps its room before the chips do (MOD-115 M4 C5, D-4): it is drawn whole
    /// when it leaves the active chip and its marks room, and clipped with `…` to what they
    /// leave otherwise; the chips window around the active one in the rest
    /// ([`chip_strip_after`]). Without it the title gives way first (M3 D-10).
    #[must_use]
    pub fn title_first(mut self) -> Self {
        self.title_first = true;
        self
    }

    /// The border's style whatever the focus (a Runs flow node coloured by its status): it
    /// replaces both the accent and the faint role.
    #[must_use]
    pub fn border(mut self, style: Style) -> Self {
        self.border = Some(style);
        self
    }

    /// Sub-tab chips on the top border, `active` being the current one.
    #[must_use]
    pub fn sub_tabs(mut self, titles: &'a [&'a str], active: usize) -> Self {
        self.sub_tabs = titles;
        self.active = active;
        self
    }

    /// Hints on the bottom border, drawn only while the pane is focused.
    #[must_use]
    pub fn hints(mut self, hints: Line<'a>) -> Self {
        self.hints = Some(hints);
        self
    }

    /// A note on the bottom border, right-aligned and drawn focused or not: what a hidden pane
    /// says about bringing it back (`R-TUI-12`). It takes only the rule the hints left, less
    /// the one rule cell that parts it from them.
    #[must_use]
    pub fn note(mut self, note: Line<'a>) -> Self {
        self.note = Some(note);
        self
    }

    /// Sets the focus.
    #[must_use]
    pub fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    /// The focused border's style (the tab accent); without it the theme's focus accent.
    #[must_use]
    pub fn accent(mut self, accent: Style) -> Self {
        self.accent = Some(accent);
        self
    }

    /// A scrollbar for `total` rows scrolled to `offset`.
    #[must_use]
    pub fn scrollbar(mut self, total: usize, offset: usize) -> Self {
        self.scrollbar = Some((total, offset));
        self
    }

    /// Keeps the scrollbar beside the rows of `rows` only (Chat's transcript, between its banner
    /// and its composer), measured against their height. `rows` is an absolute rect, the one the
    /// scrolled content was drawn into; it is cut to the inner rect, so a rect past it loses the
    /// rows it does not share. The default is the whole inner rect.
    #[must_use]
    pub fn scrollbar_span(mut self, rows: Rect) -> Self {
        self.span = Some(rows);
        self
    }

    /// [`scrollbar`](Self::scrollbar) and, when the span names its rows,
    /// [`scrollbar_span`](Self::scrollbar_span), from one [`ScrollSpan`].
    #[must_use]
    pub fn scroll(self, span: ScrollSpan) -> Self {
        let pane = self.scrollbar(span.total, span.offset);
        match span.rows {
            Some(rows) => pane.scrollbar_span(rows),
            None => pane,
        }
    }

    /// The inner rect a pane drawn into `area` gives its content, without drawing it: what
    /// [`Pane::render`] returns. Content drawn there before the pane survives it (MOD-115 M3 C3).
    #[must_use]
    pub fn inner(area: Rect) -> Rect {
        Block::bordered().inner(area)
    }

    /// Draws the pane into `area` and returns the inner rect for its content. The scrollbar, if
    /// any, is measured against that rect's height (or its [`span`](Self::scrollbar_span)) and
    /// replaces part of the right border.
    pub fn render(self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
        self.render_into(frame.buffer_mut(), area, theme)
    }

    /// [`Pane::render`] on a bare buffer (MOD-115 M4 D-16): what a widget drawn inside another
    /// widget's `render` (a Runs flow node) has.
    pub fn render_into(self, buf: &mut Buffer, area: Rect, theme: &Theme) -> Rect {
        let focused = self.focus == Focus::Focused;
        let border = match (self.border, focused) {
            (Some(border), _) => border,
            (None, true) => self.accent.unwrap_or(theme.accent),
            (None, false) => theme.border_faint,
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border);
        let inner = Self::inner(area);
        block.render(area, buf);
        // The border rows and the right edge are written cell by cell below: a pane whose rect
        // runs past the buffer draws only what lies on it (RV1 L4).
        let top_on = area.y < buf.area.bottom() && area.y >= buf.area.y;
        if area.width < 2 || area.height == 0 || area.intersection(buf.area).is_empty() {
            return inner;
        }

        // The chips and marks are patched over the border, bold when focused in monochrome: the
        // border's bold is taken off all but the active chip, so only it reads bold (MOD-80 D5).
        let faint = theme.dim.remove_modifier(Modifier::BOLD);
        let strip = if self.title_first {
            chip_strip_after(area, self.title, self.sub_tabs, self.active)
        } else {
            chip_strip(area, self.sub_tabs, self.active)
        };
        let limit = strip_limit(area, &strip);
        let mut cut = false;
        for (index, (rect, title)) in strip.chips.iter().zip(self.sub_tabs).enumerate() {
            let wanted = cells::cell_width(title) + 2;
            if usize::from(rect.width) < wanted {
                cut = true;
            }
            if rect.width == 0 {
                continue;
            }
            let label_style = if index == self.active {
                theme.active_tab
            } else {
                faint
            };
            if !top_on {
                continue;
            }
            // A chip that fits is drawn from the borrowed title; only a clipped one is built.
            if usize::from(rect.width) >= wanted && plain(title) {
                put(buf, rect.x, rect.y, " ", rect.width, Style::new());
                put(buf, rect.x + 1, rect.y, title, rect.width - 1, label_style);
                put(buf, rect.right() - 1, rect.y, " ", 1, Style::new());
            } else {
                let chip = Line::from(vec![
                    Span::raw(" "),
                    Span::styled(*title, label_style),
                    Span::raw(" "),
                ]);
                let chip = cells::clip_line(chip, rect.width);
                set_line(buf, rect.x, rect.y, &chip, rect.width);
            }
        }
        for (mark, glyph) in [(strip.before, BEFORE), (strip.after, AFTER)] {
            if let Some(rect) = mark
                && top_on
            {
                put(buf, rect.x, rect.y, glyph, 1, faint);
            }
        }

        // The title gives way before any chip does (MOD-115 M3 D-10): once one is cut off, the
        // active chip carries the name and the title is not drawn at all. Asked first (M4 C5),
        // it keeps the cells `chip_strip_after` left it, whatever was cut.
        if !self.title.is_empty() && (self.title_first || !cut) && top_on {
            let start = area.x + 2;
            let room = if self.title_first {
                first_title_room(area, self.title, self.sub_tabs, self.active, &strip)
            } else {
                limit.saturating_sub(1).saturating_sub(start)
            };
            if room > 2 {
                let style = if focused {
                    border.add_modifier(Modifier::BOLD)
                } else {
                    theme.dim
                };
                // ` title `, built only when it has to be clipped.
                let width = cells::cell_width(self.title) + 2;
                if width <= usize::from(room) && plain(self.title) {
                    put(buf, start, area.y, " ", 1, style);
                    put(buf, start + 1, area.y, self.title, room - 1, style);
                    let end = start + u16::try_from(width).unwrap_or(room) - 1;
                    put(buf, end, area.y, " ", 1, style);
                } else {
                    let text = cells::clip(&format!(" {} ", self.title), usize::from(room));
                    put(buf, start, area.y, &text, room, style);
                }
            }
        }

        // The hints are painted over the rule cells they take and no other: a hint line's own
        // style (`chips::key_line`'s `theme.dim`) must not restyle the rest of the bottom rule,
        // nor stack on the border's bold in monochrome (RV1 H1).
        if focused
            && let Some(hints) = self.hints
            && area.height >= 2
        {
            let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
            paint_bottom_hints(buf, row, hints, Style::new());
        }

        if let Some(note) = self.note
            && area.height >= 2
        {
            let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
            paint_bottom_note(buf, row, note, theme.dim);
        }

        if let Some((total, offset)) = self.scrollbar {
            let rows = self.span.map_or(inner, |rows| rows.intersection(inner));
            paint_scrollbar(buf, area, rows, total, offset, border);
        }
        inner
    }
}

/// How the pane that holds the keyboard (a prompt, a form, an editor, a compose area, a pane
/// drawn over the content) is drawn (MOD-115 M4 C4): focused unless something covers the tab
/// (`ctx.pane_focus`), or the shell's activity pane is on screen (`activity`) and has the focus.
/// The Backlog's and the Skills tab's one rule (RV3-L4).
#[must_use]
pub fn holder(ctx: &Ctx<'_>, activity: bool) -> Focus {
    let focus = ctx.focus();
    if activity && focus == PaneId::Activity {
        Focus::Unfocused
    } else {
        ctx.pane_focus(focus)
    }
}

/// MOD-115 M2 §7 (T7 repair): where a list + detail tab draws its list and its detail. While
/// what the detail shows takes every key (`capturing`: a form, a prompt, a compose area) and
/// another pane is zoomed, so the detail's rect is empty, the detail takes the whole `area` and
/// the list nothing: what is typed is on screen. Otherwise the panes the layout gave.
#[must_use]
pub fn capture_area(areas: &PaneAreas, area: Rect, capturing: bool) -> (Option<Rect>, Rect) {
    if areas.detail.is_empty() && capturing {
        (None, area)
    } else {
        (areas.list, areas.detail)
    }
}

/// What scrolls in a view, for its pane's scrollbar (MOD-115 M3 RV1 M2): `total` rows scrolled
/// to `offset`, beside `rows` (absolute, as drawn) or, when `None`, the whole inner rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollSpan {
    /// The rows in all.
    pub total: usize,
    /// The first row shown.
    pub offset: usize,
    /// The rect the rows are drawn into; `None` is the pane's whole inner rect.
    pub rows: Option<Rect>,
}

impl ScrollSpan {
    /// `total` rows scrolled to `offset`, beside the whole inner rect.
    #[must_use]
    pub const fn new(total: usize, offset: usize) -> Self {
        Self {
            total,
            offset,
            rows: None,
        }
    }

    /// Beside `rows` only: the rect the scrolled rows are drawn into.
    #[must_use]
    pub const fn rows(mut self, rows: Rect) -> Self {
        self.rows = Some(rows);
        self
    }
}

/// The thumb on the right border beside `rows` for `total` rows scrolled to `offset`; nothing
/// when they all fit.
fn paint_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    rows: Rect,
    total: usize,
    offset: usize,
    style: Style,
) {
    let viewport = usize::from(rows.height);
    if viewport == 0 || total <= viewport || area.width < 2 {
        return;
    }
    let offset = clamp_offset(offset, total, viewport);
    let length = (viewport * viewport / total).clamp(1, viewport);
    let travel = viewport - length;
    let max = total - viewport;
    let top = (offset * travel + max / 2) / max;
    let x = area.right() - 1;
    for row in top..top + length {
        let y = rows
            .y
            .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
        if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
            cell.set_symbol(THUMB).set_style(style);
        }
    }
}

/// Whether `text` holds no control character, so it draws as it is, without
/// `cells::clip`'s flattening (a control character is one blank cell there).
fn plain(text: &str) -> bool {
    !text.chars().any(char::is_control)
}

/// `text` at `(x, y)`, at most `width` cells, when that cell is on `buf`; else nothing.
fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, width: u16, style: Style) {
    if buf.area.contains(Position::new(x, y)) {
        buf.set_stringn(x, y, text, usize::from(width), style);
    }
}

/// [`Buffer::set_line`] when `(x, y)` is on `buf`; else nothing.
fn set_line(buf: &mut Buffer, x: u16, y: u16, line: &Line<'_>, width: u16) {
    if buf.area.contains(Position::new(x, y)) {
        buf.set_line(x, y, line, width);
    }
}

/// Where each sub-tab chip sits on the top border of a pane drawn into `area`, one rect per title,
/// `active` being the current one: [`chip_strip`]'s chips. A chip cut off gets a zero-width rect
/// no click hits.
#[must_use]
pub fn sub_tab_rects(area: Rect, titles: &[&str], active: usize) -> Vec<Rect> {
    chip_strip(area, titles, active).chips
}

/// The leftmost cell a drawn chip or mark of `strip` takes on the top border of a pane drawn into
/// `area`, else the right corner: where a title must stop. A chip cut to nothing takes none.
fn strip_limit(area: Rect, strip: &ChipStrip) -> u16 {
    strip
        .chips
        .iter()
        .filter(|rect| rect.width > 0)
        .map(|rect| rect.x)
        .chain(
            [strip.before, strip.after]
                .into_iter()
                .flatten()
                .map(|rect| rect.x),
        )
        .fold(area.right() - 1, u16::min)
}

/// The cells [`Pane::render`] gives a title asked first ([`Pane::title_first`]) beside `strip`,
/// the strip [`chip_strip_after`] lays out for it.
fn first_title_room(
    area: Rect,
    title: &str,
    titles: &[&str],
    active: usize,
    strip: &ChipStrip,
) -> u16 {
    title_cells(area, title, titles, active)
        .min(strip_limit(area, strip).saturating_sub(area.x + 2))
}

impl Pane<'_> {
    /// Whether a pane drawn into `area`, `title` asked first ([`Pane::title_first`]) beside the
    /// sub-tabs `titles` with `active` the current one, draws ` title ` whole on its top border
    /// (MOD-115 M4 RV2-L7): what [`Pane::render`] decides, so a caller that keeps a cut title's
    /// line of its own (the Backlog's Body) asks the kit rather than mirroring its arithmetic.
    #[must_use]
    pub fn title_fits(area: Rect, title: &str, titles: &[&str], active: usize) -> bool {
        if title.is_empty() || area.width < 2 || area.height == 0 {
            return false;
        }
        let strip = chip_strip_after(area, title, titles, active);
        let room = first_title_room(area, title, titles, active, &strip);
        room > 2 && cells::cell_width(title) + 2 <= usize::from(room)
    }
}

/// The cells a title asked first takes on the top border of a pane drawn into `area` (MOD-115 M4
/// C5): ` title ` whole, or what the active chip and the marks it alone needs leave, less the
/// rule cell after the corner. An empty title takes none.
fn title_cells(area: Rect, title: &str, titles: &[&str], active: usize) -> u16 {
    if title.is_empty() {
        return 0;
    }
    let avail = area.width.saturating_sub(2);
    let reserve = match titles.len() {
        0 => 0,
        count => {
            let active = active.min(count - 1);
            let chip = cells::cell_width(titles[active]).saturating_add(2);
            let marks = usize::from(active > 0) + usize::from(active < count - 1);
            u16::try_from(chip + marks).unwrap_or(u16::MAX)
        }
    };
    let wanted = u16::try_from(cells::cell_width(title).saturating_add(2)).unwrap_or(u16::MAX);
    wanted.min(avail.saturating_sub(reserve).saturating_sub(1))
}

/// The sub-tab strip of a pane drawn into `area` whose `title` is asked first
/// ([`Pane::title_first`], MOD-115 M4 C5): [`chip_strip`]'s window, grown right then left from
/// `active`, in the cells right of the title. What [`Pane::render`] draws then, for a future hit
/// test. With no title, or no room for one, it is [`chip_strip`].
#[must_use]
pub fn chip_strip_after(area: Rect, title: &str, titles: &[&str], active: usize) -> ChipStrip {
    let cells = title_cells(area, title, titles, active);
    if cells == 0 {
        return chip_strip(area, titles, active);
    }
    // The strip's own left "corner" is the title's last cell, so its cells between the corners
    // are the pane's less the rule cell after the corner and the title.
    let left = area.x.saturating_add(1).saturating_add(cells);
    let rest = Rect::new(left, area.y, area.right().saturating_sub(left), area.height);
    chip_strip(rest, titles, active)
}

/// The mark in front of a strip that has chips cut off on its left.
const BEFORE: &str = "\u{2039}";

/// The mark behind a strip that has chips cut off on its right.
const AFTER: &str = "\u{203a}";

/// Where the sub-tab strip sits on a pane drawn into `area` (MOD-115 M3 D3): what
/// [`Pane::render`] draws and what a click is hit-tested against, from one computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipStrip {
    /// One rect per title; width 0 = cut off (no click hits it).
    pub chips: Vec<Rect>,
    /// The `‹` mark, when chips are cut off on the left.
    pub before: Option<Rect>,
    /// The `›` mark, when chips are cut off on the right.
    pub after: Option<Rect>,
}

/// The sub-tab strip of a pane drawn into `area`, `active` being the current chip.
///
/// A chip is ` title ` (two cells wider than the title). The strip sits between the corners and
/// ends flush with the right one:
/// - when every chip fits, every chip is shown;
/// - otherwise a window starts at the `active` chip and grows one chip at a time, right then
///   left, alternately, while the chip and the marks it then needs fit; `‹` and `›` (one cell
///   each) stand for the chips cut off on either side;
/// - when the active chip and its marks do not fit, the marks go first; when the active chip
///   alone does not fit, it is drawn clipped with `…`; with fewer than 3 cells, nothing is.
#[must_use]
pub fn chip_strip(area: Rect, titles: &[&str], active: usize) -> ChipStrip {
    let count = titles.len();
    let corner = area.right().saturating_sub(1);
    let mut strip = ChipStrip {
        chips: vec![Rect::new(corner, area.y, 0, 1); count],
        before: None,
        after: None,
    };
    if count == 0 {
        return strip;
    }
    let widths: Vec<u32> = titles
        .iter()
        .map(|title| {
            u32::try_from(cells::cell_width(title))
                .unwrap_or(u32::MAX)
                .saturating_add(2)
        })
        .collect();
    let avail = u32::from(area.width.saturating_sub(2));
    let last = count - 1;
    // The cells the marks of a window `lo..=hi` take.
    let marks = |lo: usize, hi: usize| u32::from(lo > 0) + u32::from(hi < last);
    let total = widths
        .iter()
        .fold(0u32, |sum, width| sum.saturating_add(*width));
    let (lo, hi, marked) = if total <= avail {
        (0, last, false)
    } else if avail < 3 {
        return strip;
    } else {
        let active = active.min(last);
        let (mut lo, mut hi) = (active, active);
        let mut used = widths[active];
        loop {
            let mut grew = false;
            if hi < last && used.saturating_add(widths[hi + 1]) + marks(lo, hi + 1) <= avail {
                hi += 1;
                used += widths[hi];
                grew = true;
            }
            if lo > 0 && used.saturating_add(widths[lo - 1]) + marks(lo - 1, hi) <= avail {
                lo -= 1;
                used += widths[lo];
                grew = true;
            }
            if !grew {
                break;
            }
        }
        (lo, hi, used.saturating_add(marks(lo, hi)) <= avail)
    };
    // Every width below is at most `avail`, which is a `u16`.
    let cells = |width: u32| u16::try_from(width.min(avail)).unwrap_or(0);
    let mark_cells = if marked { marks(lo, hi) } else { 0 };
    let span = widths[lo..=hi]
        .iter()
        .fold(mark_cells, |sum, width| sum.saturating_add(*width));
    let mut x = corner.saturating_sub(cells(span));
    if marked && lo > 0 {
        strip.before = Some(Rect::new(x, area.y, 1, 1));
        x += 1;
    }
    for (rect, width) in strip.chips[lo..=hi].iter_mut().zip(&widths[lo..=hi]) {
        let width = cells(*width);
        *rect = Rect::new(x, area.y, width, 1);
        x += width;
    }
    if marked && hi < last {
        strip.after = Some(Rect::new(x, area.y, 1, 1));
    }
    strip
}

/// Draws `hints` over the horizontal border glyphs of `row`, a pane's bottom border (what
/// [`Pane::hints`] puts there). They start two cells in and are fitted to `row.width - 4` by
/// whole keys ([`fit_hints`]). The hint is drawn only over the run of plain horizontal rule that
/// starts there: where text of the view's own stands on the border or a corner does, the hint
/// is fitted to the rule cells before it, or whole if it fits in the run. A row whose first cell
/// there is not a rule gets nothing, so a row that is not a border simply loses the hint.
/// `style` is the hint line's base style.
pub fn paint_bottom_hints(buf: &mut Buffer, row: Rect, hints: Line<'_>, style: Style) {
    let width = row.width.saturating_sub(4);
    let row = row.intersection(buf.area);
    if width == 0 || row.is_empty() {
        return;
    }
    let start = row.x + 2;
    let run = (start..start.saturating_add(width))
        .take_while(|x| *x < row.right() && is_rule(buf[(*x, row.y)].symbol()))
        .count();
    let room = u16::try_from(run).unwrap_or(width).min(width);
    if room == 0 {
        return;
    }
    let line = fit_hints(hints.patch_style(style), room);
    let shown = u16::try_from(line.width()).unwrap_or(room).min(room);
    let origin = Rect::new(start, row.y, shown, 1).intersection(buf.area);
    if origin.is_empty() {
        return;
    }
    let mut scratch = Buffer::empty(origin);
    line.render(origin, &mut scratch);
    for col in origin.x..origin.right() {
        buf[(col, origin.y)] = scratch[(col, origin.y)].clone();
    }
}

/// Between two keys of a hint row: [`Keys::hint`](crate::keys::Keys::hint)'s separator.
const HINT_SEP: &str = " \u{b7} ";

/// What a hint cut by whole keys ends in: a space, then `…` for the keys dropped. Two cells, not
/// the separator's three, so no key that fits beside it is dropped for the mark's sake.
const HINT_CUT: &str = " \u{2026}";

/// `hints` fitted to `room` cells by whole keys (MOD-115 M4 T3, carried): the one cut rule for
/// every pane's hints. Whole when it fits; else the most leading keys (parted by ` · `) that
/// leave room for ` …`, which says the rest was dropped; else `…` alone. Never half a key
/// (`u unci…`), so a view lists its own keys first and the keys it can spare last. The spans
/// kept keep their styles, the mark takes the style of the span its separator starts in, and
/// the line keeps its own style.
#[must_use]
pub fn fit_hints(hints: Line<'_>, room: u16) -> Line<'static> {
    let room = usize::from(room);
    let text: String = hints
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    if cells::cell_width(&text) <= room {
        spans.extend(
            hints
                .spans
                .iter()
                .map(|span| Span::styled(span.content.clone().into_owned(), span.style)),
        );
    } else if room > 0 {
        let cut = text
            .rmatch_indices(HINT_SEP)
            .map(|(at, _)| at)
            .find(|&at| cells::cell_width(&text[..at]) + cells::cell_width(HINT_CUT) <= room);
        match cut {
            Some(at) => {
                let mut start = 0;
                let mut mark = Style::new();
                for span in &hints.spans {
                    let len = span.content.len();
                    if at < start + len {
                        let kept = &span.content[..at - start];
                        if !kept.is_empty() {
                            spans.push(Span::styled(kept.to_owned(), span.style));
                        }
                        mark = span.style;
                        break;
                    }
                    spans.push(Span::styled(span.content.clone().into_owned(), span.style));
                    start += len;
                }
                match spans.last_mut() {
                    Some(last) if last.style == mark => last.content.to_mut().push_str(HINT_CUT),
                    _ => spans.push(Span::styled(HINT_CUT, mark)),
                }
            }
            None => {
                let style = hints.spans.first().map_or(Style::new(), |span| span.style);
                spans.push(Span::styled("\u{2026}", style));
            }
        }
    }
    let mut fitted = Line::from(spans);
    fitted.style = hints.style;
    fitted.alignment = hints.alignment;
    fitted
}

/// Draws `note` over the horizontal border glyphs of `row`, right-aligned: the twin of
/// [`paint_bottom_hints`], ending two cells before the right corner. It is clipped to
/// `row.width - 4` and drawn only over the run of plain rule that ends there, so it stops (with
/// an ellipsis) one rule cell short of the first cell that is not one, a hint or the tab's own
/// text, so a rule cell always parts the two. `style` is the note's base style.
pub fn paint_bottom_note(buf: &mut Buffer, row: Rect, note: Line<'_>, style: Style) {
    let width = row.width.saturating_sub(4);
    let row = row.intersection(buf.area);
    if width == 0 || row.is_empty() {
        return;
    }
    // One past the note's last cell: the corner and one rule cell stay to its right.
    let end = row.right().saturating_sub(2);
    let run = (1..=width)
        .take_while(|k| end >= row.x + k && is_rule(buf[(end - k, row.y)].symbol()))
        .count();
    let run = u16::try_from(run).unwrap_or(width).min(width);
    // A run cut short by a hint or the tab's own text keeps one rule cell beside it, so the
    // note never reads as one word with what stopped it; a run of the whole width already
    // leaves the rule cell after the left corner.
    let room = if run < width {
        run.saturating_sub(1)
    } else {
        run
    };
    if room == 0 {
        return;
    }
    let line = cells::clip_line(note.patch_style(style), room);
    let shown = u16::try_from(line.width()).unwrap_or(room).min(room);
    let origin = Rect::new(end - shown, row.y, shown, 1).intersection(buf.area);
    if origin.is_empty() {
        return;
    }
    let mut scratch = Buffer::empty(origin);
    line.render(origin, &mut scratch);
    for col in origin.x..origin.right() {
        buf[(col, origin.y)] = scratch[(col, origin.y)].clone();
    }
}

/// `offset` clamped so a scrolled list stops once its last row is in view.
#[must_use]
pub fn clamp_offset(offset: usize, total: usize, viewport: usize) -> usize {
    offset.min(total.saturating_sub(viewport))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// `pane` drawn into the whole of a `width` by `height` terminal: the buffer and the inner rect.
    fn draw(width: u16, height: u16, theme: &Theme, pane: Pane<'_>) -> (Buffer, Rect) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        let mut inner = Rect::default();
        terminal
            .draw(|frame| inner = pane.render(frame, frame.area(), theme))
            .expect("draw");
        (terminal.backend().buffer().clone(), inner)
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    /// MOD-115 M4 RV2-L7: `Pane::title_fits` is what `render` draws: for every width and title
    /// length around the edge, every active chip of a seven-chip strip and of a two-chip one, and
    /// no chips, it is true exactly when the top border carries ` title ` whole.
    #[test]
    fn title_fits_says_what_the_border_draws() {
        let seven = ["Body", "Runs", "Graph", "Docs", "Notes", "Prompt", "Reqs"];
        let two = ["Skills", "Templates"];
        let strips: [&[&str]; 3] = [&seven, &two, &[]];
        for titles in strips {
            for active in 0..titles.len().max(1) {
                for width in 4u16..=62 {
                    for length in 1..=50 {
                        let title = "t".repeat(length);
                        let area = Rect::new(0, 0, width, 3);
                        let (buffer, _) = draw(
                            width,
                            3,
                            &Theme::default(),
                            Pane::new(&title).title_first().sub_tabs(titles, active),
                        );
                        let top = row(&buffer, 0);
                        assert_eq!(
                            Pane::title_fits(area, &title, titles, active),
                            top.contains(&format!(" {title} ")),
                            "{width} cells, {length}-cell title, chip {active} of {titles:?}: {top}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_title_sits_on_the_top_border_after_the_corner() {
        let (buffer, inner) = draw(20, 5, &Theme::default(), Pane::new("Items"));
        assert_eq!(row(&buffer, 0), format!("╭─ Items {}╮", "─".repeat(10)));
        assert_eq!(row(&buffer, 4), format!("╰{}╯", "─".repeat(18)));
        assert_eq!(
            inner,
            Rect::new(1, 1, 18, 3),
            "the content goes inside the border"
        );
    }

    #[test]
    fn a_long_title_is_clipped_with_an_ellipsis_and_never_reaches_the_corner() {
        let (buffer, _) = draw(12, 3, &Theme::default(), Pane::new("A very long title"));
        let top = row(&buffer, 0);
        assert!(top.starts_with("╭─ "), "{top}");
        assert!(top.contains('…'), "{top}");
        assert!(
            top.ends_with("─╮"),
            "the corner and its rule are untouched: {top}"
        );
    }

    #[test]
    fn sub_tab_chips_end_flush_with_the_right_corner() {
        let area = Rect::new(2, 1, 30, 5);
        let rects = sub_tab_rects(area, &["Body", "Notes"], 0);
        // " Body " is 6 cells, " Notes " is 7: together 13, starting at right - 1 - 13.
        assert_eq!(area.right() - 1 - 13, 18);
        assert_eq!(rects, vec![Rect::new(18, 1, 6, 1), Rect::new(24, 1, 7, 1)]);
        assert_eq!(rects[1].right(), area.right() - 1);
    }

    #[test]
    fn the_chips_are_drawn_where_the_rects_say_and_the_active_one_is_styled() {
        let theme = Theme::default();
        let titles = ["Body", "Notes"];
        let (buffer, _) = draw(
            30,
            4,
            &theme,
            Pane::new("Item").sub_tabs(&titles, 1).focus(Focus::Focused),
        );
        let rects = sub_tab_rects(Rect::new(0, 0, 30, 4), &titles, 1);
        let text = |rect: Rect| -> String {
            (rect.x..rect.right())
                .map(|x| buffer[(x, 0)].symbol())
                .collect()
        };
        assert_eq!(text(rects[0]), " Body ");
        assert_eq!(text(rects[1]), " Notes ");
        let active = &buffer[(rects[1].x + 1, 0)];
        let idle = &buffer[(rects[0].x + 1, 0)];
        assert_eq!(active.style().fg, theme.active_tab.fg);
        assert!(
            active
                .modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert_eq!(idle.style().fg, theme.dim.fg);
        assert_eq!(row(&buffer, 0).chars().last(), Some('╮'));
    }

    #[test]
    fn the_title_is_clipped_before_a_chip_is_dropped() {
        let titles = ["Body", "Notes"];
        let (buffer, _) = draw(
            24,
            3,
            &Theme::default(),
            Pane::new("Backlog items").sub_tabs(&titles, 0),
        );
        let top = row(&buffer, 0);
        assert!(top.contains(" Body  Notes "), "both chips survive: {top}");
        assert!(top.contains('…') || top.contains("Backlog"), "{top}");
        assert!(top.starts_with("╭─ "), "{top}");
    }

    #[test]
    fn chips_that_do_not_fit_keep_the_active_one_and_mark_the_rest_at_a_narrow_width() {
        let area = Rect::new(0, 0, 14, 3);
        let strip = chip_strip(area, &["Body", "Notes", "Graph"], 0);
        // 12 cells between the corners: " Body " (6) and the `›` mark (1) fit, " Notes " (7)
        // does not, so the strip is `[ Body ][›]`, ending flush with the right corner.
        assert_eq!(strip.chips[0], Rect::new(6, 0, 6, 1));
        assert_eq!(strip.chips[1].width, 0);
        assert_eq!(strip.chips[2].width, 0);
        assert_eq!(strip.before, None, "nothing is cut on the left");
        assert_eq!(strip.after, Some(Rect::new(12, 0, 1, 1)));
        let (buffer, _) = draw(
            14,
            3,
            &Theme::default(),
            Pane::new("").sub_tabs(&["Body", "Notes", "Graph"], 0),
        );
        let top = row(&buffer, 0);
        assert!(
            top.contains("Body") && !top.contains("Notes") && !top.contains("Graph"),
            "{top}"
        );
        assert!(top.ends_with(" Body ›╮"), "{top}");
        assert!(
            sub_tab_rects(Rect::new(0, 0, 2, 3), &["Body"], 0)[0].width == 0,
            "no room at all"
        );
    }

    /// The ten Settings sections: 87 cells of chips, more than an 80-column pane holds.
    const SECTIONS: [&str; 10] = [
        "Agents",
        "Hierarchy",
        "Kinds",
        "Personas",
        "Boxes",
        "Prompt",
        "Queue",
        "Connection",
        "Qdrant",
        "Secrets",
    ];

    #[test]
    fn chips_that_do_not_fit_keep_the_active_one_and_mark_the_rest() {
        let area = Rect::new(0, 0, 80, 3);
        for active in [0, 4, 9] {
            let strip = chip_strip(area, &SECTIONS, active);
            assert!(strip.chips[active].width > 0, "active {active}: {strip:?}");
            assert_eq!(
                strip.before.is_some(),
                strip.chips[0].width == 0,
                "`‹` only when something is cut on the left (active {active})"
            );
            assert_eq!(
                strip.after.is_some(),
                strip.chips[9].width == 0,
                "`›` only when something is cut on the right (active {active})"
            );
            let (buffer, _) = draw(
                80,
                3,
                &Theme::default(),
                Pane::new("").sub_tabs(&SECTIONS, active),
            );
            let top = row(&buffer, 0);
            assert!(
                top.contains(&format!(" {} ", SECTIONS[active])),
                "active {active}: {top}"
            );
            assert_eq!(top.contains('‹'), strip.before.is_some(), "{top}");
            assert_eq!(top.contains('›'), strip.after.is_some(), "{top}");
            assert!(top.ends_with('╮'), "{top}");
        }
        let first = chip_strip(area, &SECTIONS, 0);
        assert!(first.before.is_none() && first.after.is_some(), "{first:?}");
        let last = chip_strip(area, &SECTIONS, 9);
        assert!(last.before.is_some() && last.after.is_none(), "{last:?}");
        let middle = chip_strip(area, &SECTIONS, 4);
        assert!(
            middle.before.is_some() && middle.after.is_some(),
            "{middle:?}"
        );
    }

    #[test]
    fn the_rects_agree_with_what_is_drawn() {
        let theme = Theme::default();
        for width in 20..=120u16 {
            for active in 0..SECTIONS.len() {
                let area = Rect::new(0, 0, width, 3);
                let strip = chip_strip(area, &SECTIONS, active);
                let (buffer, _) = draw(width, 3, &theme, Pane::new("").sub_tabs(&SECTIONS, active));
                let text = |rect: Rect| -> String {
                    (rect.x..rect.right())
                        .map(|x| buffer[(x, 0)].symbol())
                        .collect()
                };
                let shown: Vec<usize> = (0..SECTIONS.len())
                    .filter(|&i| strip.chips[i].width > 0)
                    .collect();
                assert!(shown.contains(&active), "{width}/{active}: {strip:?}");
                for &i in &shown {
                    assert_eq!(
                        text(strip.chips[i]),
                        format!(" {} ", SECTIONS[i]),
                        "{width}/{active}: chip {i}"
                    );
                }
                // The shown chips are one contiguous run, the marks at its ends, and the
                // whole strip ends flush with the right corner.
                let (lo, hi) = (shown[0], shown[shown.len() - 1]);
                assert_eq!(shown.len(), hi - lo + 1, "{width}/{active}");
                for pair in shown.windows(2) {
                    assert_eq!(strip.chips[pair[0]].right(), strip.chips[pair[1]].x);
                }
                assert_eq!(strip.before.is_some(), lo > 0, "{width}/{active}");
                assert_eq!(strip.after.is_some(), hi < SECTIONS.len() - 1);
                if let Some(before) = strip.before {
                    assert_eq!(text(before), "‹", "{width}/{active}");
                    assert_eq!(before.right(), strip.chips[lo].x);
                }
                let end = strip.after.map_or(strip.chips[hi].right(), |after| {
                    assert_eq!(text(after), "›", "{width}/{active}");
                    assert_eq!(after.x, strip.chips[hi].right());
                    after.right()
                });
                assert_eq!(end, width - 1, "{width}/{active}");
                assert_eq!(row(&buffer, 0).chars().last(), Some('╮'));
            }
        }
    }

    #[test]
    fn a_strip_that_fits_is_today_s_strip() {
        let area = Rect::new(2, 1, 30, 5);
        for active in 0..2 {
            let strip = chip_strip(area, &["Body", "Notes"], active);
            assert_eq!(
                strip.chips,
                vec![Rect::new(18, 1, 6, 1), Rect::new(24, 1, 7, 1)]
            );
            assert_eq!((strip.before, strip.after), (None, None));
        }
    }

    #[test]
    fn a_strip_with_no_room_for_the_marks_drops_them_then_clips_the_active_chip() {
        // " Connection " is 12 cells; 12 between the corners leaves no room for its `‹`.
        let strip = chip_strip(Rect::new(0, 0, 14, 3), &["Agents", "Connection"], 1);
        assert_eq!(strip.chips[1], Rect::new(1, 0, 12, 1));
        assert_eq!((strip.before, strip.after), (None, None));
        assert_eq!(strip.chips[0].width, 0);
        // 8 between the corners: the active chip alone, clipped with an ellipsis.
        let strip = chip_strip(Rect::new(0, 0, 10, 3), &["Agents", "Connection"], 1);
        assert_eq!(strip.chips[1], Rect::new(1, 0, 8, 1));
        let (buffer, _) = draw(
            10,
            3,
            &Theme::default(),
            Pane::new("").sub_tabs(&["Agents", "Connection"], 1),
        );
        assert_eq!(row(&buffer, 0), "╭ Connec…╮");
        // Fewer than 3 cells: nothing at all.
        let strip = chip_strip(Rect::new(0, 0, 4, 3), &["Agents", "Connection"], 1);
        assert!(strip.chips.iter().all(|rect| rect.width == 0));
        assert_eq!((strip.before, strip.after), (None, None));
        assert!(chip_strip(Rect::new(0, 0, 40, 3), &[], 0).chips.is_empty());
    }

    #[test]
    fn the_title_gives_way_before_a_chip() {
        let (buffer, _) = draw(
            80,
            3,
            &Theme::default(),
            Pane::new("Agents")
                .sub_tabs(&SECTIONS, 0)
                .focus(Focus::Focused),
        );
        let top = row(&buffer, 0);
        assert_eq!(
            top.matches("Agents").count(),
            1,
            "the active chip carries the name, the title is not drawn: {top}"
        );
        assert!(top.starts_with("╭──"), "{top}");
        assert!(!top.contains('…'), "no clipped title either: {top}");
    }

    /// MOD-80 D5 (MOD-115 M3 §4.1): on a focused pane's border, bold in monochrome, only the
    /// active chip is bold; the other chips and the marks are dim, never bold.
    #[test]
    fn on_a_focused_border_only_the_active_chip_is_bold_without_colour() {
        let theme = Theme::monochrome();
        let area = Rect::new(0, 0, 80, 3);
        let strip = chip_strip(area, &SECTIONS, 4);
        let pane = Pane::new("")
            .sub_tabs(&SECTIONS, 4)
            .focus(Focus::Focused)
            .accent(theme.tab_accent("settings"));
        let (buffer, _) = draw(80, 3, &theme, pane);
        for (index, chip) in strip.chips.iter().enumerate() {
            if chip.width < 2 {
                continue;
            }
            let label = &buffer[(chip.x + 1, 0)];
            if index == 4 {
                assert!(
                    label.modifier.contains(Modifier::BOLD),
                    "{index}: {label:?}"
                );
                assert!(
                    !label.modifier.contains(Modifier::DIM),
                    "{index}: {label:?}"
                );
            } else {
                assert!(
                    !label.modifier.contains(Modifier::BOLD),
                    "{index}: {label:?}"
                );
                assert!(label.modifier.contains(Modifier::DIM), "{index}: {label:?}");
            }
        }
        for mark in [strip.before, strip.after] {
            let mark = mark.expect("both marks at active 4");
            let cell = &buffer[(mark.x, 0)];
            assert!(!cell.modifier.contains(Modifier::BOLD), "{mark:?}");
            assert!(cell.modifier.contains(Modifier::DIM), "{mark:?}");
        }
    }

    #[test]
    fn the_marks_are_dim_without_colour() {
        let theme = Theme::monochrome();
        let area = Rect::new(0, 0, 80, 3);
        let strip = chip_strip(area, &SECTIONS, 4);
        let (buffer, _) = draw(80, 3, &theme, Pane::new("").sub_tabs(&SECTIONS, 4));
        for mark in [strip.before, strip.after] {
            let mark = mark.expect("both marks at active 4");
            let cell = &buffer[(mark.x, 0)];
            assert!(cell.modifier.contains(Modifier::DIM), "{mark:?}");
            assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
        }
        let (coloured, _) = draw(
            80,
            3,
            &Theme::default(),
            Pane::new("").sub_tabs(&SECTIONS, 4),
        );
        let mark = strip.before.expect("a left mark");
        assert_eq!(
            coloured[(mark.x, 0)].style().fg,
            Theme::default().dim.fg,
            "the dim role"
        );
    }

    #[test]
    fn content_drawn_before_the_frame_survives_it() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).expect("a test terminal");
        let mut drawn = Rect::default();
        let mut measured = Rect::default();
        terminal
            .draw(|frame| {
                measured = Pane::inner(frame.area());
                frame.buffer_mut().set_string(
                    measured.x,
                    measured.y,
                    "hello",
                    Style::new().fg(Color::Yellow),
                );
                drawn = Pane::new("T")
                    .focus(Focus::Focused)
                    .accent(accent())
                    .scrollbar(10, 0)
                    .render(frame, frame.area(), &theme);
            })
            .expect("draw");
        assert_eq!(measured, drawn, "`inner` is the rect `render` returns");
        assert_eq!(measured, Rect::new(1, 1, 18, 3));
        let buffer = terminal.backend().buffer();
        assert_eq!(row(buffer, 1), "│hello             ┃");
        assert_eq!(buffer[(1, 1)].fg, Color::Yellow, "its style survives too");
        assert_eq!(buffer[(0, 0)].fg, Color::Magenta, "and the frame is drawn");
    }

    #[test]
    fn the_scrollbar_stays_inside_its_span() {
        // A 10 by 10 pane: inner rows 1..=8; the span is inner rows 2..6, terminal rows 3..=6.
        let thumb = |offset: usize, span: (u16, u16)| -> Vec<u16> {
            let (buffer, _) = draw(
                10,
                10,
                &Theme::default(),
                Pane::new("")
                    .scrollbar(20, offset)
                    .scrollbar_span(Rect::new(1, 1 + span.0, 8, span.1)),
            );
            (0..10)
                .filter(|&y| buffer[(9, y)].symbol() == THUMB)
                .collect()
        };
        assert_eq!(thumb(0, (2, 4)), vec![3], "at the top of the span");
        assert_eq!(thumb(16, (2, 4)), vec![6], "the end of the span");
        assert_eq!(thumb(99, (2, 4)), vec![6]);
        for offset in 0..20 {
            assert!(
                thumb(offset, (2, 4)).iter().all(|y| (3..=6).contains(y)),
                "{offset}"
            );
        }
        // A span past the inner rect is cut to it.
        assert!(thumb(99, (6, 10)).iter().all(|y| (7..=8).contains(y)));
        assert_eq!(thumb(99, (6, 10)), vec![8]);
        // A span that holds every row draws no thumb.
        let (buffer, _) = draw(
            10,
            10,
            &Theme::default(),
            Pane::new("")
                .scrollbar(3, 0)
                .scrollbar_span(Rect::new(1, 1, 8, 4)),
        );
        assert!((0..10).all(|y| buffer[(9, y)].symbol() != THUMB));
    }

    #[test]
    fn hints_start_two_cells_in_and_only_while_focused() {
        let hints = || Line::from("enter open");
        let (focused, _) = draw(
            24,
            4,
            &Theme::default(),
            Pane::new("T").hints(hints()).focus(Focus::Focused),
        );
        assert_eq!(row(&focused, 3), format!("╰─enter open{}╯", "─".repeat(11)));
        let (unfocused, _) = draw(24, 4, &Theme::default(), Pane::new("T").hints(hints()));
        assert_eq!(row(&unfocused, 3), format!("╰{}╯", "─".repeat(22)));
    }

    #[test]
    fn hints_are_trimmed_to_the_width_and_never_touch_a_corner() {
        let hints = Line::from("enter open · e edit · d delete · ? help");
        let (buffer, _) = draw(
            20,
            3,
            &Theme::default(),
            Pane::new("T").hints(hints).focus(Focus::Focused),
        );
        let bottom = row(&buffer, 2);
        assert!(bottom.starts_with("╰─"), "{bottom}");
        assert!(bottom.ends_with("─╯"), "{bottom}");
        assert!(
            bottom.contains('…'),
            "a clipped hint line ends in an ellipsis: {bottom}"
        );
        assert_eq!(bottom.chars().count(), 20);
        // Too narrow for any hint: nothing is drawn, the corners stay.
        let (tiny, _) = draw(
            4,
            3,
            &Theme::default(),
            Pane::new("T").hints(Line::from("x")).focus(Focus::Focused),
        );
        assert_eq!(row(&tiny, 2), "╰──╯");
    }

    #[test]
    fn a_focused_border_takes_the_accent_and_an_unfocused_one_the_faint_role() {
        let theme = Theme::default();
        let (focused, _) = draw(
            10,
            3,
            &theme,
            Pane::new("").focus(Focus::Focused).accent(accent()),
        );
        let (unfocused, _) = draw(10, 3, &theme, Pane::new("").accent(accent()));
        for buffer in [&focused, &unfocused] {
            assert_eq!(buffer[(0, 0)].symbol(), "╭");
        }
        for (x, y) in [(0, 0), (9, 0), (0, 1), (5, 2), (9, 2)] {
            assert_eq!(focused[(x, y)].fg, Color::Magenta, "({x},{y})");
            assert_eq!(unfocused[(x, y)].fg, Color::Indexed(240), "({x},{y})");
        }
        let (default_accent, _) = draw(10, 3, &theme, Pane::new("").focus(Focus::Focused));
        assert_eq!(
            default_accent[(0, 0)].fg,
            Color::Cyan,
            "no accent given: the focus accent"
        );
    }

    /// The thumb's rows, top to bottom, on the right border of a 6-row pane (4 inner rows).
    fn thumb_rows(total: usize, offset: usize) -> Vec<u16> {
        let (buffer, _) = draw(
            10,
            6,
            &Theme::default(),
            Pane::new("").scrollbar(total, offset),
        );
        (1..5)
            .filter(|&y| buffer[(9, y)].symbol() == THUMB)
            .collect()
    }

    #[test]
    fn the_scrollbar_thumb_moves_down_and_stops_at_the_end() {
        assert_eq!(thumb_rows(8, 0), vec![1, 2], "at the top");
        assert_eq!(
            thumb_rows(8, 4),
            vec![3, 4],
            "the last row in view: the thumb is at the bottom"
        );
        assert_eq!(
            thumb_rows(8, 40),
            vec![3, 4],
            "scrolled past the end is the end"
        );
        let middle = thumb_rows(8, 2);
        assert_eq!(middle.len(), 2);
        assert!(middle[0] > 1 && middle[1] < 4, "{middle:?}");
    }

    #[test]
    fn no_scrollbar_when_everything_fits() {
        assert!(thumb_rows(4, 0).is_empty());
        assert!(thumb_rows(0, 0).is_empty());
        let (buffer, _) = draw(10, 6, &Theme::default(), Pane::new("").scrollbar(4, 0));
        assert_eq!(buffer[(9, 2)].symbol(), "│");
    }

    #[test]
    fn clamp_offset_stops_at_the_last_full_page() {
        assert_eq!(clamp_offset(0, 10, 4), 0);
        assert_eq!(clamp_offset(6, 10, 4), 6);
        assert_eq!(clamp_offset(9, 10, 4), 6);
        assert_eq!(
            clamp_offset(3, 2, 4),
            0,
            "a list shorter than the viewport never scrolls"
        );
    }

    #[test]
    fn monochrome_keeps_focus_visible_without_colour() {
        let theme = Theme::monochrome();
        let accent = theme.tab_accent("backlog");
        let (focused, _) = draw(
            10,
            3,
            &theme,
            Pane::new("T").focus(Focus::Focused).accent(accent),
        );
        let (unfocused, _) = draw(10, 3, &theme, Pane::new("T").accent(accent));
        let (bare, _) = draw(10, 3, &theme, Pane::new("T").focus(Focus::Focused));
        assert!(focused[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(
            bare[(0, 0)].modifier.contains(Modifier::BOLD),
            "even with no accent given"
        );
        assert!(unfocused[(0, 0)].modifier.contains(Modifier::DIM));
        assert!(!unfocused[(0, 0)].modifier.contains(Modifier::BOLD));
        for buffer in [&focused, &unfocused, &bare] {
            for y in 0..3 {
                for x in 0..10 {
                    assert_eq!(buffer[(x, y)].fg, Color::Reset, "({x},{y})");
                    assert_eq!(buffer[(x, y)].bg, Color::Reset, "({x},{y})");
                }
            }
        }
    }

    fn bordered_row(text: &str) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(
            0,
            0,
            u16::try_from(text.chars().count()).unwrap(),
            1,
        ));
        buffer.set_string(0, 0, text, Style::new());
        buffer
    }

    #[test]
    fn bottom_hints_overwrite_only_horizontal_rules() {
        let mut buffer = bordered_row(&format!("╰{}╯", "─".repeat(16)));
        paint_bottom_hints(
            &mut buffer,
            Rect::new(0, 0, 18, 1),
            Line::from("a b"),
            Style::new(),
        );
        assert_eq!(row(&buffer, 0), format!("╰─a b{}╯", "─".repeat(12)));
        // Anything that is not a plain rule is kept, the hint cell dropped.
        let mut text = bordered_row("│ some body text │");
        paint_bottom_hints(
            &mut text,
            Rect::new(0, 0, 18, 1),
            Line::from("a b"),
            Style::new(),
        );
        assert_eq!(row(&text, 0), "│ some body text │");
        let mut mixed = bordered_row(&format!("╰──x{}╯", "─".repeat(13)));
        paint_bottom_hints(
            &mut mixed,
            Rect::new(0, 0, 18, 1),
            Line::from("abcd"),
            Style::new(),
        );
        // The hint runs up to the first cell that is not a rule and ends there with an
        // ellipsis, instead of skipping that cell and going on past it.
        assert_eq!(row(&mixed, 0), format!("╰─…x{}╯", "─".repeat(13)));
        let mut room = bordered_row(&format!("╰──x{}╯", "─".repeat(13)));
        paint_bottom_hints(
            &mut room,
            Rect::new(0, 0, 18, 1),
            Line::from("a"),
            Style::new(),
        );
        assert_eq!(
            row(&room, 0),
            format!("╰─ax{}╯", "─".repeat(13)),
            "a hint that fits before the text is drawn whole, no ellipsis"
        );
    }

    #[test]
    fn bottom_hints_are_clipped_and_leave_the_corners_alone() {
        let mut buffer = bordered_row(&format!("╰{}╯", "─".repeat(10)));
        paint_bottom_hints(
            &mut buffer,
            Rect::new(0, 0, 12, 1),
            Line::from("enter open and more"),
            Style::new(),
        );
        let text = row(&buffer, 0);
        assert!(
            text.starts_with("╰─") && text.ends_with("─╯") && text.contains('…'),
            "{text}"
        );
        assert_eq!(text.chars().count(), 12);
    }

    /// MOD-115 M4 T3 (carried): a hint too wide for its border drops whole keys from the end
    /// and ends in ` …`, never half a key; what is kept keeps its styles. The mark takes two
    /// cells, so a key that fits with it stays (Settings › Hierarchy's `i infer` at 80 columns).
    /// Not even the first key fitting leaves the mark alone.
    #[test]
    fn a_cut_hint_drops_whole_keys_from_the_end_and_says_so() {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);
        let line = || {
            Line::from(vec![
                Span::styled("j/k move", bold),
                Span::styled(" \u{b7} e edit \u{b7} d delete", dim),
            ])
        };
        assert_eq!(
            fit_hints(line(), 28).spans,
            line().spans,
            "whole when it fits"
        );
        assert_eq!(
            fit_hints(line(), 27).spans,
            vec![
                Span::styled("j/k move", bold),
                Span::styled(" \u{b7} e edit \u{2026}", dim)
            ]
        );
        assert_eq!(
            fit_hints(line(), 19).to_string(),
            "j/k move \u{b7} e edit \u{2026}",
            "the last key that fits with the mark stays"
        );
        for room in 10..19 {
            assert_eq!(
                fit_hints(line(), room).spans,
                vec![
                    Span::styled("j/k move", bold),
                    Span::styled(" \u{2026}", dim)
                ],
                "{room}"
            );
        }
        assert_eq!(fit_hints(line(), 9).to_string(), "\u{2026}");
        assert_eq!(fit_hints(line(), 1).to_string(), "\u{2026}");
        assert!(fit_hints(line(), 0).spans.is_empty());
        let styled = Line::styled("a b \u{b7} c d", dim);
        assert_eq!(
            fit_hints(styled, 6).style,
            dim,
            "the line's own style stays"
        );
    }

    /// MOD-115 M4 T3 (carried): at every width a pane's hint is whole, whole keys and ` …`, or
    /// the mark alone: it never ends inside a key (`u unci…`, `N n…`), and never touches a corner.
    #[test]
    fn no_pane_hint_ends_inside_a_key_at_any_width() {
        let full = "j/k move \u{b7} Enter open \u{b7} n new \u{b7} e edit \u{b7} f filter \u{b7} \
                    / search \u{b7} N next match \u{b7} u uncite \u{b7} Ctrl+e $EDITOR \u{b7} ? keys";
        for width in 20..=140 {
            let pane = Pane::new("T").hints(Line::from(full)).focus(Focus::Focused);
            let (buffer, _) = draw(width, 3, &Theme::default(), pane);
            let bottom = row(&buffer, 2);
            assert!(bottom.starts_with("\u{2570}\u{2500}"), "{width}: {bottom}");
            assert!(bottom.ends_with("\u{2500}\u{256f}"), "{width}: {bottom}");
            let shown = bottom.trim_matches(['\u{2570}', '\u{256f}', '\u{2500}']);
            let kept = shown.strip_suffix(" \u{2026}");
            let whole_keys = kept.is_some_and(|kept| full.starts_with(&format!("{kept} \u{b7} ")));
            assert!(
                shown == full || shown == "\u{2026}" || whole_keys,
                "{width}: a hint ends inside a key: {shown:?}"
            );
            // No key that would fit with the mark is dropped.
            let room = usize::from(width - 4);
            let kept = kept.map_or(0, str::len);
            if shown != full {
                let end = full[kept..]
                    .match_indices(" \u{b7} ")
                    .map(|(at, _)| kept + at)
                    .find(|&at| at > kept)
                    .unwrap_or(full.len());
                let next = full[..end].chars().count();
                assert!(
                    next + " \u{2026}".chars().count() > room,
                    "{width}: a key that fits was dropped: {shown:?}"
                );
            }
        }
    }

    #[test]
    fn a_bottom_note_ends_two_cells_before_the_corner() {
        let mut buffer = bordered_row(&format!("╰{}╯", "─".repeat(16)));
        paint_bottom_note(
            &mut buffer,
            Rect::new(0, 0, 18, 1),
            Line::from("a b"),
            Style::new().fg(Color::Yellow),
        );
        assert_eq!(row(&buffer, 0), format!("╰{}a b─╯", "─".repeat(12)));
        assert_eq!(buffer[(13, 0)].fg, Color::Yellow, "the note's style");
    }

    #[test]
    fn a_bottom_note_takes_only_the_rule_the_hints_left() {
        let text = format!("╰─abcdef{}╯", "─".repeat(6));
        let mut clipped = bordered_row(&text);
        paint_bottom_note(
            &mut clipped,
            Rect::new(0, 0, 15, 1),
            Line::from("123456789"),
            Style::new(),
        );
        assert_eq!(
            row(&clipped, 0),
            "╰─abcdef─123…─╯",
            "one rule cell stays between the note and what stopped it"
        );
        let mut exact = bordered_row(&text);
        paint_bottom_note(
            &mut exact,
            Rect::new(0, 0, 15, 1),
            Line::from("1234"),
            Style::new(),
        );
        assert_eq!(row(&exact, 0), "╰─abcdef─1234─╯");
        let mut fits = bordered_row(&text);
        paint_bottom_note(
            &mut fits,
            Rect::new(0, 0, 15, 1),
            Line::from("ab"),
            Style::new(),
        );
        assert_eq!(row(&fits, 0), "╰─abcdef───ab─╯");
    }

    #[test]
    fn a_bottom_note_skips_a_row_that_is_not_a_border_or_has_no_room() {
        let mut text = bordered_row("│ some body text │");
        paint_bottom_note(
            &mut text,
            Rect::new(0, 0, 18, 1),
            Line::from("z unzoom"),
            Style::new(),
        );
        assert_eq!(row(&text, 0), "│ some body text │");
        let mut tiny = bordered_row("╰──╯");
        paint_bottom_note(
            &mut tiny,
            Rect::new(0, 0, 4, 1),
            Line::from("z"),
            Style::new(),
        );
        assert_eq!(row(&tiny, 0), "╰──╯");
        let mut long = bordered_row(&format!("╰{}╯", "─".repeat(10)));
        paint_bottom_note(
            &mut long,
            Rect::new(0, 0, 12, 1),
            Line::from("F2 show list · p layouts"),
            Style::new(),
        );
        let shown = row(&long, 0);
        assert!(
            shown.starts_with("╰─") && shown.ends_with("─╯") && shown.contains('…'),
            "{shown}"
        );
        assert_eq!(shown.chars().count(), 12);
    }

    #[test]
    fn a_panes_note_sits_right_aligned_on_the_bottom_border_focused_or_not() {
        let theme = Theme::default();
        let (unfocused, _) = draw(24, 4, &theme, Pane::new("T").note(Line::from("z unzoom")));
        assert_eq!(row(&unfocused, 3), format!("╰{}z unzoom─╯", "─".repeat(13)));
        let (focused, _) = draw(
            30,
            4,
            &theme,
            Pane::new("T")
                .hints(Line::from("enter open"))
                .note(Line::from("z unzoom"))
                .focus(Focus::Focused),
        );
        assert_eq!(
            row(&focused, 3),
            format!("╰─enter open{}z unzoom─╯", "─".repeat(8))
        );
        let (tight, _) = draw(
            22,
            4,
            &theme,
            Pane::new("T")
                .hints(Line::from("enter open"))
                .note(Line::from("z unzoom"))
                .focus(Focus::Focused),
        );
        assert_eq!(
            row(&tight, 3),
            "╰─enter open─z unzo…─╯",
            "the note never runs into the hints"
        );
    }

    /// RV1 H1: a hint line with a style of its own (`chips::key_line`'s `theme.dim`) styles its
    /// own cells only; the rule past it keeps the focused border's accent, and in monochrome the
    /// rule stays bold, not dim, while the hint cells are dim, not bold.
    #[test]
    fn a_styled_hint_line_restyles_only_its_own_cells() {
        let theme = Theme::default();
        let pane = Pane::new("T")
            .hints(Line::styled("enter open", theme.dim))
            .focus(Focus::Focused)
            .accent(accent());
        let (buffer, _) = draw(24, 4, &theme, pane);
        assert_eq!(row(&buffer, 3), format!("╰─enter open{}╯", "─".repeat(11)));
        for x in [0, 1, 12, 17, 22, 23] {
            assert_eq!(buffer[(x, 3)].fg, Color::Magenta, "rule cell {x}");
        }
        for x in 2..12 {
            assert_eq!(Some(buffer[(x, 3)].fg), theme.dim.fg, "hint cell {x}");
        }

        let mono = Theme::monochrome();
        let pane = Pane::new("T")
            .hints(Line::styled("enter open", mono.dim))
            .focus(Focus::Focused)
            .accent(mono.tab_accent("settings"));
        let (buffer, _) = draw(24, 4, &mono, pane);
        for x in [1, 12, 17, 22] {
            let cell = &buffer[(x, 3)];
            assert!(
                cell.modifier.contains(Modifier::BOLD),
                "rule cell {x}: {cell:?}"
            );
            assert!(
                !cell.modifier.contains(Modifier::DIM),
                "rule cell {x}: {cell:?}"
            );
        }
        for x in 2..12 {
            let cell = &buffer[(x, 3)];
            assert!(
                cell.modifier.contains(Modifier::DIM),
                "hint cell {x}: {cell:?}"
            );
            assert!(
                !cell.modifier.contains(Modifier::BOLD),
                "hint cell {x}: {cell:?}"
            );
        }
    }

    /// RV1 L4: a pane drawn into a rect that runs past the buffer draws what fits and never
    /// indexes a cell outside it (the scrollbar did).
    #[test]
    fn a_pane_past_the_buffer_draws_what_fits() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(12, 6)).expect("a test terminal");
        terminal
            .draw(|frame| {
                for area in [
                    Rect::new(0, 0, 16, 10),
                    Rect::new(4, 3, 20, 20),
                    Rect::new(30, 30, 6, 6),
                ] {
                    let _ = Pane::new("Title")
                        .sub_tabs(&["Ab", "Cd"], 0)
                        .hints(Line::from("x y"))
                        .note(Line::from("n"))
                        .focus(Focus::Focused)
                        .scrollbar(40, 20)
                        .render(frame, area, &theme);
                }
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "╭");
        assert_eq!(buffer[(4, 3)].symbol(), "╭");
    }

    /// RV1 L6: the scrollbar's span is an absolute rect, cut to the inner rect.
    #[test]
    fn the_scrollbar_span_is_an_absolute_rect_cut_to_the_inner_one() {
        let thumb = |rows: Rect| -> Vec<u16> {
            let (buffer, _) = draw(
                10,
                10,
                &Theme::default(),
                Pane::new("").scrollbar(20, 99).scrollbar_span(rows),
            );
            (0..10)
                .filter(|&y| buffer[(9, y)].symbol() == THUMB)
                .collect()
        };
        // Terminal rows 3..=6: the thumb ends on row 6.
        assert_eq!(thumb(Rect::new(1, 3, 8, 4)), vec![6]);
        // A rect past the inner one (rows 1..=8) is cut to it, above and below.
        assert_eq!(thumb(Rect::new(0, 7, 10, 10)), vec![8]);
        assert!(
            thumb(Rect::new(0, 0, 10, 3))
                .iter()
                .all(|y| (1..=2).contains(y))
        );
        // A rect clear of the inner one draws no thumb.
        assert!(thumb(Rect::new(0, 20, 10, 4)).is_empty());
    }

    /// RV1 M2: a [`ScrollSpan`] carries its rows to the pane in one call.
    #[test]
    fn a_scroll_span_sets_the_total_the_offset_and_the_rows() {
        let span = ScrollSpan::new(20, 99).rows(Rect::new(1, 3, 8, 4));
        assert_eq!((span.total, span.offset), (20, 99));
        let (buffer, _) = draw(10, 10, &Theme::default(), Pane::new("").scroll(span));
        let rows: Vec<u16> = (0..10)
            .filter(|&y| buffer[(9, y)].symbol() == THUMB)
            .collect();
        assert_eq!(rows, vec![6]);
        let (whole, _) = draw(
            10,
            10,
            &Theme::default(),
            Pane::new("").scroll(ScrollSpan::new(20, 99)),
        );
        assert_eq!(
            whole[(9, 8)].symbol(),
            THUMB,
            "no rows: the whole inner rect"
        );
    }

    /// The Backlog detail's seven sub-tabs (MOD-115 M4 D-4): 46 cells of chips.
    const DETAIL: [&str; 7] = ["Body", "Runs", "Graph", "Docs", "Notes", "Prompt", "Reqs"];

    /// D-4, C5: at the default 100 columns the detail pane is 45 wide; asked first, the title
    /// keeps its room and the chips window around the active one in what is left.
    #[test]
    fn the_title_keeps_its_room_when_asked_first() {
        let pane = Pane::new("FEAT-1 · TUI scaffold")
            .sub_tabs(&DETAIL, 0)
            .focus(Focus::Focused)
            .title_first();
        let (buffer, _) = draw(45, 4, &Theme::default(), pane);
        let top = row(&buffer, 0);
        assert_eq!(
            top,
            format!("╭─ FEAT-1 · TUI scaffold {} Body  Runs ›╮", "─".repeat(6)),
            "{top}"
        );
        let strip = chip_strip_after(Rect::new(0, 0, 45, 4), "FEAT-1 · TUI scaffold", &DETAIL, 0);
        assert_eq!(strip.chips[0], Rect::new(31, 0, 6, 1));
        assert_eq!(strip.chips[1], Rect::new(37, 0, 6, 1));
        assert!(strip.chips[2..].iter().all(|chip| chip.width == 0));
        assert_eq!(strip.after, Some(Rect::new(43, 0, 1, 1)));
        assert_eq!(strip.before, None);
    }

    #[test]
    fn a_long_title_first_still_leaves_the_active_chip_and_its_marks() {
        let title = "FEAT-1 · a title far too long for any detail pane to hold whole";
        let pane = Pane::new(title).sub_tabs(&DETAIL, 3).title_first();
        let (buffer, _) = draw(45, 4, &Theme::default(), pane);
        let top = row(&buffer, 0);
        assert!(top.starts_with("╭─ FEAT-1 · a title"), "{top}");
        assert!(top.ends_with("…‹ Docs ›╮"), "{top}");
        let strip = chip_strip_after(Rect::new(0, 0, 45, 4), title, &DETAIL, 3);
        assert_eq!(strip.chips[3].width, 6, "the active chip is whole");
        assert!(strip.before.is_some() && strip.after.is_some(), "{strip:?}");
        assert_eq!(
            (0..DETAIL.len())
                .filter(|&i| strip.chips[i].width > 0)
                .count(),
            1
        );
    }

    #[test]
    fn title_first_with_room_for_every_chip_is_the_default_layout() {
        let theme = Theme::default();
        for active in 0..DETAIL.len() {
            let pane = || {
                Pane::new("FEAT-1 · TUI scaffold")
                    .sub_tabs(&DETAIL, active)
                    .focus(Focus::Focused)
            };
            let (plain, _) = draw(100, 4, &theme, pane());
            let (first, _) = draw(100, 4, &theme, pane().title_first());
            assert_eq!(plain, first, "active {active}");
            assert_eq!(
                chip_strip_after(
                    Rect::new(0, 0, 100, 4),
                    "FEAT-1 · TUI scaffold",
                    &DETAIL,
                    active
                ),
                chip_strip(Rect::new(0, 0, 100, 4), &DETAIL, active)
            );
        }
        assert_eq!(
            chip_strip_after(Rect::new(0, 0, 45, 4), "", &DETAIL, 2),
            chip_strip(Rect::new(0, 0, 45, 4), &DETAIL, 2),
            "no title, the whole strip"
        );
    }

    /// M3 D-10 still holds for a pane that does not ask for its title first (Settings).
    #[test]
    fn the_default_precedence_is_unchanged() {
        let pane = Pane::new("FEAT-1 · TUI scaffold").sub_tabs(&DETAIL, 0);
        let (buffer, _) = draw(45, 4, &Theme::default(), pane);
        let top = row(&buffer, 0);
        assert!(!top.contains("FEAT-1"), "the title gives way: {top}");
        assert!(top.contains(" Body "), "{top}");
    }

    #[test]
    fn a_border_style_overrides_the_focus() {
        let theme = Theme::default();
        let status = Style::new().fg(Color::Red);
        for focus in [Focus::Focused, Focus::Unfocused] {
            let pane = Pane::new("node")
                .focus(focus)
                .accent(accent())
                .border(status);
            let (buffer, _) = draw(20, 4, &theme, pane);
            for (x, y) in [(0, 0), (19, 0), (0, 3), (19, 3), (5, 3), (0, 1)] {
                assert_eq!(buffer[(x, y)].fg, Color::Red, "{focus:?} ({x},{y})");
            }
        }
    }

    #[test]
    fn render_into_draws_what_render_draws() {
        let theme = Theme::default();
        let pane = || {
            Pane::new("Runs")
                .sub_tabs(&DETAIL, 1)
                .hints(Line::raw("j/k move"))
                .note(Line::raw("hidden"))
                .scrollbar(40, 3)
                .focus(Focus::Focused)
                .accent(accent())
        };
        let (drawn, inner) = draw(50, 8, &theme, pane());
        let area = Rect::new(0, 0, 50, 8);
        let mut buffer = Buffer::empty(area);
        let into = pane().render_into(&mut buffer, area, &theme);
        assert_eq!(into, inner);
        assert_eq!(buffer, drawn);
    }

    /// MOD-115 M2 §7 (RV3): a detail that takes every key gets the whole area only while another
    /// pane is zoomed; otherwise each pane keeps the rect the layout gave it.
    #[test]
    fn capture_area_gives_a_capturing_detail_the_area_only_when_zoomed_away() {
        let area = Rect::new(0, 0, 100, 30);
        let list = Rect::new(0, 0, 55, 30);
        let detail = Rect::new(55, 0, 45, 30);
        let shown = PaneAreas {
            list: Some(list),
            detail,
            activity: None,
            list_hidden: None,
            activity_hidden: None,
        };
        assert_eq!(capture_area(&shown, area, true), (Some(list), detail));
        assert_eq!(capture_area(&shown, area, false), (Some(list), detail));
        let zoomed = PaneAreas {
            list: Some(area),
            detail: Rect::default(),
            ..shown
        };
        assert_eq!(
            capture_area(&zoomed, area, false),
            (Some(area), Rect::default())
        );
        assert_eq!(capture_area(&zoomed, area, true), (None, area));
    }
}

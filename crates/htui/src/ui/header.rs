//! The one-row header that replaces the top bar and the tab strip (MOD-115): workspace and tab
//! chips on the left, working and waiting counts, store, box and the key chips on the right
//! (`R-TUI-1`, `R-TUI-11`).
//!
//! Every field comes from [`TopBarState`], which only [`App::observe_reply`](crate::app::App)
//! writes, so the header cannot disagree with what the store answered. The right side is dropped
//! part by part when the row is narrow, lowest priority first (box name, key chips, working
//! count, store, waiting count); a [`Flash`] replaces it altogether and, being the one thing the
//! person must read, is truncated only after the tab chips are gone and the workspace is cut to a
//! few cells. [`render`] returns the rects it drew the workspace and each tab chip into
//! ([`HeaderHits`]); M1 wires no mouse handler to them.
//!
//! The key chips read `F9 about ? keys`; both chords come from [`Keys`].

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::TopBarState;
use crate::keys::{Act, Keys, Stack};
use crate::ui::kit::chips;
use crate::ui::tabs::registry::{TabId, TabRegistry};
use crate::ui::{Theme, cells};

/// Separator between the right side's parts.
const SEP: &str = " \u{b7} ";

/// Cells between the workspace, the tab chips and the right side.
const GAP: usize = 2;

/// The fewest cells of the workspace name kept (ellipsis included) while a flash needs the room.
const WORKSPACE_FLOOR: usize = 4;

/// Superscript digits for the first nine tabs; a tenth tab and beyond are written as plain digits.
const SUPERSCRIPT: [&str; 9] = [
    "\u{b9}", "\u{b2}", "\u{b3}", "\u{2074}", "\u{2075}", "\u{2076}", "\u{2077}", "\u{2078}",
    "\u{2079}",
];

/// The key chips at the right end of the header: `(action, word)`, each label resolved through
/// [`Keys`] so a rebound action shows its new chord and an unbound one is skipped: `F9 about ? keys`.
const HEADER_CHIPS: &[(Act, &str)] = &[(Act::About, "about"), (Act::Help, "keys")];

/// A message that replaces the header's right side for a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flash {
    /// The text.
    pub text: String,
    /// Its style.
    pub style: Style,
}

/// Where the header's clickable parts landed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderHits {
    /// The workspace name.
    pub workspace: Rect,
    /// Each tab chip that fitted.
    pub tabs: Vec<(TabId, Rect)>,
}

/// MOD-80 D6: the store label in `warning` while the store is degraded (`connecting`, or
/// `offline \u{b7} <age>`, `Backend::label`) and `base` for `online` and `memory`.
fn store_style(label: &str, theme: &Theme) -> Style {
    if label == "connecting" || label.starts_with("offline") {
        theme.warning
    } else {
        theme.base
    }
}

/// `n` cells as a `u16`, saturating.
fn cells_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// The cells `spans` occupy.
fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| cells::cell_width(&s.content)).sum()
}

/// One tab chip, ready to draw.
struct TabChip {
    id: TabId,
    span: Span<'static>,
    width: usize,
}

/// The style of the active chip: a filled block in the tab's accent colour, or, when the accent
/// names no colour (monochrome), the theme's bold and underlined `active_tab`.
fn active_style(accent: Style, theme: &Theme) -> Style {
    match accent.fg {
        Some(colour) => Style::new()
            .fg(Color::Black)
            .bg(colour)
            .add_modifier(Modifier::BOLD),
        None => theme.active_tab,
    }
}

/// Every registered tab as a chip ` \u{b9}Backlog `: its position as a superscript digit, then
/// its title. The active one is [`active_style`]; the others are dim.
fn tab_chips(tabs: &TabRegistry, theme: &Theme) -> Vec<TabChip> {
    let active = tabs.active_id();
    tabs.titles()
        .into_iter()
        .enumerate()
        .map(|(index, (id, title))| {
            let number = SUPERSCRIPT
                .get(index)
                .map_or_else(|| (index + 1).to_string(), |s| (*s).to_owned());
            let text = format!(" {number}{title} ");
            let style = if Some(id) == active {
                active_style(theme.tab_accent(id.0), theme)
            } else {
                theme.dim
            };
            TabChip {
                id,
                width: cells::cell_width(&text),
                span: Span::styled(text, style),
            }
        })
        .collect()
}

/// One part of the header's right side. A lower `rank` is kept longer when the row is narrow.
struct Part {
    rank: u8,
    spans: Vec<Span<'static>>,
    width: usize,
}

impl Part {
    fn text(rank: u8, text: String, style: Style) -> Self {
        Self {
            rank,
            width: cells::cell_width(&text),
            spans: vec![Span::styled(text, style)],
        }
    }
}

/// The right side's parts in display order: `N working`, `M waiting`, store, box, key chips.
fn right_parts(state: &TopBarState, keys: &Keys, theme: &Theme) -> Vec<Part> {
    let (working, waiting) = state
        .waiting
        .as_ref()
        .map_or((0, 0), |view| (view.working, view.waiting()));
    // MOD-69 plan T3.3, MOD-80 D6: the waiting part in `warning` when a person is owed
    // something; no plural rule, so "1 working \u{b7} 1 waiting" reads as written.
    let waiting_style = if waiting > 0 {
        theme.warning
    } else {
        theme.base
    };
    let box_name = if state.box_name.is_empty() {
        "no box"
    } else {
        &state.box_name
    };
    let mut parts = vec![
        Part::text(3, format!("{working} working"), theme.base),
        Part::text(1, format!("{waiting} waiting"), waiting_style),
    ];
    if !state.store.is_empty() {
        parts.push(Part::text(
            2,
            state.store.clone(),
            store_style(&state.store, theme),
        ));
    }
    parts.push(Part::text(5, box_name.to_owned(), theme.base));
    let pairs: Vec<(String, &str)> = HEADER_CHIPS
        .iter()
        .map(|(act, word)| (keys.label(Stack::BASE, *act).unwrap_or_default(), *word))
        .collect();
    let line = chips::chip_line(&pairs, theme);
    if !line.spans.is_empty() {
        parts.push(Part {
            rank: 4,
            width: spans_width(&line.spans),
            spans: line.spans,
        });
    }
    parts
}

/// The cells `parts` take joined by [`SEP`].
fn joined_width(parts: &[Part]) -> usize {
    let text: usize = parts.iter().map(|p| p.width).sum();
    text + cells::cell_width(SEP) * parts.len().saturating_sub(1)
}

/// `parts` with the lowest-priority ones removed until they fit in `room` cells, as one line of
/// spans and its width; `None` when not even the highest-priority part fits.
fn fit_right(mut parts: Vec<Part>, room: usize) -> Option<(Vec<Span<'static>>, usize)> {
    while joined_width(&parts) > room {
        let worst = parts.iter().enumerate().max_by_key(|(_, p)| p.rank)?.0;
        parts.remove(worst);
    }
    if parts.is_empty() {
        return None;
    }
    let width = joined_width(&parts);
    Some((join(parts), width))
}

/// `parts` joined by a dim [`SEP`].
fn join(parts: Vec<Part>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(SEP));
        }
        spans.extend(part.spans);
    }
    spans
}

/// Draws the header into a one-row `area` and returns where its clickable parts landed.
///
/// With a `flash` the right side is the flash alone; the tab chips give way to it first, then the
/// workspace name is cut, and only then is the flash itself truncated. Without one the left side
/// (workspace, tab chips) is drawn whole where it fits and the right side keeps as many parts as
/// the rest of the row allows.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TopBarState,
    tabs: &TabRegistry,
    flash: Option<&Flash>,
    keys: &Keys,
    theme: &Theme,
) -> HeaderHits {
    let width = usize::from(area.width);
    if width == 0 || area.height == 0 {
        return HeaderHits::default();
    }
    let workspace = if state.workspace.is_empty() {
        "no workspace"
    } else {
        &state.workspace
    };

    // The flash is placed first: nothing but the workspace's floor is allowed to take its room.
    let flash_text = flash.map(|flash| {
        let floor = cells::cell_width(workspace).min(WORKSPACE_FLOOR);
        let room = width.saturating_sub(floor + GAP);
        (cells::clip(&flash.text, room), flash.style)
    });
    let mut right: Option<(Vec<Span<'static>>, usize)> = flash_text
        .filter(|(text, _)| !text.is_empty())
        .map(|(text, style)| {
            let cells = cells::cell_width(&text);
            (vec![Span::styled(text, style)], cells)
        });
    let reserved = right.as_ref().map_or(0, |(_, cells)| cells + GAP);

    let workspace_text = cells::clip(workspace, width.saturating_sub(reserved));
    let workspace_width = cells::cell_width(&workspace_text);

    let tabs_room = width.saturating_sub(workspace_width + GAP + reserved);
    let mut shown: Vec<TabChip> = Vec::new();
    let mut tabs_width = 0;
    for chip in tab_chips(tabs, theme) {
        if tabs_width + chip.width > tabs_room {
            break;
        }
        tabs_width += chip.width;
        shown.push(chip);
    }

    if flash.is_none() {
        let used = workspace_width + if tabs_width > 0 { GAP + tabs_width } else { 0 };
        let room = width.saturating_sub(used + GAP);
        right = fit_right(right_parts(state, keys, theme), room);
    }

    let buf = frame.buffer_mut();
    let y = area.y;
    buf.set_stringn(
        area.x,
        y,
        &workspace_text,
        width,
        if state.workspace.is_empty() {
            theme.dim
        } else {
            theme.title
        },
    );
    let mut hits = HeaderHits {
        workspace: Rect::new(area.x, y, cells_u16(workspace_width), 1),
        tabs: Vec::with_capacity(shown.len()),
    };
    let mut x = area.x + cells_u16(workspace_width + GAP);
    for chip in shown {
        let rect = Rect::new(x, y, cells_u16(chip.width), 1);
        buf.set_span(x, y, &chip.span, rect.width);
        hits.tabs.push((chip.id, rect));
        x += rect.width;
    }
    if let Some((spans, cells)) = right {
        let at = area.right().saturating_sub(cells_u16(cells));
        buf.set_line(at, y, &Line::from(spans), cells_u16(cells));
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Context;
    use crate::ui::tabs::{BacklogTab, SkillsTab};
    use htui_core::model::ItemId;
    use htui_worker::{WaitingReason, WaitingRow, WaitingView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn registry() -> TabRegistry {
        let mut registry = TabRegistry::new();
        registry.register(Box::new(BacklogTab::new()));
        registry.register(Box::new(SkillsTab::new()));
        registry
    }

    fn state() -> TopBarState {
        TopBarState {
            workspace: "Platform".to_owned(),
            box_name: "DESKTOP-HTUI".to_owned(),
            store: "memory".to_owned(),
            waiting: Some(WaitingView {
                working: 1,
                rows: Vec::new(),
                permissions_known: true,
                offline: false,
            }),
        }
    }

    /// The header of `state` drawn `width` cells wide, and where its parts landed.
    fn draw_with(
        width: u16,
        state: &TopBarState,
        flash: Option<&Flash>,
        keys: &Keys,
        theme: &Theme,
    ) -> (Buffer, HeaderHits) {
        let tabs = registry();
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("a test terminal");
        let mut hits = HeaderHits::default();
        terminal
            .draw(|frame| {
                hits = render(frame, frame.area(), state, &tabs, flash, keys, theme);
            })
            .expect("draw");
        (terminal.backend().buffer().clone(), hits)
    }

    fn draw(width: u16, state: &TopBarState) -> (Buffer, HeaderHits) {
        draw_with(width, state, None, Keys::compiled(), &Theme::default())
    }

    fn line(buffer: &Buffer) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    fn row(key: &str) -> WaitingRow {
        WaitingRow {
            item: ItemId::new(),
            item_key: key.to_owned(),
            run: None,
            step: None,
            step_label: String::new(),
            reason: WaitingReason::Unblock,
            text: "blocked".to_owned(),
            since: chrono::DateTime::UNIX_EPOCH,
        }
    }

    /// The column of the first cell of `needle` in `buffer`'s row, counting cells.
    fn column(buffer: &Buffer, needle: &str) -> u16 {
        let text = line(buffer);
        let byte = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} in {text:?}"));
        cells_u16(text[..byte].chars().count())
    }

    /// The header, wide enough for everything, reads left to right: workspace, tab chips, then
    /// working, waiting, store, box and the key chips.
    #[test]
    fn a_wide_header_shows_every_part_in_order() {
        let (buffer, _) = draw(120, &state());
        let text = line(&buffer);
        let order = [
            "Platform",
            "\u{b9}Backlog",
            "\u{b2}Skills",
            "1 working",
            "0 waiting",
            "memory",
            "DESKTOP-HTUI",
            "F9",
            "about",
            "?",
            "keys",
        ];
        let columns: Vec<u16> = order.iter().map(|part| column(&buffer, part)).collect();
        assert!(columns.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert!(text.ends_with("keys"), "the chips end the row: {text}");
    }

    /// Narrowing drops the right side's parts lowest priority first: box, key chips, working,
    /// store; the waiting count outlives them all, and the left side is never cut for them.
    #[test]
    fn the_right_side_drops_its_lowest_priority_part_first_when_narrow() {
        // Left is 8 + 2 + 10 + 9 = 29 cells; the right side needs 68, joined, and a gap of 2.
        let kept = |width: u16| -> Vec<&'static str> {
            let (buffer, _) = draw(width, &state());
            let text = line(&buffer);
            assert!(
                text.contains("Platform") && text.contains("Skills"),
                "{text}"
            );
            ["DESKTOP-HTUI", "keys", "1 working", "memory", "0 waiting"]
                .into_iter()
                .filter(|part| text.contains(part))
                .collect()
        };
        assert_eq!(
            kept(99),
            ["DESKTOP-HTUI", "keys", "1 working", "memory", "0 waiting"]
        );
        assert_eq!(kept(98), ["keys", "1 working", "memory", "0 waiting"]);
        assert_eq!(kept(70), ["1 working", "memory", "0 waiting"]);
        assert_eq!(kept(55), ["memory", "0 waiting"]);
        assert_eq!(kept(45), ["0 waiting"]);
        assert!(kept(35).is_empty());
    }

    /// A flash replaces the whole right side, in its own style, and leaves the left alone.
    #[test]
    fn a_flash_replaces_the_right_side() {
        let theme = Theme::default();
        let flash = Flash {
            text: "save failed".to_owned(),
            style: theme.error,
        };
        let (buffer, _) = draw_with(100, &state(), Some(&flash), Keys::compiled(), &theme);
        let text = line(&buffer);
        assert!(text.starts_with("Platform"), "{text}");
        assert!(text.contains("\u{b9}Backlog"), "{text}");
        assert!(text.ends_with("save failed"), "{text}");
        for gone in ["working", "waiting", "memory", "DESKTOP-HTUI", "keys"] {
            assert!(!text.contains(gone), "{gone}: {text}");
        }
        let at = column(&buffer, "save failed");
        assert_eq!(buffer[(at, 0)].fg, Color::Red);
    }

    /// The sticky editor status is the flash: the tab chips yield to it, then the workspace, and
    /// only a flash wider than the row is truncated, with an ellipsis, at the right edge.
    #[test]
    fn a_sticky_editor_status_survives_truncation() {
        let theme = Theme::default();
        let status = "editing FEAT-12 body in nvim, :wq to save";
        let flash = Flash {
            text: status.to_owned(),
            style: theme.dim,
        };
        // 8 + 2 + 41 = 51 fits in 60: whole, but no tab chip is left beside it.
        let (roomy, hits) = draw_with(60, &state(), Some(&flash), Keys::compiled(), &theme);
        let text = line(&roomy);
        assert!(text.starts_with("Platform"), "{text}");
        assert!(text.ends_with(status), "{text}");
        assert!(!text.contains("Backlog"), "{text}");
        assert!(hits.tabs.is_empty());
        // Narrower: the workspace is cut to what is left before the flash loses a cell.
        let (tight, _) = draw_with(48, &state(), Some(&flash), Keys::compiled(), &theme);
        let text = line(&tight);
        assert!(text.ends_with(status), "{text}");
        assert!(text.starts_with("Plat\u{2026}"), "{text}");
        // Narrower than the status itself: the flash is truncated, at the edge, not dropped.
        let (cut, _) = draw_with(30, &state(), Some(&flash), Keys::compiled(), &theme);
        let text = line(&cut);
        assert!(text.contains("editing FEAT-12"), "{text}");
        assert!(text.ends_with('\u{2026}'), "{text}");
        assert_eq!(cells::cell_width(&text), 30);
    }

    /// A tab chip that does not fit is not offered as a click target.
    #[test]
    fn hits_name_the_workspace_and_every_tab_chip_that_was_drawn() {
        let (buffer, hits) = draw(120, &state());
        assert_eq!(hits.workspace, Rect::new(0, 0, 8, 1));
        assert_eq!(
            hits.tabs,
            vec![
                (TabId("backlog"), Rect::new(10, 0, 10, 1)),
                (TabId("skills"), Rect::new(20, 0, 9, 1)),
            ]
        );
        let text = |rect: Rect| -> String {
            (rect.x..rect.right())
                .map(|x| buffer[(x, 0)].symbol())
                .collect()
        };
        assert_eq!(text(hits.workspace), "Platform");
        assert_eq!(text(hits.tabs[0].1), " \u{b9}Backlog ");
        assert_eq!(text(hits.tabs[1].1), " \u{b2}Skills ");
        // Too narrow for the second chip: only the first is a target.
        let (_, narrow) = draw(25, &state());
        assert_eq!(narrow.tabs.len(), 1, "{narrow:?}");
        assert_eq!(narrow.tabs[0].0, TabId("backlog"));
    }

    #[test]
    fn the_active_tab_is_a_block_in_its_accent_and_the_others_are_dim() {
        let theme = Theme::default();
        let (buffer, hits) = draw(120, &state());
        let active = &buffer[(hits.tabs[0].1.x + 1, 0)];
        assert_eq!(active.bg, Color::Cyan, "the backlog accent as a block");
        assert_eq!(active.fg, Color::Black);
        assert!(active.modifier.contains(Modifier::BOLD));
        let idle = &buffer[(hits.tabs[1].1.x + 1, 0)];
        assert_eq!(idle.fg, theme.dim.fg.expect("dim has a colour"));
        assert_eq!(idle.bg, Color::Reset);
    }

    /// MOD-80 D5: under the monochrome theme only modifiers can mark the active tab, so it is
    /// bold and underlined and the others are neither; no cell names a colour.
    #[test]
    fn the_active_tab_is_bold_and_underlined_without_colour() {
        let theme = Theme::monochrome();
        let (buffer, hits) = draw_with(120, &state(), None, Keys::compiled(), &theme);
        let marked = Modifier::BOLD | Modifier::UNDERLINED;
        let active = &buffer[(hits.tabs[0].1.x + 1, 0)];
        assert_eq!(active.symbol(), "\u{b9}");
        assert!(active.modifier.contains(marked), "{active:?}");
        let other = &buffer[(hits.tabs[1].1.x + 1, 0)];
        assert!(!other.modifier.intersects(marked), "{other:?}");
        for x in 0..120 {
            assert_eq!(buffer[(x, 0)].fg, Color::Reset, "fg at x = {x}");
            assert_eq!(buffer[(x, 0)].bg, Color::Reset, "bg at x = {x}");
        }
    }

    /// The chips' chords are whatever `Keys` says: `F9 about ? keys` by default, help rebound to
    /// `F12` reads `F12`, unbound the chip (and its word) is gone, never a bare word.
    #[test]
    fn a_key_chip_is_labelled_through_keys() {
        let theme = Theme::default();
        assert!(line(&draw(120, &state()).0).ends_with(" F9  about   ?  keys"));
        let f12 = Keys::defaults().with_chords(Context::Global, Act::Help, &["f12"]);
        let (buffer, _) = draw_with(120, &state(), None, &f12, &theme);
        assert!(line(&buffer).ends_with(" F12  keys"), "{}", line(&buffer));
        let unbound = Keys::defaults().with_chords(Context::Global, Act::Help, &[]);
        let (buffer, _) = draw_with(120, &state(), None, &unbound, &theme);
        let text = line(&buffer);
        assert!(
            !text.contains("keys") && text.ends_with(" F9  about"),
            "{text}"
        );
        let no_about = Keys::defaults().with_chords(Context::Global, Act::About, &[]);
        let (buffer, _) = draw_with(120, &state(), None, &no_about, &theme);
        let text = line(&buffer);
        assert!(
            !text.contains("about") && text.ends_with(" ?  keys"),
            "{text}"
        );
    }

    /// MOD-69 plan T3.3, MOD-80 D6: `M waiting` is a warning exactly when a person is owed
    /// something.
    #[test]
    fn the_waiting_count_is_a_warning_only_when_non_zero() {
        let theme = Theme::default();
        let mut state = state();
        let (idle, _) = draw(120, &state);
        let at = column(&idle, "0 waiting");
        assert_ne!(idle[(at, 0)].style().fg, theme.warning.fg, "nothing owed");

        state.waiting = Some(WaitingView {
            working: 0,
            rows: vec![row("FEAT-2"), row("FEAT-3")],
            permissions_known: true,
            offline: false,
        });
        let (owed, _) = draw(120, &state);
        let at = column(&owed, "2 waiting");
        assert_eq!(owed[(at, 0)].symbol(), "2");
        assert_eq!(owed[(at, 0)].style().fg, theme.warning.fg);
        let working = column(&owed, "0 working");
        assert_ne!(
            owed[(working, 0)].style().fg,
            theme.warning.fg,
            "working is never a warning"
        );
    }

    /// MOD-80 D6: `connecting` and `offline \u{b7} <age>` are degraded; `online` and `memory` are not.
    #[test]
    fn store_style_warns_only_while_degraded() {
        let theme = Theme::default();
        assert_eq!(store_style("online", &theme), theme.base);
        assert_eq!(store_style("memory", &theme), theme.base);
        assert_eq!(store_style("connecting", &theme), theme.warning);
        assert_eq!(store_style("offline \u{b7} 3m", &theme), theme.warning);
    }

    /// MOD-80 review L3: `store_style` keys on the text `Backend::label` writes, so it is fed
    /// the real labels: a renamed label fails here rather than silently drawing `base`.
    #[tokio::test]
    async fn store_style_reads_the_labels_the_backend_writes() {
        use chrono::{TimeDelta, Utc};
        use htui_core::model::BoxId;
        use htui_core::store::MemStore;
        use htui_store::{Backend, CacheStore, Identity, PgStore};

        let theme = Theme::default();
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "top-bar-labels", 1)
            .await
            .expect("open a throwaway mirror");
        let identity = Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            std::time::Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");
        let cases = [
            (Backend::memory(MemStore::new()), theme.base),
            (
                Backend::Online {
                    pg,
                    cache: cache.clone(),
                },
                theme.base,
            ),
            (
                Backend::Offline {
                    cache: cache.clone(),
                    since: None,
                },
                theme.warning,
            ),
            (
                Backend::Offline {
                    cache,
                    since: Some(Utc::now() - TimeDelta::seconds(220)),
                },
                theme.warning,
            ),
        ];
        for (backend, style) in cases {
            let label = backend.label();
            assert_eq!(store_style(&label, &theme), style, "{label}");
        }
    }

    /// MOD-80 D6: the header draws a degraded store label as a warning.
    #[test]
    fn a_degraded_store_label_is_drawn_as_a_warning() {
        let theme = Theme::default();
        let warning = theme.warning.fg.expect("warning has a colour");
        for (store, fg) in [
            ("offline \u{b7} 3m", warning),
            ("connecting", warning),
            ("online", Color::Reset),
        ] {
            let mut state = state();
            state.store = store.to_owned();
            let (buffer, _) = draw(120, &state);
            let at = column(&buffer, store);
            assert_eq!(buffer[(at, 0)].fg, fg, "{store}: {}", line(&buffer));
        }
        assert_eq!(warning, Color::Yellow);
    }

    /// MOD-69 blueprint A-6: before the first reply the header reads zero for both counts and
    /// names no store.
    #[test]
    fn before_the_first_reply_both_counts_read_zero() {
        let (buffer, _) = draw(120, &TopBarState::default());
        let text = line(&buffer);
        assert!(text.starts_with("no workspace"), "{text}");
        assert!(
            text.ends_with("0 working \u{b7} 0 waiting \u{b7} no box \u{b7}  F9  about   ?  keys"),
            "{text}"
        );
    }

    #[test]
    fn a_zero_sized_area_draws_nothing_and_hits_nothing() {
        let tabs = registry();
        let mut terminal = Terminal::new(TestBackend::new(10, 2)).expect("terminal");
        let mut hits = HeaderHits::default();
        terminal
            .draw(|frame| {
                hits = render(
                    frame,
                    Rect::new(0, 0, 0, 1),
                    &state(),
                    &tabs,
                    None,
                    Keys::compiled(),
                    &Theme::default(),
                );
            })
            .expect("draw");
        assert_eq!(hits, HeaderHits::default());
    }
}

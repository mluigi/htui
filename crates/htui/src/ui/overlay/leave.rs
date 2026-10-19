//! The leave dialog (MOD-115 M2 D6, blueprint §9): what leaving htui stops, `Enter` (or a second
//! `ctrl-c`) leaves, `Esc` stays. It replaces the quit question, and `global.quit` always opens
//! it (`R-TUI-18`).
//!
//! The wording promises nothing about work surviving the exit (MD6, PRD Q3): leaving closes htui
//! and the in-process work stops with it (`RunRuntime::shutdown` aborts after a grace window).
//! There is no "stop all" choice yet: until MOD-116/MOD-125 it would do what leaving does.
//!
//! What is live comes from the same counts the header reads (`TopBarState::waiting`, the open
//! workspace only; other workspaces are listed once MOD-116 tracks them) and from the open
//! in-pane edit, `Ctx::open_edit`. Before the first count lands the dialog cannot know, so it
//! says work may be running and shows `–` for both counts.
//!
//! Every answer resolves through [`Keys`] on [`views::LEAVE`], and the chips name the chords in
//! force. Shaped like the quit question it replaces: a centred, modal box sized to its text.
//! Task 6 registers it and deletes `quit_confirm.rs`.

use crossterm::event::KeyEvent;
use htui_core::model::{Scope, Status};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::keys::{Act, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::chips::chip_line;
use crate::ui::kit::modal::{Modal, ModalSize};
use crate::ui::kit::status::mark;
use crate::ui::overlay::registry::{Overlay, OverlayId};

/// The box's title, which is also its question.
const TITLE: &str = "Leave htui?";

/// What the box says when something is live (D6).
const LIVE: &str = "Leaving closes htui. Work it is running stops with it:";

/// What the box says when nothing is live (D6).
const NOTHING_LIVE: &str = "Nothing is running. Leave htui?";

/// The open edit's row (D6): leaving drops the editor with `App`.
const OPEN_EDIT: &str = "\u{270e} an open edit (unsaved changes are lost)";

/// A count the dialog does not know yet: no `Waiting` reply has landed.
const UNKNOWN: &str = "\u{2013}";

/// The margin in front of each line of the box.
const LEAD: &str = " ";

/// The indent of a live row under the sentence.
const ROW: &str = "  ";

/// Between the two counts.
const SEP: &str = " \u{b7} ";

/// Between the counts and the workspace name.
const WORKSPACE_GAP: &str = "   ";

/// The words after the two chips.
const LEAVE_WORD: &str = "leave";
const STAY_WORD: &str = "stay";

/// Asks before `global.quit` closes htui, naming what stops with it.
#[derive(Debug, Default)]
pub struct LeaveDialog;

impl LeaveDialog {
    /// Identity of the dialog; Task 6's `register_all` registers the factory under it.
    pub const ID: OverlayId = OverlayId("leave");

    /// The dialog.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// The box's contents: the sentence, the live rows when something is live, a blank line,
    /// then the two answers as key chips. Live means a run working or a row waiting in the open
    /// workspace, an open in-pane edit, or counts not known yet (the dialog does not claim that
    /// nothing runs before it knows). The chip labels come from `Keys` through
    /// [`views::LEAVE`], so a rebound answer shows its new chord.
    fn lines(ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let counts = ctx
            .top_bar
            .waiting
            .as_ref()
            .map(|view| (view.working, view.waiting()));
        let editing = ctx.open_edit().is_some();
        let live = editing || counts.is_none_or(|(working, waiting)| working + waiting > 0);

        let mut lines = Vec::new();
        if live {
            lines.push(Line::styled(format!("{LEAD}{LIVE}"), theme.base));
            lines.push(Self::counts_row(ctx, counts));
            if editing {
                lines.push(Line::styled(format!("{LEAD}{ROW}{OPEN_EDIT}"), theme.base));
            }
        } else {
            lines.push(Line::styled(format!("{LEAD}{NOTHING_LIVE}"), theme.base));
        }
        lines.push(Line::raw(""));

        let keys = ctx.keys();
        let answers = chip_line(
            &[
                (
                    keys.label(views::LEAVE, Act::LeaveConfirm)
                        .unwrap_or_default(),
                    LEAVE_WORD,
                ),
                (
                    keys.label(views::LEAVE, Act::LeaveStay).unwrap_or_default(),
                    STAY_WORD,
                ),
            ],
            theme,
        );
        let mut chips = vec![Span::raw(LEAD)];
        chips.extend(answers.spans);
        lines.push(Line::from(chips));
        lines
    }

    /// `  ◐ 3 working · ⚠ 1 waiting   Graphics`: the header's two counts for the open workspace,
    /// with the status glyphs, the waiting count in `warning` while someone waits, `–` for both
    /// while no count has landed, then the workspace name when there is one.
    fn counts_row(ctx: &Ctx<'_>, counts: Option<(usize, usize)>) -> Line<'static> {
        let theme = ctx.theme;
        let (working, waiting) = counts.map_or_else(
            || (UNKNOWN.to_owned(), UNKNOWN.to_owned()),
            |(working, waiting)| (working.to_string(), waiting.to_string()),
        );
        let waiting_style = match counts {
            Some((_, waiting)) if waiting > 0 => theme.warning,
            _ => theme.base,
        };
        let working_glyph = mark(Status::InProgress).glyph;
        let waiting_glyph = mark(Status::AwaitingApproval).glyph;
        let mut spans = vec![
            Span::styled(format!("{LEAD}{ROW}"), theme.base),
            Span::styled(format!("{working_glyph} {working} working"), theme.base),
            Span::styled(SEP, theme.base),
            Span::styled(format!("{waiting_glyph} {waiting} waiting"), waiting_style),
        ];
        let workspace = &ctx.top_bar.workspace;
        if !workspace.is_empty() {
            spans.push(Span::styled(
                format!("{WORKSPACE_GAP}{workspace}"),
                theme.base,
            ));
        }
        Line::from(spans)
    }
}

impl Overlay for LeaveDialog {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        TITLE
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::LEAVE)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // `Enter` and `ctrl-c` are `leave.confirm`; `Esc` and `n` are `leave.stay`.
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::LEAVE, chord) {
            match act {
                Act::LeaveConfirm => {
                    // No `Close`: the loop stops and `App` drops, the editor with it.
                    ctx.emit(Action::Quit);
                    return Handled::Consumed;
                }
                Act::LeaveStay => {
                    ctx.emit(Action::Overlay(OverlayAction::Close));
                    return Handled::Consumed;
                }
                _ => continue, // `global.help`: the shell's
            }
        }
        // Everything else is swallowed by `is_modal`: `q` opens no second dialog.
        Handled::Pass
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let lines = Self::lines(ctx);
        let widest = lines.iter().map(Line::width).max().unwrap_or(0);
        let content = Modal::new(self.title())
            .size(ModalSize::Fit {
                content_w: u16::try_from(widest).unwrap_or(u16::MAX),
                content_h: u16::try_from(lines.len()).unwrap_or(u16::MAX),
            })
            .render(frame, area, ctx.theme);
        frame.render_widget(Paragraph::new(lines), content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keys::{Keys, load_str};
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use htui_core::model::{ItemId, ProjectRef, WorkspaceId};
    use htui_worker::{WaitingReason, WaitingRow, WaitingView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// Everything a `Ctx` borrows; `keys` replaces the compiled defaults when set.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        theme: Theme,
        keys: Option<Keys>,
        open_edit: Option<String>,
        emit: Emit,
    }

    /// One waiting row; only the count matters here.
    fn row() -> WaitingRow {
        WaitingRow {
            item: ItemId::new(),
            item_key: "BL-1".to_owned(),
            run: None,
            step: None,
            step_label: String::new(),
            reason: WaitingReason::Unblock,
            text: "blocked".to_owned(),
            since: chrono::DateTime::UNIX_EPOCH,
        }
    }

    impl Bench {
        /// Nothing live: the counts are in and both are zero, no edit is open.
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: Vec::new(),
                },
                projects: Vec::new(),
                top_bar: TopBarState {
                    workspace: "Graphics".to_owned(),
                    waiting: Some(WaitingView::default()),
                    ..TopBarState::default()
                },
                theme: Theme::default(),
                keys: None,
                open_edit: None,
                emit: Emit::default(),
            }
        }

        /// `working` runs and `waiting` rows in the open workspace.
        fn counting(mut self, working: usize, waiting: usize) -> Self {
            self.top_bar.waiting = Some(WaitingView {
                working,
                rows: (0..waiting).map(|_| row()).collect(),
                ..WaitingView::default()
            });
            self
        }

        /// No count has landed yet.
        fn uncounted(mut self) -> Self {
            self.top_bar.waiting = None;
            self
        }

        /// An in-pane edit is open.
        fn editing(mut self, label: &str) -> Self {
            self.open_edit = Some(label.to_owned());
            self
        }

        /// The bench with a loaded key file in force.
        fn with_keys(mut self, keys: Keys) -> Self {
            self.keys = Some(keys);
            self
        }

        fn with_theme(mut self, theme: Theme) -> Self {
            self.theme = theme;
            self
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                self.keys.as_ref().unwrap_or(Keys::compiled()),
                &self.theme,
                Origin::Overlay(LeaveDialog::ID),
                &self.emit,
            )
            .with_open_edit(self.open_edit.as_deref())
        }

        /// Sends one chord (a strict key-file spec) to the dialog.
        fn key(&self, dialog: &mut LeaveDialog, spec: &str) -> Handled {
            let chord = KeyChord::parse_strict(spec).expect("a valid spec");
            dialog.on_key(chord.to_event(), &mut self.ctx())
        }

        /// The dialog drawn over a blank 100×30 frame.
        fn terminal(&self) -> Terminal<TestBackend> {
            let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
            term.draw(|frame| LeaveDialog::new().render(frame, frame.area(), &self.ctx()))
                .expect("the overlay draws");
            term
        }

        /// The dialog drawn over a blank 100×30 frame, one line per row.
        fn render(&self) -> String {
            let term = self.terminal();
            let buffer = term.backend().buffer();
            let mut out = String::new();
            for y in 0..buffer.area.height {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    x += u16::try_from(cell_width(symbol).max(1)).unwrap_or(1);
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }
    }

    const LIVE: &str = "Leaving closes htui. Work it is running stops with it:";
    const NOTHING: &str = "Nothing is running. Leave htui?";
    const EDIT: &str = "✎ an open edit (unsaved changes are lost)";

    #[test]
    fn it_is_a_modal_named_leave_htui_on_the_leave_stack() {
        let dialog = LeaveDialog::new();
        assert_eq!(dialog.id(), OverlayId("leave"));
        assert_eq!(dialog.title(), "Leave htui?");
        assert!(dialog.is_modal());
        let stack = dialog.key_stack().expect("the dialog names its stack");
        let keys = Keys::compiled();
        let enter = KeyChord::parse_strict("enter").expect("a valid spec");
        assert_eq!(
            keys.actions(stack, enter),
            keys.actions(views::LEAVE, enter)
        );
        assert_eq!(keys.actions(stack, enter), [Act::LeaveConfirm]);
    }

    #[test]
    fn with_work_running_the_box_names_the_counts_and_the_workspace() {
        let frame = Bench::new().counting(3, 1).render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(
            frame.contains("  ◐ 3 working · ⚠ 1 waiting   Graphics"),
            "{frame}"
        );
        assert!(!frame.contains(EDIT), "no edit is open: {frame}");
        assert!(!frame.contains(NOTHING), "{frame}");
        assert!(frame.contains("Enter  leave"), "{frame}");
        assert!(frame.contains("Esc  stay"), "{frame}");
    }

    #[test]
    fn an_open_edit_is_live_and_gets_its_row() {
        let frame = Bench::new().editing("BL-1 body").render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(
            frame.contains("  ◐ 0 working · ⚠ 0 waiting   Graphics"),
            "{frame}"
        );
        assert!(frame.contains(&format!("   {EDIT}")), "{frame}");
    }

    #[test]
    fn a_waiting_row_alone_is_live() {
        let frame = Bench::new().counting(0, 2).render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(frame.contains("◐ 0 working · ⚠ 2 waiting"), "{frame}");
    }

    #[test]
    fn with_nothing_live_the_box_asks_one_question() {
        let frame = Bench::new().render();
        assert!(frame.contains(NOTHING), "{frame}");
        assert!(!frame.contains(LIVE), "{frame}");
        assert!(!frame.contains("working"), "no counts row: {frame}");
        assert!(!frame.contains(EDIT), "{frame}");
        assert!(frame.contains("Enter  leave"), "the same chips: {frame}");
        assert!(frame.contains("Esc  stay"), "the same chips: {frame}");
    }

    #[test]
    fn before_the_first_count_the_box_assumes_work_and_shows_dashes() {
        let frame = Bench::new().uncounted().render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(
            frame.contains("◐ – working · ⚠ – waiting   Graphics"),
            "{frame}"
        );
    }

    #[test]
    fn without_a_workspace_name_the_counts_row_ends_at_the_counts() {
        let mut bench = Bench::new().counting(1, 0);
        bench.top_bar.workspace = String::new();
        let lines = LeaveDialog::lines(&bench.ctx());
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        assert!(
            text.iter()
                .any(|l| l.ends_with("◐ 1 working · ⚠ 0 waiting")),
            "{text:?}"
        );
    }

    #[test]
    fn enter_and_ctrl_c_leave() {
        let bench = Bench::new().counting(3, 1);
        let mut dialog = LeaveDialog::new();
        for spec in ["enter", "ctrl-c"] {
            assert_eq!(bench.key(&mut dialog, spec), Handled::Consumed, "{spec}");
            let emitted = bench.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Quit]),
                "{spec} leaves and does nothing else: {emitted:?}"
            );
        }
    }

    #[test]
    fn esc_and_n_stay() {
        let bench = Bench::new().counting(3, 1);
        let mut dialog = LeaveDialog::new();
        for spec in ["esc", "n"] {
            assert_eq!(bench.key(&mut dialog, spec), Handled::Consumed, "{spec}");
            let emitted = bench.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Overlay(OverlayAction::Close)]),
                "{spec} closes the dialog and nothing else: {emitted:?}"
            );
        }
    }

    #[test]
    fn every_other_key_is_left_to_the_modal_swallow() {
        let bench = Bench::new();
        let mut dialog = LeaveDialog::new();
        for spec in ["q", "y", "N", "space", "ctrl-y", "ctrl-q", "?"] {
            assert_eq!(bench.key(&mut dialog, spec), Handled::Pass, "{spec}");
            assert!(bench.emit.is_empty(), "{spec} emits nothing");
        }
    }

    #[test]
    fn the_chips_and_the_answer_follow_a_rebind() {
        let keys = load_str("[leave]\nconfirm = [\"f8\"]\n").expect("the file loads");
        let bench = Bench::new().with_keys(keys);

        let frame = bench.render();
        assert!(frame.contains("F8  leave"), "{frame}");
        assert!(!frame.contains("Enter  leave"), "{frame}");
        assert!(frame.contains("Esc  stay"), "{frame}");

        let mut dialog = LeaveDialog::new();
        assert_eq!(bench.key(&mut dialog, "f8"), Handled::Consumed);
        assert!(matches!(bench.emit.take().as_slice(), [Action::Quit]));
        assert_eq!(
            bench.key(&mut dialog, "enter"),
            Handled::Pass,
            "enter is unbound"
        );
        assert!(bench.emit.is_empty());
    }

    #[test]
    fn the_waiting_count_is_a_warning_only_when_someone_waits() {
        let theme = Theme::default();
        let style_of = |bench: &Bench| {
            LeaveDialog::lines(&bench.ctx())
                .iter()
                .flat_map(|line| line.spans.iter())
                .find(|span| span.content.contains("waiting"))
                .map(|span| span.style)
                .expect("a waiting span")
        };
        assert_eq!(style_of(&Bench::new().counting(0, 1)), theme.warning);
        assert_eq!(style_of(&Bench::new().counting(1, 0)), theme.base);
        assert_eq!(style_of(&Bench::new().uncounted()), theme.base);
        assert_ne!(theme.warning, theme.base);
    }

    #[test]
    fn under_monochrome_no_cell_names_a_colour() {
        for bench in [
            Bench::new(),
            Bench::new().counting(3, 1).editing("BL-1 body"),
            Bench::new().uncounted(),
        ] {
            let term = bench.with_theme(Theme::monochrome()).terminal();
            let buffer = term.backend().buffer();
            for cell in &buffer.content {
                assert_eq!(
                    (cell.fg, cell.bg),
                    (Color::Reset, Color::Reset),
                    "{:?}",
                    cell.symbol()
                );
            }
        }
    }

    #[test]
    fn the_live_box() {
        insta::assert_snapshot!(Bench::new().counting(3, 1).render());
    }

    #[test]
    fn the_box_with_an_open_edit() {
        insta::assert_snapshot!(Bench::new().counting(3, 1).editing("BL-1 body").render());
    }

    #[test]
    fn the_box_with_nothing_live() {
        insta::assert_snapshot!(Bench::new().render());
    }

    #[test]
    fn the_box_before_the_first_count() {
        insta::assert_snapshot!(Bench::new().uncounted().render());
    }
}

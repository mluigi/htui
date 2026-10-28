//! The `R-STO-5` confirmation: pending migrations are never applied without a human (plan D11).
//!
//! The shell opens this overlay when a [`StoreReply::StoreState`] carries a pending count; `y`
//! (`confirm.yes`) applies, `n` and `Esc` (`confirm.no`) leave the schema alone and the shell
//! reading from the local mirror. It holds no store handle and no channel: the count arrives
//! through `on_reply`, the decision leaves through `Ctx` (`R-NF-3`).
//!
//! Drawn on the MOD-115 kit (M4 E6, C7): a [`kit::modal::Ask`](Ask), the question in the box and
//! its keys on the bottom border, never blank.

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::modal::Ask;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crossterm::event::KeyEvent;

/// The keys on the bottom border (MOD-67 D9): `confirm.yes` and `confirm.no` are handled here,
/// `overlay.close` (the same effect as `no`) by the shell.
const HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "apply"),
    Hint::Pair(Act::ConfirmNo, Act::OverlayClose, "stay offline"),
];

/// Asks before `StoreRequest::ApplyMigrations`.
#[derive(Debug, Default)]
pub struct MigrationPrompt {
    /// From the [`StoreReply::StoreState`] reply; `0` while it has not landed.
    pending: usize,
    /// Whether a reply has landed: "reading the store" and "0 pending" are different screens.
    loaded: bool,
}

impl MigrationPrompt {
    /// Identity of the prompt; `register_all` registers the factory under it and names it as
    /// `App::migration_overlay`.
    pub const ID: OverlayId = OverlayId("migration_prompt");

    /// A prompt with no count yet; the count arrives with the first reply.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the box asks. Never blank, and never `1 migrations` (MOD-1 plan D11).
    fn question(&self) -> String {
        match (self.loaded, self.pending) {
            (false, _) => "reading the store".to_owned(),
            (true, 0) => "the schema is up to date; nothing to apply.".to_owned(),
            (true, 1) => "1 schema migration is pending. Apply it now?".to_owned(),
            (true, n) => format!("{n} schema migrations are pending. Apply them now?"),
        }
    }
}

impl Overlay for MigrationPrompt {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Schema"
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::MIGRATION)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // How the count reaches the overlay: the factory signature is `Fn() -> Box<dyn Overlay>`
        // and must stay that way, so the shell cannot hand it a constructor argument.
        vec![StoreRequest::StoreState]
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // `y`/`Y` and `n`/`Esc`/`N` are the derived `[migration]` rows (MOD-67 PA-1): `Esc` is
        // `confirm.no` first, then `overlay.close` (`SHADOWING`); both close, nothing applied.
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::MIGRATION, chord) {
            match act {
                Act::ConfirmYes => {
                    ctx.emit(Action::Store(StoreRequest::ApplyMigrations));
                    ctx.emit(Action::Overlay(OverlayAction::Close));
                    return Handled::Consumed;
                }
                Act::ConfirmNo => {
                    // The backend stays as it is: schema pending, no refresher, reads from the
                    // mirror.
                    ctx.emit(Action::Overlay(OverlayAction::Close));
                    return Handled::Consumed;
                }
                _ => continue, // `overlay.close` or `global.help`: the shell's
            }
        }
        // Everything else is swallowed by `is_modal`: `ctrl-y` applies nothing.
        Handled::Pass
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::StoreState {
            migrations_pending, ..
        } = reply
        {
            self.pending = migrations_pending.unwrap_or(0);
            self.loaded = true;
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let hint = ctx.keys().hint(views::MIGRATION, HINT);
        // The box fits the question; on a screen narrower than it, the question wraps.
        Ask::new(self.title())
            .line(Line::styled(self.question(), ctx.theme.base))
            .hint(&hint)
            .render(frame, area, ctx.theme);
    }
}

#[cfg(test)]
mod tests {
    use super::MigrationPrompt;
    use crate::app::{Ctx, Emit, TopBarState};
    use crate::keys::{Keys, load_str};
    use crate::store_worker::{Origin, StoreReply};
    use crate::ui::Theme;
    use crate::ui::overlay::registry::Overlay;
    use htui_core::model::{Scope, WorkspaceId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The prompt, told `pending`, drawn over a blank `width`×`height` frame under `keys`, one
    /// line per row.
    fn drawn(pending: Option<usize>, keys: &Keys, (width, height): (u16, u16)) -> Vec<String> {
        let (scope, top_bar, theme, emit) = (
            Scope {
                workspace_id: WorkspaceId::default(),
                project_ids: Vec::new(),
            },
            TopBarState::default(),
            Theme::default(),
            Emit::default(),
        );
        let ctx = || {
            Ctx::new(
                &scope,
                &[],
                &top_bar,
                keys,
                &theme,
                Origin::Overlay(MigrationPrompt::ID),
                &emit,
            )
        };
        let mut prompt = MigrationPrompt::new();
        if let Some(pending) = pending {
            prompt.on_reply(
                &StoreReply::StoreState {
                    label: "online".to_owned(),
                    migrations_pending: Some(pending),
                    below_target: None,
                },
                &mut ctx(),
            );
        }
        let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        term.draw(|frame| prompt.render(frame, frame.area(), &ctx()))
            .expect("the prompt draws");
        let buffer = term.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    /// MOD-115 M4 E6 (C7): the prompt is a kit question: a rounded box titled `Schema`, the
    /// question right under the title, and the keys on the bottom border, not in the box.
    #[test]
    fn the_prompt_is_a_question_with_its_keys_on_the_bottom_border() {
        let lines = drawn(Some(3), Keys::compiled(), (80, 24));
        let top = lines
            .iter()
            .position(|line| line.contains("\u{256d} Schema "))
            .unwrap_or_else(|| panic!("a rounded box titled Schema: {lines:#?}"));
        assert!(
            lines[top + 1].contains("3 schema migrations are pending. Apply them now?"),
            "{lines:#?}"
        );
        let bottom = &lines[top + 2];
        assert!(
            bottom.contains('\u{2570}'),
            "the box is one row tall: {lines:#?}"
        );
        assert!(
            bottom.contains(" y apply \u{b7} n/Esc stay offline "),
            "the keys ride the bottom border: {lines:#?}"
        );
    }

    /// The border's keys come from the keys in force.
    #[test]
    fn a_rebound_confirm_key_is_named_on_the_border() {
        let keys = load_str("version = 1\n[migration]\nyes = \"a\"\n").expect("the keys load");
        let lines = drawn(Some(1), &keys, (80, 24));
        assert!(
            lines.iter().any(|line| line.contains("a apply")),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("1 schema migration is pending. Apply it now?")),
            "{lines:#?}"
        );
    }

    /// At 60 columns a long question wraps inside the box rather than losing its end.
    #[test]
    fn a_narrow_screen_wraps_the_question() {
        let lines = drawn(Some(12), Keys::compiled(), (40, 12));
        let text = lines.join("\n");
        assert!(text.contains("Apply them"), "{text}");
        assert!(text.contains("now?"), "{text}");
        assert!(text.contains("12 schema"), "{text}");
    }

    #[test]
    fn the_question_is_never_blank_and_never_reads_one_migrations() {
        let mut prompt = MigrationPrompt::new();
        assert_eq!(prompt.question(), "reading the store");

        prompt.loaded = true;
        assert_eq!(
            prompt.question(),
            "the schema is up to date; nothing to apply."
        );

        prompt.pending = 1;
        assert_eq!(
            prompt.question(),
            "1 schema migration is pending. Apply it now?"
        );

        prompt.pending = 3;
        assert_eq!(
            prompt.question(),
            "3 schema migrations are pending. Apply them now?"
        );
    }
}

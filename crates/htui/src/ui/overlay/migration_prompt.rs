//! The `R-STO-5` confirmation: pending migrations are never applied without a human (plan D11).
//!
//! The shell opens this overlay when a [`StoreReply::StoreState`] carries a pending count; `y`
//! applies, `n` and `Esc` leave the schema alone and the shell reading from the local mirror. It
//! holds no store handle and no channel: the count arrives through `on_reply`, the decision leaves
//! through `Ctx` (`R-NF-3`).
//!
//! Shaped exactly like [`WorkspaceSwitcher`](super::WorkspaceSwitcher): a centred, modal box with
//! its own hint line, never blank.

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crossterm::event::{KeyCode, KeyEvent};

/// The line under the question. `Esc` is the wildcard overlay binding; `y` and `n` are handled
/// here.
const HINT: &str = "y apply · n / Esc stay offline";

/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;

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

    /// The box's contents: the question, a blank line, then the hint.
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        vec![
            Line::styled(self.question(), theme.base),
            Line::raw(""),
            Line::styled(HINT, theme.dim),
        ]
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

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // How the count reaches the overlay: the factory signature is `Fn() -> Box<dyn Overlay>`
        // and must stay that way, so the shell cannot hand it a constructor argument.
        vec![StoreRequest::StoreState]
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('y' | 'Y') => {
                ctx.emit(Action::Store(StoreRequest::ApplyMigrations));
                ctx.emit(Action::Overlay(OverlayAction::Close));
                Handled::Consumed
            }
            KeyCode::Char('n' | 'N') => {
                // The backend stays as it is: schema pending, no refresher, reads from the mirror.
                ctx.emit(Action::Overlay(OverlayAction::Close));
                Handled::Consumed
            }
            // `Esc` falls through to the wildcard overlay binding, which closes it — the same
            // effect as `n`. Everything else is swallowed by `is_modal`.
            _ => Handled::Pass,
        }
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
        let lines = self.lines(ctx.theme);
        let widest = lines.iter().map(Line::width).max().unwrap_or(0);
        let width = u16::try_from(widest)
            .unwrap_or(u16::MAX)
            .saturating_add(CHROME);
        let height = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2);
        let box_area = centered(area, width, height);

        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(Span::styled(format!(" {} ", self.title()), ctx.theme.title)),
            ),
            box_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::MigrationPrompt;

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

//! The project type section of the Settings tab (MOD-146 D8): the `project_type` registry,
//! read-only.
//!
//! A section like `Personas`: the registry is global, so the read is unscoped and a scope change
//! moves nothing. It holds no store handle (`R-NF-3`): it names [`StoreRequest::ProjectTypes`]
//! and keeps the last [`StoreReply::ProjectTypes`]. `project_type` is not mirrored, so offline the
//! read is refused, the section says "unavailable" and offers no key. There is no write and no
//! mode: user-defined types and their editor are ANA-33's.

use htui_core::model::{ProjectType, Scope};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::settings::{SectionId, SettingsSection, wrapped};
use crossterm::event::KeyEvent;

/// The keys over a readable registry: `j/k select` with the default keys.
pub const HINT_BROWSE: HintSpec = &[Hint::Pair(Act::ListDown, Act::ListUp, "select")];

/// The keys while the registry is unavailable: none (Personas' rule).
pub const HINT_UNAVAILABLE: HintSpec = &[];

/// The request name of the read, as [`StoreRequest::name`] spells it.
pub const READ_NAME: &str = "project_types";

/// Before the first read answered.
pub const NOT_READ: &str = "project types not read yet";

/// The opening of a refused read, drawn `{UNAVAILABLE}: {message}`.
pub const UNAVAILABLE: &str = "project types unavailable";

/// A readable, empty registry.
pub const NO_TYPES: &str = "no project types";

/// The mark of a type whose `coding` is set.
pub const CODING: &str = "coding";

/// The mark of a type a new project cannot take (`workspace`).
pub const NOT_SELECTABLE: &str = "not selectable";

/// The separator of a row's segments.
const DOT: &str = " \u{b7} ";

/// `Settings > Project types` (MOD-146 D8).
#[derive(Debug, Default)]
pub struct ProjectTypesSection {
    /// The last `ProjectTypes` reply, in slug order.
    types: Vec<ProjectType>,
    /// Whether any read has answered yet: before it, the pane says [`NOT_READ`].
    read: bool,
    /// `Some(message)` after `Failed { request: READ_NAME }`.
    unavailable: Option<String>,
    /// Index into `types`; no wrap.
    cursor: usize,
}

impl ProjectTypesSection {
    /// Stable identity.
    pub const ID: SectionId = SectionId("project_types");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts the cursor back inside the list after a read replaced the rows.
    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.types.len().saturating_sub(1));
    }

    /// The hint spec: none while the registry is unavailable.
    fn hint_spec(&self) -> HintSpec {
        if self.unavailable.is_some() {
            HINT_UNAVAILABLE
        } else {
            HINT_BROWSE
        }
    }

    /// One line per type, the cursor row in `theme.selected` with its description wrapped under
    /// it in `theme.dim`, scrolled so both stay on screen.
    fn lines(&self, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
        if let Some(why) = &self.unavailable {
            return wrapped(&format!("{UNAVAILABLE}: {why}"), usize::from(width).max(1))
                .into_iter()
                .map(|line| Line::styled(line, theme.error))
                .collect();
        }
        if !self.read {
            return vec![Line::styled(NOT_READ, theme.dim)];
        }
        if self.types.is_empty() {
            return vec![Line::styled(NO_TYPES, theme.dim)];
        }
        let width = usize::from(width);
        let mut lines = Vec::new();
        let mut cursor_end = 0;
        for (index, row) in self.types.iter().enumerate() {
            if index == self.cursor {
                lines.push(Line::styled(row_line(row), theme.selected));
                for line in wrapped(&row.description, width.saturating_sub(4).max(1)) {
                    lines.push(Line::styled(format!("    {line}"), theme.dim));
                }
                cursor_end = lines.len();
            } else {
                lines.push(Line::styled(row_line(row), theme.base));
            }
        }
        let skip = cursor_end.saturating_sub(usize::from(height));
        lines.into_iter().skip(skip).collect()
    }
}

/// `Name · slug`, then `· coding` and `· not selectable` where they hold.
fn row_line(row: &ProjectType) -> String {
    let mut line = format!("{}{DOT}{}", row.name, row.slug);
    if row.coding {
        line.push_str(DOT);
        line.push_str(CODING);
    }
    if !row.selectable {
        line.push_str(DOT);
        line.push_str(NOT_SELECTABLE);
    }
    line
}

impl SettingsSection for ProjectTypesSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Project types"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `project_type` is global, so the read does not change with the workspace.
        vec![StoreRequest::ProjectTypes]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    /// `list.down`/`list.up` through `PROJECT_TYPES_BROWSE`; every other chord passes. The tab
    /// took `settings.*` first.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::PROJECT_TYPES_BROWSE, chord) {
            match act {
                Act::ListDown => {
                    if self.cursor + 1 < self.types.len() {
                        self.cursor += 1;
                    }
                }
                Act::ListUp => self.cursor = self.cursor.saturating_sub(1),
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::PROJECT_TYPES_BROWSE)
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::ProjectTypes(rows) => {
                self.types.clone_from(rows);
                self.read = true;
                self.unavailable = None;
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.types.clear();
                self.read = true;
                self.unavailable = Some(message.clone());
            }
            _ => return,
        }
        self.clamp();
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        super::key_line(
            keys.hint(views::PROJECT_TYPES_BROWSE, self.hint_spec()),
            theme,
        )
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        frame.render_widget(
            Paragraph::new(self.lines(area.width, area.height, ctx.theme)),
            area,
        );
    }
}

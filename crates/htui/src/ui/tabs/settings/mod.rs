//! The Settings tab: a strip of sections, one per area of configuration.
//!
//! A registry for the same reason the tabs and the detail sub-tabs are one (MOD-1 plan D5):
//! MOD-2's agent registry, MOD-7's box profile, MOD-9's skills and MOD-15's hierarchy each want a
//! section of this tab, and each should be a file plus one `register` line rather than a `match`
//! arm growing in here. [`SettingsSection`] mirrors
//! [`DetailTab`](crate::ui::tabs::backlog::detail::DetailTab) one level across, with
//! [`on_scope_change`](SettingsSection::on_scope_change) where the detail pane has
//! `on_item_change`: a section is scoped to a workspace, not to an item.

pub mod agents;
pub mod hierarchy;

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::registry::{Tab, TabId};
use crossterm::event::{KeyCode, KeyEvent};

pub use agents::AgentsSection;
pub use hierarchy::HierarchySection;

/// Stable identity of a Settings section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectionId(pub &'static str);

impl core::fmt::Display for SectionId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// One area of configuration.
///
/// A section holds no store handle (`R-NF-3`): it names the reads it wants in
/// [`wants_requests`](SettingsSection::wants_requests) and is handed every reply the tab receives.
pub trait SettingsSection {
    /// Stable identity.
    fn id(&self) -> SectionId;
    /// Label in the section strip.
    fn title(&self) -> &str;
    /// Reads this section needs. Issued on activation and after every scope change, never during
    /// render.
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>;
    /// The scope changed: drop cached rows whose ids belong to the previous workspace.
    fn on_scope_change(&mut self, scope: &Scope);
    /// Whether this section is consuming every printable key right now, so the tab must not take
    /// `h`/`l`/`[`/`]`/`Left`/`Right` for section cycling (ANA-10 §4.9; the chat tab's rule,
    /// `chat/mod.rs:403-412`, one tab across). Derived from a mode, never a flag.
    fn captures_input(&self) -> bool {
        false
    }
    /// A key the tab did not use for section navigation.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// A reply addressed to the Settings tab. Sections that do not care ignore it.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Draws below the section strip.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}

/// The registered sections, in strip order, plus which one is active.
#[derive(Default)]
pub struct SettingsRegistry {
    sections: Vec<Box<dyn SettingsSection>>,
    active: usize,
}

impl core::fmt::Debug for SettingsRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SettingsRegistry")
            .field(
                "sections",
                &self
                    .sections
                    .iter()
                    .map(|section| section.id())
                    .collect::<Vec<_>>(),
            )
            .field("active", &self.active)
            .finish()
    }
}

impl SettingsRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a section. Registration order is strip order and cycling order.
    pub fn register(&mut self, section: Box<dyn SettingsSection>) {
        self.sections.push(section);
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// How many sections are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// The active section, or `None` while the registry is empty.
    #[must_use]
    pub fn active(&self) -> Option<&dyn SettingsSection> {
        self.sections.get(self.active).map(AsRef::as_ref)
    }

    /// Id of the active section, or `None` while the registry is empty.
    #[must_use]
    pub fn active_id(&self) -> Option<SectionId> {
        self.active().map(SettingsSection::id)
    }

    /// Activates the next section, wrapping (`l`, `]`, `Right`).
    pub fn cycle_next(&mut self) {
        if !self.sections.is_empty() {
            self.active = (self.active + 1) % self.sections.len();
        }
    }

    /// Activates the previous section, wrapping (`h`, `[`, `Left`).
    pub fn cycle_prev(&mut self) {
        if !self.sections.is_empty() {
            self.active = (self.active + self.sections.len() - 1) % self.sections.len();
        }
    }

    /// Every section's id and title, in strip order.
    #[must_use]
    pub fn titles(&self) -> Vec<(SectionId, &str)> {
        self.sections
            .iter()
            .map(|section| (section.id(), section.title()))
            .collect()
    }
}

/// The Settings tab: `R-TUI-8`'s configuration surface, one section at a time.
#[derive(Debug, Default)]
pub struct SettingsTab {
    /// The registered sections.
    sections: SettingsRegistry,
}

impl SettingsTab {
    /// Identity of the Settings tab.
    pub const ID: TabId = TabId("settings");

    /// A tab with no sections registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A tab over the given sections, in strip order.
    #[must_use]
    pub fn with_sections(sections: Vec<Box<dyn SettingsSection>>) -> Self {
        let mut registry = SettingsRegistry::new();
        for section in sections {
            registry.register(section);
        }
        Self { sections: registry }
    }

    /// Hands one key to the active section, or passes it on while nothing is registered.
    fn delegate(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.sections.sections.get_mut(self.sections.active) {
            Some(section) => section.on_key(key, ctx),
            None => Handled::Pass,
        }
    }
}

impl Tab for SettingsTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Settings"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        // Every section's reads, not only the active one's: the tab is small, the reads are few,
        // and a section that is populated before it is first looked at never renders an empty
        // frame on the way in — the same trade the detail pane makes.
        self.sections
            .sections
            .iter()
            .flat_map(|section| section.wants_requests(scope))
            .collect()
    }

    fn on_scope_change(&mut self, scope: &Scope) {
        for section in &mut self.sections.sections {
            section.on_scope_change(scope);
        }
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A section that is taking typed text answers first: `l` is a letter there, not a cycle.
        if self
            .sections
            .active()
            .is_some_and(SettingsSection::captures_input)
        {
            return self.delegate(key, ctx);
        }
        match key.code {
            KeyCode::Char('l') | KeyCode::Char(']') | KeyCode::Right => {
                self.sections.cycle_next();
                return Handled::Consumed;
            }
            KeyCode::Char('h') | KeyCode::Char('[') | KeyCode::Left => {
                self.sections.cycle_prev();
                return Handled::Consumed;
            }
            _ => {}
        }
        self.delegate(key, ctx)
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        for section in &mut self.sections.sections {
            section.on_reply(reply, ctx);
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let block = Block::new().borders(Borders::ALL).title(" Settings ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [strip, content] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
        render_strip(frame, strip, &self.sections, ctx.theme);

        match self.sections.active() {
            Some(section) => section.render(frame, content, ctx),
            None => message(
                frame,
                content,
                "Workspace, project, repo and kind management arrives with MOD-15.",
                ctx.theme,
            ),
        }
    }
}

/// Draws the section strip, the active one accented.
pub fn render_strip(frame: &mut Frame<'_>, area: Rect, registry: &SettingsRegistry, theme: &Theme) {
    let active = registry.active_id();
    let spans: Vec<Span<'_>> = registry
        .titles()
        .into_iter()
        .map(|(id, title)| {
            let style: Style = if Some(id) == active {
                theme.accent
            } else {
                theme.dim
            };
            Span::styled(format!(" {title} "), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The one line a section renders instead of a blank pane.
pub fn message(frame: &mut Frame<'_>, area: Rect, text: &str, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Line::styled(text.to_owned(), theme.dim)).wrap(Wrap { trim: true }),
        area,
    );
}

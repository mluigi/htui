//! The Settings tab: one pane, its sections (one per area of configuration) as chips on its top
//! border (MOD-115 M3 D4).
//!
//! A registry for the same reason the tabs and the detail sub-tabs are one (MOD-1 plan D5):
//! MOD-2's agent registry, MOD-7's `Boxes` section (every box of the user, this box marked),
//! MOD-9's skills, MOD-15's hierarchy, MOD-26's persona registry (`Personas`, milestone 2),
//! MOD-10's secret provider (`Secrets`, milestone 4), MOD-12's queue caps (`Queue`, milestone 2)
//! and MOD-146's project types (`Project types`) each want a section of this tab, and each should
//! be a file plus one `register` line rather than a `match` arm growing in here.
//! [`SettingsSection`] mirrors
//! [`DetailTab`](crate::ui::tabs::backlog::detail::DetailTab) one level across, with
//! [`on_scope_change`](SettingsSection::on_scope_change) where the detail pane has
//! `on_item_change`: a section is scoped to a workspace, not to an item.

pub mod agents;
pub mod boxes;
pub mod connection;
pub mod hierarchy;
pub mod kinds;
pub mod personas;
pub mod project_types;
pub mod prompt;
/// The Qdrant DSN settings section.
pub mod qdrant;
/// The queue settings section (MOD-12 M2).
pub mod queue;
pub mod secrets;

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::pane::{Pane, ScrollSpan};
use crate::ui::shell::{PaneId, TabLoc};
use crate::ui::tabs::registry::{Tab, TabId};
use crossterm::event::KeyEvent;

pub use agents::AgentsSection;
pub use boxes::BoxesSection;
pub use connection::ConnectionSection;
pub use hierarchy::HierarchySection;
pub use kinds::KindsSection;
pub use personas::PersonasSection;
pub use project_types::ProjectTypesSection;
pub use prompt::PromptSection;
pub use qdrant::QdrantSection;
pub use queue::QueueSection;
pub use secrets::SecretsSection;

/// An optional column: an empty field clears it rather than storing `""`.
///
/// Here rather than in one section because both of them have a field whose empty text means "no
/// value" (MOD-15 M4 D14): one parser for one convention, so the two cannot drift on what `""`
/// stores.
pub(crate) fn some_text(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

/// The `y`/`n` fields of the section editors; anything else is refused rather than guessed at
/// (M4 D14 — the repo editor's `primary (y/n)` and the phase editor's `gate_hard (y/n)` are one
/// convention).
pub(crate) fn yes_or_no(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// Whether a notice is one the user has to act on rather than read, and so belongs in
/// `theme.error`.
///
/// The kinds section, the hierarchy section, the prompt section and the boxes section coin their
/// compare-and-set sentences from the same two openings — the row moved under an open editor, or it
/// is gone — and all of them draw them the same way (M4 D14).
pub(crate) fn is_error(notice: &str) -> bool {
    notice.starts_with("changed elsewhere") || notice.starts_with("deleted elsewhere")
}

/// A question's answers as its sentence ends them (MOD-67 M6, PA-6): the first chords of
/// `confirm.yes` and `confirm.no` through `stack`, `y / n` under the defaults; an unbound side
/// drops, both unbound is empty.
pub(crate) fn answers(keys: &Keys, stack: Stack<'_>) -> String {
    [Act::ConfirmYes, Act::ConfirmNo]
        .into_iter()
        .filter_map(|act| keys.label(stack, act))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// `question` ended by `answers` ([`answers`]'s text) after one space, or alone when there are
/// none (MOD-67 M6, PA-6).
pub(crate) fn asked(question: &str, answers: &str) -> String {
    if answers.is_empty() {
        question.to_owned()
    } else {
        format!("{question} {answers}")
    }
}

/// What the same miss says with **no editor open** to retry from.
///
/// `Enter retries against the current row` would name a row that is not on screen: the editor went
/// with a scope change (M4 B-12), or a read that answered something else was taken for this write's
/// reply and closed it (M4 H-9). The write did not apply either way, and this is the only line that
/// can say so and say what to do about it. (M4 D14, promoted for M5.)
pub(crate) const CHANGED_ELSEWHERE_CLOSED: &str =
    "changed elsewhere; nothing was written \u{2014} reopen the editor and retry";

/// What a compare-and-set miss says when the row the editor opened on is gone from the reload.
///
/// The hierarchy section keeps its own, differently worded, sentence: M3's snapshots pin it.
/// (M4 D14, promoted for M5.)
pub(crate) const DELETED_ELSEWHERE: &str = "deleted elsewhere \u{2014} the editor was closed";

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
    /// `settings.next_section`/`prev_section`'s chords for section cycling (ANA-10 §4.9; the
    /// chat tab's rule, `chat/mod.rs:403-412`, one tab across). Derived from a mode, never a
    /// flag.
    fn captures_input(&self) -> bool {
        false
    }
    /// A key the tab did not use for section navigation.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// This section's key stack in its current mode (MOD-67 D4), or `None` while the section is
    /// not converted: the shell then keeps today's status line, `?` box and `Stack::BASE`
    /// dispatch, and the tab cycles through `views::SETTINGS_TAB`. Defaulted, so no section
    /// changes until its lane converts it.
    fn key_stack(&self) -> Option<Stack<'static>> {
        None
    }
    /// Whether a bracketed paste is offered to this section now. Defaulted to
    /// [`captures_input`](SettingsSection::captures_input); a section that opens its own field for
    /// a paste — the login pane's redirect (MOD-22 review R2-L3) — says so here without claiming
    /// `h`/`l`.
    fn takes_paste(&self) -> bool {
        self.captures_input()
    }
    /// A bracketed paste, offered only while [`takes_paste`](SettingsSection::takes_paste) is true
    /// (MOD-22 review M-1): the focused field takes it whole. Defaulted to `Pass`, which the shell
    /// drops.
    fn on_paste(&mut self, _text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    /// A reply addressed to the Settings tab. Sections that do not care ignore it.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Whether this section's pane says a refused `request` itself, so the shell keeps it off the
    /// status line ([`Tab::absorbs_failure`], MOD-146 R1 L4). Defaulted to `false`.
    fn absorbs_failure(&self, _request: &str) -> bool {
        false
    }
    /// Draws inside the Settings pane's inner rect (MOD-115 M3 D4): the pane's frame, its
    /// section chips, hints and scrollbar are the tab's.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
    /// This section's keys (its mode's spec through its stack, plus what a write in flight adds),
    /// for the Settings pane's bottom border (MOD-67 M6 T2, MOD-115 M3 D4). Defaulted to `None`.
    fn key_hint(&self, _keys: &Keys, _theme: &Theme) -> Option<Line<'static>> {
        None
    }
    /// This section's scroll as of its last render, for the Settings pane's scrollbar (MOD-115
    /// M3 D-11), or `None` when it does not scroll: the rows in all, the first one shown and,
    /// when the section scrolls only part of the pane, the rect those rows were drawn into
    /// ([`ScrollSpan::rows`], RV1 M2), so the bar is measured against them and sits beside
    /// them; without it, against the whole inner rect. A section draws before the pane's frame
    /// (C3), so a section that scrolls records it in a `Cell` in its
    /// [`render`](SettingsSection::render). Defaulted to `None`.
    fn scroll(&self) -> Option<ScrollSpan> {
        None
    }
}

/// What a compare-and-set miss says while an editor is open (M4 D8, PRD D8): the text is kept, the
/// token is not, and the retry is the user's. `retry` is the label of the key that saves again
/// through the editor's stack (`field.submit`, or `form.save` for a `TextArea` editor), `None`
/// when unbound (MOD-115 M3 MD-A, D-5). Under the compiled keys it reads as M2's
/// `CHANGED_ELSEWHERE` did, which M3's T4 deleted once no section named `Enter` literally.
#[must_use]
pub(crate) fn changed_elsewhere(retry: Option<&str>) -> String {
    match retry {
        Some(key) => format!(
            "changed elsewhere since you opened it \u{2014} reloaded; {key} retries against the current row"
        ),
        None => "changed elsewhere since you opened it \u{2014} reloaded; save again to retry against the current row"
            .to_owned(),
    }
}

/// What the pane says with no section registered (only a test registers none; `register_all`
/// registers ten).
const NO_SECTIONS: &str = "No settings sections in this build.";
/// The next step of [`NO_SECTIONS`].
const NO_SECTIONS_NEXT: &str = "Install a build of htui that registers them.";

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

    /// Activates the next section, wrapping (`settings.next_section`).
    pub fn cycle_next(&mut self) {
        if !self.sections.is_empty() {
            self.active = (self.active + 1) % self.sections.len();
        }
    }

    /// Activates the previous section, wrapping (`settings.prev_section`).
    pub fn cycle_prev(&mut self) {
        if !self.sections.is_empty() {
            self.active = (self.active + self.sections.len() - 1) % self.sections.len();
        }
    }

    /// Activates the section with this id (`TabAction::FocusSection`, MOD-15 M6 D7).
    ///
    /// `false` when it is not registered, and nothing moves: a build that dropped a section must
    /// not land the user on a different one than the action named, and must not die either.
    ///
    /// The first *addressable* jump this registry has — `cycle_next`/`cycle_prev` are the only
    /// other movement — and [`TabRegistry::focus`](crate::ui::tabs::registry::TabRegistry::focus)
    /// one level up is the shape it copies.
    pub fn focus(&mut self, id: SectionId) -> bool {
        match self.sections.iter().position(|section| section.id() == id) {
            Some(idx) => {
                self.active = idx;
                true
            }
            None => false,
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

    /// The active section's stack (MOD-67 D4): `None` while it is unconverted.
    fn key_stack(&self) -> Option<Stack<'static>> {
        self.sections.active().and_then(SettingsSection::key_stack)
    }

    /// Section cycling through `settings.next_section`/`prev_section` (MOD-67 M3, L-B Q10):
    /// resolved through the active section's stack, else `views::SETTINGS_TAB`. Chord equality
    /// includes modifiers, so `ctrl-l` or `ctrl-h` no longer cycle (ANA-26 §2.6 defect 1), and
    /// plain `right` no longer does either (MOD-115 M2 D4). Every other key goes to the active
    /// section.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A section that is taking typed text answers first: `l` is a letter there, not a cycle.
        if self
            .sections
            .active()
            .is_some_and(SettingsSection::captures_input)
        {
            return self.delegate(key, ctx);
        }
        let stack = self
            .sections
            .active()
            .and_then(SettingsSection::key_stack)
            .unwrap_or(views::SETTINGS_TAB);
        match ctx.keys().actions(stack, KeyChord::from_event(key)).first() {
            Some(Act::NextSection) => {
                self.sections.cycle_next();
                Handled::Consumed
            }
            Some(Act::PrevSection) => {
                self.sections.cycle_prev();
                Handled::Consumed
            }
            _ => self.delegate(key, ctx),
        }
    }

    /// Only to a section that is taking text (MOD-22 review M-1); anywhere else a paste is not
    /// a key and is not a cycle, it is nothing.
    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        match self.sections.sections.get_mut(self.sections.active) {
            Some(section) if section.takes_paste() => section.on_paste(text, ctx),
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        for section in &mut self.sections.sections {
            section.on_reply(reply, ctx);
        }
    }

    /// Any section's: every reply goes to every section, so the one that issued the read is
    /// among them.
    fn absorbs_failure(&self, request: &str) -> bool {
        self.sections
            .sections
            .iter()
            .any(|section| section.absorbs_failure(request))
    }

    /// The one tab that has sections to focus (MOD-15 M6 D7).
    ///
    /// No re-activation is needed on the way in: [`wants_requests`](Tab::wants_requests) above
    /// collects **every** section's reads, not the active one's, so a section switched to has
    /// already been handed whatever the tab was handed.
    fn focus_section(&mut self, section: SectionId) -> bool {
        self.sections.focus(section)
    }

    /// MOD-115 M2 §8: the active section, by id and title; Settings has no cursor of its own.
    fn location(&self) -> TabLoc {
        TabLoc {
            subtab: self.sections.active_id().map(|id| id.0),
            subtab_title: self
                .sections
                .active()
                .map(|section| section.title().to_owned()),
            cursor: None,
        }
    }

    /// MOD-115 M2 §8: back to a [`location`](Tab::location)'s section, as
    /// [`focus_section`](Tab::focus_section) moves to one. Nothing while the active section is
    /// taking typed text: a move would leave its field behind.
    fn restore(&mut self, loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool {
        if self
            .sections
            .active()
            .is_some_and(SettingsSection::captures_input)
        {
            return false;
        }
        loc.subtab
            .is_some_and(|id| self.sections.focus(SectionId(id)))
    }

    /// The active section's keys (MOD-67 M6 T2). Kept for the `Tab` contract: since MOD-115 M3
    /// D4 the pane draws them itself, and the shell no longer paints them for Settings.
    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        self.sections
            .active()
            .and_then(|section| section.key_hint(ctx.keys(), ctx.theme))
    }

    /// MOD-115 M3 D4: Settings draws its own kit pane, so the shell paints no focus or hints
    /// over it.
    fn draws_chrome(&self) -> bool {
        true
    }

    /// MOD-115 M3 D4: one kit pane, titled by the active section, with every section as a chip
    /// on its top border (the active one kept and the rest marked when they overflow, D3), the
    /// section's hints on its bottom border and the section's scrollbar on its right one. A
    /// single pane is `PaneId::Detail` (C1). The section draws into the pane's inner rect first,
    /// so its scroll is known when the frame is drawn (C3, D-11).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let section = self.sections.active();
        let inner = Pane::inner(area);
        match section {
            Some(section) => section.render(frame, inner, ctx),
            None => empty::render(
                frame,
                inner,
                NO_SECTIONS,
                NextStep(NO_SECTIONS_NEXT),
                ctx.theme,
            ),
        }

        let titles: Vec<&str> = self
            .sections
            .sections
            .iter()
            .map(|section| section.title())
            .collect();
        let mut pane = Pane::new(section.map_or("Settings", SettingsSection::title))
            .sub_tabs(&titles, self.sections.active)
            .focus(ctx.pane_focus(PaneId::Detail))
            .accent(ctx.accent());
        if let Some(hints) = section.and_then(|section| section.key_hint(ctx.keys(), ctx.theme)) {
            pane = pane.hints(hints);
        }
        if let Some(span) = section.and_then(SettingsSection::scroll) {
            pane = pane.scroll(span);
        }
        pane.render(frame, area, ctx.theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::store_worker::Origin;
    use crate::ui::shell::PaneId;
    use htui_core::model::WorkspaceId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Modifier, Style};

    /// The ten sections `register_all` registers, by title, in strip order.
    const TEN: [&str; 10] = [
        "Agents",
        "Hierarchy",
        "Kinds",
        "Prompt",
        "Connection",
        "Qdrant",
        "Boxes",
        "Personas",
        "Secrets",
        "Queue",
    ];

    /// A section that draws `{title} body` on its first row and `last` on its last, hands the
    /// pane `hint` and reports `scroll`.
    struct Fake {
        title: &'static str,
        hint: Option<&'static str>,
        scroll: Option<ScrollSpan>,
    }

    impl Fake {
        fn boxed(title: &'static str) -> Box<dyn SettingsSection> {
            Box::new(Self {
                title,
                hint: Some("j/k move"),
                scroll: None,
            })
        }
    }

    impl SettingsSection for Fake {
        fn id(&self) -> SectionId {
            SectionId(self.title)
        }
        fn title(&self) -> &str {
            self.title
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, frame: &mut Frame<'_>, area: Rect, _ctx: &Ctx<'_>) {
            let buf = frame.buffer_mut();
            buf.set_string(area.x, area.y, format!("{} body", self.title), Style::new());
            buf.set_string(area.x, area.bottom() - 1, "last", Style::new());
        }
        fn key_hint(&self, _keys: &Keys, _theme: &Theme) -> Option<Line<'static>> {
            self.hint.map(Line::raw)
        }
        fn scroll(&self) -> Option<ScrollSpan> {
            self.scroll
        }
    }

    fn tab_of(titles: &[&'static str]) -> SettingsTab {
        SettingsTab::with_sections(titles.iter().map(|title| Fake::boxed(title)).collect())
    }

    /// `tab` drawn into a `width` by `height` bench, the shell's focus on `focus` when given
    /// (a bench without one focuses the list, M2 B7).
    fn draw(
        tab: &SettingsTab,
        width: u16,
        height: u16,
        focus: Option<PaneId>,
        theme: &Theme,
    ) -> Buffer {
        let scope = Scope {
            workspace_id: WorkspaceId::default(),
            project_ids: Vec::new(),
        };
        let (top_bar, emit) = (TopBarState::default(), Emit::default());
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            Keys::compiled(),
            theme,
            Origin::Tab(SettingsTab::ID),
            &emit,
        );
        let ctx = match focus {
            Some(pane) => ctx.with_focus(pane),
            None => ctx,
        };
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), &ctx))
            .expect("the tab draws");
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    fn rows(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height).map(|y| row(buffer, y)).collect()
    }

    /// MOD-115 M3 D4: Settings draws its own frame, so the shell neither restyles it nor paints
    /// its hint adapter over it.
    #[test]
    fn settings_draws_its_own_chrome() {
        assert!(SettingsTab::new().draws_chrome());
        assert!(tab_of(&TEN).draws_chrome());
    }

    /// MOD-115 M3 D4: one rounded pane titled by the active section; the `┌ Settings ─` box is
    /// gone.
    #[test]
    fn the_frame_is_one_rounded_pane_titled_by_the_section() {
        let buffer = draw(
            &tab_of(&["Agents", "Kinds"]),
            60,
            8,
            Some(PaneId::Detail),
            &Theme::default(),
        );
        let lines = rows(&buffer);
        assert!(lines[0].starts_with("╭─ Agents "), "{lines:#?}");
        assert!(lines[0].ends_with('╮'), "{lines:#?}");
        assert!(
            lines[7].starts_with('╰') && lines[7].ends_with('╯'),
            "{lines:#?}"
        );
        let all = lines.concat();
        assert!(
            !all.contains('┌') && !all.contains("Settings"),
            "{lines:#?}"
        );
    }

    /// MOD-115 M3 D4: the sections are chips on the top border, flush with its right corner,
    /// and the strip row under it is gone: the section draws into the whole inner rect.
    #[test]
    fn the_ten_sections_are_chips_on_the_top_border_and_the_section_fills_the_pane() {
        let buffer = draw(
            &tab_of(&TEN),
            100,
            10,
            Some(PaneId::Detail),
            &Theme::default(),
        );
        let lines = rows(&buffer);
        let strip: String = TEN.iter().map(|title| format!(" {title} ")).collect();
        assert!(lines[0].ends_with(&format!("{strip}╮")), "{lines:#?}");
        assert!(lines[1].starts_with("│Agents body"), "{lines:#?}");
        assert!(lines[8].starts_with("│last"), "{lines:#?}");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("Hierarchy"))
                .count(),
            1,
            "no strip inside the pane: {lines:#?}"
        );
    }

    /// MOD-115 M3 D3/D4: at 80 columns the ten chips overflow; the active one stays, the ones cut
    /// off are marked, and the pane's title gives way (D-10).
    #[test]
    fn at_80_columns_the_active_chip_is_shown_and_the_cut_ones_marked() {
        let mut tab = tab_of(&TEN);
        for (index, title) in TEN.iter().enumerate() {
            let buffer = draw(&tab, 80, 6, Some(PaneId::Detail), &Theme::default());
            let top = row(&buffer, 0);
            assert!(top.contains(&format!(" {title} ")), "{index}: {top}");
            assert_eq!(
                top.contains('‹'),
                !top.contains(" Agents "),
                "{index}: {top}"
            );
            assert_eq!(
                top.contains('›'),
                !top.contains(" Queue "),
                "{index}: {top}"
            );
            assert!(top.contains('‹') || top.contains('›'), "{index}: {top}");
            assert!(
                row(&buffer, 1).starts_with(&format!("│{title} body")),
                "{index}"
            );
            tab.sections.cycle_next();
        }
    }

    /// MOD-115 M3 D4: the active section's hint is on the bottom border, while the pane is
    /// focused only (C1: a single pane is `PaneId::Detail`).
    #[test]
    fn the_section_hint_is_on_the_bottom_border_when_focused_only() {
        let tab = tab_of(&["Agents", "Kinds"]);
        let focused = draw(&tab, 60, 6, Some(PaneId::Detail), &Theme::default());
        assert!(
            row(&focused, 5).starts_with("╰─j/k move─"),
            "{:#?}",
            rows(&focused)
        );
        let unfocused = draw(&tab, 60, 6, None, &Theme::default());
        assert!(!rows(&unfocused).concat().contains("j/k move"));
        assert!(
            row(&unfocused, 5).starts_with("╰────"),
            "{:#?}",
            rows(&unfocused)
        );
    }

    /// MOD-115 M3 D-11: the pane's scrollbar is the section's, as of its last render.
    #[test]
    fn the_scrollbar_follows_the_section_scroll() {
        let thumb = |tab: &SettingsTab| {
            let buffer = draw(tab, 40, 10, Some(PaneId::Detail), &Theme::default());
            (1..9)
                .filter(|&y| buffer[(39, y)].symbol() == "┃")
                .collect::<Vec<u16>>()
        };
        let still = SettingsTab::with_sections(vec![Box::new(Fake {
            title: "Kinds",
            hint: None,
            scroll: None,
        })]);
        assert!(thumb(&still).is_empty());
        let top = SettingsTab::with_sections(vec![Box::new(Fake {
            title: "Kinds",
            hint: None,
            scroll: Some(ScrollSpan::new(32, 0)),
        })]);
        let bottom = SettingsTab::with_sections(vec![Box::new(Fake {
            title: "Kinds",
            hint: None,
            scroll: Some(ScrollSpan::new(32, 24)),
        })]);
        let (top, bottom) = (thumb(&top), thumb(&bottom));
        assert_eq!(top.first(), Some(&1), "{top:?}");
        assert_eq!(bottom.last(), Some(&8), "{bottom:?}");
        assert!(top.len() < 8 && bottom.len() < 8, "{top:?} {bottom:?}");
    }

    /// RV1 M2: a section that scrolls part of the pane (a list over a notice line) says which
    /// rows, and the bar is measured against them and sits beside them only: it reaches their
    /// last row at the end and never the rows under them.
    #[test]
    fn the_scrollbar_sits_beside_the_rows_the_section_scrolls() {
        // A 40 by 10 pane: inner rows 1..=8; the section scrolls rows 1..=6 over two of its own.
        let rows = Rect::new(1, 1, 38, 6);
        let thumb = |offset: usize| {
            let tab = SettingsTab::with_sections(vec![Box::new(Fake {
                title: "Kinds",
                hint: None,
                scroll: Some(ScrollSpan::new(12, offset).rows(rows)),
            })]);
            let buffer = draw(&tab, 40, 10, Some(PaneId::Detail), &Theme::default());
            (1..9)
                .filter(|&y| buffer[(39, y)].symbol() == "┃")
                .collect::<Vec<u16>>()
        };
        assert_eq!(thumb(0).first(), Some(&1));
        assert_eq!(
            thumb(6).last(),
            Some(&6),
            "the end is the last scrolled row"
        );
        for offset in 0..12 {
            assert!(
                thumb(offset).iter().all(|y| (1..=6).contains(y)),
                "{offset}"
            );
        }
    }

    /// MOD-80 D5: under the monochrome theme only modifiers can mark the active section, now a
    /// chip on the pane's top border.
    #[test]
    fn active_section_is_bold_and_underlined_without_colour() {
        let tab = tab_of(&["Connection", "Qdrant"]);
        let buffer = draw(&tab, 40, 4, Some(PaneId::Detail), &Theme::monochrome());
        let top = row(&buffer, 0);
        let marked = Modifier::BOLD | Modifier::UNDERLINED;
        let at = |text: &str| {
            let byte = top.find(text).expect("the chip is drawn");
            u16::try_from(top[..byte].chars().count()).expect("narrow")
        };
        let active = &buffer[(at(" Connection  Qdrant ") + 1, 0)];
        assert_eq!(active.symbol(), "C");
        assert!(active.modifier.contains(marked), "{active:?}");
        let other = &buffer[(at(" Qdrant ") + 1, 0)];
        assert_eq!(other.symbol(), "Q");
        // The others are neither, not even the focused border's bold (blueprint §4.1).
        assert!(!other.modifier.intersects(marked), "{other:?}");
        assert!(other.modifier.contains(Modifier::DIM), "{other:?}");
        assert!(!active.modifier.contains(Modifier::DIM), "{active:?}");
    }

    /// MOD-115 M3 D4: with no section registered the pane says so and what to do, through
    /// `kit::empty`.
    #[test]
    fn no_section_is_an_empty_state_with_a_next_step() {
        let buffer = draw(
            &SettingsTab::new(),
            70,
            8,
            Some(PaneId::Detail),
            &Theme::default(),
        );
        let all = rows(&buffer).concat();
        assert!(row(&buffer, 0).starts_with("╭─ Settings "), "{all}");
        assert!(all.contains(NO_SECTIONS), "{all}");
        assert!(all.contains(NO_SECTIONS_NEXT), "{all}");
    }

    /// MOD-115 M3 MD-A, C4: the compare-and-set sentence names the key that retries through
    /// `Keys`. Under the compiled keys it is the old sentence byte for byte; rebound, it names
    /// the new chord; unbound, it names no key.
    #[test]
    fn changed_elsewhere_names_the_retry_key_in_force() {
        let compiled = Keys::compiled().label(views::CAPTURE, Act::FieldSubmit);
        assert_eq!(
            changed_elsewhere(compiled.as_deref()),
            "changed elsewhere since you opened it \u{2014} reloaded; Enter retries against the current row"
        );
        let rebound =
            crate::keys::load_str("version = 2\n[field]\nsubmit = [\"ctrl-t\"]\n").expect("loads");
        let label = rebound.label(views::CAPTURE, Act::FieldSubmit);
        let label = label.expect("ctrl-t is bound");
        assert!(label.to_ascii_lowercase().contains("ctrl"), "{label}");
        let text = changed_elsewhere(Some(&label));
        assert!(text.contains(&format!("{label} retries")), "{text}");
        assert!(!text.contains("Enter"), "{text}");
        let unbound = changed_elsewhere(None);
        assert!(!unbound.contains("Enter"), "{unbound}");
        assert!(unbound.contains("save again"), "{unbound}");
        for text in [text, unbound] {
            assert!(is_error(&text), "still an error notice: {text}");
        }
    }

    /// MOD-115 M2 §8: the location names the active section; restoring it activates that
    /// section, and a location naming no registered section restores nothing.
    #[test]
    fn the_location_is_the_section_and_restore_returns_to_it() {
        use crate::app::{Emit, TopBarState};
        use crate::store_worker::Origin;
        use htui_core::model::WorkspaceId;

        let mut tab = SettingsTab::with_sections(vec![
            Box::new(ConnectionSection::new()),
            Box::new(QdrantSection::new()),
        ]);
        let first = tab.location();
        assert_eq!(first.subtab, tab.sections.active_id().map(|id| id.0));
        assert_eq!(first.subtab_title.as_deref(), Some("Connection"));
        assert_eq!(first.cursor, None);
        tab.sections.cycle_next();
        let second = tab.location();
        assert_ne!(second, first);

        let scope = Scope {
            workspace_id: WorkspaceId::default(),
            project_ids: Vec::new(),
        };
        let (top_bar, theme, emit) = (TopBarState::default(), Theme::default(), Emit::default());
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            Keys::compiled(),
            &theme,
            Origin::Tab(SettingsTab::ID),
            &emit,
        );
        assert!(tab.restore(&first, &mut ctx));
        assert_eq!(tab.location(), first);
        assert!(tab.restore(&second, &mut ctx));
        assert_eq!(tab.location(), second);
        let gone = TabLoc {
            subtab: Some("no-such-section"),
            ..TabLoc::default()
        };
        assert!(!tab.restore(&gone, &mut ctx));
        assert!(!tab.restore(&TabLoc::default(), &mut ctx));
        assert_eq!(tab.location(), second, "nothing moved");
    }

    /// MOD-67 M6 PA-6: a question ends in the answers in force, and in none when none is bound.
    #[test]
    fn answers_are_the_confirm_keys_in_force() {
        let keys = Keys::defaults();
        assert_eq!(answers(&keys, views::CONNECTION_CONFIRM), "y / n");
        assert_eq!(asked("clear it?", "y / n"), "clear it? y / n");
        // MOD-115 M2 MD7: `[]` unbinds only `no`'s other chords; `Esc` is never configurable.
        let no_unbound = crate::keys::load_str("version = 1\n[confirm]\nno = []\n").expect("loads");
        assert_eq!(answers(&no_unbound, views::CONNECTION_CONFIRM), "y / Esc");
        let both_unbound =
            crate::keys::load_str("version = 1\n[confirm]\nyes = []\nno = []\n").expect("loads");
        assert_eq!(answers(&both_unbound, views::SECRETS_CONFIRM), "Esc");
        let all_unbound = Keys::defaults()
            .with_chords(crate::keys::Context::Confirm, Act::ConfirmYes, &[])
            .with_chords(crate::keys::Context::Confirm, Act::ConfirmNo, &[]);
        assert_eq!(answers(&all_unbound, views::SECRETS_CONFIRM), "");
        assert_eq!(asked("clear it?", ""), "clear it?");
    }
}

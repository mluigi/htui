//! The about panel (`F9`): what the current tab is for and its main actions (MOD-115).
//!
//! Two lines in a faint rounded pane: the tab's purpose in a sentence, then its main actions as
//! key chips. Every chord comes from [`Keys`] through the tab's key stack, so a rebound action
//! shows its new key and an unbound one is left out rather than shown as a bare word. The text
//! lives in one static table keyed by [`TabId`]; M1 leaves the tabs themselves unchanged.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::keys::{Act, Keys, Stack};
use crate::ui::kit::chips;
use crate::ui::kit::pane::{Focus, Pane};
use crate::ui::tabs::registry::TabId;
use crate::ui::{Theme, cells};

/// One tab's about text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct About {
    /// What the tab is for, in a sentence.
    pub purpose: &'static str,
    /// Its main actions, each with the word shown beside its key; an act with no label in the
    /// current keys is skipped.
    pub actions: &'static [(Act, &'static str)],
}

/// The Backlog tab.
const BACKLOG: About = About {
    purpose: "Your work items: open one for its body, runs and notes, and queue it for an agent.",
    actions: &[
        (Act::BacklogNewItem, "new item"),
        (Act::BacklogOpenFilter, "filter"),
        (Act::BacklogQueue, "queue"),
        (Act::Edit, "edit"),
    ],
};

/// The Chat tab.
const CHAT: About = About {
    purpose: "Talk to a coding agent, or read back a chat that has ended.",
    actions: &[
        (Act::ChatCompose, "write"),
        (Act::ChatNextAgent, "next agent"),
        (Act::ChatThoughts, "thoughts"),
        (Act::ChatEnd, "end chat"),
    ],
};

/// The Skills tab.
const SKILLS: About = About {
    purpose: "The skills and prompt templates your agents draw on, and their versions.",
    actions: &[
        (Act::SkillsSwitchView, "library or templates"),
        (Act::New, "new"),
        (Act::Edit, "edit"),
        (Act::LibraryImport, "import"),
    ],
};

/// The Settings tab.
const SETTINGS: About = About {
    purpose: "Agents, kinds, the store connection, boxes and the other settings, one section each.",
    actions: &[
        (Act::NextSection, "next section"),
        (Act::PrevSection, "previous section"),
        (Act::Edit, "edit"),
    ],
};

/// The Requirements tab.
const REQUIREMENTS: About = About {
    purpose: "The requirements tree, and which items cite each requirement.",
    actions: &[
        (Act::RequirementsNewArea, "new area"),
        (Act::RequirementsAmend, "amend"),
        (Act::RequirementsFilter, "filter"),
    ],
};

/// The about text of `tab`, if it has one: the five main tabs do, anything else (a test double,
/// the reserved workspaces and logs tabs before they exist) does not.
#[must_use]
pub fn about_for(tab: TabId) -> Option<&'static About> {
    match tab.0 {
        "backlog" => Some(&BACKLOG),
        "chat" => Some(&CHAT),
        "skills" => Some(&SKILLS),
        "settings" => Some(&SETTINGS),
        "requirements" => Some(&REQUIREMENTS),
        _ => None,
    }
}

/// Draws `about` into `area` (four rows fill the pane: two border rows, two content rows), labels
/// resolved through `keys` for `stack`. An action with no label in `stack` is skipped.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    about: &About,
    keys: &Keys,
    stack: Stack<'_>,
    theme: &Theme,
) {
    let inner = Pane::new("About")
        .focus(Focus::Unfocused)
        .render(frame, area, theme);
    // One cell of air between the border and the text.
    let inner = Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    if inner.is_empty() {
        return;
    }
    let pairs: Vec<(String, &str)> = about
        .actions
        .iter()
        .map(|(act, word)| (keys.label(stack, *act).unwrap_or_default(), *word))
        .collect();
    let purpose = Line::styled(
        cells::clip(about.purpose, usize::from(inner.width)),
        theme.base,
    );
    let actions = cells::clip_line(chips::chip_line(&pairs, theme), inner.width);
    frame.render_widget(Paragraph::new(vec![purpose, actions]), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{Context, Layer};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    const TABS: [&str; 5] = ["backlog", "chat", "skills", "settings", "requirements"];

    fn draw(width: u16, height: u16, about: &About, keys: &Keys, stack: Stack<'_>) -> Buffer {
        draw_themed(width, height, about, keys, stack, &Theme::default())
    }

    fn draw_themed(
        width: u16,
        height: u16,
        about: &About,
        keys: &Keys,
        stack: Stack<'_>,
        theme: &Theme,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), about, keys, stack, theme))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    const HELP_ONLY: About = About {
        purpose: "A purpose.",
        actions: &[(Act::Help, "keys")],
    };

    #[test]
    fn every_main_tab_has_a_one_sentence_purpose_and_actions() {
        for id in TABS {
            let about = about_for(TabId(id)).unwrap_or_else(|| panic!("{id} has about text"));
            assert!(about.purpose.ends_with('.'), "{id}: {}", about.purpose);
            assert!(!about.actions.is_empty(), "{id}");
            assert!(
                cells::cell_width(about.purpose) <= 90,
                "{id}: a purpose that fits one row of an ordinary terminal"
            );
        }
        assert_eq!(about_for(TabId("no-such-tab")), None);
        assert_eq!(about_for(TabId("workspaces")), None, "reserved, not built");
    }

    #[test]
    fn the_panel_shows_the_purpose_then_the_action_chips() {
        let buffer = draw(60, 4, &HELP_ONLY, Keys::compiled(), Stack::BASE);
        assert!(
            row(&buffer, 0).starts_with("\u{256d}\u{2500} About "),
            "{}",
            row(&buffer, 0)
        );
        assert_eq!(
            row(&buffer, 1).trim_matches(['\u{2502}', ' ']),
            "A purpose."
        );
        assert_eq!(row(&buffer, 2).trim_matches(['\u{2502}', ' ']), "?  keys");
        assert!(
            row(&buffer, 3).starts_with('\u{2570}'),
            "{}",
            row(&buffer, 3)
        );
    }

    /// A chord is whatever the keys in force say: rebound, the chip follows.
    #[test]
    fn a_chip_label_is_resolved_through_keys() {
        use crate::keys::Context as C;
        let f9 = Keys::defaults().with_chords(C::Global, Act::Help, &["f9"]);
        let buffer = draw(60, 4, &HELP_ONLY, &f9, Stack::BASE);
        assert!(row(&buffer, 2).contains("F9  keys"), "{}", row(&buffer, 2));
    }

    /// An act that the stack does not reach, or that is unbound, shows no bare word.
    #[test]
    fn an_action_with_no_label_is_skipped() {
        const MIXED: About = About {
            purpose: "A purpose.",
            actions: &[(Act::ChatCompose, "write"), (Act::Help, "keys")],
        };
        let keys = Keys::compiled();
        assert_eq!(keys.label(Stack::BASE, Act::ChatCompose), None);
        let buffer = draw(60, 4, &MIXED, keys, Stack::BASE);
        let actions = row(&buffer, 2);
        assert!(actions.contains("keys"), "{actions}");
        assert!(!actions.contains("write"), "{actions}");
        // Through the chat's own layer the same act has a label and is shown.
        let layers = [Layer::all(Context::Chat)];
        let buffer = draw(60, 4, &MIXED, keys, Stack::new(&layers));
        assert!(row(&buffer, 2).contains("write"), "{}", row(&buffer, 2));
    }

    #[test]
    fn a_narrow_panel_clips_with_an_ellipsis_and_keeps_its_border() {
        let about = about_for(TabId("backlog")).expect("backlog has about text");
        let buffer = draw(24, 4, about, Keys::compiled(), Stack::BASE);
        let purpose = row(&buffer, 1);
        assert!(purpose.contains('\u{2026}'), "{purpose}");
        assert!(
            purpose.starts_with('\u{2502}') && purpose.ends_with('\u{2502}'),
            "{purpose}"
        );
        assert_eq!(cells::cell_width(&purpose), 24);
    }

    #[test]
    fn a_panel_too_small_for_text_draws_only_its_border() {
        let buffer = draw(30, 2, &HELP_ONLY, Keys::compiled(), Stack::BASE);
        assert!(!row(&buffer, 0).contains("purpose") && !row(&buffer, 1).contains("purpose"));
        let tiny = draw(2, 4, &HELP_ONLY, Keys::compiled(), Stack::BASE);
        assert!(!row(&tiny, 1).contains("purpose"));
    }

    #[test]
    fn the_panel_names_no_colour_under_monochrome() {
        let theme = Theme::monochrome();
        let buffer = draw_themed(60, 4, &HELP_ONLY, Keys::compiled(), Stack::BASE, &theme);
        for y in 0..4 {
            for x in 0..60 {
                assert_eq!(buffer[(x, y)].fg, Color::Reset, "({x},{y})");
                assert_eq!(buffer[(x, y)].bg, Color::Reset, "({x},{y})");
            }
        }
        assert!(
            buffer[(0, 0)].modifier.contains(Modifier::DIM),
            "a faint border"
        );
    }

    #[test]
    fn the_border_is_the_faint_role_not_a_tab_accent() {
        let buffer = draw(40, 4, &HELP_ONLY, Keys::compiled(), Stack::BASE);
        assert_eq!(buffer[(0, 0)].fg, Color::Indexed(240));
    }
}

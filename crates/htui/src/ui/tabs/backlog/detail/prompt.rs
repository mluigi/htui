//! The Prompt sub-tab: the prompt this item would be given, and the audit that explains it
//! (MOD-2 plan D102, blueprint B.16).
//!
//! The sixth sub-tab, and the only one that renders something no row holds: the reply carries an
//! assembled prompt, not a `SELECT`. It still holds no store handle (`R-NF-3`, `runs.rs:5`) — the
//! Backlog tab issues the request with the other five reads and the deferred task answers it.
//!
//! **Nothing here parses the text.** The header lines are built from `trim_record`'s own fields and
//! the body is the canonical text verbatim, so a `</section>` inside a document body is inert
//! (blueprint H-28).

use htui_core::model::ItemId;
use htui_core::prompt::{Section, TrimRecord};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::preview::PromptPreview;
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crossterm::event::{KeyCode, KeyEvent};

/// How wide the label gutter is: every header line is `label` then the value.
const LABEL: usize = 10;

/// The separator between the audit header and the prompt text.
const RULE: &str = "────────────────────────────────────────";

/// The prompt one item would be given, with its digest, its budget, its section table, its excerpt
/// audit and its declared stand-ins above the canonical text.
#[derive(Debug, Default)]
pub struct PromptTab {
    /// The selected item, so a reply for another one is dropped.
    item: Option<ItemId>,
    /// The last reply for [`item`](Self::item).
    preview: Option<PromptPreview>,
    /// Why there is no preview, when the **store** failed rather than the assembler.
    ///
    /// Held separately from [`preview`](Self::preview) because the two are different facts: an
    /// `Err` outcome is a prompt that refused to assemble and still has a template, a budget and a
    /// record; this is a read that never got that far. Without it the pane would sit on
    /// "Assembling the prompt…" for the life of the selection (plan D11: never a blank pane, and
    /// never a lie either).
    failed: Option<String>,
    /// First visible line.
    scroll: Scroll,
    /// The rendered rows, rebuilt on every reply. Held rather than recomputed per frame: the
    /// prompt text is up to the whole token budget, and a render runs on the UI task.
    lines: Vec<Row>,
}

/// One rendered row of the pane: its text, and whether it is drawn in `Theme::dim` (MOD-9 D46).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Row {
    text: String,
    dim: bool,
}

impl Row {
    /// A row in the normal style.
    fn plain(text: String) -> Self {
        Self { text, dim: false }
    }

    /// This row as the rows it renders to: one per `\n`-separated piece, a trailing `\r` dropped,
    /// each keeping the style. For every row `render_lines` builds this equals the old
    /// `rows.join("\n").lines()` (review finding M5): only the last row could differ, and it is a
    /// line of `assembled.text.lines()` or an `AssembleError` sentence, neither of which ends in
    /// `\n`.
    fn split(self) -> impl Iterator<Item = Self> {
        let dim = self.dim;
        self.text
            .split('\n')
            .map(|piece| piece.strip_suffix('\r').unwrap_or(piece).to_owned())
            .map(move |text| Self { text, dim })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl PromptTab {
    /// Identity of the Prompt sub-tab.
    pub const ID: DetailId = DetailId("prompt");

    /// A sub-tab with no preview yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The template name one step around [`available`](PromptPreview::available) from the one
    /// showing, or `None` when there is nothing to cycle to.
    ///
    /// Wrapping, and a list of one cycles to itself — which is deliberately *not* nothing: the
    /// request still goes out, so `n` on a single-template project re-previews rather than
    /// silently doing nothing.
    fn cycle(&self, forward: bool) -> Option<String> {
        let preview = self.preview.as_ref()?;
        let names = &preview.available;
        if names.is_empty() {
            return None;
        }
        let current = preview
            .template
            .as_ref()
            .and_then(|row| names.iter().position(|name| *name == row.name))
            .unwrap_or(0);
        let step = if forward { 1 } else { names.len() - 1 };
        names.get((current + step) % names.len()).cloned()
    }

    /// Rebuilds [`lines`](Self::lines) from the reply.
    ///
    /// The `join` and the re-split are what [`render`](DetailTab::render) used to do **every
    /// frame**, on the UI task, over up to a whole token budget of text — review finding M5. They
    /// are the same two operations, run once per reply, so every row is byte-for-byte the one
    /// `Text::raw(lines.join("\n"))` produced, including the rows a note with an embedded newline
    /// would have contributed. What it buys is that one element of [`lines`](Self::lines) is now
    /// exactly one rendered row, which is what the field's doc already claimed and what makes
    /// [`Scroll`]'s clamp against `lines.len()` true rather than a lower bound.
    fn rebuild(&mut self) {
        let rows = match self.preview.as_ref() {
            Some(preview) => render_lines(preview),
            None => Vec::new(),
        };
        self.lines = rows.into_iter().flat_map(Row::split).collect();
    }
}

/// Every line the pane shows, top to bottom (blueprint B.16).
fn render_lines(preview: &PromptPreview) -> Vec<Row> {
    let mut lines = Vec::new();
    let template = match preview.template.as_ref() {
        Some(row) => format!("{} v{}", row.name, row.version),
        None => "none".to_owned(),
    };
    lines.push(Row::plain(labelled("template", &template)));
    lines.push(Row::plain(labelled(
        "picker",
        &format!("n / p \u{b7} {} template(s)", preview.available.len()),
    )));

    // Bound once, and used again at the bottom for the text. The second `match` that used to read
    // `preview.outcome` there needed an `unreachable!` arm to say what this binding says by
    // construction — and an `unreachable!` in a render path is a panic in the event loop if the
    // reasoning behind it ever stops holding (finding L5).
    let assembled = match &preview.outcome {
        Ok(assembled) => {
            lines.push(Row::plain(labelled("digest", &assembled.digest)));
            assembled
        }
        Err(message) => {
            // A refusal is a preview: it is what the run would have said, in the words it would
            // have used, and it is the one thing this pane can tell a maintainer who has just
            // mistyped a `token_budget`.
            lines.push(Row::plain(labelled("refused", message)));
            return lines;
        }
    };
    let record = &assembled.trim;

    lines.push(Row::plain(labelled(
        "budget",
        &format!(
            "{} ({}) \u{b7} reserve {:.2} \u{b7} target {}",
            record.budget,
            record.budget_source.as_str(),
            record.reserve,
            record.target
        ),
    )));
    lines.push(Row::plain(labelled(
        "tokens",
        &format!(
            "{} \u{2192} {} \u{b7} {}",
            record.estimated_before, record.estimated_after, record.estimator
        ),
    )));
    lines.extend(excerpt_lines(record).into_iter().map(Row::plain));
    lines.push(Row::default());
    lines.extend(section_lines(&record.sections).into_iter().map(Row::plain));
    lines.push(Row::default());
    for (index, note) in record.notes.iter().enumerate() {
        let label = if index == 0 { "notes" } else { "" };
        lines.push(Row::plain(labelled(label, note)));
    }
    lines.push(Row::plain(RULE.to_owned()));
    lines.extend(
        assembled
            .text
            .lines()
            .map(|line| Row::plain(line.to_owned())),
    );
    lines
}

/// The `excerpts` and `roots` lines of §4.5's audit.
///
/// Two lines rather than one because `roots` is what ANA-5 §12 criterion 12 is about and a
/// 43-column detail pane would clip it off the end of a combined line.
fn excerpt_lines(record: &TrimRecord) -> Vec<String> {
    let audit = &record.excerpts;
    let providers = if audit.provider_set.is_empty() {
        "no provider".to_owned()
    } else {
        audit.provider_set.join(", ")
    };
    let roots = if audit.roots.is_empty() {
        // Plan D110: no `repo` row is read in this milestone, so there is no repo to say `no_path`
        // about. The declared stand-in note below says the same thing in full.
        "none \u{b7} no_path (no run_step_tree, no repo_box_path)".to_owned()
    } else {
        audit
            .roots
            .iter()
            .map(|root| format!("{} {}", root.repo, root.source.as_str()))
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    };
    vec![
        labelled(
            "excerpts",
            &format!(
                "{providers} \u{b7} considered {} \u{b7} selected {}",
                audit.considered, audit.selected
            ),
        ),
        labelled("roots", &roots),
    ]
}

/// The section table: a header and one row per `trim_record.sections` entry, in render order.
fn section_lines(sections: &[Section]) -> Vec<String> {
    let mut lines = vec![format!(
        "{:<24}{:>8}{:>8}  {}",
        "section", "before", "after", "strategy"
    )];
    for section in sections {
        let mut row = format!(
            "{:<24}{:>8}{:>8}  {}",
            section.name.render(),
            section.tokens_before,
            section.tokens_after,
            section.strategy.as_str(),
        );
        // The marker's two numbers are the record's own (blueprint rule 11), so a reader can check
        // the elision in the text against the row that accounts for it.
        if let (Some(elided_lines), Some(elided_bytes)) =
            (section.elided_lines, section.elided_bytes)
        {
            row.push_str(&format!(
                "  elided {elided_lines} lines / {elided_bytes} bytes"
            ));
        }
        if let Some(stubbed) = section.stubbed {
            row.push_str(&format!("  stubbed {stubbed}"));
        }
        if let Some(dropped) = section.dropped {
            row.push_str(&format!("  dropped {dropped}"));
        }
        lines.push(row);
    }
    lines
}

/// `label` in the gutter, then `value`. An empty label is a continuation line.
fn labelled(label: &str, value: &str) -> String {
    format!("{label:<LABEL$}{value}")
}

impl DetailTab for PromptTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Prompt"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.preview = None;
        self.failed = None;
        self.lines.clear();
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let forward = match key.code {
            KeyCode::Char('n') => true,
            KeyCode::Char('p') => false,
            _ => return self.scroll.on_key(key, self.lines.len()),
        };
        let (Some(item), Some(name)) = (self.item, self.cycle(forward)) else {
            return Handled::Pass;
        };
        ctx.request(StoreRequest::PromptPreview {
            item,
            template_name: Some(name),
            scope: ctx.scope.clone(),
        });
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Failed { request, message } = reply
            && *request == crate::store_worker::PROMPT_PREVIEW
        {
            self.preview = None;
            self.lines.clear();
            self.failed = Some(message.clone());
            self.scroll.reset();
            return;
        }
        let StoreReply::PromptPreview(preview) = reply else {
            return;
        };
        self.failed = None;
        // A preview of another item is a reply to a selection the user has already left. The
        // shell's staleness index drops most of them; this is the one that keeps a burst of `j`
        // presses from showing FEAT-1's prompt under CLEAN-1's header.
        if Some(preview.item) != self.item {
            return;
        }
        self.preview = Some((**preview).clone());
        self.rebuild();
        self.scroll.reset();
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        let Some(preview) = self.preview.as_ref() else {
            let text = match self.failed.as_deref() {
                Some(why) => format!("No preview: {why}"),
                None => "Assembling the prompt\u{2026}".to_owned(),
            };
            message(frame, area, &text, ctx.theme);
            return;
        };
        if preview.available.is_empty() {
            message(
                frame,
                area,
                "No prompt templates in this project.",
                ctx.theme,
            );
            return;
        }
        // No wrap: a wrapped prompt would renumber every line of the audit above it, and the pane
        // is scrolled rather than reflowed.
        //
        // Only the window, and therefore no `scroll`: `Paragraph::scroll` still needs the whole
        // `Text` built before it can throw all but `area.height` rows of it away, and building it
        // meant copying and re-parsing the prompt — up to ~400 KB for a 100 k-token one — on the UI
        // task, once per frame (finding M5). Skipping the rows here instead renders the identical
        // cells, because the pane does not wrap and one element of `lines` is one row.
        let window: Vec<Line<'_>> = self
            .lines
            .iter()
            .skip(self.scroll.skip())
            .take(usize::from(area.height))
            .map(|row| {
                if row.dim {
                    Line::styled(row.text.as_str(), ctx.theme.dim)
                } else {
                    Line::raw(row.text.as_str())
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(Text::from(window)), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::{Activation, ChoiceReason, SkillChoice, SkillLevel};
    use ratatui::buffer::Buffer;
    use ratatui::widgets::Widget as _;

    #[test]
    fn the_window_renders_the_cells_the_scrolled_whole_used_to() {
        // Finding M5 is a **cost** change and must not be a behaviour change, and the snapshot
        // suite only ever renders this pane at offset zero. So the claim is pinned here, directly:
        // for every offset — including the two past the end, where `Paragraph::scroll` quietly
        // renders nothing — the window this pane now builds paints the identical cells that
        // `Text::raw(lines.join("\n")).scroll((offset, 0))` painted.
        let rows: Vec<String> = (0..40)
            .map(|n| format!("row {n} with enough text to reach the right edge"))
            .collect();
        let area = Rect::new(0, 0, 20, 8);
        for offset in [0u16, 1, 7, 33, 39, 40, 100] {
            let mut whole = Buffer::empty(area);
            Paragraph::new(Text::raw(rows.join("\n")))
                .scroll((offset, 0))
                .render(area, &mut whole);

            let window: Vec<Line<'_>> = rows
                .iter()
                .skip(usize::from(offset))
                .take(usize::from(area.height))
                .map(|row| Line::raw(row.as_str()))
                .collect();
            let mut only = Buffer::empty(area);
            Paragraph::new(Text::from(window)).render(area, &mut only);

            assert_eq!(whole, only, "the two disagree at offset {offset}");
        }
    }

    #[test]
    fn one_element_of_lines_is_one_rendered_row() {
        // The other half of M5: `rebuild` now does the split `Text::raw` used to do per frame, so
        // a note carrying a newline still contributes two rows and a blank separator still
        // contributes one. Without the `join`/`lines()` round trip this is where they would part.
        let rows = [
            Row::plain("template  prd v1".to_owned()),
            Row::default(),
            Row::plain("notes     first\nsecond".to_owned()),
            Row::plain("last".to_owned()),
        ];
        let split: Vec<String> = rows
            .into_iter()
            .flat_map(Row::split)
            .map(|row| row.text)
            .collect();
        assert_eq!(
            split,
            ["template  prd v1", "", "notes     first", "second", "last",],
            "the blank line survives and the embedded newline becomes its own row"
        );
    }

    #[test]
    fn an_inactive_choice_is_a_dim_row() {
        // MOD-9 D46 / D65: one row per skill choice between the section table and the notes, the
        // `skills` label on the first; an active choice in the normal style, an inactive one dim.
        let mut assembled = htui_core::prompt::assemble(
            &htui_core::prompt::fixtures::phase_implement_attempt2(),
            &htui_core::scrub::MinimalScrubber::new([]),
        )
        .expect("the fixture assembles");
        assembled.trim.skill_choices.push(SkillChoice {
            skill: htui_core::fixtures::ids::SKILL_TESTS,
            name: "legacy\nstyle".to_owned(),
            version: None,
            level: SkillLevel::Global,
            activation: Activation::Off,
            active: false,
            reason: ChoiceReason::Off,
        });
        let preview = PromptPreview {
            item: htui_core::fixtures::ids::HTUI_FEAT_1,
            available: vec!["implement".to_owned()],
            template: Some(htui_core::prompt::TemplateRef {
                name: "implement".to_owned(),
                version: 1,
            }),
            outcome: Ok(assembled),
        };

        let rows = render_lines(&preview);
        let first = labelled(
            "skills",
            "rust-style v2 \u{b7} project \u{b7} always \u{2192} active",
        );
        let second = labelled("", "legacy style v? \u{b7} global \u{b7} off \u{2192} off");
        let at = rows
            .iter()
            .position(|row| row.text == first)
            .unwrap_or_else(|| panic!("the active choice is a row: {rows:#?}"));
        assert!(
            !rows[at].dim,
            "an active choice is drawn in the normal style"
        );
        assert_eq!(
            rows.get(at + 1),
            Some(&Row {
                text: second,
                dim: true
            }),
            "the inactive choice follows, dim, its name on one row"
        );
        assert_eq!(
            rows.get(at + 2),
            Some(&Row::default()),
            "a blank row closes the block"
        );
        assert!(
            rows.get(at + 3)
                .is_some_and(|row| row.text.starts_with("notes")),
            "the notes follow the block"
        );
        let table = rows
            .iter()
            .position(|row| row.text.starts_with("section "))
            .expect("the section table is rendered");
        assert!(table < at, "the block sits below the section table");
    }
}

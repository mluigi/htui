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
use ratatui::text::Text;
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
    /// The rendered lines, rebuilt on every reply. Held rather than recomputed per frame: the
    /// prompt text is up to the whole token budget, and a render runs on the UI task.
    lines: Vec<String>,
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
    fn rebuild(&mut self) {
        self.lines = match self.preview.as_ref() {
            Some(preview) => render_lines(preview),
            None => Vec::new(),
        };
    }
}

/// Every line the pane shows, top to bottom (blueprint B.16).
fn render_lines(preview: &PromptPreview) -> Vec<String> {
    let mut lines = Vec::new();
    let template = match preview.template.as_ref() {
        Some(row) => format!("{} v{}", row.name, row.version),
        None => "none".to_owned(),
    };
    lines.push(labelled("template", &template));
    lines.push(labelled(
        "picker",
        &format!("n / p \u{b7} {} template(s)", preview.available.len()),
    ));

    let record = match &preview.outcome {
        Ok(assembled) => {
            lines.push(labelled("digest", &assembled.digest));
            &assembled.trim
        }
        Err(message) => {
            // A refusal is a preview: it is what the run would have said, in the words it would
            // have used, and it is the one thing this pane can tell a maintainer who has just
            // mistyped a `token_budget`.
            lines.push(labelled("refused", message));
            return lines;
        }
    };

    lines.push(labelled(
        "budget",
        &format!(
            "{} ({}) \u{b7} reserve {:.2} \u{b7} target {}",
            record.budget,
            record.budget_source.as_str(),
            record.reserve,
            record.target
        ),
    ));
    lines.push(labelled(
        "tokens",
        &format!(
            "{} \u{2192} {} \u{b7} {}",
            record.estimated_before, record.estimated_after, record.estimator
        ),
    ));
    lines.extend(excerpt_lines(record));
    lines.push(String::new());
    lines.extend(section_lines(&record.sections));
    lines.push(String::new());
    for (index, note) in record.notes.iter().enumerate() {
        let label = if index == 0 { "notes" } else { "" };
        lines.push(labelled(label, note));
    }
    lines.push(RULE.to_owned());

    match &preview.outcome {
        Ok(assembled) => lines.extend(assembled.text.lines().map(str::to_owned)),
        Err(_) => unreachable!("the `Err` arm returned above"),
    }
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
        frame.render_widget(
            Paragraph::new(Text::raw(self.lines.join("\n"))).scroll((self.scroll.offset(), 0)),
            area,
        );
    }
}

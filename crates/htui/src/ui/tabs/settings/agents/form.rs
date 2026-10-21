//! The Agents forms (MOD-115 M3 D7): the registry create and edit form, the tool-paths form,
//! and the notices their writes answer with.

use chrono::{DateTime, Utc};
use htui_agent::launch::AgentLaunch;
use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_core::model::{Agent, AgentId, AgentSummary, Transport};
use ratatui::style::Style;
use ratatui::text::Line;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::agent_settings::{self, AgentDraft, DraftFields, FIELD_LABELS, LITERAL_LAUNCH, Refusal};
use crate::store_worker::StoreRequest;
use crate::ui::cells::cell_width;
use crate::ui::kit::form::{Field as FormRow, Form, Value};
use crate::ui::{TextField, Theme};

use super::table::verdict;

/// What an edit that changed nothing says as it closes (plan D239's "unchanged closes").
pub(super) const UNCHANGED: &str = "nothing changed; nothing was written";

/// `m` on a row whose launch does not parse (MOD-66 B14): "literal" would misdescribe it.
pub(super) const LAUNCH_UNREADABLE: &str =
    "this row's launch does not parse; nothing declares a tool";

/// What a missing row says when no form is open to close: the write was `t`'s, or the form went
/// before its answer came back. Opens like
/// [`DELETED_ELSEWHERE`](crate::ui::tabs::settings::DELETED_ELSEWHERE), so
/// [`is_error`](crate::ui::tabs::settings::is_error) draws it the same.
pub(super) const GONE_CLOSED: &str = "deleted elsewhere; nothing was written";

/// What `Created` adds for a `cli` row with no `settings.cli` block (plan R-9, blueprint F-13): such
/// a row resolves to the bare adapter id `cli`, which no build registers. Decided from the row's
/// data alone, never its name (`R-AGT-5`).
pub(super) const NEEDS_CLI_BLOCK: &str =
    " \u{b7} a `cli` row needs a settings.cli block to chat (adapter id `cli` is not registered)";

/// What a spent token says when fields the user changed were changed elsewhere too (review M-1),
/// before their labels ([`clash_notice`]): shorter than
/// [`changed_elsewhere`](crate::ui::tabs::settings::changed_elsewhere) so labels fit the note's
/// [`NOTE_WIDTH`]. Labels only, never a value. Opens like it, so
/// [`is_error`](crate::ui::tabs::settings::is_error) draws it the same.
///
/// `retry` is the label of the key that saves again through `AGENTS_FORM` (`field.submit`,
/// MOD-115 M3 MD-A), `None` when unbound: `Enter` under the compiled keys, as the sentence was
/// written before.
pub(super) fn changed_on_both_sides(retry: Option<&str>) -> String {
    match retry {
        Some(key) => format!(
            "changed elsewhere \u{2014} reloaded; {key} retries \u{b7} also changed elsewhere: "
        ),
        None => "changed elsewhere \u{2014} reloaded; save again to retry \u{b7} also changed \
                 elsewhere: "
            .to_owned(),
    }
}

/// The widest note line the running app draws (MOD-23 re-review Low-2): the Settings pane's
/// border costs two of the harness's 100 and `kit::list`'s gutter, which the note sits behind,
/// one more (MOD-115 M3); the note is one line with no wrap.
pub(super) const NOTE_WIDTH: usize = 97;

/// The tool-paths form (MOD-66 D10, B7). It holds its own row identity (MOD-23 F-14), never an
/// index into the table. Its `Debug` is written by hand: it prints how many paths were
/// `opened`, never the paths, as [`PathField`]'s buffer never prints its text (review N3).
pub(super) struct PathsForm {
    /// The row.
    pub(super) agent_id: AgentId,
    /// Its name, for the header.
    pub(super) name: String,
    /// The stored `probe.manual`, restricted to the declared tools: the prefill, and what `Enter`
    /// compares against ("unchanged closes").
    pub(super) opened: BTreeMap<String, String>,
    /// One per declared tool, in `discovery.tools` (name) order.
    pub(super) fields: Vec<PathField>,
    /// Index into `fields`.
    pub(super) focus: usize,
}

impl core::fmt::Debug for PathsForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PathsForm")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("opened", &self.opened.len())
            .field("fields", &self.fields)
            .field("focus", &self.focus)
            .finish()
    }
}

/// One input of the tool-paths form, labelled with a tool name: a runtime string (MOD-66 D11).
#[derive(Debug)]
pub(super) struct PathField {
    /// The `${tool}` name this path is for.
    pub(super) tool: String,
    /// The buffer; its `Debug` never prints the text.
    pub(super) input: TextField,
}

/// The open form (plan D231): which write it makes, its fields in tab order, and which one has
/// focus.
///
/// It holds its own row identity (blueprint F-14) and never an index into the table: a create or a
/// failed read can re-sort or clear the rows under an open form.
#[derive(Debug)]
pub(super) struct Editor {
    /// The write `Enter` makes.
    pub(super) target: Target,
    /// The inputs, labelled from [`FIELD_LABELS`].
    pub(super) fields: Vec<Field>,
    /// Index into `fields`.
    pub(super) focus: usize,
}

/// What an open form writes.
#[derive(Debug)]
pub(super) enum Target {
    /// `n`: a new row, whose `settings` start as this row's (OQ-5), named in the header.
    Create {
        /// The highlighted row when `n` was pressed; `None` on an empty table.
        settings_from: Option<(AgentId, String)>,
    },
    /// `e`: one existing row, as a compare-and-set on its `updated_at`.
    Edit {
        /// The row.
        agent_id: AgentId,
        /// Its name, for the header (plan D233: never edited).
        name: String,
        /// The token: the row's `updated_at` as a registry reply answered it (MOD-40 F-17).
        expected: DateTime<Utc>,
        /// The draft the row prefilled, which `Enter` compares against ("unchanged closes").
        opened: AgentDraft,
        /// Whether the draft in flight changes `transport`, `command` or `args` (plan D246).
        relaunch: bool,
    },
}

/// One labelled input of the form: `settings/hierarchy.rs`'s `Field`, private here.
#[derive(Debug)]
pub(super) struct Field {
    /// One of [`FIELD_LABELS`].
    pub(super) label: &'static str,
    /// The buffer; its `Debug` never prints the text.
    pub(super) input: TextField,
}

impl Editor {
    /// The create form (plan D231): all nine fields, with `transport`, `billing` and `enabled`
    /// holding the values a new row most often wants, so a literal `command` is all it needs.
    /// `mods` starts at `n` even when `settings_from` names a row that opted in (MOD-91 D6,
    /// blueprint B7): the copy is the isolated side.
    pub(super) fn create(settings_from: Option<(AgentId, String)>) -> Self {
        let texts = [
            "",
            Transport::Acp.as_str(),
            "",
            "",
            "",
            "",
            htui_core::model::Billing::Subscription.as_str(),
            "y",
            "n",
        ];
        Self {
            target: Target::Create { settings_from },
            fields: FIELD_LABELS
                .iter()
                .zip(texts)
                .map(|(label, text)| Field::new(label, text))
                .collect(),
            focus: 0,
        }
    }

    /// The edit form over one row: every field but `name` (plan D233), prefilled from
    /// [`agent_settings::draft_of`] so the prefill and the "unchanged" check read `launch` the way
    /// the worker does. `env` has no field (D235).
    pub(super) fn edit(summary: &AgentSummary) -> Self {
        let opened = agent_settings::draft_of(&summary.agent);
        let texts = prefill(&opened);
        Self {
            fields: FIELD_LABELS[1..]
                .iter()
                .zip(texts)
                .map(|(label, text)| Field::new(label, &text))
                .collect(),
            target: Target::Edit {
                agent_id: summary.agent.id,
                name: summary.agent.name.clone(),
                expected: summary.agent.updated_at,
                opened,
                relaunch: false,
            },
            focus: 0,
        }
    }

    /// A spent token under an open edit form (review M-1): rebase the form onto `current`, the row
    /// as the re-read answered it, and answer the labels of the fields changed on both sides.
    ///
    /// A field whose text is still what the old baseline prefilled takes `current`'s text, so a
    /// retry carries another writer's change instead of reverting it (`EditAgent` sends the whole
    /// draft). A field the user changed keeps its text; it is a clash when `current` changed it
    /// too, to something else. Then the token and the "unchanged" baseline are `current`'s.
    pub(super) fn rebase(
        &mut self,
        updated_at: DateTime<Utc>,
        current: AgentDraft,
    ) -> Vec<&'static str> {
        let Target::Edit {
            expected,
            opened,
            relaunch,
            ..
        } = &mut self.target
        else {
            return Vec::new();
        };
        let before = prefill(opened);
        let after = prefill(&current);
        let mut clashes = Vec::new();
        // The edit form's fields are `FIELD_LABELS[1..]`, which is `prefill`'s order.
        for ((field, old), new) in self.fields.iter_mut().zip(before).zip(after) {
            if field.text() == old {
                if new != old {
                    field.input = TextField::with_text(&new);
                }
            } else if new != old && field.text() != new {
                clashes.push(field.label);
            }
        }
        *expected = updated_at;
        *opened = current;
        *relaunch = false;
        clashes
    }

    /// The text of the field labelled `label`, or `""` when the form has none (the edit form's
    /// `name`).
    pub(super) fn text(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|field| field.label == label)
            .map_or("", Field::text)
    }

    /// Moves the focus to the field a refusal names; a field this form does not have (`launch`)
    /// leaves it where it is.
    pub(super) fn focus_on(&mut self, label: &str) {
        if let Some(index) = self.fields.iter().position(|field| field.label == label) {
            self.focus = index;
        }
    }

    /// The form's request, `None` for an edit that changed nothing, or the first refusal in tab
    /// order (plan D247). A create also refuses a name the table already lists (D234); the store's
    /// `UNIQUE` stays the authority.
    pub(super) fn request(
        &mut self,
        agents: &[AgentSummary],
    ) -> Result<Option<StoreRequest>, Refusal> {
        let name = self.text(FIELD_LABELS[0]).to_owned();
        let draft = agent_settings::draft_from_fields(&DraftFields {
            transport: self.text(FIELD_LABELS[1]),
            command: self.text(FIELD_LABELS[2]),
            args: self.text(FIELD_LABELS[3]),
            models: self.text(FIELD_LABELS[4]),
            default_model: self.text(FIELD_LABELS[5]),
            billing: self.text(FIELD_LABELS[6]),
            enabled: self.text(FIELD_LABELS[7]),
            user_extensions: self.text(FIELD_LABELS[8]),
        });
        match &mut self.target {
            Target::Create { settings_from } => {
                let name = agent_settings::parse_name(&name)?;
                if agents.iter().any(|summary| summary.agent.name == name) {
                    return Err(Refusal {
                        field: FIELD_LABELS[0],
                        reason: format!("`{name}` is already registered"),
                    });
                }
                Ok(Some(StoreRequest::CreateAgent {
                    name,
                    draft: draft?,
                    settings_from: settings_from.as_ref().map(|(id, _)| *id),
                }))
            }
            Target::Edit {
                agent_id,
                expected,
                opened,
                relaunch,
                ..
            } => {
                let draft = draft?;
                if draft == *opened {
                    return Ok(None);
                }
                *relaunch = opened.transport != draft.transport
                    || opened.command != draft.command
                    || opened.args != draft.args;
                Ok(Some(StoreRequest::EditAgent {
                    agent_id: *agent_id,
                    expected: *expected,
                    draft,
                }))
            }
        }
    }

    /// The pane's first line: what `Enter` writes, and where a new row's `settings` come from.
    pub(super) fn header(&self) -> String {
        match &self.target {
            Target::Create {
                settings_from: Some((_, name)),
            } => format!("new agent \u{b7} settings from {name}"),
            Target::Create {
                settings_from: None,
            } => "new agent \u{b7} settings {}".to_owned(),
            Target::Edit { name, .. } => format!("edit {name}"),
        }
    }

    /// The form (MOD-115 M3 D3): one `kit::form` row per field, the focused one carrying the
    /// marker in `accent` and the cursor, every label in one column: the longest label and one
    /// (`default model`, 14), which leaves 81 cells of text at the bordered 98 (D231).
    pub(super) fn lines(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>> {
        self.fields
            .iter()
            .fold(Form::new(), |form, field| {
                form.field(FormRow::new(field.label, Value::Text(&field.input)))
            })
            .focus(self.focus)
            .editing(true)
            .lines(width, accent, theme)
    }
}

impl PathsForm {
    /// The form over `summary`, or the refusal sentence: [`LAUNCH_UNREADABLE`] for a `launch`
    /// that does not parse, [`LITERAL_LAUNCH`] for no `discovery` or no `tools` (MOD-66 D10,
    /// B14). Fields in `discovery.tools` order, prefilled from the stored `probe.manual`. An
    /// unreadable snapshot prefills nothing (blueprint H-21).
    pub(super) fn open(summary: &AgentSummary) -> Result<Self, &'static str> {
        let launch =
            AgentLaunch::deserialize(&summary.agent.launch).map_err(|_| LAUNCH_UNREADABLE)?;
        let tools: Vec<String> = launch
            .discovery
            .map(|discovery| discovery.tools.into_keys().collect())
            .unwrap_or_default();
        if tools.is_empty() {
            return Err(LITERAL_LAUNCH);
        }
        let opened: BTreeMap<String, String> = summary
            .on_box
            .as_ref()
            .and_then(ProbeSnapshot::from_row)
            .map(|snapshot| snapshot.manual)
            .unwrap_or_default()
            .into_iter()
            .filter(|(tool, _)| tools.contains(tool))
            .collect();
        let fields = tools
            .into_iter()
            .map(|tool| PathField {
                input: TextField::with_text(opened.get(&tool).map_or("", String::as_str)),
                tool,
            })
            .collect();
        Ok(Self {
            agent_id: summary.agent.id,
            name: summary.agent.name.clone(),
            opened,
            fields,
            focus: 0,
        })
    }

    /// The form's request (MOD-66 D9, D10): `Ok(None)` for an unchanged map, else one
    /// `SetToolPaths` with every non-blank field trimmed (an empty map clears). A field
    /// `parse_tool_path` refuses is `Err((its index, the sentence))`, the first in tab order.
    pub(super) fn request(&self) -> Result<Option<StoreRequest>, (usize, String)> {
        let mut paths = BTreeMap::new();
        for (index, field) in self.fields.iter().enumerate() {
            let text = field.input.text().unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let path = agent_settings::parse_tool_path(&field.tool, text)
                .map_err(|sentence| (index, sentence))?;
            paths.insert(field.tool.clone(), path);
        }
        if paths == self.opened {
            return Ok(None);
        }
        Ok(Some(StoreRequest::SetToolPaths {
            agent_id: self.agent_id,
            paths,
        }))
    }

    /// The pane's first line: whose paths `Enter` writes, and what an empty field means.
    pub(super) fn header(&self) -> String {
        format!("tool paths for {} \u{b7} empty = no manual path", self.name)
    }

    /// One `kit::form` row per tool, [`Editor::lines`]' shape (MOD-66 D11, B8; MOD-115 M3 D3).
    /// The label column is the longest tool name in cells and one, capped at a third of `width`
    /// (the form's own cap). Each name is `cells::fit` to the column (MOD-60): padded when it fits,
    /// else cut to at most one cell less than the column and ending in `…`, then padded back, so
    /// every label ends at the same cell.
    pub(super) fn lines(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>> {
        self.fields
            .iter()
            .fold(Form::new(), |form, field| {
                form.field(FormRow::new(&field.tool, Value::Text(&field.input)))
            })
            .focus(self.focus)
            .editing(true)
            .lines(width, accent, theme)
    }
}

impl Field {
    /// A field labelled `label` holding `text`, cursor at the end.
    pub(super) fn new(label: &'static str, text: &str) -> Self {
        Self {
            label,
            input: TextField::with_text(text),
        }
    }

    /// What was typed. Never masked here, so [`TextField::text`] always answers.
    pub(super) fn text(&self) -> &str {
        self.input.text().unwrap_or_default()
    }
}

/// [`changed_on_both_sides`] (naming `retry`) with the clashing labels in form order, as many as
/// fit [`NOTE_WIDTH`], then ` +N more` for the rest (MOD-23 re-review Low-2). Greedy and
/// deterministic: a label is shown only if it and the count still owed after it fit, so the
/// line never passes the width.
pub(super) fn clash_notice(clashes: &[&str], retry: Option<&str>) -> String {
    let mut notice = changed_on_both_sides(retry);
    let mut shown = 0;
    for (index, label) in clashes.iter().enumerate() {
        let separator = if index == 0 { "" } else { ", " };
        let after = clashes.len() - index - 1;
        let owed = if after == 0 {
            String::new()
        } else {
            format!(" +{after} more")
        };
        let width =
            cell_width(&notice) + cell_width(separator) + cell_width(label) + cell_width(&owed);
        if width > NOTE_WIDTH {
            break;
        }
        notice.push_str(separator);
        notice.push_str(label);
        shown += 1;
    }
    let hidden = clashes.len() - shown;
    if hidden > 0 {
        notice.push_str(&format!(" +{hidden} more"));
    }
    notice
}

/// The edit form's text for `draft`, in `FIELD_LABELS[1..]` order: what the form prefills, and
/// what a `Stale` rebase compares against (review M-1).
pub(super) fn prefill(draft: &AgentDraft) -> [String; 8] {
    [
        draft.transport.as_str().to_owned(),
        draft.command.clone(),
        agent_settings::format_args(&draft.args),
        agent_settings::format_models(&draft.models),
        draft.default_model.clone().unwrap_or_default(),
        draft.billing.as_str().to_owned(),
        if draft.enabled { "y" } else { "n" }.to_owned(),
        if draft.user_extensions { "y" } else { "n" }.to_owned(),
    ]
}

/// Whether a row cannot chat for want of a `settings.cli` block (plan R-9, blueprint F-13): a `cli`
/// row without one resolves to the bare adapter id `cli`. Read from the row's data, never its name
/// (`R-AGT-5`), and without `htui_agent`'s private `adapter_id_from`.
pub(super) fn needs_cli_block(agent: &Agent) -> bool {
    agent.transport == Transport::Cli && agent.settings.get("cli").is_none()
}

/// What `AgentWritten::ToolPaths` says (MOD-66 D10): the row and the probe's verdict, as its
/// `kit::label` mark (MOD-83), never a path.
pub(super) fn tool_paths_saved(name: &str, status: ProbeStatus) -> String {
    format!(
        "tool paths saved for `{name}` \u{b7} this box: {}",
        verdict(status)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-60: the tool-name column is measured and cut in cells. A CJK name that fits the
    /// column by `char` count is twice as wide on screen; it is cut with `…` so every label,
    /// wide or not, ends at the same cell. At an odd column (63 / 3 = 21) the cut lands on a
    /// cluster boundary and `…` fills the last cell; at an even one (60 / 3 = 20) a wide glyph
    /// cannot fill the last cell before `…`, so `cells::fit` pads one space after it. Both pin
    /// the exact label, so an over-clip padded back to the column cannot pass. Since MOD-115 M3
    /// the row is `kit::form`'s: the gutter, the label column, two blanks, the value.
    #[test]
    fn a_wide_tool_name_is_fitted_to_the_label_column() {
        let wide = "\u{6f22}";
        let cases = [
            (63, format!("{}\u{2026}", wide.repeat(10))),
            (60, format!("{}\u{2026} ", wide.repeat(9))),
        ];
        for (width, expected) in cases {
            let column = usize::from(width) / 3;
            let form = PathsForm {
                agent_id: AgentId::new(),
                name: "agent".to_owned(),
                opened: BTreeMap::new(),
                fields: [wide.repeat(20), "git".to_owned()]
                    .into_iter()
                    .map(|tool| PathField {
                        tool,
                        input: TextField::new(),
                    })
                    .collect(),
                focus: 0,
            };

            let lines: Vec<String> = form
                .lines(width, Style::new(), &Theme::default())
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect()
                })
                .collect();

            // Both labels fill the column exactly, so both values start at the same cell.
            assert_eq!(cell_width(&expected), column, "width {width}");
            assert!(
                lines[0].starts_with(&format!("\u{258c}{expected}  ")),
                "width {width}: {:?}",
                lines[0]
            );
            assert!(
                lines[1].starts_with(&format!(" {:<column$}  ", "git")),
                "width {width}: {:?}",
                lines[1]
            );
        }
    }
}

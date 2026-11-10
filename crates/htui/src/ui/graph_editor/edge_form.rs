//! MOD-141 D9: the edge form - relation, the `if` predicate as one line of JSON parsed live with
//! `IfPredicate::parse`, and the loop limit where the relation takes one. Key-agnostic: the
//! editor maps its acts onto these methods.

use crossterm::event::KeyEvent;
use htui_core::model::predicate::{IfPredicate, PredicateError};
use htui_core::model::{EdgeId, EdgeKind, EdgePredicate, GraphBody, PhaseId, StepGraphEdge};
use ratatui::style::Style;
use ratatui::text::Line;

use super::{EdgeSpec, RELATIONS, relation_word, step_name};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::{FieldOutcome, TextField, Theme};

/// An `if` link saved with nothing typed in its predicate.
pub(crate) const PREDICATE_REQUIRED: &str = "`if` needs a predicate, e.g. {\"verify\":\"failed\"}";

/// A loop limit that is not a whole number of at least 0.
pub(crate) const LOOP_IS_A_NUMBER: &str = "`loop limit` is a whole number, 0 or more";

/// A predicate that is not JSON at all.
pub(crate) fn predicate_not_json(error: &serde_json::Error) -> String {
    format!("`if` is not JSON: {error}")
}

/// A predicate that is JSON but outside the closed `if` set.
pub(crate) fn predicate_refused(error: &PredicateError) -> String {
    format!("`if`: {error}")
}

/// A relation that needs a loop limit saved without one.
pub(crate) fn loop_limit_required(kind: EdgeKind) -> String {
    format!("`loop limit` is required for {}", relation_word(kind))
}

/// Which link the form writes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeTarget {
    /// A new link `from` → `to`.
    New {
        /// The source step.
        from: PhaseId,
        /// The target step.
        to: PhaseId,
    },
    /// The body's link with this id.
    Edit(EdgeId),
}

/// One row of the edge form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeRow {
    /// The relation, a choice over [`RELATIONS`].
    Relation,
    /// The `if` predicate, one line of JSON.
    Predicate,
    /// The loop limit.
    LoopLimit,
}

/// The open edge form (D9).
pub(crate) struct EdgeForm {
    /// The link it writes back to.
    target: EdgeTarget,
    /// The chosen relation.
    kind: EdgeKind,
    /// The predicate's JSON, as typed.
    predicate: TextField,
    /// The loop limit, as typed.
    loop_limit: TextField,
    /// The focused row; always one of [`rows`](Self::rows).
    focus: EdgeRow,
}

/// The target, the relation and the focus, never what was typed (H-7).
impl core::fmt::Debug for EdgeForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EdgeForm")
            .field("target", &self.target)
            .field("kind", &self.kind)
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl EdgeForm {
    /// A new `next` link `from` → `to`, nothing typed.
    pub(crate) fn new(from: PhaseId, to: PhaseId) -> Self {
        Self {
            target: EdgeTarget::New { from, to },
            kind: EdgeKind::Next,
            predicate: TextField::new(),
            loop_limit: TextField::new(),
            focus: EdgeRow::Relation,
        }
    }

    /// The form on `edge`: its relation, its predicate as compact JSON and its loop limit.
    pub(crate) fn edit(edge: &StepGraphEdge) -> Self {
        let predicate = edge
            .predicate
            .as_ref()
            .map_or_else(String::new, |predicate| predicate.0.to_string());
        let loop_limit = edge
            .loop_limit
            .map_or_else(String::new, |limit| limit.to_string());
        Self {
            target: EdgeTarget::Edit(edge.id),
            kind: edge.kind,
            predicate: TextField::with_text(&predicate),
            loop_limit: TextField::with_text(&loop_limit),
            focus: EdgeRow::Relation,
        }
    }

    /// The form with its relation set to `kind` (the links list's cycling).
    #[must_use]
    pub(crate) fn with_kind(mut self, kind: EdgeKind) -> Self {
        self.kind = kind;
        if !self.rows().contains(&self.focus) {
            self.focus = EdgeRow::Relation;
        }
        self
    }

    /// The link the form writes back to.
    pub(crate) const fn target(&self) -> EdgeTarget {
        self.target
    }

    /// The chosen relation.
    #[cfg(test)]
    pub(crate) const fn kind(&self) -> EdgeKind {
        self.kind
    }

    /// The focused row.
    pub(crate) fn focused(&self) -> EdgeRow {
        self.focus
    }

    /// The rows shown: `Relation` always; `Predicate` only for `if`; `LoopLimit` only for
    /// `asks you`, `changes requested` and `fails`.
    pub(crate) fn rows(&self) -> Vec<EdgeRow> {
        let mut rows = vec![EdgeRow::Relation];
        if self.kind.takes_predicate() {
            rows.push(EdgeRow::Predicate);
        }
        if takes_loop_limit(self.kind) {
            rows.push(EdgeRow::LoopLimit);
        }
        rows
    }

    /// Focus on the next row, wrapping.
    pub(crate) fn next_field(&mut self) {
        self.step_focus(true);
    }

    /// Focus on the previous row, wrapping.
    pub(crate) fn prev_field(&mut self) {
        self.step_focus(false);
    }

    /// One row on, `forward` or back, wrapping over [`rows`](Self::rows).
    fn step_focus(&mut self, forward: bool) {
        let rows = self.rows();
        let at = rows.iter().position(|row| *row == self.focus).unwrap_or(0);
        let next = if forward {
            (at + 1) % rows.len()
        } else {
            (at + rows.len() - 1) % rows.len()
        };
        self.focus = rows[next];
    }

    /// On the relation row, one relation on or back through [`RELATIONS`], wrapping. Off it,
    /// nothing.
    pub(crate) fn cycle(&mut self, forward: bool) {
        if self.focus == EdgeRow::Relation {
            self.kind = cycle_kind(self.kind, forward);
        }
    }

    /// The focused text row takes the key; [`FieldOutcome::Pass`] on the relation row.
    pub(crate) fn on_text_key(&mut self, key: KeyEvent) -> FieldOutcome {
        match self.text_mut() {
            Some(field) => field.on_key(key),
            None => FieldOutcome::Pass,
        }
    }

    /// The focused text row takes the paste; the relation row ignores it.
    pub(crate) fn on_paste(&mut self, text: &str) {
        if let Some(field) = self.text_mut() {
            field.on_paste(text);
        }
    }

    /// The focused text field, if a text row has the focus.
    fn text_mut(&mut self) -> Option<&mut TextField> {
        match self.focus {
            EdgeRow::Relation => None,
            EdgeRow::Predicate => Some(&mut self.predicate),
            EdgeRow::LoopLimit => Some(&mut self.loop_limit),
        }
    }

    /// The predicate as typed, trimmed.
    fn predicate_text(&self) -> &str {
        self.predicate.text().unwrap_or_default().trim()
    }

    /// D9 live parse, shown under the predicate row: `None` when nothing is typed, when it
    /// parses, or when the relation is not `if`.
    pub(crate) fn predicate_error(&self) -> Option<String> {
        let text = self.predicate_text();
        if !self.kind.takes_predicate() || text.is_empty() {
            return None;
        }
        parse_predicate(text).err()
    }

    /// The link's editable part, or the first refusal in row order.
    ///
    /// `if` needs a predicate in the closed set. `changes requested` and `fails` need a loop
    /// limit, `asks you` may have one, `next` and `if` have none whatever was typed.
    pub(crate) fn build(&self) -> Result<EdgeSpec, String> {
        let predicate = if self.kind.takes_predicate() {
            let text = self.predicate_text();
            if text.is_empty() {
                return Err(PREDICATE_REQUIRED.to_owned());
            }
            Some(parse_predicate(text)?)
        } else {
            None
        };
        let loop_limit = if takes_loop_limit(self.kind) {
            let text = self.loop_limit.text().unwrap_or_default().trim();
            if text.is_empty() {
                if self.kind.needs_loop_limit() {
                    return Err(loop_limit_required(self.kind));
                }
                None
            } else {
                match text.parse::<i32>() {
                    Ok(limit) if limit >= 0 => Some(limit),
                    _ => return Err(LOOP_IS_A_NUMBER.to_owned()),
                }
            }
        } else {
            None
        };
        Ok(EdgeSpec {
            kind: self.kind,
            predicate,
            loop_limit,
        })
    }

    /// The form's title: `link implement → review`, from the body's step names.
    pub(crate) fn title(&self, body: &GraphBody) -> String {
        let ends = match self.target {
            EdgeTarget::New { from, to } => Some((from, to)),
            EdgeTarget::Edit(id) => body
                .edges
                .iter()
                .find(|edge| edge.id == id)
                .map(|edge| (edge.from, edge.to)),
        };
        let name = |id: PhaseId| step_name(body, id).unwrap_or("a missing step").to_owned();
        ends.map_or_else(
            || "link".to_owned(),
            |(from, to)| format!("link {} \u{2192} {}", name(from), name(to)),
        )
    }

    /// The form's rows `width` cells wide, the focused one being edited.
    pub(crate) fn lines(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>> {
        let rows = self.rows();
        let error = self.predicate_error();
        let limit_help = if self.kind.needs_loop_limit() {
            "how many times the loop may go round; required"
        } else {
            "how many times the loop may go round; empty for no limit"
        };
        let mut form = Form::new().editing(true);
        for row in &rows {
            let field = match row {
                EdgeRow::Relation => {
                    Field::new("relation", Value::Choice(relation_word(self.kind)))
                }
                EdgeRow::Predicate => {
                    let field = Field::new("predicate", Value::Text(&self.predicate))
                        .help("one line of JSON, e.g. {\"verify\":\"failed\"}");
                    match &error {
                        Some(error) => field.error(error.clone()),
                        None => field,
                    }
                }
                EdgeRow::LoopLimit => {
                    Field::new("loop limit", Value::Text(&self.loop_limit)).help(limit_help)
                }
            };
            form = form.field(field);
        }
        let focus = rows.iter().position(|row| *row == self.focus).unwrap_or(0);
        form.focus(focus).lines(width, accent, theme)
    }
}

/// Whether `kind` shows the loop limit row: required or optional.
const fn takes_loop_limit(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::AsksYou | EdgeKind::ChangesRequested | EdgeKind::Fails
    )
}

/// `text` as a predicate in the closed set, or the sentence refusing it.
fn parse_predicate(text: &str) -> Result<EdgePredicate, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| predicate_not_json(&error))?;
    IfPredicate::parse(&value).map_err(|error| predicate_refused(&error))?;
    Ok(EdgePredicate(value))
}

/// One step through [`RELATIONS`], wrapping.
pub(crate) fn cycle_kind(kind: EdgeKind, forward: bool) -> EdgeKind {
    let at = RELATIONS.iter().position(|each| *each == kind).unwrap_or(0);
    let len = RELATIONS.len();
    let next = if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    };
    RELATIONS[next]
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::model::StepGraphId;
    use ratatui::style::Color;
    use serde_json::json;

    use super::*;

    fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    /// The lines as text, trailing blanks dropped.
    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn type_text(form: &mut EdgeForm, text: &str) {
        for c in text.chars() {
            let outcome = form.on_text_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            assert_eq!(outcome, FieldOutcome::Consumed);
        }
    }

    fn ends() -> (PhaseId, PhaseId) {
        (PhaseId::new(), PhaseId::new())
    }

    /// A form on a fresh link with relation `kind`, focus on `row`.
    fn form_on(kind: EdgeKind, row: EdgeRow) -> EdgeForm {
        let (from, to) = ends();
        let mut form = EdgeForm::new(from, to).with_kind(kind);
        while form.focused() != row {
            form.next_field();
        }
        form
    }

    fn edge(
        kind: EdgeKind,
        predicate: Option<serde_json::Value>,
        limit: Option<i32>,
    ) -> StepGraphEdge {
        let (from, to) = ends();
        StepGraphEdge {
            id: EdgeId::new(),
            graph_id: StepGraphId::new(),
            from,
            to,
            kind,
            predicate: predicate.map(EdgePredicate),
            loop_limit: limit,
        }
    }

    #[test]
    fn a_new_link_is_next_with_no_predicate_or_limit() {
        let (from, to) = ends();
        let form = EdgeForm::new(from, to);
        assert_eq!(form.target(), EdgeTarget::New { from, to });
        assert_eq!(form.focused(), EdgeRow::Relation);
        assert_eq!(
            form.build(),
            Ok(EdgeSpec {
                kind: EdgeKind::Next,
                predicate: None,
                loop_limit: None,
            })
        );
    }

    #[test]
    fn cycle_kind_runs_next_if_asks_you_changes_requested_fails_and_wraps() {
        let mut kind = EdgeKind::Next;
        let mut seen = vec![kind];
        for _ in 0..5 {
            kind = cycle_kind(kind, true);
            seen.push(kind);
        }
        assert_eq!(
            seen,
            [
                EdgeKind::Next,
                EdgeKind::If,
                EdgeKind::AsksYou,
                EdgeKind::ChangesRequested,
                EdgeKind::Fails,
                EdgeKind::Next,
            ]
        );
        assert_eq!(cycle_kind(EdgeKind::Next, false), EdgeKind::Fails);
        assert_eq!(cycle_kind(EdgeKind::If, false), EdgeKind::Next);

        // The form cycles on the relation row only.
        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        form.cycle(true);
        assert_eq!(
            form.kind(),
            EdgeKind::If,
            "off the relation row nothing moves"
        );
        form.prev_field();
        form.cycle(true);
        assert_eq!(form.kind(), EdgeKind::AsksYou);
        form.cycle(false);
        form.cycle(false);
        assert_eq!(form.kind(), EdgeKind::Next);
    }

    #[test]
    fn if_needs_a_predicate() {
        let form = form_on(EdgeKind::If, EdgeRow::Predicate);
        assert_eq!(form.build(), Err(PREDICATE_REQUIRED.to_owned()));
        assert_eq!(
            form.predicate_error(),
            None,
            "nothing typed is not an error yet"
        );

        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        type_text(&mut form, "  ");
        assert_eq!(form.build(), Err(PREDICATE_REQUIRED.to_owned()));

        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        type_text(&mut form, r#"{"verify":"failed"}"#);
        assert_eq!(form.predicate_error(), None);
        assert_eq!(
            form.build(),
            Ok(EdgeSpec {
                kind: EdgeKind::If,
                predicate: Some(EdgePredicate(json!({"verify": "failed"}))),
                loop_limit: None,
            })
        );
    }

    #[test]
    fn a_predicate_must_be_json() {
        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        type_text(&mut form, "{verify");
        let error = form.predicate_error().expect("a live error");
        assert!(error.starts_with("`if` is not JSON: "), "{error}");
        assert_eq!(form.build(), Err(error));
    }

    #[test]
    fn a_predicate_outside_the_closed_set_is_refused_live() {
        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        type_text(&mut form, r#"{"weather":"sunny"}"#);
        let refusal =
            IfPredicate::parse(&json!({"weather": "sunny"})).expect_err("outside the set");
        assert_eq!(form.predicate_error(), Some(format!("`if`: {refusal}")));
        assert_eq!(form.build(), Err(format!("`if`: {refusal}")));
    }

    #[test]
    fn loop_limit_is_required_on_changes_requested_and_fails() {
        for kind in [EdgeKind::ChangesRequested, EdgeKind::Fails] {
            let form = form_on(kind, EdgeRow::LoopLimit);
            assert_eq!(form.build(), Err(loop_limit_required(kind)));
            let mut form = form_on(kind, EdgeRow::LoopLimit);
            type_text(&mut form, "2");
            assert_eq!(
                form.build(),
                Ok(EdgeSpec {
                    kind,
                    predicate: None,
                    loop_limit: Some(2),
                })
            );
        }
        assert_eq!(
            loop_limit_required(EdgeKind::ChangesRequested),
            "`loop limit` is required for changes requested"
        );
    }

    #[test]
    fn loop_limit_is_optional_on_asks_you_and_dropped_on_next_and_if() {
        let form = form_on(EdgeKind::AsksYou, EdgeRow::LoopLimit);
        assert_eq!(form.build().map(|spec| spec.loop_limit), Ok(None));
        let mut form = form_on(EdgeKind::AsksYou, EdgeRow::LoopLimit);
        type_text(&mut form, "3");
        assert_eq!(form.build().map(|spec| spec.loop_limit), Ok(Some(3)));

        // A limit typed under `fails`, then the relation changed: `next` and `if` drop it.
        let mut form = form_on(EdgeKind::Fails, EdgeRow::LoopLimit);
        type_text(&mut form, "4");
        let form = form.with_kind(EdgeKind::Next);
        assert_eq!(form.focused(), EdgeRow::Relation, "the limit row went away");
        assert_eq!(form.build().map(|spec| spec.loop_limit), Ok(None));
        let mut form = form.with_kind(EdgeKind::If);
        form.next_field();
        type_text(&mut form, r#"{"item_tag":"ui"}"#);
        assert_eq!(form.build().map(|spec| spec.loop_limit), Ok(None));
    }

    #[test]
    fn loop_limit_is_0_or_more() {
        for (typed, want) in [
            ("0", Ok(Some(0))),
            (" 7 ", Ok(Some(7))),
            ("-1", Err(LOOP_IS_A_NUMBER.to_owned())),
            ("two", Err(LOOP_IS_A_NUMBER.to_owned())),
            ("1.5", Err(LOOP_IS_A_NUMBER.to_owned())),
        ] {
            let mut form = form_on(EdgeKind::ChangesRequested, EdgeRow::LoopLimit);
            type_text(&mut form, typed);
            assert_eq!(form.build().map(|spec| spec.loop_limit), want, "{typed:?}");
        }
    }

    #[test]
    fn edit_prefills_kind_predicate_and_limit() {
        let link = edge(EdgeKind::If, Some(json!({"verify": "passed"})), None);
        let form = EdgeForm::edit(&link);
        assert_eq!(form.target(), EdgeTarget::Edit(link.id));
        assert_eq!(form.kind(), EdgeKind::If);
        assert_eq!(
            form.predicate.text(),
            Some(r#"{"verify":"passed"}"#),
            "compact JSON"
        );
        assert_eq!(
            form.build(),
            Ok(EdgeSpec {
                kind: EdgeKind::If,
                predicate: Some(EdgePredicate(json!({"verify": "passed"}))),
                loop_limit: None,
            })
        );

        let link = edge(EdgeKind::ChangesRequested, None, Some(2));
        let form = EdgeForm::edit(&link);
        assert_eq!(form.kind(), EdgeKind::ChangesRequested);
        assert_eq!(form.loop_limit.text(), Some("2"));
        assert_eq!(form.build().map(|spec| spec.loop_limit), Ok(Some(2)));
    }

    #[test]
    fn rows_follow_the_relation() {
        let (from, to) = ends();
        let form = EdgeForm::new(from, to);
        let rows = |kind| form_on(kind, EdgeRow::Relation).rows();
        assert_eq!(form.rows(), [EdgeRow::Relation]);
        assert_eq!(rows(EdgeKind::Next), [EdgeRow::Relation]);
        assert_eq!(rows(EdgeKind::If), [EdgeRow::Relation, EdgeRow::Predicate]);
        for kind in [
            EdgeKind::AsksYou,
            EdgeKind::ChangesRequested,
            EdgeKind::Fails,
        ] {
            assert_eq!(rows(kind), [EdgeRow::Relation, EdgeRow::LoopLimit]);
        }
        // Focus wraps over the rows shown.
        let mut form = form_on(EdgeKind::Fails, EdgeRow::Relation);
        form.next_field();
        assert_eq!(form.focused(), EdgeRow::LoopLimit);
        form.next_field();
        assert_eq!(form.focused(), EdgeRow::Relation);
        form.prev_field();
        assert_eq!(form.focused(), EdgeRow::LoopLimit);
        // The relation row takes no text.
        form.prev_field();
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(form.on_text_key(key), FieldOutcome::Pass);
    }

    #[test]
    fn debug_never_prints_typed_text() {
        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        type_text(&mut form, r#"{"item_tag":"hunter2"}"#);
        let printed = format!("{form:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("Predicate"), "{printed}");
    }

    #[test]
    fn title_names_both_steps() {
        let (from, to) = ends();
        let body = body_with(&[(from, "implement"), (to, "review")]);
        assert_eq!(
            EdgeForm::new(from, to).title(&body),
            "link implement \u{2192} review"
        );
    }

    /// A body holding just the named steps, no edges.
    fn body_with(steps: &[(PhaseId, &str)]) -> GraphBody {
        use htui_core::model::{
            CommandQueue, Gate, GraphScope, NodeBody, ProjectId, Split, StepGraph, StepGraphPhase,
        };
        let now = chrono::DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant");
        let graph = StepGraphId::new();
        GraphBody {
            graph: StepGraph {
                id: graph,
                scope: GraphScope::Project(ProjectId::new()),
                name: "feature".to_owned(),
                description: String::new(),
                is_override: false,
                revision: 1,
                created_at: now,
                updated_at: now,
            },
            nodes: steps
                .iter()
                .enumerate()
                .map(|(at, (id, name))| NodeBody {
                    phase: StepGraphPhase {
                        id: *id,
                        graph_id: graph,
                        position: i32::try_from(at).expect("a few steps"),
                        name: (*name).to_owned(),
                        fan_out: 1,
                        gate: Gate::Never,
                        gate_hard: false,
                        retry_limit: 0,
                        input_kinds: Vec::new(),
                        output_kind: (*name).to_owned(),
                        isolation: None,
                        command_queue: CommandQueue::Off,
                        verify_command: None,
                        template_name: (*name).to_owned(),
                        template_version: None,
                        token_budget: None,
                        persona_id: None,
                        split: Split::None,
                        palette: false,
                        plan_scope: None,
                        planner_persona_id: None,
                        planner_agent_id: None,
                        planner_model: None,
                        integrate_persona_id: None,
                        integrate_agent_id: None,
                        integrate_model: None,
                        max_parts: None,
                        updated_at: now,
                    },
                    agents: Vec::new(),
                    parts: Vec::new(),
                })
                .collect(),
            edges: Vec::new(),
        }
    }

    #[test]
    fn edge_form_if_error() {
        let mut form = form_on(EdgeKind::If, EdgeRow::Predicate);
        type_text(&mut form, r#"{"verify":"maybe"}"#);
        let lines = form.lines(56, accent(), &Theme::default());
        insta::assert_snapshot!("edge_form_if_error", text(&lines));
    }

    #[test]
    fn edge_form_changes_requested() {
        let link = edge(EdgeKind::ChangesRequested, None, Some(2));
        let form = EdgeForm::edit(&link);
        let lines = form.lines(56, accent(), &Theme::default());
        insta::assert_snapshot!("edge_form_changes_requested", text(&lines));
    }
}

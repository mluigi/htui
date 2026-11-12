//! MOD-141 M2 D9: the part form - one default part of a divide step: key, title, brief, owned
//! paths, acceptance, agent and model (none, or one of the step's own candidates) and persona.
//! Key-agnostic: the editor maps its acts onto these methods.

use crossterm::event::KeyEvent;
use htui_core::model::{AgentId, PersonaId, PhaseId, PhasePart};
use ratatui::style::Style;
use ratatui::text::Line;

use super::Lookups;
use super::node_form::{Seat, comma_list, pair_word, persona_options, persona_word, step, text_of};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::{FieldOutcome, TextField, Theme};

/// The key left empty.
pub(crate) const PART_KEY_REQUIRED: &str = "`key` is required";

/// A key another part of the same step already has (`phase_part`'s key is unique in its phase).
pub(crate) fn part_key_taken(key: &str) -> String {
    format!("`key` {key} is taken by another part of this step")
}

/// The persona row's help (D9: per-part personas run from MOD-140 on).
pub(crate) const PART_PERSONA_NOTE: &str = "per-part personas run from MOD-140 on";

/// The agent row's help: admission refuses a part agent the step does not list.
const AGENT_HELP: &str = "one of the step's own agents (a run refuses any other)";

/// The rows, in focus order.
const ROWS: usize = 7;
const KEY: usize = 0;
const TITLE: usize = 1;
const BRIEF: usize = 2;
const PATHS: usize = 3;
const ACCEPTANCE: usize = 4;
const AGENT: usize = 5;
const PERSONA: usize = 6;

/// Which part of the step form the part form writes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartTarget {
    /// A new part, appended.
    New,
    /// The step form's part at this index.
    Edit(usize),
}

/// The focused row's kind, for the editor's key mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartRow {
    /// A text row: it takes typed keys and pastes.
    Text,
    /// A choice row: it cycles.
    Choice,
}

/// The open part form (D9).
#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]
pub(crate) struct PartForm {
    /// The part it writes back to.
    target: PartTarget,
    /// `phase_part.key`.
    key: TextField,
    /// `phase_part.title`.
    title: TextField,
    /// `phase_part.brief`.
    brief: TextField,
    /// `phase_part.owned_paths`, comma separated.
    owned_paths: TextField,
    /// `phase_part.acceptance`; empty for none.
    acceptance: TextField,
    /// `phase_part.agent_id` and `phase_part.model`.
    agent: Option<Seat>,
    /// `phase_part.persona_id`.
    persona: Option<PersonaId>,
    /// The step form's candidates when the part form opened: the agents a part may name.
    candidates: Vec<(AgentId, String)>,
    /// The focused row, `0..ROWS`.
    focus: usize,
}

/// The target, the focus and the candidate count, never what was typed (H-7).
impl core::fmt::Debug for PartForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PartForm")
            .field("target", &self.target)
            .field("focus", &self.focus)
            .field("candidates", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]
impl PartForm {
    /// The form prefilled from `part` (blank for `None`), writing back to `target`. `candidates`
    /// are the step form's own, as they are now: the agents the part's agent row offers.
    pub(crate) fn new(
        part: Option<&PhasePart>,
        target: PartTarget,
        candidates: Vec<(AgentId, String)>,
    ) -> Self {
        let text = |text: Option<&str>| TextField::with_text(text.unwrap_or(""));
        Self {
            target,
            key: text(part.map(|part| part.key.as_str())),
            title: text(part.map(|part| part.title.as_str())),
            brief: text(part.map(|part| part.brief.as_str())),
            owned_paths: TextField::with_text(
                &part.map_or_else(String::new, |part| part.owned_paths.join(", ")),
            ),
            acceptance: text(part.and_then(|part| part.acceptance.as_deref())),
            agent: part.and_then(|part| part.agent_id.map(|agent| (agent, part.model.clone()))),
            persona: part.and_then(|part| part.persona_id),
            candidates,
            focus: KEY,
        }
    }

    /// The part the form writes back to.
    pub(crate) const fn target(&self) -> PartTarget {
        self.target
    }

    /// The focused row's kind.
    pub(crate) fn focused(&self) -> PartRow {
        match self.focus {
            AGENT | PERSONA => PartRow::Choice,
            _ => PartRow::Text,
        }
    }

    /// Focus on the next row, wrapping.
    pub(crate) fn next_field(&mut self) {
        self.focus = (self.focus + 1) % ROWS;
    }

    /// Focus on the previous row, wrapping.
    pub(crate) fn prev_field(&mut self) {
        self.focus = (self.focus + ROWS - 1) % ROWS;
    }

    /// The focused text field, if a text row has the focus.
    fn text_mut(&mut self) -> Option<&mut TextField> {
        match self.focus {
            KEY => Some(&mut self.key),
            TITLE => Some(&mut self.title),
            BRIEF => Some(&mut self.brief),
            PATHS => Some(&mut self.owned_paths),
            ACCEPTANCE => Some(&mut self.acceptance),
            _ => None,
        }
    }

    /// The focused text row takes the key; [`FieldOutcome::Pass`] off a text row.
    pub(crate) fn on_text_key(&mut self, key: KeyEvent) -> FieldOutcome {
        self.text_mut()
            .map_or(FieldOutcome::Pass, |field| field.on_key(key))
    }

    /// The focused text row takes the paste; a choice row ignores it.
    pub(crate) fn on_paste(&mut self, text: &str) {
        if let Some(field) = self.text_mut() {
            // An unmasked field always takes a paste.
            field.on_paste(text);
        }
    }

    /// The focused choice row moves one option, wrapping: the agent over none and the step's
    /// candidates, the persona over none and the registry. A current value the options lack is
    /// kept as the last option. A text row does nothing.
    pub(crate) fn cycle(&mut self, forward: bool, lookups: &Lookups) {
        match self.focus {
            AGENT => {
                let mut options: Vec<Option<Seat>> = core::iter::once(None)
                    .chain(self.candidates.iter().map(|(agent, model)| {
                        Some((*agent, (!model.is_empty()).then(|| model.clone())))
                    }))
                    .collect();
                if !options.contains(&self.agent) {
                    options.push(self.agent.clone());
                }
                self.agent = step(&options, &self.agent, forward);
            }
            PERSONA => {
                self.persona = step(
                    &persona_options(lookups, self.persona),
                    &self.persona,
                    forward,
                );
            }
            _ => {}
        }
    }

    /// The part of step `phase`, at position 0 (the step form renumbers). `others` are the
    /// step's other parts, whose keys this one may not take.
    ///
    /// # Errors
    /// [`PART_KEY_REQUIRED`], or [`part_key_taken`].
    pub(crate) fn build(&self, phase: PhaseId, others: &[PhasePart]) -> Result<PhasePart, String> {
        let key = text_of(&self.key);
        if key.is_empty() {
            return Err(PART_KEY_REQUIRED.to_owned());
        }
        if others.iter().any(|part| part.key == key) {
            return Err(part_key_taken(key));
        }
        let acceptance = text_of(&self.acceptance);
        Ok(PhasePart {
            phase_id: phase,
            position: 0,
            key: key.to_owned(),
            title: text_of(&self.title).to_owned(),
            brief: text_of(&self.brief).to_owned(),
            owned_paths: comma_list(text_of(&self.owned_paths)),
            acceptance: (!acceptance.is_empty()).then(|| acceptance.to_owned()),
            agent_id: self.agent.as_ref().map(|(agent, _)| *agent),
            model: self.agent.as_ref().and_then(|(_, model)| model.clone()),
            persona_id: self.persona,
        })
    }

    /// The form's title: `new part of research`, or `part ui of research`.
    pub(crate) fn title(&self, step: &str) -> String {
        match self.target {
            PartTarget::New => format!("new part of {step}"),
            PartTarget::Edit(_) => format!("part {} of {step}", text_of(&self.key)),
        }
    }

    /// The form's rows `width` cells wide, the focused one being edited.
    pub(crate) fn lines(
        &self,
        lookups: &Lookups,
        width: u16,
        accent: Style,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let agent = self.agent.as_ref().map_or_else(
            || "none".to_owned(),
            |(agent, model)| pair_word(*agent, model.as_deref().unwrap_or(""), lookups),
        );
        let persona = persona_word(self.persona, lookups);
        Form::new()
            .editing(true)
            .field(Field::new("key", Value::Text(&self.key)).help("unique in this step"))
            .field(Field::new("title", Value::Text(&self.title)))
            .field(Field::new("brief", Value::Text(&self.brief)).help("what this part does"))
            .field(
                Field::new("owned paths", Value::Text(&self.owned_paths))
                    .help("repo:glob, comma separated"),
            )
            .field(Field::new("acceptance", Value::Text(&self.acceptance)).help("empty for none"))
            .field(Field::new("agent", Value::Choice(&agent)).help(AGENT_HELP))
            .field(Field::new("persona", Value::Choice(&persona)).help(PART_PERSONA_NOTE))
            .focus(self.focus)
            .lines(width, accent, theme)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;

    use super::*;
    use crate::ui::graph_editor::node_form::tests::{accent, lookups, text};

    fn phase() -> PhaseId {
        PhaseId::from_uuid(uuid::Uuid::from_u128(0x0141_0001))
    }

    /// A part of `phase()` keyed `key`, titled `key`, nothing else set.
    fn part(key: &str) -> PhasePart {
        PhasePart {
            phase_id: phase(),
            position: 0,
            key: key.to_owned(),
            title: key.to_owned(),
            brief: String::new(),
            owned_paths: Vec::new(),
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        }
    }

    /// The step's candidates: claude · opus, then agy with no model.
    fn candidates() -> Vec<(AgentId, String)> {
        vec![
            (ids::AGENT_CLAUDE, "opus".to_owned()),
            (ids::AGENT_AGY, String::new()),
        ]
    }

    fn blank() -> PartForm {
        PartForm::new(None, PartTarget::New, candidates())
    }

    /// Replaces what text row `row` holds by typing `text` into it.
    fn set_text(form: &mut PartForm, row: usize, text: &str) {
        form.focus = row;
        for _ in 0..64 {
            let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
        for c in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
    }

    #[test]
    fn the_rows_wrap_and_say_their_kind() {
        use PartRow::{Choice, Text};
        let mut form = blank();
        let mut seen = vec![form.focused()];
        for _ in 0..ROWS {
            form.next_field();
            seen.push(form.focused());
        }
        assert_eq!(seen, [Text, Text, Text, Text, Text, Choice, Choice, Text]);
        form.prev_field();
        assert_eq!(form.focus, PERSONA);
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(form.on_text_key(key), FieldOutcome::Pass, "a choice row");
        form.on_paste("ignored");
        assert_eq!(form.persona, None);
    }

    #[test]
    fn a_part_key_is_required_and_unique_in_the_step() {
        let mut form = blank();
        assert_eq!(form.build(phase(), &[]), Err(PART_KEY_REQUIRED.to_owned()));
        set_text(&mut form, KEY, "   ");
        assert_eq!(form.build(phase(), &[]), Err(PART_KEY_REQUIRED.to_owned()));
        set_text(&mut form, KEY, " ui ");
        assert_eq!(
            form.build(phase(), &[part("store"), part("ui")]),
            Err(part_key_taken("ui"))
        );
        assert_eq!(
            form.build(phase(), &[part("store")]).map(|part| part.key),
            Ok("ui".to_owned())
        );
        // Keys are compared as stored: another case is another key.
        assert_eq!(
            form.build(phase(), &[part("UI")]).map(|part| part.key),
            Ok("ui".to_owned())
        );
    }

    #[test]
    fn part_paths_split_on_commas_and_acceptance_empty_is_none() {
        let mut form = blank();
        set_text(&mut form, KEY, "ui");
        set_text(&mut form, TITLE, "  The UI ");
        set_text(&mut form, BRIEF, " draw it ");
        set_text(&mut form, PATHS, " htui:crates/a/**, ,htui:crates/b ,");
        set_text(&mut form, ACCEPTANCE, "   ");
        let mut want = part("ui");
        want.title = "The UI".to_owned();
        want.brief = "draw it".to_owned();
        want.owned_paths = vec!["htui:crates/a/**".to_owned(), "htui:crates/b".to_owned()];
        assert_eq!(form.build(phase(), &[]), Ok(want.clone()));
        set_text(&mut form, ACCEPTANCE, " cargo test ");
        want.acceptance = Some("cargo test".to_owned());
        assert_eq!(form.build(phase(), &[]), Ok(want));

        // Untouched, the form builds back the part it opened on.
        let mut full = part("store");
        full.brief = "keep it".to_owned();
        full.owned_paths = vec!["htui:a".to_owned(), "htui:b/**".to_owned()];
        full.acceptance = Some("make".to_owned());
        full.agent_id = Some(ids::AGENT_CLAUDE);
        full.model = Some("opus".to_owned());
        full.persona_id = Some(ids::PERSONA_REVIEWER);
        let form = PartForm::new(Some(&full), PartTarget::Edit(1), candidates());
        assert_eq!(form.target(), PartTarget::Edit(1));
        assert_eq!(form.build(phase(), &[part("ui")]), Ok(full));
    }

    #[test]
    fn the_part_agent_cycles_none_then_the_steps_candidates_and_keeps_an_unknown_one() {
        let lookups = lookups();
        let mut form = blank();
        set_text(&mut form, KEY, "ui");
        form.focus = AGENT;
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups);
            seen.push(form.agent.clone());
        }
        assert_eq!(
            seen,
            [
                Some((ids::AGENT_CLAUDE, Some("opus".to_owned()))),
                Some((ids::AGENT_AGY, None)),
                None,
            ],
            "none, then the step's candidates in order; an empty model is none"
        );
        form.cycle(false, &lookups);
        assert_eq!(
            form.build(phase(), &[])
                .map(|part| (part.agent_id, part.model)),
            Ok((Some(ids::AGENT_AGY), None))
        );

        // An agent the step does not list (sonnet is not a candidate) is kept and shown.
        let mut stray = part("ui");
        stray.agent_id = Some(ids::AGENT_CLAUDE);
        stray.model = Some("sonnet".to_owned());
        let mut form = PartForm::new(Some(&stray), PartTarget::Edit(0), candidates());
        form.focus = AGENT;
        let shown = text(&form.lines(&lookups, 80, accent(), &Theme::default()));
        assert!(shown.contains("claude \u{b7} sonnet"), "{shown}");
        assert!(shown.contains(AGENT_HELP), "{shown}");
        form.cycle(true, &lookups);
        assert_eq!(form.agent, None, "it sits last; forward wraps to none");
        form.cycle(false, &lookups);
        assert_eq!(
            form.agent,
            Some((ids::AGENT_AGY, None)),
            "once left, it is no longer offered: back goes to the last candidate"
        );

        // With no candidates the agent is none or what it was.
        let mut form = PartForm::new(None, PartTarget::New, Vec::new());
        form.focus = AGENT;
        form.cycle(true, &lookups);
        assert_eq!(form.agent, None);
    }

    #[test]
    fn the_part_persona_cycles_none_then_the_registry() {
        let lookups = lookups();
        let mut form = blank();
        form.focus = PERSONA;
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups);
            seen.push(form.persona);
        }
        assert_eq!(
            seen,
            [
                Some(ids::PERSONA_REVIEWER),
                Some(ids::PERSONA_ARCHITECT),
                None
            ]
        );
        let shown = text(&form.lines(&lookups, 80, accent(), &Theme::default()));
        assert!(shown.contains(PART_PERSONA_NOTE), "{shown}");

        // An id the registry lacks is kept, shown by id, and sits last.
        let unknown = PersonaId::new();
        let mut stray = part("ui");
        stray.persona_id = Some(unknown);
        let mut form = PartForm::new(Some(&stray), PartTarget::Edit(0), candidates());
        let shown = text(&form.lines(&lookups, 80, accent(), &Theme::default()));
        assert!(shown.contains(&unknown.to_string()), "{shown}");
        form.focus = PERSONA;
        form.cycle(true, &lookups);
        assert_eq!(form.persona, None);
    }

    #[test]
    fn the_title_names_the_part_and_its_step() {
        assert_eq!(blank().title("research"), "new part of research");
        let form = PartForm::new(Some(&part("ui")), PartTarget::Edit(0), candidates());
        assert_eq!(form.title("research"), "part ui of research");
    }

    #[test]
    fn part_form_debug_never_prints_typed_text() {
        let mut form = blank();
        set_text(&mut form, KEY, "hunter2");
        set_text(&mut form, BRIEF, "swordfish");
        set_text(&mut form, PATHS, "secret:path");
        let printed = format!("{form:?}");
        for typed in ["hunter2", "swordfish", "secret:path"] {
            assert!(!printed.contains(typed), "{printed}");
        }
        assert!(printed.contains("PartForm"), "{printed}");
        assert!(printed.contains("candidates: 2"), "{printed}");
    }

    #[test]
    fn part_form_edit() {
        let mut ui = part("ui");
        ui.title = "UI".to_owned();
        ui.brief = "the step form's rows".to_owned();
        ui.owned_paths = vec!["htui:crates/htui/src/ui/**".to_owned()];
        ui.acceptance = Some("cargo test -p htui".to_owned());
        ui.agent_id = Some(ids::AGENT_CLAUDE);
        ui.model = Some("opus".to_owned());
        ui.persona_id = Some(ids::PERSONA_REVIEWER);
        let mut form = PartForm::new(Some(&ui), PartTarget::Edit(0), candidates());
        form.focus = PERSONA;
        let lines = form.lines(&lookups(), 56, accent(), &Theme::default());
        insta::assert_snapshot!("part_form_edit", text(&lines));
    }
}

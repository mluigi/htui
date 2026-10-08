//! Hint rows, the status line and the `?` box, generated from the active stack (ANA-26 §7.6).
//!
//! Every label comes from the keys in force, so a rebound action shows its new chord (MOD-67 D7,
//! D8). A hint row and the status line take the first chord of each action; a `?` box line lists
//! all of them. Rows sharing a help label (the nine `select_tab_*`) collapse into one entry, and
//! an unbound action drops out, except `global.quit`, which always ends with the fixed `Ctrl+c`.
//!
//! MOD-67 M3: every label goes through the layer that resolves it, so a modal layer's filtered
//! chords never show (D5). The status line, the `?` box and its closer render a stack (D7, D8):
//! `Stack::BASE` gives today's text, a view's stack its own.

use crossterm::event::KeyCode;
use unicode_width::UnicodeWidthStr;

use super::{Act, CTRL_C, Context, KeyChord, Keys, Stack};

/// The separator between entries: U+00B7 with a space on each side, as the status line has
/// always written it.
const SEP: &str = " · ";

/// Where a continuation row of a packed [`HelpLine`] starts.
const INDENT: &str = "  ";

/// One element of a hint spec (ANA-26 §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// `"{label} {text}"`, e.g. `e edit DSN`; the label alone when `text` is empty.
    One(Act, &'static str),
    /// `"{label}/{label} {text}"`, e.g. `j/k rows`; one side unbound renders the other alone,
    /// and an empty `text` renders the labels alone (`j/k`).
    Pair(Act, Act, &'static str),
    /// Every admitted chord of the act, joined by `/`, then `text`: `n/Esc cancel` (MOD-67 D9).
    All(Act, &'static str),
    /// Fixed text: a widget's own key (`Enter store`, `Esc cancel`) or a note (`typed text is
    /// never shown`). Never dropped, unless it is empty.
    Text(&'static str),
}

/// A view's hint row, as data: `const BROWSE: HintSpec = &[...]`.
pub type HintSpec = &'static [Hint];

/// One logical line of the `?` box: a heading and its entries (D8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpLine {
    /// `Global`, `Overlay`, or a tab's title for its legacy rows.
    pub heading: String,
    /// `"{chords} {help}"` entries, e.g. `q/Ctrl+c quit`.
    pub entries: Vec<String>,
}

impl HelpLine {
    /// A line from its parts (the shell builds the legacy tab line with it).
    #[must_use]
    pub fn new(heading: impl Into<String>, entries: Vec<String>) -> Self {
        Self {
            heading: heading.into(),
            entries,
        }
    }

    /// `"{heading}: {entries joined by " · "}"`.
    #[must_use]
    pub fn text(&self) -> String {
        format!("{}: {}", self.heading, self.entries.join(SEP))
    }

    /// The line packed into rows at most `width` cells wide (`unicode-width`), breaking only
    /// between entries. Continuation rows start with two spaces. A single entry wider than
    /// `width` gets a row of its own (and is clipped when drawn).
    #[must_use]
    pub fn rows(&self, width: usize) -> Vec<String> {
        let mut rows = Vec::new();
        let mut row = format!("{}: ", self.heading);
        let mut empty = true;
        for entry in &self.entries {
            if empty {
                row.push_str(entry);
            } else if row.width() + SEP.width() + entry.width() <= width {
                row.push_str(SEP);
                row.push_str(entry);
            } else {
                rows.push(std::mem::replace(&mut row, format!("{INDENT}{entry}")));
            }
            empty = false;
        }
        rows.push(row);
        rows
    }
}

impl Keys {
    /// The first chord of `act` through `stack` that its layer admits (`Layer::admits_chord`),
    /// as `KeyChord::label` writes it; `None` when there is none. Prose such as
    /// `format!("press {}", …)` uses it from M3 on.
    #[must_use]
    pub fn label(&self, stack: Stack<'_>, act: Act) -> Option<String> {
        self.labels(stack, act).into_iter().next()
    }

    /// Every chord of `act` through `stack` that its layer admits, as labels (MOD-67 D9).
    #[must_use]
    pub fn labels(&self, stack: Stack<'_>, act: Act) -> Vec<String> {
        self.resolve_row(stack, act)
            .map(|(layer, row)| {
                row.chords
                    .iter()
                    .filter(|chord| layer.admits_chord(**chord))
                    .map(KeyChord::label)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A view's hint row: each element through `stack`, unbound ones dropped, joined by ` · `
    /// (ANA-26 §7.6). `One` and `Pair` take the first admitted chord, `All` every one; `Text`
    /// is written as is.
    #[must_use]
    pub fn hint(&self, stack: Stack<'_>, spec: HintSpec) -> String {
        let labelled = |labels: Vec<String>, text: &str| {
            (!labels.is_empty()).then(|| with_text(&labels.join("/"), text))
        };
        let element = |hint: &Hint| match *hint {
            Hint::One(act, text) => labelled(self.label(stack, act).into_iter().collect(), text),
            Hint::Pair(first, second, text) => labelled(
                [first, second]
                    .into_iter()
                    .filter_map(|act| self.label(stack, act))
                    .collect(),
                text,
            ),
            Hint::All(act, text) => labelled(self.labels(stack, act), text),
            Hint::Text(text) => (!text.is_empty()).then(|| text.to_owned()),
        };
        spec.iter()
            .filter_map(element)
            .collect::<Vec<_>>()
            .join(SEP)
    }

    /// The status line (D7, MOD-67 M3 PA-9) from `stack`'s global layers: quit first, always,
    /// as its first admitted chord, else `Ctrl+c`; then every other `Global` row in catalogue
    /// order that is `offered` and has an admitted chord, each through the **first** global
    /// layer that offers it (`Stack::admits`), its first admitted chord, rows sharing a help
    /// label collapsed to the first (the digits), joined by ` · `. `Stack::BASE` gives the M1
    /// line; a stack with one global layer renders exactly as before (MOD-67 M4 PA-2).
    #[must_use]
    pub fn status_line(&self, stack: Stack<'_>, offered: impl Fn(Act) -> bool) -> String {
        // The first global layer that offers `act` (a `TABS` layer before `MODAL`, M4 PA-2).
        let layer_of = |act: Act| {
            stack
                .layers()
                .iter()
                .copied()
                .enumerate()
                .find(|&(index, layer)| {
                    layer.context() == Context::Global && stack.admits(index, act)
                })
                .map(|(_, layer)| layer)
        };
        let admitted =
            |row: &&super::Row| row.context == Context::Global && layer_of(row.act).is_some();
        let first_chord = |row: &super::Row| {
            layer_of(row.act).and_then(|layer| {
                row.chords
                    .iter()
                    .copied()
                    .find(|chord| layer.admits_chord(*chord))
            })
        };
        let quit = self
            .rows
            .iter()
            .filter(admitted)
            .find(|row| row.act == Act::Quit)
            .and_then(first_chord)
            .unwrap_or(CTRL_C);
        let quit_help = Act::Quit.spec().map_or("quit", |spec| spec.help);
        let mut shown: Vec<&str> = vec![quit_help];
        let mut entries = vec![format!("{} {quit_help}", quit.label())];
        for row in self.rows.iter().filter(admitted) {
            if row.act == Act::Quit || !offered(row.act) || shown.contains(&row.help) {
                continue;
            }
            let Some(first) = first_chord(row) else {
                continue;
            };
            shown.push(row.help);
            entries.push(format!("{} {}", first.label(), row.help));
        }
        entries.join(SEP)
    }

    /// The `?` box (D8, MOD-67 M3): one line per layer of `stack`, narrowest first, under the
    /// layer's `Context::heading`. A row is listed under the first layer that admits it and has
    /// a row for it (the `actions` shadowing rule, L-B Q6), with every chord that layer admits;
    /// rows sharing a help label merge, and a row with no admitted chord drops. `offered` is
    /// consulted for `Global` rows only. Every global layer of the stack renders as **one**
    /// `Global:` line at the last global layer's place (MOD-67 M4 PA-2): quit first (its admitted
    /// chords, none when no global layer admits `Quit`, then the fixed `Ctrl+c`), then the rows
    /// in layer order, an act listed under the first global layer that offers it. A layer that
    /// lists nothing has no line.
    #[must_use]
    pub fn help_lines(&self, stack: Stack<'_>, offered: impl Fn(Act) -> bool) -> Vec<HelpLine> {
        let mut seen: Vec<Act> = Vec::new();
        let mut lines = Vec::new();
        let last_global = stack.global().map(|(index, _)| index);
        // Shared by every global layer: opened with quit's placeholder, pushed at the last one.
        let quit_help = Act::Quit.spec().map_or("quit", |spec| spec.help);
        let mut global_merged: Vec<(&str, Vec<KeyChord>)> = vec![(quit_help, Vec::new())];
        for (index, layer) in stack.layers().iter().enumerate() {
            let is_global = layer.context() == Context::Global;
            let mut here = Vec::new();
            let mut local: Vec<(&str, Vec<KeyChord>)> = Vec::new();
            let merged = if is_global {
                &mut global_merged
            } else {
                &mut local
            };
            let rows = self
                .rows
                .iter()
                .filter(|row| row.context == layer.context() && stack.admits(index, row.act));
            for row in rows {
                if seen.contains(&row.act) {
                    continue; // shadowed by a narrower layer
                }
                here.push(row.act);
                if row.context == Context::Global && row.act != Act::Quit && !offered(row.act) {
                    continue;
                }
                let chords: Vec<KeyChord> = row
                    .chords
                    .iter()
                    .copied()
                    .filter(|chord| layer.admits_chord(*chord))
                    .collect();
                match merged.iter_mut().find(|(help, _)| *help == row.help) {
                    Some((_, known)) => known.extend(chords),
                    None => merged.push((row.help, chords)),
                }
            }
            seen.extend(here);
            if is_global && last_global != Some(index) {
                continue; // the one `Global:` line sits at the last global layer
            }
            if is_global
                && let Some((_, quit)) = merged.first_mut()
                && !quit.contains(&CTRL_C)
            {
                quit.push(CTRL_C);
            }
            let entries: Vec<String> = merged
                .iter()
                .filter(|(_, chords)| !chords.is_empty())
                .map(|(help, chords)| format!("{} {help}", chord_list(chords)))
                .collect();
            if !entries.is_empty() {
                lines.push(HelpLine::new(layer.context().heading(), entries));
            }
        }
        lines
    }

    /// One `?` box line for `context` (D8): every offered and bound action, all its chords
    /// joined by `/`, rows sharing a help label merged into one entry. `Quit` always appears and
    /// ends with the fixed `Ctrl+c`. `None` if the context offers nothing.
    #[must_use]
    pub fn help_line(&self, context: Context, offered: impl Fn(Act) -> bool) -> Option<HelpLine> {
        let mut merged: Vec<(&str, Vec<KeyChord>)> = Vec::new();
        for row in self.rows.iter().filter(|row| row.context == context) {
            let quit = row.act == Act::Quit;
            if !quit && !offered(row.act) {
                continue;
            }
            let mut chords = row.chords.clone();
            if quit && !chords.contains(&CTRL_C) {
                chords.push(CTRL_C);
            }
            if chords.is_empty() {
                continue;
            }
            match merged.iter_mut().find(|(help, _)| *help == row.help) {
                Some((_, known)) => known.extend(chords),
                None => merged.push((row.help, chords)),
            }
        }
        if merged.is_empty() {
            return None;
        }
        let entries = merged
            .iter()
            .map(|(help, chords)| format!("{} {help}", chord_list(chords)))
            .collect();
        Some(HelpLine::new(context.heading(), entries))
    }

    /// The box's last line, from the `global.help` chords that `stack`'s global layer admits:
    /// `"?/F1 closes this box"` on `Stack::BASE`, `"F1 closes this box"` in a modal stack;
    /// `None` if there is none (help unbound, filtered out, or no global layer).
    #[must_use]
    pub fn help_closer(&self, stack: Stack<'_>) -> Option<String> {
        let (index, layer) = stack.global()?;
        if !stack.admits(index, Act::Help) {
            return None;
        }
        let chords: Vec<KeyChord> = self
            .chords(Context::Global, Act::Help)
            .iter()
            .copied()
            .filter(|chord| layer.admits_chord(*chord))
            .collect();
        (!chords.is_empty()).then(|| format!("{} closes this box", chord_list(&chords)))
    }
}

/// `"{labels} {text}"`, or the labels alone when `text` is empty (MOD-67 M3, L-B Q3).
fn with_text(labels: &str, text: &str) -> String {
    if text.is_empty() {
        labels.to_owned()
    } else {
        format!("{labels} {text}")
    }
}

/// One merged entry's chords: `1-9` for a run of three or more consecutive plain characters,
/// else every label joined by `/` (`q/Ctrl+c`, `1/2/x/4`).
fn chord_list(chords: &[KeyChord]) -> String {
    let plain: Option<Vec<char>> = chords
        .iter()
        .map(|chord| match chord.code {
            KeyCode::Char(c) if chord.mods.is_empty() => Some(c),
            _ => None,
        })
        .collect();
    if let Some(plain) = plain
        && let [first, .., last] = plain[..]
        && plain.len() >= 3
        && plain
            .windows(2)
            .all(|pair| u32::from(pair[1]) == u32::from(pair[0]) + 1)
    {
        return format!("{first}-{last}");
    }
    chords
        .iter()
        .map(KeyChord::label)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{HelpLine, Hint};
    use crate::keys::{Act, Context, Keys, Layer, Stack, views};

    const BARE: &str = "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help";
    const FULL: &str = "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help \
                        · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue";

    /// What the shell offers without `register_all`: every act but the four that name a view.
    fn bare(act: Act) -> bool {
        !matches!(act, Act::Workspaces | Act::Find | Act::Waiting | Act::Queue)
    }

    fn all(_: Act) -> bool {
        true
    }

    fn global_line(keys: &Keys, offered: fn(Act) -> bool) -> String {
        keys.help_line(Context::Global, offered)
            .expect("the global context offers something")
            .text()
    }

    #[test]
    fn the_status_line_without_register_all_is_todays() {
        assert_eq!(Keys::compiled().status_line(Stack::BASE, bare), BARE);
    }

    #[test]
    fn the_status_line_with_every_global_offered() {
        assert_eq!(Keys::compiled().status_line(Stack::BASE, all), FULL);
    }

    #[test]
    fn an_unbound_action_drops_out_of_the_status_line_and_a_hint() {
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &[]);
        // PA-9: quit always leads the status line, as `Ctrl+c` once its own chords are gone.
        assert!(
            keys.status_line(Stack::BASE, all)
                .starts_with("Ctrl+c quit · Tab next tab")
        );
        let spec = &[Hint::One(Act::Quit, "quit"), Hint::One(Act::Help, "help")];
        assert_eq!(keys.hint(Stack::BASE, spec), "? help");
    }

    #[test]
    fn a_pair_renders_both_labels_or_the_bound_one() {
        let layers = [Layer::all(Context::List)];
        let stack = Stack::new(&layers);
        let spec = &[Hint::Pair(Act::ListDown, Act::ListUp, "rows")];
        assert_eq!(Keys::compiled().hint(stack, spec), "j/k rows");
        let keys = Keys::defaults().with_chords(Context::List, Act::ListUp, &[]);
        assert_eq!(keys.hint(stack, spec), "j rows");
        let keys = keys.with_chords(Context::List, Act::ListDown, &[]);
        assert_eq!(keys.hint(stack, spec), "");
    }

    #[test]
    fn label_takes_the_first_chord_through_the_stack() {
        let keys = Keys::compiled();
        assert_eq!(keys.label(Stack::BASE, Act::Help).as_deref(), Some("?"));
        let f1 = Keys::defaults().with_chords(Context::Global, Act::Help, &["f1"]);
        assert_eq!(f1.label(Stack::BASE, Act::Help).as_deref(), Some("F1"));
        let unbound = Keys::defaults().with_chords(Context::Global, Act::Help, &[]);
        assert_eq!(unbound.label(Stack::BASE, Act::Help), None);
        assert_eq!(keys.label(Stack::BASE, Act::OverlayClose), None);
    }

    #[test]
    fn the_help_lines_follow_d8() {
        let keys = Keys::compiled();
        assert_eq!(
            global_line(keys, bare),
            "Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab \
             · ?/F1 help"
        );
        assert_eq!(
            global_line(keys, all),
            "Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab \
             · ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue"
        );
        assert_eq!(
            keys.help_line(Context::Overlay, all)
                .map(|line| line.text())
                .as_deref(),
            Some("Overlay: Esc close")
        );
        assert_eq!(
            keys.help_closer(Stack::BASE).as_deref(),
            Some("?/F1 closes this box")
        );
    }

    #[test]
    fn quit_lists_ctrl_c_even_when_q_is_unbound() {
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &[]);
        assert!(global_line(&keys, all).starts_with("Global: Ctrl+c quit · "));
    }

    #[test]
    fn a_broken_digit_run_is_listed_chord_by_chord() {
        let keys = Keys::defaults().with_chords(Context::Global, Act::SelectTab3, &["x"]);
        let line = keys.help_line(Context::Global, all).expect("a global line");
        assert!(
            line.entries
                .iter()
                .any(|entry| entry == "1/2/x/4/5/6/7/8/9 select tab"),
            "{:?}",
            line.entries
        );
    }

    #[test]
    fn rows_break_between_entries_only() {
        let entries = ["aaa", "bbb", "ccc"].map(str::to_owned).to_vec();
        assert_eq!(
            HelpLine::new("Global", entries).rows(16),
            ["Global: aaa", "  bbb · ccc"]
        );
        let global = Keys::compiled()
            .help_line(Context::Global, all)
            .expect("a global line");
        assert_eq!(
            global.rows(88),
            [
                "Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab",
                "  ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue",
            ]
        );
    }

    #[test]
    fn an_empty_text_renders_the_labels_alone() {
        let layers = [Layer::all(Context::Common), Layer::all(Context::List)];
        let spec = &[
            Hint::Pair(Act::ListDown, Act::ListUp, ""),
            Hint::One(Act::Edit, "edit"),
            Hint::One(Act::Reload, ""),
        ];
        assert_eq!(
            Keys::compiled().hint(Stack::new(&layers), spec),
            "j/k · e edit · r"
        );
    }

    #[test]
    fn all_lists_every_admitted_chord_and_text_is_fixed() {
        let keys = Keys::compiled();
        let layers = [Layer::all(Context::Confirm)];
        let spec = &[
            Hint::All(Act::ConfirmNo, "cancel"),
            Hint::Text("Enter store"),
            Hint::Text(""),
        ];
        assert_eq!(
            keys.hint(Stack::new(&layers), spec),
            "n/Esc cancel · Enter store"
        );
        let spec = &[Hint::All(Act::Help, "help"), Hint::All(Act::Quit, "quit")];
        assert_eq!(keys.hint(views::CAPTURE, spec), "F1 help");
        assert_eq!(keys.hint(Stack::BASE, spec), "?/F1 help · q quit");
        assert_eq!(keys.labels(views::CAPTURE, Act::Help), ["F1".to_owned()]);
        assert_eq!(keys.label(views::CAPTURE, Act::Help).as_deref(), Some("F1"));
    }

    #[test]
    fn the_status_line_follows_the_stacks_global_layer() {
        let keys = Keys::compiled();
        assert_eq!(keys.status_line(views::AGENTS_BROWSE, bare), BARE);
        assert_eq!(keys.status_line(views::AGENTS_BROWSE, all), FULL);
        let modal_all = "Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue";
        for stack in [views::AGENTS_FORM, views::CAPTURE, views::KINDS_CONFIRM] {
            assert_eq!(keys.status_line(stack, all), modal_all);
            assert_eq!(keys.status_line(stack, bare), "Ctrl+c quit · F1 help");
        }
        for stack in [views::SWITCHER, views::WAITING_LIST, views::MIGRATION] {
            assert_eq!(keys.status_line(stack, all), "Ctrl+c quit · ? help");
            assert_eq!(keys.status_line(stack, bare), "Ctrl+c quit · ? help");
        }
        assert_eq!(
            keys.status_line(views::CONCEPTS_QUERY, all),
            "Ctrl+c quit · F1 help"
        );
    }

    fn texts(lines: Vec<HelpLine>) -> Vec<String> {
        lines.iter().map(HelpLine::text).collect()
    }

    #[test]
    fn the_help_lines_list_an_act_under_its_narrowest_layer() {
        let keys = Keys::compiled();
        assert_eq!(
            texts(keys.help_lines(views::QUEUE_BROWSE, bare)),
            [
                "Queue: e/Enter edit",
                "Settings: l/]/Right next section · h/[/Left previous section",
                "Common: r reload · Esc dismiss",
                "List: j/Down down · k/Up up",
                "Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab \
                 · ?/F1 help",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::SWITCHER, all)),
            [
                "Workspaces: Enter switch workspace",
                "List: j/Down down · k/Up up",
                "Overlay: Esc close",
                "Global: Ctrl+c quit · ?/F1 help",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::MIGRATION, all)),
            [
                "Schema: y/Y yes · n/Esc/N no",
                "Overlay: Esc close",
                "Global: Ctrl+c quit · ?/F1 help",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::AGENTS_FORM, bare)),
            [
                "Agents: Tab/Down next field · Shift+Tab/Up previous field",
                "Global: Ctrl+c quit · F1 help",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::CAPTURE, all)),
            ["Global: Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue"]
        );
    }

    /// MOD-67 M4 PA-2: every global layer of a stack feeds the status line and one `Global:`
    /// line of the `?` box; the closer follows the last (modal) one.
    #[test]
    fn a_tabs_layer_shows_on_the_status_line_and_in_one_global_line() {
        use views::TEMPLATES_EDITOR;
        let keys = Keys::compiled();
        let tabs = "Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help";
        assert_eq!(
            keys.status_line(TEMPLATES_EDITOR, all),
            format!("{tabs} · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue")
        );
        assert_eq!(keys.status_line(TEMPLATES_EDITOR, bare), tabs);
        assert_eq!(
            texts(keys.help_lines(TEMPLATES_EDITOR, all)),
            [
                "Skills: Ctrl+g ask agent".to_owned(),
                "Form: Ctrl+s save · Ctrl+e $EDITOR".to_owned(),
                format!("Global: {tabs} · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue"),
            ]
        );
        assert_eq!(
            keys.help_closer(TEMPLATES_EDITOR).as_deref(),
            Some("F1 closes this box")
        );
        assert!(!TEMPLATES_EDITOR.passes(crate::keys::KeyChord::parse_strict("tab").expect("tab")));
    }

    #[test]
    fn the_closer_follows_the_global_layer() {
        let keys = Keys::compiled();
        assert_eq!(
            keys.help_closer(Stack::BASE).as_deref(),
            Some("?/F1 closes this box")
        );
        assert_eq!(
            keys.help_closer(views::SWITCHER).as_deref(),
            Some("?/F1 closes this box")
        );
        for stack in [views::CONCEPTS_QUERY, views::CAPTURE] {
            assert_eq!(
                keys.help_closer(stack).as_deref(),
                Some("F1 closes this box")
            );
        }
        let unbound = Keys::defaults().with_chords(Context::Global, Act::Help, &[]);
        assert_eq!(unbound.help_closer(Stack::BASE), None);
        let question_only = Keys::defaults().with_chords(Context::Global, Act::Help, &["?"]);
        assert_eq!(question_only.help_closer(views::CAPTURE), None);
    }
}

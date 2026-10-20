//! Hint rows, the status line and the `?` box, generated from the active stack (ANA-26 §7.6).
//!
//! Every label comes from the keys in force, so a rebound action shows its new chord (MOD-67 D7,
//! D8). A hint row and the status line take the first chord of each action; a `?` box line lists
//! all of them. Rows sharing a help label (the nine `select_tab_*`) collapse into one entry, and
//! an unbound action drops out. MOD-115 M2: `ctrl-c` is `global.quit`'s own default chord, listed
//! like any other, so nothing is appended to it.
//!
//! MOD-67 M3: every label goes through the layer that resolves it, so a modal layer's filtered
//! chords never show (D5). The status line, the `?` box and its closer render a stack (D7, D8):
//! `Stack::BASE` gives today's text, a view's stack its own.

use unicode_width::UnicodeWidthStr;

use super::{Act, Context, KeyChord, Keys, SHADOWING, Stack};

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
    /// [`Pair`](Self::Pair), each label as the header's chips write it
    /// ([`KeyChord::short_label`]: an arrow is `←→↑↓`): a choice's `←/→ choose` (MOD-115 M2 RV1
    /// H2), which reads as the literal hint did until a side moves.
    Glyphs(Act, Act, &'static str),
    /// Every admitted chord of the act, joined by `/`, then `text`: `n/Esc cancel` (MOD-67 D9).
    All(Act, &'static str),
    /// The act's first two admitted chords, joined by `/`, then `text`: a toggle such as
    /// `skills.switch_view`, whose six defaults `All` would spell out, reads `h/l view` (MOD-67
    /// M4 R1). One admitted chord renders alone.
    Two(Act, &'static str),
    /// The first admitted chord of each act, merged as a `?` box entry merges them (`1-9` for a
    /// run of three or more consecutive plain characters, else joined by `/`), then `text`: the
    /// nine `answer.option_*` read `1-9 answer` until one moves (MOD-115 M2 T5). An unbound act
    /// drops out, and the element with it when every act is unbound.
    Many(&'static [Act], &'static str),
    /// Fixed text: a widget's own key (`Enter store`, `Esc cancel`) or a note (`typed text is
    /// never shown`). Never dropped, unless it is empty.
    Text(&'static str),
}

/// A view's hint row, as data: `const BROWSE: HintSpec = &[...]`.
pub type HintSpec = &'static [Hint];

/// One logical line of the `?` box: a heading and its entries (D8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpLine {
    /// `Global`, `Overlay`, or a context's heading.
    pub heading: String,
    /// `"{chords} {help}"` entries, e.g. `q/Ctrl+c quit`.
    pub entries: Vec<String>,
}

impl HelpLine {
    /// A line from its parts.
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

    /// MOD-115 M2 §8: the first chord of `act` through `stack` that its layer admits and that
    /// reaches it (`act` is the chord's first candidate, so no narrower act takes it), as
    /// [`KeyChord::short_label`] writes it: the header's `⌫ back` / `Alt→ forward` chips. `None`
    /// when no chord reaches it.
    #[must_use]
    pub fn reachable_label(&self, stack: Stack<'_>, act: Act) -> Option<String> {
        self.reachable_chord(stack, act)
            .map(|chord| chord.short_label())
    }

    /// The chord [`reachable_label`](Self::reachable_label) names: the first chord of `act`
    /// through `stack` that its layer admits and whose first candidate is `act`. What the
    /// hidden-pane note and the activity placeholder label (`KeyChord::label`), so a key a
    /// narrower act takes (`p` in Backlog > Runs, blueprint D-1) is never offered; and the key
    /// the wheel stands for (MD8). `None` when no chord reaches it.
    #[must_use]
    pub fn reachable_chord(&self, stack: Stack<'_>, act: Act) -> Option<KeyChord> {
        let (layer, row) = self.resolve_row(stack, act)?;
        row.chords
            .iter()
            .copied()
            .filter(|chord| layer.admits_chord(*chord))
            .find(|chord| self.actions(stack, *chord).first() == Some(&act))
    }

    /// The first chord of `act` through `stack` that its layer admits: what [`label`](Self::label)
    /// writes, as a chord.
    fn first_chord(&self, stack: Stack<'_>, act: Act) -> Option<KeyChord> {
        let (layer, row) = self.resolve_row(stack, act)?;
        row.chords
            .iter()
            .copied()
            .find(|chord| layer.admits_chord(*chord))
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
    /// (ANA-26 §7.6). `One` and `Pair` take the first admitted chord, `Two` the first two, `All`
    /// every one; `Text` is written as is.
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
            Hint::Glyphs(first, second, text) => labelled(
                [first, second]
                    .into_iter()
                    .filter_map(|act| self.first_chord(stack, act))
                    .map(|chord| chord.short_label())
                    .collect(),
                text,
            ),
            Hint::Two(act, text) => {
                labelled(self.labels(stack, act).into_iter().take(2).collect(), text)
            }
            Hint::All(act, text) => labelled(self.labels(stack, act), text),
            Hint::Many(acts, text) => {
                let chords: Vec<KeyChord> = acts
                    .iter()
                    .filter_map(|&act| self.first_chord(stack, act))
                    .collect();
                (!chords.is_empty()).then(|| with_text(&chord_list(&chords), text))
            }
            Hint::Text(text) => (!text.is_empty()).then(|| text.to_owned()),
        };
        spec.iter()
            .filter_map(element)
            .collect::<Vec<_>>()
            .join(SEP)
    }

    /// The status line (D7, MOD-67 M3 PA-9) from `stack`'s global layers: quit first, as its
    /// first admitted chord (left out when the stack admits none); then every other `Global` row
    /// in catalogue order that is `offered` and has an admitted chord, each through the
    /// **first** global layer that offers it (`Stack::admits`), its first admitted chord, rows
    /// sharing a help
    /// label collapsed to the first (the digits), joined by ` · `. `Stack::BASE` gives the M1
    /// line; a stack with one global layer renders exactly as before (MOD-67 M4 PA-2). Nothing
    /// draws it since MOD-115 (the header's chips and the `?` box carry the same keys), so it is
    /// compiled only for tests and the `testkit` feature.
    #[cfg(any(test, feature = "testkit"))]
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
            .and_then(first_chord);
        let quit_help = Act::Quit.spec().map_or("quit", |spec| spec.help);
        let mut shown: Vec<&str> = vec![quit_help];
        let mut entries: Vec<String> = quit
            .map(|quit| format!("{} {quit_help}", quit.label()))
            .into_iter()
            .collect();
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
    /// a row for it (the `actions` shadowing rule, L-B Q6), with every chord that layer admits
    /// but those a listed narrower act always takes ([`SHADOWING`], MOD-115 M2 T5R) and those a
    /// version-1 file's kept row steps aside from there (`stepped_aside`, MOD-115 M2 RV4);
    /// rows sharing a help label merge, and a row with no admitted chord drops. `offered` is
    /// consulted for `Global` and `Layout` rows only (the shell's: a layout key acts only on a
    /// tab with a list and a detail, MOD-115 M2 D2). Every global layer of the stack renders as **one**
    /// `Global:` line at the last global layer's place (MOD-67 M4 PA-2): quit first (its admitted
    /// chords, none when no global layer admits `Quit`), then the rows
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
                if matches!(row.context, Context::Global | Context::Layout)
                    && row.act != Act::Quit
                    && !offered(row.act)
                {
                    continue;
                }
                let chords: Vec<KeyChord> = row
                    .chords
                    .iter()
                    .copied()
                    .filter(|chord| {
                        layer.admits_chord(*chord)
                            && !self.shadowed(stack, *chord, row.act)
                            && !self.stepped_aside(stack, *chord, row.act)
                    })
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

    /// Whether `chord` never reaches `act` in `stack` and the box lists the act that takes it
    /// (MOD-115 M2 T5R): a narrower candidate ahead of `act` forms a [`SHADOWING`] pair with it
    /// (the narrower act always wins). Over the Requirements form `l` always sets the priority
    /// later, so the box lists it under that row only, not as the choice's next option. A text
    /// field's act never hides a wider one (MOD-115 M2 T6): the widget passes what it does not
    /// use, so the concepts search's `Esc` is the field's cancel and still `overlay.close`, and
    /// the box lists it under both.
    fn shadowed(&self, stack: Stack<'_>, chord: KeyChord, act: Act) -> bool {
        self.actions(stack, chord)
            .into_iter()
            .take_while(|&candidate| candidate != act)
            .any(|narrower| {
                SHADOWING.contains(&(narrower, act))
                    && narrower
                        .spec()
                        .is_none_or(|spec| spec.context != Context::Field)
            })
    }

    /// Whether `act`, a kept row of a version-1 file, steps aside from `chord` in `stack`
    /// (MOD-115 M2 RV4): [`Keys::actions`] drops it there for an older entry that added the
    /// chord (`validate::v1_steps_aside`), so the box lists the chord under that entry only.
    /// `[confirm] yes = ["space"]` puts `Space` under `Confirm` on the Runs close-out warning,
    /// not also under `Choice` as the toggle.
    fn stepped_aside(&self, stack: Stack<'_>, chord: KeyChord, act: Act) -> bool {
        self.version == 1
            && super::validate::steps_aside(act)
            && !self.actions(stack, chord).contains(&act)
    }

    /// One `?` box line for `context` (D8): every offered and bound action, all its chords
    /// joined by `/`, rows sharing a help label merged into one entry. `Quit` always appears while
    /// it is bound. `None` if the context offers nothing.
    #[must_use]
    pub fn help_line(&self, context: Context, offered: impl Fn(Act) -> bool) -> Option<HelpLine> {
        let mut merged: Vec<(&str, Vec<KeyChord>)> = Vec::new();
        for row in self.rows.iter().filter(|row| row.context == context) {
            let quit = row.act == Act::Quit;
            if !quit && !offered(row.act) {
                continue;
            }
            let chords = row.chords.clone();
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
    let plain: Option<Vec<char>> = chords.iter().map(KeyChord::plain_char).collect();
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

    // MOD-115 M2: the pane keys close the line (the shell offers them from Task 6 on).
    const BARE: &str = "q quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1 select tab · ? help \
                        · F9 about · Tab next pane · Shift+Tab previous pane";
    const FULL: &str = "q quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1 select tab · ? help \
                        · F9 about · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue \
                        · Tab next pane · Shift+Tab previous pane";

    /// What the shell offers without `register_all`: every act but the four that name a view.
    fn bare(act: Act) -> bool {
        !matches!(act, Act::Workspaces | Act::Find | Act::Waiting | Act::Queue)
    }

    fn all(_: Act) -> bool {
        true
    }

    /// MOD-115 M2 T6: a text field's editing keys, the `?` box line of every capturing stack.
    const FIELD_LINE: &str = "Text field: Enter submit · Esc cancel · Left cursor left · Right cursor \
                              right · Home line start · End line end · Backspace delete back · Del \
                              delete forward · Up line up · Down line down · PgUp page up · PgDn \
                              page down";

    /// MOD-115 M2 T6: the history keys, the `?` box line of every tab browse stack.
    const HISTORY_LINE: &str = "History: Backspace/Alt+Left back · Alt+Right forward";

    fn global_line(keys: &Keys, offered: fn(Act) -> bool) -> String {
        keys.help_line(Context::Global, offered)
            .expect("the global context offers something")
            .text()
    }

    /// T5 repair: `Many` lists each act's first admitted chord, merged like a `?` box entry, so
    /// the nine answers read `1-9` until one moves; an unbound act drops out, and so does the
    /// element when every act is unbound.
    #[test]
    fn many_merges_the_first_chord_of_each_act() {
        const ANSWERS: &[Act] = &[Act::AnswerOption1, Act::AnswerOption2, Act::AnswerOption3];
        let spec = &[Hint::Many(ANSWERS, "answer"), Hint::Text("end")];
        assert_eq!(
            Keys::compiled().hint(views::CHAT_LIVE, spec),
            "1-3 answer · end"
        );
        let moved = Keys::defaults().with_chords(Context::Answer, Act::AnswerOption2, &[]);
        assert_eq!(moved.hint(views::CHAT_LIVE, spec), "1/3 answer · end");
        let none = Keys::defaults()
            .with_chords(Context::Answer, Act::AnswerOption1, &[])
            .with_chords(Context::Answer, Act::AnswerOption2, &[])
            .with_chords(Context::Answer, Act::AnswerOption3, &[]);
        assert_eq!(none.hint(views::CHAT_LIVE, spec), "end");
    }

    /// MOD-115 M2 RV1 H2: `Glyphs` is `Pair` with the header chips' labels, so a choice's arrows
    /// read `←/→` as the literal hints did, and a rebound side shows its own chord; one side
    /// unbound renders the other alone, both unbound drop the element.
    #[test]
    fn glyphs_pair_the_first_chords_as_the_chips_write_them() {
        let spec = &[
            Hint::Glyphs(Act::ChoicePrev, Act::ChoiceNext, "choose"),
            Hint::Text("end"),
        ];
        assert_eq!(
            Keys::compiled().hint(views::ITEM_FORM, spec),
            "\u{2190}/\u{2192} choose · end"
        );
        let moved = Keys::defaults().with_chords(Context::Choice, Act::ChoiceNext, &["ctrl-n"]);
        assert_eq!(
            moved.hint(views::ITEM_FORM, spec),
            "\u{2190}/Ctrl+n choose · end"
        );
        let one = Keys::defaults().with_chords(Context::Choice, Act::ChoicePrev, &[]);
        assert_eq!(one.hint(views::ITEM_FORM, spec), "\u{2192} choose · end");
        let none = one.with_chords(Context::Choice, Act::ChoiceNext, &[]);
        assert_eq!(none.hint(views::ITEM_FORM, spec), "end");
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
        // MOD-115 M2: `ctrl-c` is quit's own chord, so an unbound quit drops out like any act
        // (a key file cannot unbind it: the safety net refuses that).
        assert!(
            keys.status_line(Stack::BASE, all)
                .starts_with("Ctrl+PgDn next tab · ")
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
            "Global: q/Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1-9 select tab \
             · ?/F1 help · F9 about · Tab/Ctrl+Right next pane · Shift+Tab/Ctrl+Left previous pane"
        );
        assert_eq!(
            global_line(keys, all),
            "Global: q/Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1-9 select tab \
             · ?/F1 help · F9 about · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue \
             · Tab/Ctrl+Right next pane · Shift+Tab/Ctrl+Left previous pane"
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

    /// MOD-115 M2 (MD2): quit lists the chords in force and nothing appended: `ctrl-c` only
    /// while it is bound.
    #[test]
    fn quit_lists_only_its_own_chords() {
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &["ctrl-c"]);
        assert!(global_line(&keys, all).starts_with("Global: Ctrl+c quit · "));
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &["ctrl-x"]);
        assert!(global_line(&keys, all).starts_with("Global: Ctrl+x quit · "));
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &[]);
        assert!(global_line(&keys, all).starts_with("Global: Ctrl+PgDn next tab · "));
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
                "Global: q/Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1-9 select tab",
                "  ?/F1 help · F9 about · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue",
                "  Tab/Ctrl+Right next pane · Shift+Tab/Ctrl+Left previous pane",
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
        // MOD-115 M2: `ctrl-c` is quit's own chord now, and it passes the modal filter.
        assert_eq!(keys.hint(views::CAPTURE, spec), "F1 help · Ctrl+c quit");
        assert_eq!(keys.hint(Stack::BASE, spec), "?/F1 help · q/Ctrl+c quit");
        assert_eq!(keys.labels(views::CAPTURE, Act::Help), ["F1".to_owned()]);
        assert_eq!(keys.label(views::CAPTURE, Act::Help).as_deref(), Some("F1"));
    }

    /// MOD-67 M4 R1 L2: `Two` shows a toggle's first two admitted chords (`h/l view`), every
    /// one when it has fewer, and follows a rebind.
    #[test]
    fn two_lists_the_first_two_admitted_chords() {
        let layers = [Layer::all(Context::Skills)];
        let stack = Stack::new(&layers);
        let spec = &[Hint::Two(Act::SkillsSwitchView, "view")];
        assert_eq!(Keys::compiled().hint(stack, spec), "h/l view");
        let keys = Keys::defaults().with_chords(Context::Skills, Act::SkillsSwitchView, &["v"]);
        assert_eq!(keys.hint(stack, spec), "v view");
        let keys = Keys::defaults().with_chords(
            Context::Skills,
            Act::SkillsSwitchView,
            &["right", "left", "v"],
        );
        assert_eq!(keys.hint(stack, spec), "Right/Left view");
        let keys = Keys::defaults().with_chords(Context::Skills, Act::SkillsSwitchView, &[]);
        assert_eq!(keys.hint(stack, spec), "");
        let modal = [Layer::modal(Context::Skills)];
        let keys = Keys::defaults().with_chords(
            Context::Skills,
            Act::SkillsSwitchView,
            &["h", "f2", "l", "f3"],
        );
        assert_eq!(keys.hint(Stack::new(&modal), spec), "F2/F3 view");
    }

    #[test]
    fn the_status_line_follows_the_stacks_global_layer() {
        let keys = Keys::compiled();
        assert_eq!(keys.status_line(views::AGENTS_BROWSE, bare), BARE);
        assert_eq!(keys.status_line(views::AGENTS_BROWSE, all), FULL);
        let modal_all =
            "Ctrl+c quit · F1 help · F9 about · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue";
        for stack in [views::AGENTS_FORM, views::CAPTURE, views::KINDS_CONFIRM] {
            assert_eq!(keys.status_line(stack, all), modal_all);
            assert_eq!(
                keys.status_line(stack, bare),
                "Ctrl+c quit · F1 help · F9 about"
            );
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
                "Settings: l/]/Shift+Right next section · h/[/Shift+Left previous section",
                "Common: r reload · Esc dismiss",
                "List: j/Down down · k/Up up",
                HISTORY_LINE,
                "Global: q/Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1-9 select tab \
                 · ?/F1 help · F9 about · Tab/Ctrl+Right next pane · Shift+Tab/Ctrl+Left previous pane",
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
                // T5R: `Esc` is always `confirm.no` here (SHADOWING), listed there only.
                "Schema: y/Y yes · n/Esc/N no",
                "Global: Ctrl+c quit · ?/F1 help",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::AGENTS_FORM, bare)),
            [
                "Agents: Tab/Down next field · Shift+Tab/Up previous field",
                FIELD_LINE,
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::CAPTURE, all)),
            [
                FIELD_LINE,
                "Global: Ctrl+c quit · F1 help · F9 about · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue"
            ]
        );
        // T6: the field's `Esc` is the concepts search's close too (the field passes cancel), so
        // the box lists it under both; the hit cursor's `up`/`down` hide the field's (SHADOWING).
        assert_eq!(
            texts(keys.help_lines(views::CONCEPTS_QUERY, all)),
            [
                "Search concepts: Ctrl+d decisions only · Ctrl+p cycle project scope · Ctrl+r \
                 re-index scope · Up previous hit · Down next hit",
                "Text field: Enter submit · Esc cancel · Left cursor left · Right cursor right · \
                 Home line start · End line end · Backspace delete back · Del delete forward · \
                 PgUp page up · PgDn page down",
                "Overlay: Esc close",
                "Global: Ctrl+c quit · F1 help",
            ]
        );
    }

    /// MOD-115 M2: every layer is wired from Task 6 on, so the box lists `field`, `history` and
    /// (where the shell offers them: a tab with a list and a detail) `layout` too. The whole box
    /// of each stack is pinned, so a line no site acts on cannot slip in (T0 verify: the headings
    /// alone let the Requirements form's priority keys show before they were wired).
    #[test]
    fn the_box_lists_every_layer_and_the_layout_rows_only_where_offered() {
        let keys = Keys::compiled();
        let headings = |stack, offered: fn(Act) -> bool| -> Vec<String> {
            keys.help_lines(stack, offered)
                .into_iter()
                .map(|line| line.heading)
                .collect()
        };
        fn no_layout(act: Act) -> bool {
            act.spec()
                .is_none_or(|spec| spec.context != Context::Layout)
        }
        for stack in [views::BACKLOG_BODY, views::RUNS_BROWSE] {
            assert!(headings(stack, all).contains(&"Layout".to_owned()));
            assert!(!headings(stack, no_layout).contains(&"Layout".to_owned()));
            assert!(headings(stack, no_layout).contains(&"History".to_owned()));
        }
        assert!(headings(views::CHAT_LIVE, all).contains(&"History".to_owned()));
        assert!(!headings(views::CHAT_LIVE, all).contains(&"Layout".to_owned()));
        for stack in [views::ITEM_FORM, views::CAPTURE, views::REQUIREMENTS_FORM] {
            let heads = headings(stack, all);
            assert!(heads.contains(&"Text field".to_owned()), "{heads:?}");
            assert!(!heads.contains(&"History".to_owned()), "{heads:?}");
        }
        assert_eq!(
            texts(keys.help_lines(views::BACKLOG_BODY, all))
                .into_iter()
                .find(|line| line.starts_with("Layout")),
            Some(
                "Layout: p next layout · > wider list · < narrower list · }/Ctrl+Down larger \
                 activity · {/Ctrl+Up smaller activity · F2/Alt+1 show/hide list · F3/Alt+2 \
                 show/hide activity · z zoom"
                    .to_owned()
            )
        );
        assert_eq!(
            texts(keys.help_lines(views::REQUIREMENTS_FORM, bare)),
            [
                "Requirements: m priority must · l priority later · Tab/Down next field \
                 · Shift+Tab/Up previous field",
                FIELD_LINE,
                // T5R: on the priority `l` always sets later (SHADOWING), so it is not listed as
                // the next option.
                "Choice: Left/h previous option · Right next option · Space toggle",
                "Form: Ctrl+s save",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::RUNS_CONFIRM, bare)),
            [
                "Choice: Left/h previous option · Right/l next option · Space toggle",
                "Confirm: y yes · n/Esc no",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        // T5 repair: the cancel question answers only yes or no, the area form has no priority
        // and no choice, and the item form's pickers only move, so none lists what it ignores.
        assert_eq!(
            texts(keys.help_lines(views::RUNS_CANCEL, bare)),
            [
                "Confirm: y yes · n/Esc no",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::AREA_FORM, bare)),
            [
                "Requirements: Tab/Down next field · Shift+Tab/Up previous field",
                FIELD_LINE,
                "Form: Ctrl+s save",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::ITEM_FORM, bare)),
            [
                "Item: Tab/Down next field · Shift+Tab/Up previous field",
                FIELD_LINE,
                "Choice: Left/h previous option · Right/l next option",
                "Form: Ctrl+s save · Ctrl+e $EDITOR",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::ATTACH_FORM, bare)),
            [
                "Attachments: Ctrl+r repo picker · Tab/Down next field · Shift+Tab/Up previous \
                 field",
                FIELD_LINE,
                "Choice: Space toggle",
                "Form: Ctrl+s save",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::CHAT_LIVE, bare)),
            [
                "Chat: i/Enter compose · a next agent · Esc end chat (twice) · t thoughts · g top \
                 · G bottom",
                "Answer: 1-9 answer",
                "List: j/Down down · k/Up up",
                HISTORY_LINE,
                "Global: q/Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1-9 select tab \
                 · ?/F1 help · F9 about · Tab/Ctrl+Right next pane · Shift+Tab/Ctrl+Left previous pane",
            ]
        );
        assert_eq!(
            texts(keys.help_lines(views::CAPTURE, bare)),
            [FIELD_LINE, "Global: Ctrl+c quit · F1 help · F9 about"]
        );
    }

    /// MOD-115 M2 RV4 (MD4, RV1 repair 1): the box lists what acts, as `Keys::actions` resolves
    /// it. Under a version-1 `[confirm] yes = ["space"]` the kept `choice.toggle` steps aside on
    /// the Runs close-out warning (`validate::v1_steps_aside`), so `Space` answers yes there and
    /// the box does not list it as the toggle; where the row keeps `Space` (the filter form, the
    /// attachment form) the box still lists `Space toggle`.
    #[test]
    fn the_box_never_lists_a_row_that_steps_aside_under_a_version_1_file() {
        let keys =
            crate::keys::load_str("version = 1\n[confirm]\nyes = [\"space\"]\n").expect("loads");
        assert_eq!(
            texts(keys.help_lines(views::RUNS_CONFIRM, bare)),
            [
                "Choice: Left/h previous option · Right/l next option",
                "Confirm: Space yes · n/Esc no",
                "Global: Ctrl+c quit · F1 help · F9 about",
            ]
        );
        for stack in [views::BACKLOG_FILTER, views::ATTACH_FORM] {
            let lines = texts(keys.help_lines(stack, bare));
            assert!(
                lines
                    .iter()
                    .any(|line| line.starts_with("Choice:") && line.contains("Space toggle")),
                "{lines:?}"
            );
        }
    }

    /// MOD-67 M4 PA-2: every global layer of a stack feeds the status line and one `Global:`
    /// line of the `?` box; the closer follows the last (modal) one.
    #[test]
    fn a_tabs_layer_shows_on_the_status_line_and_in_one_global_line() {
        use views::TEMPLATES_EDITOR;
        let keys = Keys::compiled();
        let tabs = "Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · F1 help · F9 about";
        assert_eq!(
            keys.status_line(TEMPLATES_EDITOR, all),
            format!("{tabs} · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue")
        );
        assert_eq!(keys.status_line(TEMPLATES_EDITOR, bare), tabs);
        assert_eq!(
            texts(keys.help_lines(TEMPLATES_EDITOR, all)),
            [
                FIELD_LINE.to_owned(),
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

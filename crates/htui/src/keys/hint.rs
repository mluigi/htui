//! Hint rows, the status line and the `?` box, generated from the active stack (ANA-26 §7.6).
//!
//! Every label comes from the keys in force, so a rebound action shows its new chord (MOD-67 D7,
//! D8). A hint row and the status line take the first chord of each action; a `?` box line lists
//! all of them. Rows sharing a help label (the nine `select_tab_*`) collapse into one entry, and
//! an unbound action drops out, except `global.quit`, which always ends with the fixed `Ctrl+c`.

use super::{Act, Context, Keys, Stack};

/// One element of a hint spec (ANA-26 §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// `"{label} {text}"`, e.g. `e edit DSN`.
    One(Act, &'static str),
    /// `"{label}/{label} {text}"`, e.g. `j/k rows`; one side unbound renders the other alone.
    Pair(Act, Act, &'static str),
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
        todo!()
    }

    /// The line packed into rows at most `width` cells wide (`unicode-width`), breaking only
    /// between entries. Continuation rows start with two spaces. A single entry wider than
    /// `width` gets a row of its own (and is clipped when drawn).
    #[must_use]
    pub fn rows(&self, width: usize) -> Vec<String> {
        todo!("{width}")
    }
}

impl Keys {
    /// The first chord of `act` through `stack`, as `KeyChord::label` writes it; `None` when it
    /// is unbound. Prose such as `format!("press {}", …)` uses it from M3 on.
    #[must_use]
    pub fn label(&self, stack: Stack<'_>, act: Act) -> Option<String> {
        todo!("{stack:?} {act:?}")
    }

    /// A view's hint row: each element through `stack`, first chord only, unbound dropped,
    /// joined by ` · ` (ANA-26 §7.6).
    #[must_use]
    pub fn hint(&self, stack: Stack<'_>, spec: HintSpec) -> String {
        todo!("{stack:?} {spec:?}")
    }

    /// The status line (D7): the `Global` rows that are `offered` and bound, in catalogue order,
    /// the first chord of each, rows sharing a help label collapsed to the first (the digits),
    /// joined by ` · `.
    #[must_use]
    pub fn status_line(&self, offered: impl Fn(Act) -> bool) -> String {
        let _ = offered;
        todo!()
    }

    /// One `?` box line for `context` (D8): every offered and bound action, all its chords
    /// joined by `/`, rows sharing a help label merged into one entry. `Quit` always appears and
    /// ends with the fixed `Ctrl+c`. `None` if the context offers nothing.
    #[must_use]
    pub fn help_line(&self, context: Context, offered: impl Fn(Act) -> bool) -> Option<HelpLine> {
        let _ = offered;
        todo!("{context:?}")
    }

    /// The box's last line, from `global.help`'s chords: `"?/F1 closes this box"`; `None` if
    /// help is unbound.
    #[must_use]
    pub fn help_closer(&self) -> Option<String> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::{HelpLine, Hint};
    use crate::keys::{Act, Context, Keys, Layer, Stack};

    const BARE: &str = "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help";
    const FULL: &str = "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help \
                        · w workspaces · Ctrl+f find · Ctrl+w waiting";

    /// What the shell offers without `register_all`: every act but the three that name a view.
    fn bare(act: Act) -> bool {
        !matches!(act, Act::Workspaces | Act::Find | Act::Waiting)
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
        assert_eq!(Keys::compiled().status_line(bare), BARE);
    }

    #[test]
    fn the_status_line_with_every_global_offered() {
        assert_eq!(Keys::compiled().status_line(all), FULL);
    }

    #[test]
    fn an_unbound_action_drops_out_of_the_status_line_and_a_hint() {
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &[]);
        assert!(keys.status_line(all).starts_with("Tab next tab"));
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
             · ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting"
        );
        assert_eq!(
            keys.help_line(Context::Overlay, all)
                .map(|line| line.text())
                .as_deref(),
            Some("Overlay: Esc close")
        );
        assert_eq!(keys.help_closer().as_deref(), Some("?/F1 closes this box"));
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
                "  ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting",
            ]
        );
    }
}

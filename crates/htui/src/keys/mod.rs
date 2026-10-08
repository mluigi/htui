//! Named key actions (MOD-67, `docs/ANA-26.md` §7): the compiled-in catalogue, the strict chord
//! parser, context stacks and the resolver that turns a chord into ordered candidate actions, and
//! the hints and help generated from them. M2 adds the key file: [`load`] reads `keys.toml` over
//! the catalogue. M3 adds the Settings and overlay contexts, their stacks in [`views`], view
//! layers that inherit shared verbs, modal layers that filter chords, the derived view defaults
//! (`VIEW_DEFAULTS`) and a view table's overrides of shared verbs.
//!
//! D6's dispatch order lives in `App::on_key`. This module only answers "which actions does this
//! chord name in this stack" and "how is this action labelled".

pub mod catalogue;
pub mod chord;
pub mod hint;
pub mod load;
pub mod print;
pub mod stack;
pub mod validate;
pub mod views;

pub use catalogue::{Act, ActionSpec, CATALOGUE, Context, SHADOWING, STATE_GUARDED, VIEW_DEFAULTS};
pub use chord::{CTRL_C, ChordError, KeyChord};
pub use hint::{HelpLine, Hint, HintSpec};
pub use load::{FILE_NAME, KeyFileError, KeysError, load_path, load_str, resolve};
pub use print::print;
pub use stack::{DECLARED, Layer, Stack};
pub use validate::validate;

/// The keys in force: every catalogue action's chords per context (MOD-67 D9, D10). Built once
/// from the compiled defaults ([`Keys::compiled`]); M2 builds one from `keys.toml` instead
/// ([`load_str`]). Equality compares bindings only, not the file lines that set them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys {
    /// One row per `(context, act)`, in catalogue order. A row with no chords is an unbound
    /// action. It still shadows the same act in wider layers (M2's narrower overrides).
    rows: Vec<Row>,
}

/// One binding row (private: no `pub` doc may link it). Equality ignores `line` (MOD-67 M2
/// PA-3): two tables with the same bindings are equal whatever file set them, so a provenance
/// check goes through [`Keys::line`] or `--print-keys`.
#[derive(Debug, Clone)]
struct Row {
    context: Context,
    act: Act,
    help: &'static str,
    chords: Vec<KeyChord>,
    /// The `keys.toml` line of the entry that set `chords`, or `None` while they equal the
    /// catalogue default (PA-3). Validation reports on it; `--print-keys` marks it `(changed)`.
    line: Option<usize>,
    /// `Some` for a `VIEW_DEFAULTS` row (MOD-67 M3 PA-1): its extra chords, added to whatever
    /// the shared row resolves to whenever the row is derived ([`Keys::derive`]).
    extra: Option<Vec<KeyChord>>,
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context
            && self.act == other.act
            && self.help == other.help
            && self.chords == other.chords
    }
}

impl Eq for Row {}

static COMPILED: std::sync::LazyLock<Keys> = std::sync::LazyLock::new(Keys::defaults);

impl Keys {
    /// The compiled-in defaults, parsed once per process. `Ctx::new` hands these to every view
    /// that is not given `App`'s own (D9).
    #[must_use]
    pub fn compiled() -> &'static Self {
        &COMPILED
    }

    /// A fresh table of the catalogue defaults, each parsed with [`KeyChord::parse_strict`],
    /// then one derived row per [`VIEW_DEFAULTS`] entry (MOD-67 M3 PA-1): the shared row's
    /// chords followed by the view's extra ones.
    ///
    /// # Panics
    /// If a default does not parse. `every_catalogue_default_parses_strictly` and
    /// `every_view_default_is_an_extra_chord_some_stack_offers` pin that none fails.
    #[must_use]
    pub fn defaults() -> Self {
        let parse = |written: &str, context: Context, name: &str| {
            KeyChord::parse_strict(written).unwrap_or_else(|err| {
                panic!(
                    "catalogue default {written:?} of [{}] {name}: {err}",
                    context.table()
                )
            })
        };
        let catalogue = CATALOGUE.iter().map(|spec| Row {
            context: spec.context,
            act: spec.act,
            help: spec.help,
            chords: spec
                .defaults
                .iter()
                .map(|written| parse(written, spec.context, spec.name))
                .collect(),
            line: None,
            extra: None,
        });
        let derived = VIEW_DEFAULTS.iter().map(|&(context, act, extra)| {
            let name = act.spec().map_or("", |spec| spec.name);
            Row {
                context,
                act,
                help: act.spec().map_or("", |spec| spec.help),
                chords: Vec::new(),
                line: None,
                extra: Some(
                    extra
                        .iter()
                        .map(|written| parse(written, context, name))
                        .collect(),
                ),
            }
        });
        let mut keys = Self {
            rows: catalogue.chain(derived).collect(),
        };
        keys.derive(&[]);
        keys
    }

    /// Re-derives every `VIEW_DEFAULTS` row from the shared row in force (MOD-67 M3 PA-1): the
    /// shared row's chords, then each extra chord not already among them. A row the key file set
    /// (`set_by_file`) keeps its chords, and loses its line when they equal the derivation, so
    /// a printed file reads back unmarked.
    fn derive(&mut self, set_by_file: &[(Context, Act)]) {
        for index in 0..self.rows.len() {
            let Some(extra) = self.rows[index].extra.as_deref() else {
                continue;
            };
            let act = self.rows[index].act;
            let mut derived: Vec<KeyChord> = act
                .spec()
                .map(|spec| self.chords(spec.context, act).to_vec())
                .unwrap_or_default();
            for &chord in extra {
                if !derived.contains(&chord) {
                    derived.push(chord);
                }
            }
            let row = &mut self.rows[index];
            if set_by_file.contains(&(row.context, row.act)) {
                if row.chords == derived {
                    row.line = None;
                }
            } else {
                row.chords = derived;
            }
        }
    }

    /// The chords of `act` in exactly `context`, or `&[]` if there is no row or it is unbound.
    #[must_use]
    pub fn chords(&self, context: Context, act: Act) -> &[KeyChord] {
        self.row(context, act).map_or(&[], |row| &row.chords)
    }

    /// Test-only override, the M2 merge in miniature: replaces the `(context, act)` row's chords,
    /// or appends a row if none exists (a narrower override). `specs` parse strictly.
    #[cfg(test)]
    pub(crate) fn with_chords(mut self, context: Context, act: Act, specs: &[&str]) -> Self {
        let chords = specs
            .iter()
            .map(|spec| KeyChord::parse_strict(spec).expect("a test override parses"))
            .collect();
        match self
            .rows
            .iter_mut()
            .find(|row| row.context == context && row.act == act)
        {
            Some(row) => row.chords = chords,
            None => self.rows.push(Row {
                context,
                act,
                help: act.spec().map_or("", |spec| spec.help),
                chords,
                line: None,
                extra: None,
            }),
        }
        self
    }

    /// The `keys.toml` line that set `act`'s chords in exactly `context`, or `None` while they
    /// are the catalogue default (MOD-67 M2 D7). `--print-keys` marks such a row `(changed)`.
    #[must_use]
    pub fn line(&self, context: Context, act: Act) -> Option<usize> {
        self.row(context, act).and_then(|row| row.line)
    }

    /// The loader's merge (MOD-67 M2 D7): replaces the `(context, act)` row's chords and keeps
    /// `line` only when they differ from [`Keys::compiled`]'s for that row (PA-3). A missing row
    /// is a view's override of a shared verb (MOD-67 M3 D10): it is appended, and always keeps
    /// its line, since the compiled keys have no such row.
    fn set(&mut self, context: Context, act: Act, chords: Vec<KeyChord>, line: usize) {
        let changed = Self::compiled()
            .row(context, act)
            .is_none_or(|compiled| compiled.chords != chords);
        let line = changed.then_some(line);
        match self
            .rows
            .iter_mut()
            .find(|row| row.context == context && row.act == act)
        {
            Some(row) => {
                row.chords = chords;
                row.line = line;
            }
            None => self.rows.push(Row {
                context,
                act,
                help: act.spec().map_or("", |spec| spec.help),
                chords,
                line,
                extra: None,
            }),
        }
    }

    /// The row of `act` in exactly `context`.
    fn row(&self, context: Context, act: Act) -> Option<&Row> {
        self.rows
            .iter()
            .find(|row| row.context == context && row.act == act)
    }

    /// The row of `act` as the stack sees it, with its layer: in the first layer that admits
    /// `act` ([`Stack::admits`], inheritance included) and has a row for it.
    fn resolve_row(&self, stack: Stack<'_>, act: Act) -> Option<(Layer, &Row)> {
        stack
            .layers()
            .iter()
            .enumerate()
            .filter(|&(index, _)| stack.admits(index, act))
            .find_map(|(_, &layer)| self.row(layer.context(), act).map(|row| (layer, row)))
    }
}

/// `text` as a TOML basic string: wrapped in `"`, with `\` and `"` escaped and every control
/// character as `\uXXXX`. The error report and `--print-keys` both spell chords with it.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{Act, CATALOGUE, CTRL_C, Context, KeyChord, Keys, STATE_GUARDED, VIEW_DEFAULTS};

    /// Every default of `row`, parsed; the message names the row as a key file would.
    fn parsed(row: &super::ActionSpec) -> Vec<KeyChord> {
        row.defaults
            .iter()
            .map(|spec| {
                KeyChord::parse_strict(spec).unwrap_or_else(|err| {
                    panic!("[{}] {} = {spec:?}: {err}", row.context.table(), row.name)
                })
            })
            .collect()
    }

    #[test]
    fn every_catalogue_default_parses_strictly() {
        for row in CATALOGUE {
            for spec in row.defaults {
                assert!(
                    KeyChord::parse_strict(spec).is_ok(),
                    "[{}] {} = {spec:?}",
                    row.context.table(),
                    row.name
                );
            }
        }
    }

    #[test]
    fn no_two_actions_in_one_context_share_a_default_chord() {
        let guarded = |a, b| {
            STATE_GUARDED
                .iter()
                .any(|&pair| pair == (a, b) || pair == (b, a))
        };
        for (i, first) in CATALOGUE.iter().enumerate() {
            let mine = parsed(first);
            for second in &CATALOGUE[i + 1..] {
                if first.context != second.context || guarded(first.act, second.act) {
                    continue;
                }
                for chord in parsed(second) {
                    assert!(
                        !mine.contains(&chord),
                        "{:?} and {:?} share {}",
                        first.act,
                        second.act,
                        chord.label()
                    );
                }
            }
        }
    }

    #[test]
    fn no_in_capture_action_has_a_printable_chord() {
        for row in CATALOGUE.iter().filter(|row| row.in_capture) {
            for chord in parsed(row) {
                assert!(
                    !chord.is_printable(),
                    "{:?} binds {}",
                    row.act,
                    chord.label()
                );
            }
        }
    }

    #[test]
    fn the_compiled_keys_are_the_catalogue_and_never_ctrl_c() {
        let keys = Keys::compiled();
        for row in CATALOGUE {
            let chords = keys.chords(row.context, row.act);
            assert_eq!(chords, parsed(row), "{:?}", row.act);
            assert!(!chords.contains(&CTRL_C), "{:?} binds ctrl-c", row.act);
        }
        for row in &keys.rows {
            assert!(
                !row.chords.contains(&CTRL_C),
                "[{}] {:?} binds ctrl-c",
                row.context.table(),
                row.act
            );
        }
        assert_eq!(keys.rows.len(), CATALOGUE.len() + VIEW_DEFAULTS.len());
    }

    fn chords(specs: &[&str]) -> Vec<KeyChord> {
        specs
            .iter()
            .map(|spec| KeyChord::parse_strict(spec).expect("a valid spec"))
            .collect()
    }

    #[test]
    fn the_compiled_view_defaults_are_derived() {
        let keys = Keys::compiled();
        assert_eq!(
            keys.chords(Context::SettingsAgents, Act::FormNextField),
            chords(&["tab", "down"])
        );
        assert_eq!(
            keys.chords(Context::SettingsPersonas, Act::Back),
            chords(&["esc", "enter"])
        );
        assert_eq!(
            keys.chords(Context::Migration, Act::ConfirmNo),
            chords(&["n", "esc", "N"])
        );
        for &(context, act, _) in VIEW_DEFAULTS {
            assert_eq!(keys.line(context, act), None, "{context:?} {act:?}");
        }
    }

    #[test]
    fn a_derived_row_follows_the_shared_row_and_drops_a_duplicate_extra() {
        let mut keys = Keys::defaults().with_chords(Context::Form, Act::FormNextField, &["down"]);
        keys.derive(&[]);
        assert_eq!(
            keys.chords(Context::SettingsSecrets, Act::FormNextField),
            chords(&["down"])
        );
    }

    #[test]
    fn compiled_is_built_once() {
        assert!(std::ptr::eq(Keys::compiled(), Keys::compiled()));
        assert_eq!(*Keys::compiled(), Keys::defaults());
    }
}

//! The key file's semantic checks over the merged keys (MOD-67 M2 D8 steps 5-6; ANA-26 §7.4
//! step 7): collisions in every context and in every declared stack, printable chords on
//! actions offered while a field captures, and (MOD-115 M2 D7) the safety nets: the rows without
//! which htui cannot be left or a mode cannot be closed keep a chord. Before them, `give_way`
//! lets an entry of the file take its chord from an action the file leaves at its default
//! (MOD-12 M3 R1 H1).
//!
//! MOD-67 M3: the per-context pass skips view contexts (PA-4: a view context holds rows of modes
//! that never meet; the stack pass checks every declared stack), the stack pass allows a
//! reviewed [`SHADOWING`] pair (D11, PA-2), and a derived `VIEW_DEFAULTS` row the file did not
//! set is reported as the shared row it follows, on the user's line.
//!
//! MOD-57 M2 (E9): `editor.focus` never gives way, and no other action may share its chord in any
//! context: while an edit is open it acts ahead of every view. `editor.abort` left at its default
//! gives way to a global entry too (M2 R1 M-1): a file that binds `ctrl-x` in `[global]` loads.
//!
//! MOD-115 M2 D7: no key is fixed ahead of the resolver any more (`FIXED` and its pass are gone):
//! a choice's value keys, a form's `Enter`/`Esc` and the permission digits are rows, checked by
//! the passes above like every other.

use super::load::must_keep;
use super::stack::WHILE_EDITING;
use super::views::M2_LAYERS;
use super::{
    Act, CATALOGUE, Context, DECLARED, ESC, KeyChord, KeyFileError, Keys, Lost, Row, SHADOWING,
    STATE_GUARDED, Stack, V1_DEFAULTS, quote,
};

/// The user's binding wins (MOD-12 M3 R1 H1, maintainer 2026-10-08): an entry of the file that
/// shares a chord with an action in **its own context** that the file leaves at its default
/// takes the chord, and that action loses it (unbound when it has no other). A
/// [`STATE_GUARDED`] pair that shares the chord by default keeps sharing it, and
/// `overlay.close` and `editor.focus` never give way (each must keep a chord; MOD-67 M4 D8.1).
/// The other rows that must keep a chord (`global.quit`, `leave.confirm`, `leave.stay`,
/// `field.cancel`; MOD-115 M2 D-10) give one away only while they keep another, so
/// `[global] help = ["q"]` loads with a notice and quit keeps `ctrl-c`.
///
/// One default gives way across contexts (MOD-57 M2 R1 M-1): `editor.abort` gives its chord to
/// an entry of a context [`Stack::EDITOR_OPEN`] composes with `[editor]` (today `[global]`), so
/// `[global] quit = ["ctrl-x"]`, the README's example before M2, loads with a notice and leaves
/// the abort unbound. The abort may end unbound: only the asking tab offers it (E2), and the
/// editor still ends with the program's own keys.
///
/// Otherwise nothing gives way across contexts, and [`validate`]'s stack rules stand: two entries
/// on one chord are an error, and so is a chord an entry shares with another context's default
/// only where a [`DECLARED`] stack makes both candidates (the overlay stack: `[global] help`
/// against `overlay.close`, either way; any context against `editor.focus`; an `[editor] abort`
/// entry against a global default). A context no declared stack composes with the entry's is
/// not compared: `[global] quit = ["j"]` loads, and `list.down` keeps `j`.
///
/// MOD-115 M2 D8 (MD4): a file from before M2 (`version = 1`, or no version) keeps loading. A
/// default M2 added or changed (`since: 2`) that the file leaves alone gives its chord to an entry
/// of any context wherever a [`DECLARED`] stack makes both candidates for it, with the same
/// notice; `[global] quit = ["z"]` loads and `layout.zoom` loses `z`. An `[editor] focus` entry
/// takes it in any context, since E9 (d) holds focus against every row: `[editor] focus =
/// ["f2"]` loads and `layout.toggle_list` loses `f2`. Only a chord the entry
/// added gives way: one it repeats from its own default met the M2 default in the compiled keys
/// already. A `version = 2` file gets the strict rules above.
///
/// MOD-115 M2 MD7 (T5R): a kept `Esc` ([`kept_esc`]) is never taken nor given, in either pass.
pub(super) fn give_way(keys: &mut Keys) {
    let mut taken: Vec<(usize, Lost)> = Vec::new();
    for entry in keys.rows.iter().filter(|row| row.entry.is_some()) {
        for (index, default) in keys.rows.iter().enumerate() {
            if default.entry.is_some()
                || !(default.context == entry.context || abort_gives_way(entry, default))
                || matches!(default.act, Act::OverlayClose | Act::EditorFocus)
            {
                continue;
            }
            for &chord in entry
                .chords
                .iter()
                .filter(|c| !kept_esc(**c) && default.chords.contains(c))
            {
                if !allowed(STATE_GUARDED, entry, default, chord)
                    && keeps_another(&taken, index, default)
                {
                    taken.push((
                        index,
                        Lost {
                            chord,
                            from: entry.context,
                            to: entry.act,
                            line: entry.entry.unwrap_or(0),
                        },
                    ));
                }
            }
        }
    }
    if keys.version == 1 {
        for entry in keys.rows.iter().filter(|row| row.entry.is_some()) {
            for (index, default) in keys.rows.iter().enumerate() {
                if default.entry.is_some()
                    || default.context == entry.context
                    || matches!(default.act, Act::OverlayClose | Act::EditorFocus)
                {
                    continue;
                }
                // Only a chord new in M2 and one the entry added: one it repeats from its own
                // default met the M2 default in the compiled keys already, which validate.
                for &chord in entry.chords.iter().filter(|c| {
                    !kept_esc(**c)
                        && default.chords.contains(c)
                        && new_in_m2(default.act, **c)
                        && added(entry, **c)
                }) {
                    // E9 (d) holds `editor.focus` against every row in any context.
                    let meet = entry.act == Act::EditorFocus
                        || DECLARED.iter().any(|&(_, stack)| {
                            let candidates = keys.actions(stack, chord);
                            candidates.contains(&entry.act) && candidates.contains(&default.act)
                        });
                    let known = taken
                        .iter()
                        .any(|(at, lost)| *at == index && lost.chord == chord);
                    if meet && !known && keeps_another(&taken, index, default) {
                        taken.push((
                            index,
                            Lost {
                                chord,
                                from: entry.context,
                                to: entry.act,
                                line: entry.entry.unwrap_or(0),
                            },
                        ));
                    }
                }
            }
        }
    }
    for (index, lost) in taken {
        let row = &mut keys.rows[index];
        row.chords.retain(|&chord| chord != lost.chord);
        row.lost.push(lost);
    }
}

/// Whether `chord` is a kept `Esc` (MOD-115 M2 MD7): the loader puts `Esc` only on a row whose
/// compiled default (or, for a view's override of a shared verb, the shared row's) holds it, so
/// every `Esc` in the keys is one. It never takes a chord from, nor gives way to, anything:
/// `[chat] back = ["x"]` keeps back's `Esc` and `chat.end` keeps its own, so `Esc` still ends
/// the chat (T5R).
fn kept_esc(chord: KeyChord) -> bool {
    chord == ESC
}

/// Whether `default` (row `index`) may give one more chord away (MOD-115 M2 D-10): always, unless
/// it must keep a chord ([`must_keep`]) and the chords `taken` from it so far leave it only one.
fn keeps_another(taken: &[(usize, Lost)], index: usize, default: &Row) -> bool {
    must_keep(default.act).is_none()
        || taken.iter().filter(|(at, _)| *at == index).count() + 1 < default.chords.len()
}

/// Whether `chord` is a default M2 gave `act` (MOD-115 M2 D8): the row is `since: 2` and the
/// chord is not among its [`V1_DEFAULTS`].
fn new_in_m2(act: Act, chord: KeyChord) -> bool {
    act.spec().is_some_and(|spec| spec.since == 2)
        && !V1_DEFAULTS.iter().any(|&(known, defaults)| {
            known == act
                && defaults
                    .iter()
                    .any(|written| KeyChord::parse_strict(written) == Ok(chord))
        })
}

/// Whether `default`, in another context than `entry`, still gives way to it (MOD-57 M2 R1 M-1):
/// it is `editor.abort`, and `entry`'s action is in a wider layer of [`Stack::EDITOR_OPEN`].
fn abort_gives_way(entry: &Row, default: &Row) -> bool {
    let layers = Stack::EDITOR_OPEN.layers();
    default.act == Act::EditorAbort
        && default.context == Context::Editor
        && layers
            .iter()
            .filter(|layer| layer.context() != Context::Editor)
            .any(|layer| layer.context() == entry.context && layer.admits(entry.act))
}

/// Every collision, capture and safety-net error in `keys`, reported on the user's line (D8 steps
/// 5-6). The compiled defaults give none (`the_compiled_defaults_validate`).
///
/// Five passes, in this order: two actions sharing a chord in one shared or global context; two
/// candidates for one chord in a [`DECLARED`] stack; a chord shared with `editor.focus` in any
/// context (MOD-57 M2 E9 (d)); a row that must keep a chord and has none (MOD-115 M2 D7's safety
/// nets, [`must_keep`]); a printable chord on an `in_capture` action. `give_way` ran first, so a
/// default that gave its chord away (the abort's to a global entry, M2 R1 M-1) is no collision;
/// a focus chord shared with anything, and an `[editor] abort` entry on a global default, still
/// are. A pair and chord seen by two passes is reported once. A [`STATE_GUARDED`] pair may share
/// a chord only when it is a catalogue default of both actions (PA-2), and so may a
/// [`SHADOWING`] pair whose first act's layer is the narrower (MOD-67 D11).
#[must_use]
pub fn validate(keys: &Keys) -> Vec<KeyFileError> {
    validate_loaded(keys, &[])
}

/// [`validate`] after a load that dropped every chord of the rows in `dropped`: the loader
/// reported each of those chords on its line, so the safety net does not report the row again
/// for being empty (MOD-67 M6 D6: one error per mistake). `[editor] focus = ["ctrl-E"]` gives
/// the chord's error only.
#[must_use]
pub(super) fn validate_loaded(keys: &Keys, dropped: &[(Context, Act)]) -> Vec<KeyFileError> {
    check(keys, SHADOWING, STATE_GUARDED, dropped)
}

/// [`validate_loaded`] over explicit allow-lists: the demand tests drop one entry at a time.
fn check(
    keys: &Keys,
    shadowing: &[(Act, Act)],
    guarded: &[(Act, Act)],
    dropped: &[(Context, Act)],
) -> Vec<KeyFileError> {
    let mut check = Check {
        keys,
        guarded,
        reported: Vec::new(),
        errors: Vec::new(),
    };
    for (i, first) in keys.rows.iter().enumerate() {
        if first.context.is_view() {
            continue; // PA-4: the stack pass checks view rows where they meet
        }
        for second in keys.rows[i + 1..]
            .iter()
            .filter(|second| second.context == first.context)
        {
            for &chord in first.chords.iter().filter(|c| second.chords.contains(c)) {
                let place = format!("in [{}]", first.context.table());
                check.report(first, second, chord, &place);
            }
        }
    }
    for &(phrase, stack) in DECLARED {
        let mut chords: Vec<KeyChord> = Vec::new();
        for (index, layer) in stack.layers().iter().enumerate() {
            let admitted = keys
                .rows
                .iter()
                .filter(|row| row.context == layer.context() && stack.admits(index, row.act));
            for &chord in admitted.flat_map(|row| &row.chords) {
                if layer.admits_chord(chord) && !chords.contains(&chord) {
                    chords.push(chord);
                }
            }
        }
        for chord in chords {
            // Narrowest first: `actions` orders its candidates by layer.
            let rows: Vec<&Row> = keys
                .actions(stack, chord)
                .into_iter()
                .filter_map(|act| keys.resolve_row(stack, act).map(|(_, row)| row))
                .collect();
            for (i, first) in rows.iter().enumerate() {
                for second in &rows[i + 1..] {
                    let shadowed = shadowing.contains(&(first.act, second.act))
                        && !added(first, chord)
                        && !added(second, chord);
                    if !shadowed && !met_in_m2(keys, stack, second.act) {
                        check.report(first, second, chord, phrase);
                    }
                }
            }
        }
    }
    // MOD-57 M2 E9 (d): `editor.focus` acts on every screen while an edit is open, ahead of the
    // view (E1), so no other action may share its chord in any context. `report` dedups a pair the
    // passes above found (the per-context pass for `[editor]`, the stack pass for `[global]`).
    if let Some(focus) = keys.row(Context::Editor, Act::EditorFocus) {
        for row in keys.rows.iter().filter(|row| row.act != Act::EditorFocus) {
            for &chord in row
                .chords
                .iter()
                .filter(|chord| focus.chords.contains(chord))
            {
                check.report(row, focus, chord, WHILE_EDITING);
            }
        }
    }
    // MOD-115 M2 D7, D-10: the safety nets. A row htui cannot do without keeps a chord: an entry
    // that empties one is reported on its line, and so is one the merge left with none. A row
    // emptied only because the loader refused every chord it listed was reported already.
    for spec in CATALOGUE {
        if let (Some(reason), Some(row)) = (must_keep(spec.act), keys.row(spec.context, spec.act))
            && row.chords.is_empty()
            && !dropped.contains(&(spec.context, spec.act))
        {
            check.errors.push(KeyFileError {
                line: line_of(row),
                message: format!("{}: must keep at least one chord: {reason}", subject(row)),
            });
        }
    }
    for row in &keys.rows {
        if !row.act.spec().is_some_and(|spec| spec.in_capture)
            || !std::ptr::eq(source(keys, row), row)
        {
            continue; // a derived row's chords are its shared row's, reported there
        }
        for &chord in row.chords.iter().filter(|chord| chord.is_printable()) {
            let shown = quote(&chord.spec());
            check.errors.push(KeyFileError {
                line: line_of(row),
                message: format!(
                    "{} = {shown}: {shown} is typed text while a field captures: bind a ctrl or \
                     alt chord or a named key",
                    subject(row)
                ),
            });
        }
    }
    check.errors
}

/// Whether, in a version-1 file, `act` (from before M2: `since: 1`, or a changed row in
/// [`V1_DEFAULTS`] such as quit) meets `stack`'s narrower candidates only
/// since MOD-115 M2 (D8, MD4): the stack offers it only through a layer M2 added
/// ([`M2_LAYERS`], the overlays' `QUIT`). The stack pass then lets the narrower candidate keep
/// the chord, which it does: the resolver handles the first candidate, so the stack acts as it
/// did before M2. `[global] quit = ["ctrl-d"]` loads, and the concepts search keeps `ctrl-d`.
fn met_in_m2(keys: &Keys, stack: Stack<'_>, act: Act) -> bool {
    let before_m2 = act.spec().is_some_and(|spec| spec.since == 1)
        || V1_DEFAULTS.iter().any(|&(known, _)| known == act);
    keys.version == 1
        && before_m2
        && resolve_at(keys, stack, act)
            .is_some_and(|(index, _)| M2_LAYERS.contains(&stack.layers()[index]))
}

/// The layer index and row of `act` as `stack` sees it: [`Keys::resolve_row`], with the index.
fn resolve_at<'k>(keys: &'k Keys, stack: Stack<'_>, act: Act) -> Option<(usize, &'k Row)> {
    stack
        .layers()
        .iter()
        .enumerate()
        .filter(|&(index, _)| stack.admits(index, act))
        .find_map(|(index, layer)| keys.row(layer.context(), act).map(|row| (index, row)))
}

/// The line an error about `row` reports on, and the line of D6's `(line, chord)` key (MOD-67 M6
/// R1 LOW-5: one rule for every pass): the entry that set its chords, else the file's entry that
/// repeats its default, else 0 for a row the file leaves alone.
fn line_of(row: &Row) -> usize {
    row.line.or(row.entry).unwrap_or(0)
}

/// A row's identity: its context and act.
type Slot = (Context, Act);

/// The row an error about `row` names: `row` itself, or, for a derived `VIEW_DEFAULTS` row the
/// file did not set (no line), the shared row it follows, since that is what the user wrote.
fn source<'k>(keys: &'k Keys, row: &'k Row) -> &'k Row {
    if row.extra.is_none() || row.line.is_some() {
        return row;
    }
    row.act
        .spec()
        .and_then(|spec| keys.row(spec.context, row.act))
        .unwrap_or(row)
}

/// The validator's state: the pairs already reported and the errors so far.
struct Check<'k> {
    keys: &'k Keys,
    /// The state-guarded allow-list in force ([`STATE_GUARDED`] outside the demand tests).
    guarded: &'k [(Act, Act)],
    /// `((context, act), (context, act), chord)` of every reported collision, by source row.
    reported: Vec<(Slot, Slot, KeyChord)>,
    errors: Vec<KeyFileError>,
}

impl Check<'_> {
    /// Reports that `first` and `second` both bind `chord` (`place` ends the message), unless
    /// the pair was reported already or is allowed. The reported row is the one that added
    /// `chord` (it is not that row's catalogue default) when exactly one did (review L1);
    /// otherwise the one with the later line (`None`, a default, before any line), `second` on
    /// a tie. A derived row is named and placed by its [`source`].
    fn report(&mut self, first: &Row, second: &Row, chord: KeyChord, place: &str) {
        let (first_source, second_source) = (source(self.keys, first), source(self.keys, second));
        let a = (first_source.context, first_source.act);
        let b = (second_source.context, second_source.act);
        if self
            .reported
            .iter()
            .any(|&(x, y, c)| c == chord && ((x, y) == (a, b) || (x, y) == (b, a)))
            || allowed(self.guarded, first, second, chord)
        {
            return;
        }
        self.reported.push((a, b, chord));
        let (reported, other) = match (added(first, chord), added(second, chord)) {
            (true, false) => (first, second),
            (false, true) => (second, first),
            _ if first_source.line > second_source.line => (first, second),
            _ => (second, first),
        };
        let (reported_source, other_source) =
            (source(self.keys, reported), source(self.keys, other));
        let origin = other_source
            .line
            .or(other.entry)
            .map_or_else(|| "default".to_owned(), |line| format!("line {line}"));
        let shown = quote(&chord.spec());
        let other_name = other.act.spec().map_or("", |spec| spec.name);
        self.errors.push(KeyFileError {
            line: line_of(reported_source),
            message: format!(
                "{} = {shown}: {shown} is already {}.{other_name} ({origin}) {place}",
                subject(reported_source),
                other.context.table(),
            ),
        });
    }
}

/// Whether `first` and `second` may share `chord` (D8 step 5, PA-2): the pair is on `guarded`
/// (either order) and `chord` is a catalogue default of both.
fn allowed(guarded: &[(Act, Act)], first: &Row, second: &Row, chord: KeyChord) -> bool {
    let guarded = guarded
        .iter()
        .any(|&pair| pair == (first.act, second.act) || pair == (second.act, first.act));
    guarded && !added(first, chord) && !added(second, chord)
}

/// Whether `row` binds `chord` beyond its compiled defaults (a derived default counts): the
/// user added it.
fn added(row: &Row, chord: KeyChord) -> bool {
    !Keys::compiled()
        .chords(row.context, row.act)
        .contains(&chord)
}

/// `[table] name` of `row`, as an error's subject.
fn subject(row: &Row) -> String {
    format!(
        "[{}] {}",
        row.context.table(),
        row.act.spec().map_or("", |spec| spec.name)
    )
}

#[cfg(test)]
mod tests {
    use super::{check, line_of, new_in_m2, resolve_at, validate};
    use crate::keys::{
        Act, CTRL_C, Context, DECLARED, KeyChord, Keys, SHADOWING, STATE_GUARDED, load_str, views,
    };

    fn chord(spec: &str) -> KeyChord {
        KeyChord::parse_strict(spec).expect("a valid spec")
    }

    /// `src`'s full error vector as `(line, message)`, or empty when it loads.
    fn errors(src: &str) -> Vec<(usize, String)> {
        match load_str(src) {
            Ok(_) => Vec::new(),
            Err(errors) => errors
                .into_iter()
                .map(|error| (error.line, error.message))
                .collect(),
        }
    }

    fn one(line: usize, message: &str) -> Vec<(usize, String)> {
        vec![(line, message.to_owned())]
    }

    #[test]
    fn the_compiled_defaults_validate() {
        assert_eq!(validate(Keys::compiled()), []);
    }

    /// MOD-115 M2 D8 (MD4): an overlay's `QUIT` layer is new in M2, so a version-1 file's quit
    /// meets the concepts search's own keys only since M2. The narrower act keeps the chord
    /// there, as before M2, and the file loads with no notice; a version-2 file is refused.
    #[test]
    fn a_version_1_quit_on_a_concepts_search_key_loads_and_the_search_keeps_it() {
        for (spec, act) in [
            ("ctrl-d", Act::ConceptsDecisions),
            ("ctrl-p", Act::ConceptsProject),
        ] {
            let src = format!("[global]\nquit = [\"{spec}\"]\n");
            let keys = load_str(&src).unwrap_or_else(|errors| panic!("{spec}: {errors:?}"));
            assert_eq!(
                keys.chords(Context::Global, Act::Quit),
                [chord(spec), CTRL_C],
                "{spec}"
            );
            assert_eq!(
                keys.chords(Context::Concepts, act),
                Keys::compiled().chords(Context::Concepts, act),
                "{spec}"
            );
            assert_eq!(
                keys.actions(views::CONCEPTS_QUERY, chord(spec)),
                [act, Act::Quit],
                "{spec}: the search's own act comes first"
            );
            assert!(keys.notices().is_empty(), "{spec}");
            let strict = errors(&format!("version = 2\n{src}"));
            assert!(
                strict.iter().any(
                    |(line, message)| *line == 3 && message.ends_with("in the concepts search")
                ),
                "{spec}: {strict:?}"
            );
        }
    }

    /// MOD-115 M2 D8 (MD4): a version-1 `[editor] focus` on a chord only a default M2 added
    /// holds loaded before M2, so it still loads: E9 (d) holds focus against every row in any
    /// context, so the M2 default gives way to it wherever it sits, with the usual notice, and
    /// focus keeps the chord. A version-2 file is refused (`f2`, `layout.toggle_list`).
    #[test]
    fn a_version_1_focus_takes_any_chord_only_an_m2_default_holds() {
        let compiled = Keys::compiled();
        let mut checked = 0;
        for row in &compiled.rows {
            for &key in row.chords.iter().filter(|&&c| new_in_m2(row.act, c)) {
                // Only a chord nothing from before M2 holds: any other loaded no better before.
                if key == CTRL_C
                    || key.is_printable()
                    || compiled
                        .rows
                        .iter()
                        .any(|other| other.chords.contains(&key) && !new_in_m2(other.act, key))
                {
                    continue;
                }
                checked += 1;
                let src = format!("version = 1\n[editor]\nfocus = [{:?}]\n", key.spec());
                let keys = load_str(&src).unwrap_or_else(|errors| panic!("{src}: {errors:?}"));
                assert_eq!(
                    keys.chords(Context::Editor, Act::EditorFocus),
                    [key],
                    "{src}"
                );
                assert!(
                    keys.rows
                        .iter()
                        .all(|row| row.act == Act::EditorFocus || !row.chords.contains(&key)),
                    "{src}: another row kept it"
                );
                assert!(
                    keys.notices().iter().any(|notice| notice.line == 3
                        && notice.message.starts_with(&format!(
                            "[editor] focus = {:?} takes {:?} from ",
                            key.spec(),
                            key.spec()
                        ))),
                    "{src}: {:?}",
                    keys.notices()
                );
            }
        }
        assert!(checked >= 9, "{checked} chords checked");
        assert_eq!(
            errors("version = 2\n[editor]\nfocus = [\"f2\"]\n"),
            one(
                3,
                r#"[editor] focus = "f2": "f2" is already layout.toggle_list (default) while an edit is open"#
            )
        );
    }

    /// MOD-115 M2 D8 (MD4): in no stack where quit is offered only through a layer M2 added
    /// ([`views::M2_LAYERS`]) does a version-1 `[global] quit` on a chord that stack binds for
    /// another act collide there.
    #[test]
    fn a_version_1_quit_never_collides_through_a_layer_m2_added() {
        let compiled = Keys::compiled();
        let mut checked = 0;
        for &(phrase, stack) in DECLARED {
            let Some((at, _)) = resolve_at(compiled, stack, Act::Quit) else {
                continue;
            };
            if !views::M2_LAYERS.contains(&stack.layers()[at]) {
                continue;
            }
            let quit = compiled.chords(Context::Global, Act::Quit);
            let mut chords: Vec<KeyChord> = Vec::new();
            for (index, layer) in stack.layers().iter().enumerate() {
                for row in compiled.rows.iter().filter(|row| {
                    row.act != Act::Quit
                        && row.context == layer.context()
                        && stack.admits(index, row.act)
                }) {
                    chords.extend(row.chords.iter().filter(|c| !quit.contains(c)));
                }
            }
            for chord in chords {
                checked += 1;
                let src = format!("[global]\nquit = [{:?}]\n", chord.spec());
                let errors = errors(&src);
                assert!(
                    !errors
                        .iter()
                        .any(|(_, message)| message.ends_with(&format!(" {phrase}"))),
                    "{src}: {errors:?}"
                );
            }
        }
        assert!(
            checked > 0,
            "some stack offers quit through the M2 quit layer"
        );
    }

    /// MOD-67 M6 R1 LOW-5: every pass reports a row on one line, so D6's `(line, chord)` keys
    /// agree: the entry that set its chords, else the file's entry that repeats its default (the
    /// row is the user's, MOD-12 M3 R1 H1), else 0 for a row the file leaves alone.
    #[test]
    fn every_pass_reports_a_row_on_one_line() {
        let keys = load_str("[global]\nqueue = [\"ctrl-q\"]\nquit = [\"z\"]\n").expect("it loads");
        let line = |act| line_of(keys.row(Context::Global, act).expect("a global row"));
        assert_eq!(line(Act::Queue), 2, "an entry repeating the default");
        assert_eq!(line(Act::Quit), 3, "an entry setting the chords");
        assert_eq!(line(Act::Help), 0, "a row the file leaves alone");
    }

    // MOD-57 M2 E9: the editor's keys.

    /// E9 (b): `editor.focus` never gives way, so an `[editor]` entry on its chord collides in
    /// `[editor]`, and focus keeps it.
    #[test]
    fn the_abort_cannot_take_the_focus_chord() {
        assert_eq!(
            errors("[editor]\nabort = \"ctrl-4\"\n"),
            one(
                2,
                r#"[editor] abort = "ctrl-4": "ctrl-4" is already editor.focus (default) in [editor]"#
            )
        );
    }

    /// The other direction is allowed: `abort` need not stay bound, so it gives `ctrl-x` to
    /// `focus` with the usual notice.
    #[test]
    fn the_focus_may_take_ctrl_x_from_abort() {
        let keys = load_str("[editor]\nfocus = \"ctrl-x\"\n").expect("abort gives way");
        assert_eq!(
            keys.chords(Context::Editor, Act::EditorFocus),
            [chord("ctrl-x")]
        );
        assert!(keys.chords(Context::Editor, Act::EditorAbort).is_empty());
        let notices: Vec<String> = keys
            .notices()
            .into_iter()
            .map(|notice| notice.message)
            .collect();
        assert_eq!(
            notices,
            [r#"[editor] focus = "ctrl-x" takes "ctrl-x" from editor.abort, which is now unbound"#]
        );
    }

    /// M2 R1 M-1: the README's example before MOD-57 M2 (`[global] quit = ["ctrl-x"]`) still
    /// loads. `Stack::EDITOR_OPEN` stacks `[editor]` over every global action, so the global
    /// entry takes `ctrl-x` from `editor.abort` (left at its default) across the two contexts,
    /// with the usual notice; the abort is unbound (E2 confines it to the asking tab).
    #[test]
    fn a_global_entry_takes_ctrl_x_from_the_abort_with_a_notice() {
        let readme = "version = 1\n\n[global]\nquit = [\"ctrl-x\"]\n\n[overlay]\nclose = [\"esc\", \"f2\"]\n";
        let keys = load_str(readme).expect("the abort gives way");
        // MOD-115 M2 (D-9): quit keeps its implicit `ctrl-c`.
        assert_eq!(
            keys.chords(Context::Global, Act::Quit),
            [chord("ctrl-x"), CTRL_C]
        );
        assert!(keys.chords(Context::Editor, Act::EditorAbort).is_empty());
        assert_eq!(keys.line(Context::Editor, Act::EditorAbort), None);
        let notices: Vec<(usize, String)> = keys
            .notices()
            .into_iter()
            .map(|notice| (notice.line, notice.message))
            .collect();
        assert_eq!(
            notices,
            one(
                4,
                r#"[global] quit = "ctrl-x" takes "ctrl-x" from editor.abort, which is now unbound"#
            )
        );
    }

    /// M2 R1 M-1: only the abort gives way across contexts, and only to a context
    /// `Stack::EDITOR_OPEN` composes with `[editor]`: an overlay entry on `ctrl-x` never meets
    /// the abort, so the abort keeps it and nothing is noticed.
    #[test]
    fn the_abort_keeps_ctrl_x_from_a_context_it_never_meets() {
        let keys = load_str("[overlay]\nclose = [\"esc\", \"ctrl-x\"]\n").expect("loads");
        assert_eq!(
            keys.chords(Context::Editor, Act::EditorAbort),
            [chord("ctrl-x")]
        );
        assert!(keys.notices().is_empty());
    }

    /// M2 R1 M-1: an `[editor] abort` entry is the user's, so it takes nothing across contexts:
    /// a chord another context's default holds while an edit is open is still refused.
    #[test]
    fn an_abort_entry_on_a_global_default_is_still_refused() {
        assert_eq!(
            errors("[editor]\nabort = \"ctrl-q\"\n"),
            one(
                2,
                r#"[editor] abort = "ctrl-q": "ctrl-q" is already global.queue (default) while an edit is open"#
            )
        );
    }

    /// M2 R1 M-1: a global entry on the abort's chord that the file also sets is two entries on
    /// one chord: still refused. MOD-115 M2 B4: the abort question now offers quit (its
    /// `ctrl-c`) through the M2 `QUIT` layer, declared before the open edit; a version-1 file
    /// meets it there only since M2 (D8), so it still names the open edit, and a version-2 file
    /// names the question.
    #[test]
    fn a_global_entry_on_an_abort_entrys_chord_is_still_refused() {
        assert_eq!(
            errors("[editor]\nabort = \"ctrl-x\"\n[global]\nquit = \"ctrl-x\"\n"),
            one(
                4,
                r#"[global] quit = "ctrl-x": "ctrl-x" is already editor.abort (line 2) while an edit is open"#
            )
        );
        assert_eq!(
            errors("version = 2\n[editor]\nabort = \"ctrl-x\"\n[global]\nquit = \"ctrl-x\"\n"),
            one(
                5,
                r#"[global] quit = "ctrl-x": "ctrl-x" is already editor.abort (line 3) in the abort question"#
            )
        );
    }

    /// E9 (c): `editor.focus` never gives way, so a global entry on its chord is refused while
    /// an edit is open.
    #[test]
    fn a_global_entry_on_the_focus_chord_is_refused_while_an_edit_is_open() {
        assert_eq!(
            errors("[global]\nhelp = [\"?\", \"ctrl-4\"]\n"),
            one(
                2,
                r#"[global] help = "ctrl-4": "ctrl-4" is already editor.focus (default) while an edit is open"#
            )
        );
    }

    /// E9 (d): `editor.focus` acts on every screen while an edit is open, so its chord is
    /// refused in every other context, a view table included.
    #[test]
    fn the_focus_chord_is_refused_in_every_other_context() {
        assert_eq!(
            errors("[list]\ndown = \"ctrl-4\"\n"),
            one(
                2,
                r#"[list] down = "ctrl-4": "ctrl-4" is already editor.focus (default) while an edit is open"#
            )
        );
        assert_eq!(
            errors("[confirm]\nyes = \"ctrl-4\"\n"),
            one(
                2,
                r#"[confirm] yes = "ctrl-4": "ctrl-4" is already editor.focus (default) while an edit is open"#
            )
        );
        assert_eq!(
            errors("[settings.agents]\nprobe = \"ctrl-4\"\n"),
            one(
                2,
                r#"[settings.agents] probe = "ctrl-4": "ctrl-4" is already editor.focus (default) while an edit is open"#
            )
        );
    }

    /// MOD-115 M2 (MD2): `ctrl-c` binds in `[editor]` like any chord. A version-1 focus takes it
    /// from quit and the leave dialog's confirm, which had it only since M2 (D8), and each keeps
    /// another; a version-2 one collides with quit while an edit is open.
    #[test]
    fn ctrl_c_in_the_editor_table_follows_the_version_rules() {
        let keys = load_str("[editor]\nfocus = \"ctrl-c\"\n").expect("version 1 loads");
        assert_eq!(keys.chords(Context::Editor, Act::EditorFocus), [CTRL_C]);
        assert_eq!(keys.chords(Context::Global, Act::Quit), [chord("q")]);
        assert_eq!(
            keys.chords(Context::Leave, Act::LeaveConfirm),
            [chord("enter")]
        );
        assert_eq!(
            errors("version = 2\n[editor]\nfocus = \"ctrl-c\"\n")[0],
            (
                3,
                r#"[editor] focus = "ctrl-c": "ctrl-c" is already global.quit (default) while an edit is open"#
                    .to_owned()
            )
        );
    }

    #[test]
    fn two_entries_sharing_a_chord_in_one_context_are_refused_on_the_users_line() {
        assert_eq!(
            errors("[list]\ndown = [\"j\"]\ntop = [\"j\"]\n"),
            one(
                3,
                r#"[list] top = "j": "j" is already list.down (line 2) in [list]"#
            )
        );
    }

    /// MOD-12 M3 R1 H1: an entry takes a chord from an action the file leaves at its default.
    /// That action keeps its other chords, and its row is no change of the user's.
    #[test]
    fn an_entry_takes_its_chord_from_an_action_left_at_its_default() {
        let keys = load_str("[list]\ntop = [\"j\"]\n").expect("the default gives way");
        assert_eq!(keys.chords(Context::List, Act::ListTop), [chord("j")]);
        assert_eq!(keys.chords(Context::List, Act::ListDown), [chord("down")]);
        assert_eq!(keys.line(Context::List, Act::ListDown), None);

        let keys = load_str("[global]\nquit = [\"ctrl-q\"]\n").expect("the queue gives way");
        // MOD-115 M2 (D-9): quit keeps its implicit `ctrl-c`.
        assert_eq!(
            keys.chords(Context::Global, Act::Quit),
            [chord("ctrl-q"), CTRL_C]
        );
        assert!(keys.chords(Context::Global, Act::Queue).is_empty());
        assert_eq!(keys.line(Context::Global, Act::Queue), None);
    }

    /// An entry that repeats an action's default is still the user's: a second entry on its
    /// chord collides, and the report names the entry's line.
    #[test]
    fn an_entry_equal_to_its_default_keeps_its_chord() {
        assert_eq!(
            errors("[global]\nqueue = [\"ctrl-q\"]\nquit = [\"ctrl-q\"]\n"),
            one(
                3,
                r#"[global] quit = "ctrl-q": "ctrl-q" is already global.queue (line 2) in [global]"#
            )
        );
    }

    #[test]
    fn a_shared_context_no_view_offers_yet_is_still_checked() {
        assert_eq!(
            errors("[confirm]\nyes = [\"n\"]\nno = [\"n\"]\n"),
            one(
                2,
                r#"[confirm] yes = "n": "n" is already confirm.no (line 3) in [confirm]"#
            )
        );
    }

    #[test]
    fn two_user_rows_report_on_the_later_line() {
        assert_eq!(
            errors("[global]\nquit = [\"z\"]\nhelp = [\"z\"]\n"),
            one(
                3,
                r#"[global] help = "z": "z" is already global.quit (line 2) in [global]"#
            )
        );
    }

    /// Review L1: the error sits on the row that added the chord, not on the later row that
    /// keeps it as a default.
    #[test]
    fn a_collision_is_reported_where_the_chord_was_added() {
        assert_eq!(
            errors("[global]\nquit = [\"f1\"]\nhelp = [\"f1\", \"z\"]\n"),
            one(
                2,
                r#"[global] quit = "f1": "f1" is already global.help (line 3) in [global]"#
            )
        );
    }

    /// A default only gives way in its own table (MOD-12 M3 R1 H1): where a declared stack
    /// makes both candidates, a chord the user adds onto another table's default is still
    /// refused, either way. MOD-115 M2 MD7: `esc` itself is never listed, so it cannot collide.
    #[test]
    fn a_collision_over_an_overlay_is_found_in_the_overlay_stack() {
        assert_eq!(
            errors("[global]\nhelp = [\"esc\"]\n"),
            one(
                2,
                r#"[global] help = "esc": esc always closes, cancels or goes back and cannot be bound"#
            )
        );
        assert_eq!(
            errors("[overlay]\nclose = [\"esc\", \"f1\"]\n"),
            one(
                2,
                r#"[overlay] close = "f1": "f1" is already global.help (default) over an overlay"#
            )
        );
        assert_eq!(
            errors("version = 2\n[overlay]\nclose = [\"f1\"]\n"),
            one(
                3,
                r#"[overlay] close = "f1": "f1" is already global.help (default) over an overlay"#
            )
        );
    }

    /// MOD-12 M3 R1 H1 review: across contexts nothing gives way. Since MOD-67 M3 the Settings
    /// stacks compose `[global]` with `[list]`, so a global entry on a list default is refused
    /// there, and the list action keeps its chord.
    #[test]
    fn a_global_entry_on_a_list_default_is_refused_where_a_stack_composes_them() {
        assert_eq!(
            errors("[global]\nquit = [\"j\"]\n"),
            one(
                2,
                r#"[global] quit = "j": "j" is already list.down (default) in Settings > Agents"#
            )
        );
    }

    #[test]
    fn a_collision_seen_by_two_checks_is_reported_once() {
        assert_eq!(
            errors("[global]\nworkspaces = [\"z\"]\nquit = [\"z\"]\n"),
            one(
                3,
                r#"[global] quit = "z": "z" is already global.workspaces (line 2) in [global]"#
            )
        );
    }

    #[test]
    fn a_state_guarded_default_pair_is_allowed() {
        assert_eq!(errors("[common]\nback = [\"esc\"]\n"), []);
    }

    /// PA-2: the allow-list is per chord. `esc` is a default of both `back` and `dismiss`, so
    /// adding `f2` to `back` keeps that share allowed, and `dismiss` keeps `esc`; a chord the user
    /// gives both is not.
    #[test]
    fn a_state_guarded_pair_is_allowed_only_on_a_chord_both_have_by_default() {
        assert_eq!(errors("[common]\nback = [\"esc\", \"f2\"]\n"), []);
        let keys = load_str("[common]\nback = [\"esc\", \"f2\"]\n").expect("loads");
        assert_eq!(keys.chords(Context::Common, Act::Dismiss), [chord("esc")]);
        // MOD-67 M6: `backspace` (this test's chord before M6) is the directory picker's
        // `picker.parent`, which offers `common.back` too.
        assert_eq!(
            errors("[common]\nback = [\"esc\", \"backspace\"]\n"),
            one(
                2,
                r#"[common] back = "backspace": "backspace" is already picker.parent (default) in the Hierarchy directory picker"#
            )
        );
        assert_eq!(
            errors("[common]\ndismiss = [\"z\"]\nback = [\"z\"]\n"),
            one(
                3,
                r#"[common] back = "z": "z" is already common.dismiss (line 2) in [common]"#
            )
        );
    }

    #[test]
    fn an_in_capture_action_refuses_a_printable_chord() {
        let tail = "is typed text while a field captures: bind a ctrl or alt chord or a named key";
        assert_eq!(
            errors("[form]\nsave = [\"s\"]\n"),
            one(2, &format!(r#"[form] save = "s": "s" {tail}"#))
        );
        assert_eq!(
            errors("[overlay]\nclose = [\"x\"]\n"),
            one(2, &format!(r#"[overlay] close = "x": "x" {tail}"#))
        );
        assert_eq!(
            errors("[form]\nsave = [\"alt-s\", \"f2\"]\nnext_field = [\"down\"]\n"),
            []
        );
    }

    #[test]
    fn every_allow_list_entry_is_demanded() {
        let keys = Keys::compiled();
        assert_eq!(check(keys, SHADOWING, STATE_GUARDED, &[]), []);
        for entry in SHADOWING {
            let fewer: Vec<_> = SHADOWING.iter().copied().filter(|e| e != entry).collect();
            assert!(
                !check(keys, &fewer, STATE_GUARDED, &[]).is_empty(),
                "SHADOWING {entry:?} is not demanded"
            );
        }
        for entry in STATE_GUARDED {
            let fewer: Vec<_> = STATE_GUARDED
                .iter()
                .copied()
                .filter(|e| e != entry)
                .collect();
            assert!(
                !check(keys, SHADOWING, &fewer, &[]).is_empty(),
                "STATE_GUARDED {entry:?} is not demanded"
            );
        }
    }

    #[test]
    fn a_view_rows_never_collide_across_modes() {
        assert_eq!(errors("[settings.agents]\nyes = \"o\"\n"), []);
        assert_eq!(
            errors("[settings.agents]\nprobe = \"down\"\n"),
            one(
                2,
                r#"[settings.agents] probe = "down": "down" is already list.down (default) in Settings > Agents"#
            )
        );
    }

    #[test]
    fn a_user_chord_shared_with_a_shadowed_act_is_refused() {
        // MOD-115 M2 MD7: `esc` is never listed; `f7` on both sides stands in for it.
        assert_eq!(
            errors("[migration]\nyes = [\"y\", \"esc\"]\n"),
            one(
                2,
                r#"[migration] yes = "esc": esc always closes, cancels or goes back and cannot be bound"#
            )
        );
        assert_eq!(
            errors(
                "version = 2\n[overlay]\nclose = [\"f7\"]\n[migration]\nyes = [\"y\", \"f7\"]\n"
            ),
            one(
                5,
                r#"[migration] yes = "f7": "f7" is already overlay.close (line 3) in the migration prompt"#
            )
        );
        assert_eq!(errors("[settings.boxes]\nexecutor = [\"w\", \"W\"]\n"), []);
    }

    /// ANA-26 §2.2: a global rebind onto a view's letter would be shadowed there without a word.
    #[test]
    fn a_global_rebind_onto_a_view_verb_is_refused() {
        assert_eq!(
            errors("[global]\nquit = \"x\"\n"),
            [
                (
                    2,
                    r#"[global] quit = "x": "x" is already settings.agents.cancel (default) in Settings > Agents"#
                        .to_owned()
                ),
                (
                    2,
                    r#"[global] quit = "x": "x" is already skills.attach.detach (default) in Skills > Library > Attachments"#
                        .to_owned()
                ),
                // MOD-67 M5: and `x` rejects a step in the Backlog's Runs pane.
                (
                    2,
                    r#"[global] quit = "x": "x" is already backlog.runs.reject (default) in Backlog > Runs"#
                        .to_owned()
                ),
            ]
        );
    }

    /// MOD-67 M4 D8.1: `editor.focus` never gives its chord way to an `[editor]` entry, so the
    /// entry collides instead of silently unbinding the focus toggle.
    #[test]
    fn editor_focus_never_gives_way() {
        assert_eq!(
            errors("[editor]\nabort = \"ctrl-4\"\n"),
            one(
                2,
                r#"[editor] abort = "ctrl-4": "ctrl-4" is already editor.focus (default) in [editor]"#
            )
        );
    }

    /// MOD-67 M4 D8.2 (PA-4): the editor stacks are declared, so an `[editor]` chord on a global
    /// default is refused, and so is a global chord on an editor default.
    #[test]
    fn an_editor_chord_on_a_global_default_is_refused_either_way() {
        assert_eq!(
            errors("[editor]\nabort = \"f1\"\n"),
            one(
                2,
                r#"[editor] abort = "f1": "f1" is already global.help (default) while an edit is open"#
            )
        );
        // MOD-57 M2 E9: the defaulted abort key gives way to a global entry.
        assert!(load_str("[global]\nquit = [\"q\", \"ctrl-x\"]\n").is_ok());
    }

    /// MOD-115 M2 D7: no key is fixed ahead of the resolver any more. A chord the filter form,
    /// the item form or a Runs question used to keep fixed is a `[choice]` or `[field]` row's,
    /// so binding another act of the stack to it is an ordinary collision, and moving the row
    /// off it frees it.
    #[test]
    fn a_former_fixed_key_is_a_row_and_collides_as_one() {
        assert_eq!(
            errors("version = 2\n[backlog.filter_form]\nclear = [\"space\"]\n"),
            one(
                3,
                r#"[backlog.filter_form] clear = "space": "space" is already choice.toggle (default) in the Backlog filter form"#
            )
        );
        assert_eq!(
            errors("version = 2\n[backlog.runs]\nyes = [\"y\", \"right\"]\n"),
            one(
                3,
                r#"[backlog.runs] yes = "right": "right" is already choice.next (default) in a Runs question"#
            )
        );
        assert_eq!(
            errors(
                "version = 2\n[choice]\ntoggle = [\"t\"]\n[backlog.filter_form]\nclear = [\"space\"]\n"
            ),
            []
        );
    }

    /// MOD-115 M2 D7: the permission digits are `answer.option_*` rows; a view act on one
    /// collides with the answer, and moving the answer frees the digit.
    #[test]
    fn a_rebind_onto_a_permission_digit_collides_with_the_answer() {
        assert_eq!(
            errors(
                "version = 2\n[global]\nselect_tab_1 = []\n[backlog.runs]\napprove = [\"a\", \"1\"]\n"
            ),
            one(
                5,
                r#"[backlog.runs] approve = "1": "1" is already answer.option_1 (default) in Backlog > Runs"#
            )
        );
        assert_eq!(
            errors(
                "version = 2\n[global]\nselect_tab_1 = []\n[answer]\noption_1 = [\"f8\"]\n\
                 [backlog.runs]\napprove = [\"a\", \"1\"]\n"
            ),
            []
        );
        // The tab select may move between digits: the answers are state-guarded with it.
        assert_eq!(
            errors("[global]\nselect_tab_1 = [\"2\"]\nselect_tab_2 = [\"1\"]\n"),
            []
        );
    }

    /// MOD-115 M2 D7, D-10: the safety nets, as the validator reports them over merged keys:
    /// a must-keep row with no chord is refused naming why, and the compiled keys have none.
    #[test]
    fn a_must_keep_row_with_no_chord_is_refused() {
        let keys = Keys::defaults()
            .with_chords(Context::Global, Act::Quit, &[])
            .with_chords(Context::Field, Act::FieldCancel, &[])
            .with_chords(Context::Leave, Act::LeaveStay, &[]);
        let messages: Vec<String> = validate(&keys)
            .into_iter()
            .map(|error| error.message)
            .collect();
        assert_eq!(
            messages,
            [
                "[global] quit: must keep at least one chord: global.quit opens the leave dialog \
                 and leave.confirm leaves; without both htui cannot be left",
                "[leave] stay: must keep at least one chord: it is the way back from the leave \
                 dialog",
                "[field] cancel: must keep at least one chord: it is the way out of a text field",
            ]
        );
    }

    /// MOD-67 M6 D9 low 2 (PA-2): a view row the file pins to its compiled derivation while its
    /// shared row moves is the user's line, so a collision with it names that line, not the
    /// shared row's.
    #[test]
    fn a_collision_names_a_pinned_view_rows_own_line() {
        assert_eq!(
            errors(
                "version = 1\n[form]\nnext_field = [\"ctrl-n\"]\n[settings.agents]\n\
                 next_field = [\"tab\", \"down\"]\nprev_field = [\"down\"]\n"
            ),
            one(
                6,
                r#"[settings.agents] prev_field = "down": "down" is already settings.agents.next_field (line 5) in the Agents form"#
            )
        );
    }

    /// MOD-67 M5 R1 M1 (a): the Chat replay resolves its own acts before its swallow, so they may
    /// take `i`; only a global act there is shadowed.
    #[test]
    fn the_chat_replays_own_acts_may_take_its_swallowed_keys() {
        assert_eq!(errors("[common]\nback = [\"i\"]\n"), []);
        assert_eq!(errors("[chat]\nthoughts = [\"i\"]\n"), []);
    }

    /// A derived view row follows its shared row, so a printable shared chord is reported once,
    /// on the shared line.
    #[test]
    fn a_printable_shared_chord_is_reported_once_on_the_shared_line() {
        let tail = "is typed text while a field captures: bind a ctrl or alt chord or a named key";
        assert_eq!(
            errors("[form]\nnext_field = [\"x\"]\n"),
            [
                // MOD-67 M5: the Backlog filter form offers `form.next_field` beside its own
                // `x` (clear), so the chord also collides there, once.
                (
                    2,
                    r#"[form] next_field = "x": "x" is already backlog.filter_form.clear (default) in the Backlog filter form"#
                        .to_owned()
                ),
                (2, format!(r#"[form] next_field = "x": "x" {tail}"#)),
            ]
        );
    }
}

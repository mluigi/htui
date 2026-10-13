//! The key file's semantic checks over the merged keys (MOD-67 M2 D8 steps 5-6; ANA-26 §7.4
//! step 7): collisions in every context and in every declared stack, printable chords on
//! actions offered while a field captures, and (MOD-67 M5 R1) a user chord on a key a view keeps
//! for itself ahead of the resolver ([`FIXED`]). Before them, `give_way` lets an entry of the
//! file take its chord from an action the file leaves at its default (MOD-12 M3 R1 H1).
//!
//! MOD-67 M3: the per-context pass skips view contexts (PA-4: a view context holds rows of modes
//! that never meet; the stack pass checks every declared stack), the stack pass allows a
//! reviewed [`SHADOWING`] pair (D11, PA-2), and a derived `VIEW_DEFAULTS` row the file did not
//! set is reported as the shared row it follows, on the user's line.
//!
//! MOD-57 M2 (E9): `editor.focus` never gives way, and no other action may share its chord in any
//! context: while an edit is open it acts ahead of every view. `editor.abort` left at its default
//! gives way to a global entry too (M2 R1 M-1): a file that binds `ctrl-x` in `[global]` loads.

use super::stack::WHILE_EDITING;
use super::{
    Act, Context, DECLARED, FIXED, Fixed, KeyChord, KeyFileError, Keys, Lost, Row, SHADOWING,
    STATE_GUARDED, Stack, quote,
};
use crossterm::event::KeyModifiers;

/// The user's binding wins (MOD-12 M3 R1 H1, maintainer 2026-10-08): an entry of the file that
/// shares a chord with an action in **its own context** that the file leaves at its default
/// takes the chord, and that action loses it (unbound when it has no other). A
/// [`STATE_GUARDED`] pair that shares the chord by default keeps sharing it, and
/// `overlay.close` and `editor.focus` never give way (each must keep a chord; MOD-67 M4 D8.1).
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
            for &chord in entry.chords.iter().filter(|c| default.chords.contains(c)) {
                if !allowed(STATE_GUARDED, entry, default, chord) {
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
    for (index, lost) in taken {
        let row = &mut keys.rows[index];
        row.chords.retain(|&chord| chord != lost.chord);
        row.lost.push(lost);
    }
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

/// Every collision and capture error in `keys`, reported on the user's line (D8 steps 5-6).
/// The compiled defaults give none (`the_compiled_defaults_validate`).
///
/// Four passes, in this order: two actions sharing a chord in one shared or global context; two
/// candidates for one chord in a [`DECLARED`] stack; a chord shared with `editor.focus` in any
/// context (MOD-57 M2 E9 (d)); a printable chord on an `in_capture` action; an act of a stack bound
/// to a key its view keeps fixed ([`FIXED`], MOD-67 M5 R1 M1). `give_way` ran first, so a default
/// that gave its chord away (the abort's to a global entry, M2 R1 M-1) is no collision; a focus
/// chord shared with anything, and an `[editor] abort` entry on a global default, still are. A pair
/// and chord seen by two passes is reported once, and a line's chord the fixed pass would repeat
/// after a collision or capture error is not reported again (MOD-67 M6 D6). A [`STATE_GUARDED`]
/// pair may share a chord only when it is a catalogue default of both actions (PA-2), and so may a
/// [`SHADOWING`] pair whose first act's layer is the narrower (MOD-67 D11).
#[must_use]
pub fn validate(keys: &Keys) -> Vec<KeyFileError> {
    check(keys, SHADOWING, STATE_GUARDED, FIXED)
}

/// [`validate`] over explicit allow-lists: the demand tests drop one entry at a time.
fn check(
    keys: &Keys,
    shadowing: &[(Act, Act)],
    guarded: &[(Act, Act)],
    fixed: &[Fixed],
) -> Vec<KeyFileError> {
    let mut check = Check {
        keys,
        guarded,
        reported: Vec::new(),
        said: Vec::new(),
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
                    if !shadowed {
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
    for row in &keys.rows {
        if !row.act.spec().is_some_and(|spec| spec.in_capture)
            || !std::ptr::eq(source(keys, row), row)
        {
            continue; // a derived row's chords are its shared row's, reported there
        }
        for &chord in row.chords.iter().filter(|chord| chord.is_printable()) {
            let shown = quote(&chord.spec());
            let line = row.line.unwrap_or(0);
            check.said.push((line, chord));
            check.errors.push(KeyFileError {
                line,
                message: format!(
                    "{} = {shown}: {shown} is typed text while a field captures: bind a ctrl or \
                     alt chord or a named key",
                    subject(row)
                ),
            });
        }
    }
    check.fixed(fixed);
    check.errors
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
    /// `(line, chord)` of every collision and capture error, so the fixed pass, which runs last,
    /// does not report a line's chord again (MOD-67 M6 D6, PA-13).
    said: Vec<(usize, KeyChord)>,
    errors: Vec<KeyFileError>,
}

impl Check<'_> {
    /// The fixed-key pass (MOD-67 M5 R1 M1): every act a [`Fixed`] entry's stack resolves on one
    /// of its keys or on a named key's SHIFT form, in the layers the keys shadow, unless the
    /// entry shadows that act by design.
    /// A row and chord are reported once, naming the first stack, on the row's [`source`]; a
    /// line's chord an earlier pass reported is skipped (MOD-67 M6 D6: the first pass wins).
    fn fixed(&mut self, table: &[Fixed]) {
        let mut seen: Vec<(Slot, KeyChord)> = Vec::new();
        for entry in table {
            let phrase = DECLARED
                .iter()
                .find(|(_, stack)| *stack == entry.stack)
                .map_or("", |(phrase, _)| phrase);
            let first = if entry.only_global {
                entry.stack.global().map_or(usize::MAX, |(index, _)| index)
            } else {
                0
            };
            let chords = entry.chords.iter().flat_map(|written| {
                let chord = KeyChord::parse_strict(written)
                    .unwrap_or_else(|err| panic!("fixed key {written:?}: {err}"));
                // A view matches its fixed keys with `keys::is_plain`, which lets SHIFT through,
                // and a named key keeps SHIFT in its chord (a character drops it), so the
                // shifted named key is fixed too (M5 R1 fix): `shift-left` moves the option.
                let shifted = KeyChord::new(chord.code, chord.mods | KeyModifiers::SHIFT);
                std::iter::once(chord).chain((shifted != chord).then_some(shifted))
            });
            for chord in chords {
                for act in self.keys.actions(entry.stack, chord) {
                    if entry.shadowed.contains(&act) {
                        continue;
                    }
                    let Some((index, row)) = resolve_at(self.keys, entry.stack, act) else {
                        continue;
                    };
                    let row = source(self.keys, row);
                    let slot = (row.context, row.act);
                    let line = row.line.or(row.entry).unwrap_or(0);
                    if index < first
                        || seen.contains(&(slot, chord))
                        || self.said.contains(&(line, chord))
                    {
                        continue;
                    }
                    seen.push((slot, chord));
                    let shown = quote(&chord.spec());
                    self.errors.push(KeyFileError {
                        line,
                        message: format!("{} = {shown}: {shown} is fixed {phrase}", subject(row)),
                    });
                }
            }
        }
    }

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
        let line = reported_source.line.or(reported_source.entry).unwrap_or(0);
        self.said.push((line, chord));
        self.errors.push(KeyFileError {
            line,
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
    use super::{check, validate};
    use crate::keys::{
        Act, Context, FIXED, Fixed, KeyChord, Keys, SHADOWING, STATE_GUARDED, load_str,
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
        assert_eq!(keys.chords(Context::Global, Act::Quit), [chord("ctrl-x")]);
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
    /// one chord: still refused.
    #[test]
    fn a_global_entry_on_an_abort_entrys_chord_is_still_refused() {
        assert_eq!(
            errors("[editor]\nabort = \"ctrl-x\"\n[global]\nquit = \"ctrl-x\"\n"),
            one(
                4,
                r#"[global] quit = "ctrl-x": "ctrl-x" is already editor.abort (line 2) while an edit is open"#
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

    #[test]
    fn ctrl_c_is_refused_in_the_editor_table() {
        assert_eq!(
            errors("[editor]\nfocus = \"ctrl-c\"\n"),
            one(
                2,
                r#"[editor] focus = "ctrl-c": ctrl-c always quits and cannot be bound"#
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
        assert_eq!(keys.chords(Context::Global, Act::Quit), [chord("ctrl-q")]);
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
    /// refused, either way.
    /// MOD-67 M3: `esc` also dismisses in the Settings browse stacks and answers no in their
    /// questions, so those collisions follow, each pair reported once, in `DECLARED` order.
    #[test]
    fn a_collision_over_an_overlay_is_found_in_the_overlay_stack() {
        let found = errors("[global]\nhelp = [\"esc\"]\n");
        assert_eq!(
            found[0],
            (
                2,
                r#"[global] help = "esc": "esc" is already overlay.close (default) over an overlay"#
                    .to_owned()
            )
        );
        assert_eq!(
            found[1..],
            [
                (
                    2,
                    r#"[global] help = "esc": "esc" is already common.dismiss (default) in Settings > Agents"#
                        .to_owned()
                ),
                (
                    2,
                    r#"[global] help = "esc": "esc" is already confirm.no (default) in the Agents install question"#
                        .to_owned()
                ),
                // MOD-67 M4: the first stack where `esc` goes back and help is unfiltered.
                (
                    2,
                    r#"[global] help = "esc": "esc" is already common.back (default) in the Library import report"#
                        .to_owned()
                ),
                // MOD-67 M5: and `esc` ends a chat (the Chat replay's `common.back` is reported).
                (
                    2,
                    r#"[global] help = "esc": "esc" is already chat.end (default) in Chat"#
                        .to_owned()
                ),
            ]
        );
        assert_eq!(
            errors("[overlay]\nclose = [\"esc\", \"f1\"]\n"),
            one(
                2,
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
        assert_eq!(check(keys, SHADOWING, STATE_GUARDED, FIXED), []);
        for entry in SHADOWING {
            let fewer: Vec<_> = SHADOWING.iter().copied().filter(|e| e != entry).collect();
            assert!(
                !check(keys, &fewer, STATE_GUARDED, FIXED).is_empty(),
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
                !check(keys, SHADOWING, &fewer, FIXED).is_empty(),
                "STATE_GUARDED {entry:?} is not demanded"
            );
        }
        // MOD-67 M5 R1 M1: each act a `FIXED` entry shadows by design has a default on its keys.
        for (index, entry) in FIXED.iter().enumerate() {
            for act in entry.shadowed {
                let fewer: Vec<Act> = entry
                    .shadowed
                    .iter()
                    .copied()
                    .filter(|a| a != act)
                    .collect();
                let mut table: Vec<Fixed> = FIXED.to_vec();
                table[index].shadowed = fewer.leak();
                assert!(
                    !check(keys, SHADOWING, STATE_GUARDED, &table).is_empty(),
                    "FIXED[{index}] shadowed {act:?} is not demanded"
                );
            }
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
        assert_eq!(
            errors("[migration]\nyes = [\"y\", \"esc\"]\n"),
            // `migration.no` is in the entry's own table, so it gives `esc` up (MOD-12 M3 R1 H1);
            // `overlay.close` is another table's, so it is still refused.
            one(
                2,
                r#"[migration] yes = "esc": "esc" is already overlay.close (default) in the migration prompt"#
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

    /// MOD-67 M5 R1 M1: a key a view matches ahead of the resolver (`keys::FIXED`) cannot be
    /// given to an act of its stack. The filter form's option rows keep `Space`, `h`/`l`, the
    /// arrows, `Enter` and `Esc`.
    #[test]
    fn a_rebind_onto_a_filter_form_fixed_key_is_refused() {
        let fixed = |entry: &str, shown: &str| {
            one(
                2,
                &format!(
                    r#"[backlog.filter_form] {entry} = "{shown}": "{shown}" is fixed in the Backlog filter form"#
                ),
            )
        };
        assert_eq!(
            errors("[backlog.filter_form]\nclear = [\"space\"]\n"),
            fixed("clear", "space")
        );
        assert_eq!(
            errors("[backlog.filter_form]\nnext_row = [\"l\"]\n"),
            fixed("next_row", "l")
        );
        assert_eq!(
            errors("[backlog.filter_form]\nclear = [\"enter\"]\n"),
            fixed("clear", "enter")
        );
        assert_eq!(
            errors("[backlog.filter_form]\nprev_row = [\"k\", \"left\"]\n"),
            fixed("prev_row", "left")
        );
        // A shared act the form offers is refused there too, once, naming the first stack.
        // MOD-67 M6: the first declared stack that keeps the key fixed.
        assert_eq!(
            errors("[form]\nnext_field = [\"tab\", \"right\"]\n"),
            one(
                2,
                r#"[form] next_field = "right": "right" is fixed in a Requirements form"#
            )
        );
    }

    /// MOD-67 M5 R1 M1: the item form's pickers keep `Left`/`Right`, `h`/`l`, `Enter`, `Esc`; the
    /// Runs close-out warning keeps its resolution keys.
    #[test]
    fn a_rebind_onto_an_item_form_or_runs_question_fixed_key_is_refused() {
        // MOD-67 M6: the first declared stack that keeps the key fixed.
        assert_eq!(
            errors("[form]\nsave = [\"ctrl-s\", \"esc\"]\n"),
            one(
                2,
                r#"[form] save = "esc": "esc" is fixed in the Attachments form"#
            )
        );
        assert_eq!(
            errors("[backlog.runs]\nyes = [\"y\", \"right\"]\n"),
            one(
                2,
                r#"[backlog.runs] yes = "right": "right" is fixed in a Runs question"#
            )
        );
    }

    /// MOD-67 M5 R1 fix: a view matches its fixed keys with `keys::is_plain`, which lets SHIFT
    /// through, and a named key keeps its SHIFT in a chord, so `shift-left` is as fixed as `left`.
    #[test]
    fn a_shifted_named_fixed_key_is_refused_too() {
        assert_eq!(
            errors("[backlog.filter_form]\nclear = [\"shift-left\"]\n"),
            one(
                2,
                r#"[backlog.filter_form] clear = "shift-left": "shift-left" is fixed in the Backlog filter form"#
            )
        );
        assert_eq!(
            errors("[backlog.filter_form]\nnext_row = [\"shift-right\"]\n"),
            one(
                2,
                r#"[backlog.filter_form] next_row = "shift-right": "shift-right" is fixed in the Backlog filter form"#
            )
        );
        // MOD-67 M6: the first declared stack that keeps the key fixed.
        assert_eq!(
            errors("[form]\nsave = [\"ctrl-s\", \"shift-left\"]\n"),
            one(
                2,
                r#"[form] save = "shift-left": "shift-left" is fixed in a Requirements form"#
            )
        );
        assert_eq!(
            errors("[backlog.runs]\nyes = [\"y\", \"shift-right\"]\n"),
            one(
                2,
                r#"[backlog.runs] yes = "shift-right": "shift-right" is fixed in a Runs question"#
            )
        );
        // An unfixed shifted key is still free: `shift-up` is no option key.
        assert_eq!(
            errors("[backlog.filter_form]\nclear = [\"x\", \"shift-up\"]\n"),
            []
        );
    }

    /// MOD-67 M6 D5: the Requirements forms' priority keys and the Attachments form's activation
    /// keys are matched ahead of the resolver, so no act of their stacks may take one.
    #[test]
    fn a_rebind_onto_a_requirements_priority_or_attachment_key_is_refused() {
        assert_eq!(
            errors("[requirements]\nnext_field = [\"tab\", \"right\"]\n"),
            one(
                2,
                r#"[requirements] next_field = "right": "right" is fixed in a Requirements form"#
            )
        );
        assert_eq!(
            errors("[form]\nnext_field = [\"tab\", \"enter\"]\n"),
            one(
                2,
                r#"[form] next_field = "enter": "enter" is fixed in the Attachments form"#
            )
        );
        assert_eq!(
            errors("[form]\nsave = [\"ctrl-s\", \"left\"]\n"),
            one(
                2,
                r#"[form] save = "left": "left" is fixed in a Requirements form"#
            )
        );
    }

    /// MOD-67 M5 R1 M1: a parked request takes the plain digits in Backlog > Runs and in Chat; only
    /// the tab select sits on them by design.
    #[test]
    fn a_rebind_onto_a_permission_digit_is_refused() {
        assert_eq!(
            errors("[global]\nselect_tab_1 = []\n[backlog.runs]\napprove = [\"a\", \"1\"]\n"),
            one(
                4,
                r#"[backlog.runs] approve = "1": "1" is fixed in Backlog > Runs"#
            )
        );
        assert_eq!(
            errors("[global]\nselect_tab_1 = []\n[chat]\nthoughts = [\"t\", \"1\"]\n"),
            one(4, r#"[chat] thoughts = "1": "1" is fixed in Chat"#)
        );
        // The tab select may move between digits: the swallow shadows it by design.
        assert_eq!(
            errors("[global]\nselect_tab_1 = [\"2\"]\nselect_tab_2 = [\"1\"]\n"),
            []
        );
    }

    /// MOD-67 M6 D6 (PA-13): a line's chord that collides and is also fixed in the same stack
    /// is reported once, by the collision; the fixed pass does not repeat it.
    #[test]
    fn a_line_both_colliding_and_fixed_is_reported_once() {
        assert_eq!(
            errors("version = 1\n[backlog.runs]\napprove = [\"a\", \"1\"]\n"),
            one(
                3,
                r#"[backlog.runs] approve = "1": "1" is already global.select_tab_1 (default) in Backlog > Runs"#
            )
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

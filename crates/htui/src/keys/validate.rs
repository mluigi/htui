//! The key file's semantic checks over the merged keys (MOD-67 M2 D8 steps 5-6; ANA-26 §7.4
//! step 7): collisions in every context and in every declared stack, and printable chords on
//! actions offered while a field captures. Before them, [`give_way`] lets an entry of the file
//! take its chord from an action the file leaves at its default (MOD-12 M3 R1 H1).
//!
//! MOD-67 M3: the per-context pass skips view contexts (PA-4: a view context holds rows of modes
//! that never meet; the stack pass checks every declared stack), the stack pass allows a
//! reviewed [`SHADOWING`] pair (D11, PA-2), and a derived `VIEW_DEFAULTS` row the file did not
//! set is reported as the shared row it follows, on the user's line.

use super::{
    Act, Context, DECLARED, KeyChord, KeyFileError, Keys, Lost, Row, SHADOWING, STATE_GUARDED,
    quote,
};

/// The user's binding wins (MOD-12 M3 R1 H1, maintainer 2026-10-08): an entry of the file that
/// shares a chord with an action in **its own context** that the file leaves at its default
/// takes the chord, and that action loses it (unbound when it has no other). A
/// [`STATE_GUARDED`] pair that shares the chord by default keeps sharing it, and
/// `overlay.close` never gives way (it must keep a chord).
///
/// Across contexts nothing gives way, and [`validate`]'s stack rules stand: two entries on one
/// chord are an error, and so is a chord an entry shares with another context's default only
/// where a [`DECLARED`] stack makes both candidates (today the overlay stack: `[global] help`
/// against `overlay.close`, either way). A context no declared stack composes with the entry's
/// is not compared: `[global] quit = ["j"]` loads, and `list.down` keeps `j`.
pub(super) fn give_way(keys: &mut Keys) {
    let mut taken: Vec<(usize, Lost)> = Vec::new();
    for entry in keys.rows.iter().filter(|row| row.entry.is_some()) {
        for (index, default) in keys.rows.iter().enumerate() {
            if default.entry.is_some()
                || default.context != entry.context
                || default.act == Act::OverlayClose
            {
                continue;
            }
            for &chord in entry.chords.iter().filter(|c| default.chords.contains(c)) {
                if !allowed(STATE_GUARDED, entry, default, chord) {
                    taken.push((
                        index,
                        Lost {
                            chord,
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

/// Every collision and capture error in `keys`, reported on the user's line (D8 steps 5-6).
/// The compiled defaults give none (`the_compiled_defaults_validate`).
///
/// Three passes, in this order: two actions sharing a chord in one shared or global context; two
/// candidates for one chord in a [`DECLARED`] stack; a printable chord on an `in_capture`
/// action. A pair and chord seen by two passes is reported once. A [`STATE_GUARDED`] pair may
/// share a chord only when it is a catalogue default of both actions (PA-2), and so may a
/// [`SHADOWING`] pair whose first act's layer is the narrower (MOD-67 D11).
#[must_use]
pub fn validate(keys: &Keys) -> Vec<KeyFileError> {
    check(keys, SHADOWING, STATE_GUARDED)
}

/// [`validate`] over explicit allow-lists: the demand tests drop one entry at a time.
fn check(keys: &Keys, shadowing: &[(Act, Act)], guarded: &[(Act, Act)]) -> Vec<KeyFileError> {
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
                    if !shadowed {
                        check.report(first, second, chord, phrase);
                    }
                }
            }
        }
    }
    for row in &keys.rows {
        if !row.act.spec().is_some_and(|spec| spec.in_capture)
            || !std::ptr::eq(source(keys, row), row)
        {
            continue; // a derived row's chords are its shared row's, reported there
        }
        for chord in row.chords.iter().filter(|chord| chord.is_printable()) {
            let shown = quote(&chord.spec());
            check.errors.push(KeyFileError {
                line: row.line.unwrap_or(0),
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
            line: reported_source.line.or(reported_source.entry).unwrap_or(0),
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
    use crate::keys::{Act, Context, KeyChord, Keys, SHADOWING, STATE_GUARDED, load_str};

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
    /// adding `backspace` to `back` keeps that share allowed, and `dismiss` keeps `esc`; a chord
    /// the user gives both is not.
    #[test]
    fn a_state_guarded_pair_is_allowed_only_on_a_chord_both_have_by_default() {
        assert_eq!(errors("[common]\nback = [\"esc\", \"backspace\"]\n"), []);
        let keys = load_str("[common]\nback = [\"esc\", \"backspace\"]\n").expect("loads");
        assert_eq!(keys.chords(Context::Common, Act::Dismiss), [chord("esc")]);
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
        assert_eq!(check(keys, SHADOWING, STATE_GUARDED), []);
        for entry in SHADOWING {
            let fewer: Vec<_> = SHADOWING.iter().copied().filter(|e| e != entry).collect();
            assert!(
                !check(keys, &fewer, STATE_GUARDED).is_empty(),
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
                !check(keys, SHADOWING, &fewer).is_empty(),
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
            one(
                2,
                r#"[global] quit = "x": "x" is already settings.agents.cancel (default) in Settings > Agents"#
            )
        );
    }

    /// A derived view row follows its shared row, so a printable shared chord is reported once,
    /// on the shared line.
    #[test]
    fn a_printable_shared_chord_is_reported_once_on_the_shared_line() {
        let tail = "is typed text while a field captures: bind a ctrl or alt chord or a named key";
        assert_eq!(
            errors("[form]\nnext_field = [\"x\"]\n"),
            one(2, &format!(r#"[form] next_field = "x": "x" {tail}"#))
        );
    }
}

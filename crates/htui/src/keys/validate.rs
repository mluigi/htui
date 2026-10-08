//! The key file's semantic checks over the merged keys (MOD-67 M2 D8 steps 5-6; ANA-26 §7.4
//! step 7): collisions in every context and in every declared stack, and printable chords on
//! actions offered while a field captures.
//!
//! MOD-67 M3: the per-context pass skips view contexts (PA-4: a view context holds rows of modes
//! that never meet; the stack pass checks every declared stack), the stack pass allows a
//! reviewed [`SHADOWING`] pair (D11, PA-2), and a derived `VIEW_DEFAULTS` row the file did not
//! set is reported as the shared row it follows, on the user's line.

use super::{
    Act, Context, DECLARED, KeyChord, KeyFileError, Keys, Row, SHADOWING, STATE_GUARDED, quote,
};

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
            .map_or_else(|| "default".to_owned(), |line| format!("line {line}"));
        let shown = quote(&chord.spec());
        let other_name = other.act.spec().map_or("", |spec| spec.name);
        self.errors.push(KeyFileError {
            line: reported_source.line.unwrap_or(0),
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
    use super::validate;
    use crate::keys::{Keys, load_str};

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
    fn two_actions_sharing_a_chord_in_one_context_are_refused_on_the_users_line() {
        assert_eq!(
            errors("[list]\ntop = [\"j\"]\n"),
            one(
                2,
                r#"[list] top = "j": "j" is already list.down (default) in [list]"#
            )
        );
    }

    #[test]
    fn a_shared_context_no_view_offers_yet_is_still_checked() {
        assert_eq!(
            errors("[confirm]\nyes = [\"n\"]\n"),
            one(
                2,
                r#"[confirm] yes = "n": "n" is already confirm.no (default) in [confirm]"#
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
    }

    #[test]
    fn a_collision_seen_by_two_checks_is_reported_once() {
        assert_eq!(
            errors("[global]\nquit = [\"f1\"]\n"),
            one(
                2,
                r#"[global] quit = "f1": "f1" is already global.help (default) in [global]"#
            )
        );
    }

    #[test]
    fn a_state_guarded_default_pair_is_allowed() {
        assert_eq!(errors("[common]\nback = [\"esc\"]\n"), []);
    }

    /// PA-2: the allow-list is per chord. `esc` is a default of both `back` and `dismiss`, so
    /// adding `backspace` to `back` keeps that share allowed; a chord the user gives both is not.
    #[test]
    fn a_state_guarded_pair_is_allowed_only_on_a_chord_both_have_by_default() {
        assert_eq!(errors("[common]\nback = [\"esc\", \"backspace\"]\n"), []);
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
}

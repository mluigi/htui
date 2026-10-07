//! The key file's semantic checks over the merged keys (MOD-67 M2 D8 steps 5-6; ANA-26 §7.4
//! step 7): collisions in every context and in every declared stack, and printable chords on
//! actions offered while a field captures.

use super::{Act, Context, DECLARED, KeyChord, KeyFileError, Keys, Row, STATE_GUARDED, quote};

/// Every collision and capture error in `keys`, reported on the user's line (D8 steps 5-6).
/// The compiled defaults give none (`the_compiled_defaults_validate`).
///
/// Three passes, in this order: two actions sharing a chord in one context; two candidates for
/// one chord in a [`DECLARED`] stack; a printable chord on an `in_capture` action. A pair and
/// chord seen by two passes is reported once. A [`STATE_GUARDED`] pair may share a chord only
/// when it is a catalogue default of both actions (PA-2).
#[must_use]
pub fn validate(keys: &Keys) -> Vec<KeyFileError> {
    let mut check = Check {
        reported: Vec::new(),
        errors: Vec::new(),
    };
    for (i, first) in keys.rows.iter().enumerate() {
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
        for layer in stack.layers() {
            let admitted = keys
                .rows
                .iter()
                .filter(|row| row.context == layer.context() && layer.admits(row.act));
            for &chord in admitted.flat_map(|row| &row.chords) {
                if !chords.contains(&chord) {
                    chords.push(chord);
                }
            }
        }
        for chord in chords {
            let rows: Vec<&Row> = keys
                .actions(stack, chord)
                .into_iter()
                .filter_map(|act| keys.resolve_row(stack, act))
                .collect();
            for (i, first) in rows.iter().enumerate() {
                for second in &rows[i + 1..] {
                    check.report(first, second, chord, phrase);
                }
            }
        }
    }
    for row in &keys.rows {
        if !row.act.spec().is_some_and(|spec| spec.in_capture) {
            continue;
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

/// The validator's state: the pairs already reported and the errors so far.
struct Check {
    /// `((context, act), (context, act), chord)` of every reported collision.
    reported: Vec<(Slot, Slot, KeyChord)>,
    errors: Vec<KeyFileError>,
}

impl Check {
    /// Reports that `first` and `second` both bind `chord` (`place` ends the message), unless
    /// the pair was reported already or is allowed. The reported row is the one that added
    /// `chord` (it is not that row's catalogue default) when exactly one did (review L1);
    /// otherwise the one with the later line (`None`, a default, before any line), `second` on
    /// a tie.
    fn report(&mut self, first: &Row, second: &Row, chord: KeyChord, place: &str) {
        let a = (first.context, first.act);
        let b = (second.context, second.act);
        if self
            .reported
            .iter()
            .any(|&(x, y, c)| c == chord && ((x, y) == (a, b) || (x, y) == (b, a)))
            || allowed(first, second, chord)
        {
            return;
        }
        self.reported.push((a, b, chord));
        let (reported, other) = match (added(first, chord), added(second, chord)) {
            (true, false) => (first, second),
            (false, true) => (second, first),
            _ if first.line > second.line => (first, second),
            _ => (second, first),
        };
        let origin = other
            .line
            .map_or_else(|| "default".to_owned(), |line| format!("line {line}"));
        let shown = quote(&chord.spec());
        let other_name = other.act.spec().map_or("", |spec| spec.name);
        self.errors.push(KeyFileError {
            line: reported.line.unwrap_or(0),
            message: format!(
                "{} = {shown}: {shown} is already {}.{other_name} ({origin}) {place}",
                subject(reported),
                other.context.table(),
            ),
        });
    }
}

/// Whether `first` and `second` may share `chord` (D8 step 5, PA-2): the pair is on
/// [`STATE_GUARDED`] and `chord` is a catalogue default of both.
fn allowed(first: &Row, second: &Row, chord: KeyChord) -> bool {
    let guarded = STATE_GUARDED
        .iter()
        .any(|&pair| pair == (first.act, second.act) || pair == (second.act, first.act));
    guarded && !added(first, chord) && !added(second, chord)
}

/// Whether `row` binds `chord` beyond its catalogue defaults: the user added it.
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
            errors("[global]\nquit = [\"x\"]\nhelp = [\"x\"]\n"),
            one(
                3,
                r#"[global] help = "x": "x" is already global.quit (line 2) in [global]"#
            )
        );
    }

    /// Review L1: the error sits on the row that added the chord, not on the later row that
    /// keeps it as a default.
    #[test]
    fn a_collision_is_reported_where_the_chord_was_added() {
        assert_eq!(
            errors("[global]\nquit = [\"w\"]\nworkspaces = [\"w\", \"z\"]\n"),
            one(
                2,
                r#"[global] quit = "w": "w" is already global.workspaces (line 3) in [global]"#
            )
        );
    }

    #[test]
    fn a_collision_over_an_overlay_is_found_in_the_overlay_stack() {
        assert_eq!(
            errors("[global]\nhelp = [\"esc\"]\n"),
            one(
                2,
                r#"[global] help = "esc": "esc" is already overlay.close (default) over an overlay"#
            )
        );
    }

    #[test]
    fn a_collision_seen_by_two_checks_is_reported_once() {
        assert_eq!(
            errors("[global]\nquit = [\"w\"]\n"),
            one(
                2,
                r#"[global] quit = "w": "w" is already global.workspaces (default) in [global]"#
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
            errors("[common]\ndismiss = [\"x\"]\nback = [\"x\"]\n"),
            one(
                3,
                r#"[common] back = "x": "x" is already common.dismiss (line 2) in [common]"#
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

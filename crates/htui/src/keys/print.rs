//! `htui --print-keys` (MOD-67 M2 D10; ANA-26 §7.2, §7.5): the keys in force as one complete,
//! commented `keys.toml` that [`load_str`](super::load_str) reads back to the same keys.

use std::fmt::Write as _;

use super::{Context, Keys, Row, quote};

/// The header and `version = 1`. Constant, so the output is the same on every machine (B-13).
const HEADER: &str = r"# htui key bindings, as `htui --print-keys` prints them. htui reads keys.toml in its config
# directory: ~/.config/htui on Linux, ~/Library/Application Support/htui on macOS, %APPDATA%\htui
# on Windows. List only what you change: a list replaces that action's chords, [] unbinds it.
# ctrl-c always quits and cannot be listed. `htui --default-keys` ignores the file for one run.
version = 1
";

/// The keys in force as a complete `keys.toml`: a constant header, `version = 1`, then one table
/// per context in `Context::ALL` order (a context with no row prints nothing), one padded
/// `name = ["chord", …]  # help` line per action. A view's table lists its own rows, then its
/// derived `VIEW_DEFAULTS` rows, then its overrides of shared verbs (MOD-67 M3), each under the
/// shared verb's name. A line the file changed ends `(changed)`; a derived row the file did not
/// set never does; an unbound action is `name = []`. A line left at its default that gave chords
/// to the file's entries (MOD-12 M3 R1 H1) ends `(unbound by quit)` when it has none left, else
/// `(lost "j" to top)`.
#[must_use]
pub fn print(keys: &Keys) -> String {
    let mut out = HEADER.to_owned();
    for &context in Context::ALL {
        let rows: Vec<(&str, String, &str, String)> = keys
            .rows
            .iter()
            .filter(|row| row.context == context)
            .map(|row| {
                let value = row
                    .chords
                    .iter()
                    .map(|chord| quote(&chord.spec()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let name = row.act.spec().map_or("", |spec| spec.name);
                (name, format!("[{value}]"), row.help, mark(row))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let name_width = rows
            .iter()
            .map(|(name, ..)| name.chars().count())
            .max()
            .unwrap_or(0);
        let lefts: Vec<String> = rows
            .iter()
            .map(|(name, value, ..)| format!("{name:<name_width$} = {value}"))
            .collect();
        let left_width = lefts
            .iter()
            .map(|left| left.chars().count())
            .max()
            .unwrap_or(0);
        let _ = write!(out, "\n[{}]\n", context.table());
        for (left, (_, _, help, mark)) in lefts.iter().zip(&rows) {
            let _ = writeln!(out, "{left:<left_width$}  # {help}{mark}");
        }
    }
    out
}

/// What follows a row's help: ` (changed)` for the file's change, ` (unbound by quit)` or
/// ` (lost "j" to top)` for a default that gave way to the file's entries (` (unbound by
/// global.quit)` for a taker in another table), else nothing.
fn mark(row: &Row) -> String {
    if row.line.is_some() {
        return " (changed)".to_owned();
    }
    if row.lost.is_empty() {
        return String::new();
    }
    // A taker from another table (the abort's, MOD-57 M2 R1 M-1) is named with its table.
    let name = |lost: &super::Lost| {
        let name = lost.to.spec().map_or("", |spec| spec.name);
        if lost.from == row.context {
            name.to_owned()
        } else {
            format!("{}.{name}", lost.from.table())
        }
    };
    if row.chords.is_empty() {
        let mut takers: Vec<String> = Vec::new();
        for lost in &row.lost {
            if !takers.contains(&name(lost)) {
                takers.push(name(lost));
            }
        }
        format!(" (unbound by {})", takers.join(", "))
    } else {
        let taken = row
            .lost
            .iter()
            .map(|lost| format!("{} to {}", quote(&lost.chord.spec()), name(lost)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" (lost {taken})")
    }
}

#[cfg(test)]
mod tests {
    use super::print;
    use crate::keys::{Keys, load_str, quote};

    /// Blueprint §4's lines 1-26 of the default print, and the queue row (MOD-12 M3 D8): 27.
    const DEFAULT_HEAD: &str = r#"# htui key bindings, as `htui --print-keys` prints them. htui reads keys.toml in its config
# directory: ~/.config/htui on Linux, ~/Library/Application Support/htui on macOS, %APPDATA%\htui
# on Windows. List only what you change: a list replaces that action's chords, [] unbinds it.
# ctrl-c always quits and cannot be listed. `htui --default-keys` ignores the file for one run.
version = 1

[global]
quit         = ["q"]        # quit
next_tab     = ["tab"]      # next tab
prev_tab     = ["backtab"]  # previous tab
select_tab_1 = ["1"]        # select tab
select_tab_2 = ["2"]        # select tab
select_tab_3 = ["3"]        # select tab
select_tab_4 = ["4"]        # select tab
select_tab_5 = ["5"]        # select tab
select_tab_6 = ["6"]        # select tab
select_tab_7 = ["7"]        # select tab
select_tab_8 = ["8"]        # select tab
select_tab_9 = ["9"]        # select tab
help         = ["?", "f1"]  # help
workspaces   = ["w"]        # workspaces
find         = ["ctrl-f"]   # find
waiting      = ["ctrl-w"]   # waiting
queue        = ["ctrl-q"]   # queue

[overlay]
close = ["esc"]  # close"#;

    #[test]
    fn the_default_print_starts_with_the_header_global_and_overlay() {
        let printed = print(Keys::compiled());
        let head: Vec<&str> = printed.lines().take(27).collect();
        assert_eq!(head, DEFAULT_HEAD.lines().collect::<Vec<_>>());
        assert!(
            printed.ends_with(
                "[chat]\ncompose    = [\"i\", \"enter\"]  # compose\n\
                       next_agent = [\"a\"]           # next agent\n\
                       end        = [\"esc\"]         # end chat (twice)\n\
                       thoughts   = [\"t\"]           # thoughts\n\
                       top        = [\"g\"]           # top\n\
                       bottom     = [\"G\"]           # bottom\n\
                       \n\
                       [queue]\n\
                       move_down = [\"J\"]      # move down\n\
                       move_up   = [\"K\"]      # move up\n\
                       pause     = [\"P\"]      # pause / resume\n\
                       dequeue   = [\"Q\"]      # dequeue\n\
                       open      = [\"enter\"]  # open\n\
                       \n\
                       [picker]\n\
                       open       = [\"enter\", \"l\"]      # open directory\n\
                       parent     = [\"h\", \"backspace\"]  # parent directory\n\
                       choose     = [\"s\"]               # choose\n\
                       choose_dir = [\"S\"]               # choose this directory\n\
                       go_to      = [\"/\"]               # go to path\n\
                       hidden     = [\".\"]               # show hidden\n"
            ),
            "{printed}"
        );
        assert!(!printed.ends_with("\n\n"));
        assert!(printed.lines().all(|line| line == line.trim_end()));
        assert!(printed.lines().all(|line| line.chars().count() <= 100));
    }

    #[test]
    fn the_default_print_reads_back_as_the_defaults() {
        let printed = print(Keys::compiled());
        let keys = load_str(&printed).expect("the default print loads");
        assert_eq!(&keys, Keys::compiled());
        assert!(!print(&keys).contains("(changed)"));
    }

    #[test]
    fn a_changed_table_round_trips_and_keeps_its_marks() {
        let keys = load_str(
            "[global]\nquit = [\"ctrl-y\"]\nworkspaces = []\n\n[overlay]\nclose = [\"esc\", \"f2\"]\n",
        )
        .expect("the file loads");
        let printed = print(&keys);
        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert_eq!(print(&reloaded), printed);
        for line in [
            "quit         = [\"ctrl-y\"]   # quit (changed)",
            "workspaces   = []           # workspaces (changed)",
            "close = [\"esc\", \"f2\"]  # close (changed)",
        ] {
            assert!(printed.lines().any(|printed| printed == line), "{line}");
        }
        assert_eq!(printed.matches("(changed)").count(), 3);
    }

    /// MOD-57 M2 E9: an `[editor]` change prints, reads back to the same keys and is marked.
    #[test]
    fn a_changed_editor_table_round_trips() {
        let keys = load_str("[editor]\nfocus = [\"f12\"]\n").expect("the file loads");
        let printed = print(&keys);
        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert_eq!(print(&reloaded), printed);
        let focus = printed
            .lines()
            .find(|line| line.starts_with("focus "))
            .unwrap_or_else(|| panic!("no focus row in {printed}"));
        assert!(focus.starts_with(r#"focus = ["f12"]"#), "{focus}");
        assert!(focus.ends_with("# editor focus (changed)"), "{focus}");
        assert_eq!(printed.matches("(changed)").count(), 1);
    }

    /// MOD-12 M3 R1 H1: a row that gave a chord to an entry is marked with the entry's action,
    /// not `(changed)`: the file never named it. The print reads back to the same keys, with the
    /// row now an entry of its own.
    #[test]
    fn a_row_that_gave_way_names_the_entry_that_took_its_chord() {
        let keys = load_str("[global]\nquit = [\"ctrl-q\"]\n\n[list]\ntop = [\"j\"]\n")
            .expect("the file loads");
        let printed = print(&keys);
        let line = |name: &str| {
            printed
                .lines()
                .find(|line| line.starts_with(&format!("{name} ")))
                .unwrap_or_else(|| panic!("no {name} row in {printed}"))
        };
        assert!(line("queue").contains("= []"), "{printed}");
        assert!(
            line("queue").ends_with("# queue (unbound by quit)"),
            "{printed}"
        );
        assert!(line("down").contains(r#"= ["down"]"#), "{printed}");
        assert!(
            line("down").ends_with(r#"# down (lost "j" to top)"#),
            "{printed}"
        );
        assert!(line("quit").ends_with("# quit (changed)"), "{printed}");
        assert!(line("top").ends_with("# top (changed)"), "{printed}");
        assert_eq!(printed.matches("(changed)").count(), 2);

        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert!(!print(&reloaded).contains("unbound by"));
        assert!(!print(&reloaded).contains("lost \""));
    }

    /// MOD-67 M4 D8.4: `[editor]` prints and reads back like any table.
    #[test]
    fn the_editor_table_round_trips() {
        let keys = load_str("[editor]\nfocus = [\"ctrl-4\", \"f12\"]\nabort = []\n")
            .expect("the file loads");
        let printed = print(&keys);
        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert_eq!(print(&reloaded), printed);
        let table = printed
            .split("\n\n")
            .find(|table| table.starts_with("[editor]\n"))
            .expect("an editor table");
        assert_eq!(
            table.lines().collect::<Vec<_>>(),
            [
                "[editor]",
                "focus = [\"ctrl-4\", \"f12\"]  # editor focus (changed)",
                "abort = []                 # abort edit (changed)",
            ]
        );
        assert_eq!(printed.matches("(changed)").count(), 2);
    }

    /// MOD-57 M2 R1 M-1: a default that gave its chord to another table's entry names the
    /// taker with its table.
    #[test]
    fn the_abort_row_names_the_global_entry_that_took_its_chord() {
        let keys = load_str("[global]\nquit = [\"ctrl-x\"]\n").expect("the abort gives way");
        let printed = print(&keys);
        let abort = printed
            .lines()
            .find(|line| line.starts_with("abort "))
            .unwrap_or_else(|| panic!("no abort row in {printed}"));
        assert!(abort.contains("= []"), "{printed}");
        assert!(
            abort.ends_with("# abort edit (unbound by global.quit)"),
            "{printed}"
        );
        assert_eq!(load_str(&print(&keys)).as_ref(), Ok(&keys));
    }

    #[test]
    fn quote_escapes_what_toml_needs() {
        assert_eq!(quote("\\"), r#""\\""#);
        assert_eq!(quote("\""), r#""\"""#);
        assert_eq!(quote("\u{1}"), "\"\\u0001\"");
        assert_eq!(quote("ctrl-f"), "\"ctrl-f\"");
        let keys = load_str("[global]\nquit = [\"\\\\\"]\n").expect("a backslash chord loads");
        assert_eq!(load_str(&print(&keys)), Ok(keys));
    }

    /// MOD-67 M6 D9 low 1 (PA-2): a view row the file pins to its compiled derivation while the
    /// file moves its shared row is the user's change, so it prints `(changed)`, and the print
    /// reads back to the same keys and re-prints byte for byte.
    #[test]
    fn a_view_row_pinned_to_its_default_while_its_shared_row_moves_is_marked() {
        let keys = load_str(
            "version = 1\n[form]\nnext_field = [\"ctrl-n\"]\n[settings.agents]\n\
             next_field = [\"tab\", \"down\"]\n",
        )
        .expect("the file loads");
        let printed = print(&keys);
        let agents = printed
            .split("\n\n")
            .find(|table| table.starts_with("[settings.agents]\n"))
            .expect("an agents table");
        let next_field = agents
            .lines()
            .find(|line| line.starts_with("next_field "))
            .unwrap_or_else(|| panic!("no next_field row in {agents}"));
        assert!(next_field.contains(r#"= ["tab", "down"]"#), "{next_field}");
        assert!(
            next_field.ends_with("# next field (changed)"),
            "{next_field}"
        );
        assert_eq!(printed.matches("(changed)").count(), 2, "{printed}");
        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert_eq!(print(&reloaded), printed);
    }

    #[test]
    fn a_view_override_and_a_derived_row_round_trip() {
        let keys = load_str("[form]\nnext_field = [\"ctrl-n\"]\n").expect("the file loads");
        let printed = print(&keys);
        for line in [
            "next_field      = [\"ctrl-n\"]   # next field (changed)",
            "next_field     = [\"ctrl-n\", \"down\"]  # next field",
        ] {
            assert!(
                printed.lines().any(|printed| printed == line),
                "{line}\n{printed}"
            );
        }
        assert_eq!(printed.matches("(changed)").count(), 1);
        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert_eq!(print(&reloaded), printed);

        for (src, value) in [
            ("[settings.boxes]\nreload = [\"f5\"]\n", "[\"f5\"]"),
            ("[settings.boxes]\nreload = []\n", "[]"),
        ] {
            let keys = load_str(src).expect("the override loads");
            let printed = print(&keys);
            let table = printed
                .split("\n\n")
                .find(|table| table.starts_with("[settings.boxes]\n"))
                .expect("a boxes table");
            let last = table.lines().last().expect("a row");
            assert!(
                last.starts_with(&format!("reload      = {value}")),
                "{last}"
            );
            assert!(last.ends_with("# reload (changed)"), "{last}");
            let reloaded = load_str(&printed).expect("the print loads");
            assert_eq!(reloaded, keys);
            assert_eq!(print(&reloaded), printed);
        }
    }
}

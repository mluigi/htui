//! `htui --print-keys` (MOD-67 M2 D10; ANA-26 §7.2, §7.5): the keys in force as one complete,
//! commented `keys.toml` that [`load_str`](super::load_str) reads back to the same keys.

use std::fmt::Write as _;

use super::{Context, Keys, quote};

/// The header and `version = 1`. Constant, so the output is the same on every machine (B-13).
const HEADER: &str = r"# htui key bindings, as `htui --print-keys` prints them. htui reads keys.toml in its config
# directory: ~/.config/htui on Linux, ~/Library/Application Support/htui on macOS, %APPDATA%\htui
# on Windows. List only what you change: a list replaces that action's chords, [] unbinds it.
# ctrl-c always quits and cannot be listed. `htui --default-keys` ignores the file for one run.
version = 1
";

/// The keys in force as a complete `keys.toml`: a constant header, `version = 1`, then one table
/// per context in catalogue order, one padded `name = ["chord", …]  # help` line per action. A
/// line the file changed ends `(changed)`; an unbound action is `name = []`.
#[must_use]
pub fn print(keys: &Keys) -> String {
    let mut out = HEADER.to_owned();
    for &context in Context::ALL {
        let rows: Vec<(&str, String, &str, bool)> = keys
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
                (name, format!("[{value}]"), row.help, row.line.is_some())
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
        for (left, (_, _, help, changed)) in lefts.iter().zip(&rows) {
            let mark = if *changed { " (changed)" } else { "" };
            let _ = writeln!(out, "{left:<left_width$}  # {help}{mark}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::print;
    use crate::keys::{Keys, load_str, quote};

    /// Blueprint §4's lines 1-26 of the default print.
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

[overlay]
close = ["esc"]  # close"#;

    #[test]
    fn the_default_print_starts_with_the_header_global_and_overlay() {
        let printed = print(Keys::compiled());
        let head: Vec<&str> = printed.lines().take(26).collect();
        assert_eq!(head, DEFAULT_HEAD.lines().collect::<Vec<_>>());
        assert!(printed.ends_with("[waiting]\nopen = [\"enter\"]  # open step\n"));
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
            "[global]\nquit = [\"ctrl-q\"]\nworkspaces = []\n\n[overlay]\nclose = [\"esc\", \"f2\"]\n",
        )
        .expect("the file loads");
        let printed = print(&keys);
        let reloaded = load_str(&printed).expect("the print loads");
        assert_eq!(reloaded, keys);
        assert_eq!(print(&reloaded), printed);
        for line in [
            "quit         = [\"ctrl-q\"]   # quit (changed)",
            "workspaces   = []           # workspaces (changed)",
            "close = [\"esc\", \"f2\"]  # close (changed)",
        ] {
            assert!(printed.lines().any(|printed| printed == line), "{line}");
        }
        assert_eq!(printed.matches("(changed)").count(), 3);
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
}

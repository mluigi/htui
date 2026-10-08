//! `keys.toml` to [`Keys`] (MOD-67 M2; ANA-26 §7.1, §7.4 steps 1-6, §7.5): find the file, parse
//! it with spans, check names and chords strictly, merge it over the catalogue, and report every
//! error with its line. [`validate()`] adds the checks over the merged keys.

use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use super::{
    Act, ActionSpec, CATALOGUE, CTRL_C, ChordError, Context, DECLARED, KeyChord, Keys, quote,
    validate,
};

/// The key file's name under the config root.
pub const FILE_NAME: &str = "keys.toml";

/// One error in a key file: a line and what is wrong there (ANA-26 §7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFileError {
    /// The 1-based line. `0` only for a fault in the compiled defaults, which the validator's
    /// tests rule out.
    pub line: usize,
    /// Everything after `keys.toml:LINE: `, e.g.
    /// `[global] quit = "ctrl-c": ctrl-c always quits and cannot be bound`.
    pub message: String,
}

/// Why htui will not start with the key file it was given (MOD-67 M2 D9): `main` exits 2 and
/// keeps it out of Sentry, since the report carries the user's home path.
#[derive(Debug)]
pub enum KeysError {
    /// The file was read and has at least one error.
    Invalid {
        /// The file as htui looked it up.
        path: PathBuf,
        /// Every error, sorted by line.
        errors: Vec<KeyFileError>,
    },
    /// The file exists (or was named with `--keys`) and could not be read.
    Unreadable {
        /// The file as htui looked it up.
        path: PathBuf,
        /// Why reading failed; part of the message, not a `source()`.
        source: std::io::Error,
    },
}

impl KeysError {
    /// The exit status `main` ends with: always 2 (a refusal, like `WorkerExit::Refused`).
    #[must_use]
    pub const fn code(&self) -> u8 {
        2
    }
}

/// The report's last line, after every error (ANA-26 §7.5).
const HINT: &str = "Fix the file, or run `htui --default-keys` to start with the default keys.";

/// The refusal of `ctrl-c`, however it is spelled (D8 step 4).
const CTRL_C_REFUSED: &str = "ctrl-c always quits and cannot be bound";

/// ANA-26 §7.5's report: no trailing newline, since `main` prints it with `eprintln!`.
impl std::fmt::Display for KeysError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { path, errors } => {
                let file = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let count = errors.len();
                let plural = if count == 1 { "" } else { "s" };
                write!(f, "{} has {count} error{plural}:", path.display())?;
                for error in errors {
                    write!(f, "\n  {file}:{}: {}", error.line, error.message)?;
                }
                write!(f, "\n{HINT}")
            }
            Self::Unreadable { path, source } => {
                write!(f, "cannot read {}: {source}\n{HINT}", path.display())
            }
        }
    }
}

impl std::error::Error for KeysError {}

/// Parses and checks a key file's text (MOD-67 M2 D1, D2, D7, D8): `Ok` with the merged keys,
/// or every error sorted by line. A TOML syntax error is the only error then (D8 step 1).
///
/// # Errors
///
/// Every [`KeyFileError`] the text has, sorted by line (stable: same-line errors keep walk order).
pub fn load_str(src: &str) -> Result<Keys, Vec<KeyFileError>> {
    let root = match DeTable::parse(src) {
        Ok(root) => root,
        Err(error) => {
            return Err(vec![KeyFileError {
                line: error.span().map_or(1, |span| line_of(src, span.start)),
                message: format!("not valid TOML: {}", error.message()),
            }]);
        }
    };
    let mut loader = Loader {
        src,
        keys: Keys::defaults(),
        errors: Vec::new(),
        set: Vec::new(),
    };
    for (key, value) in root.get_ref() {
        loader.top_level(key, value);
    }
    let Loader {
        mut keys,
        mut errors,
        set,
        ..
    } = loader;
    // MOD-67 M3 PA-1: the view defaults follow the shared rows the file just set.
    keys.derive(&set);
    errors.extend(validate(&keys));
    errors.sort_by_key(|error| error.line);
    if errors.is_empty() {
        Ok(keys)
    } else {
        Err(errors)
    }
}

/// Reads and checks the file at `path`.
///
/// # Errors
///
/// [`KeysError::Unreadable`] for any read error (missing included: the caller named it), else
/// [`KeysError::Invalid`] with `path`.
pub fn load_path(path: &Path) -> Result<Keys, KeysError> {
    let src = std::fs::read_to_string(path).map_err(|source| KeysError::Unreadable {
        path: path.to_owned(),
        source,
    })?;
    checked(path.to_owned(), &src)
}

/// The keys in force (MOD-67 M2 D3): the defaults with `default_keys`; else the file named by
/// `keys_flag`; else `<root>/keys.toml` when it exists; else the defaults. `root` is `None`
/// when the platform has no config directory. `default_keys` wins if both are given (clap
/// refuses that pair before this runs).
///
/// # Errors
///
/// As [`load_path`] for a named file. For the root's file: a missing file is the defaults (a
/// dangling symlink too), any other read error is [`KeysError::Unreadable`], a bad file is
/// [`KeysError::Invalid`].
pub fn resolve(
    keys_flag: Option<&Path>,
    default_keys: bool,
    root: Option<&Path>,
) -> Result<Keys, KeysError> {
    if default_keys {
        return Ok(Keys::compiled().clone());
    }
    if let Some(path) = keys_flag {
        return load_path(path);
    }
    let Some(root) = root else {
        return Ok(Keys::compiled().clone());
    };
    let path = root.join(FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(src) => checked(path, &src),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Keys::compiled().clone()),
        Err(source) => Err(KeysError::Unreadable { path, source }),
    }
}

/// [`load_str`] with its errors tied to `path`.
fn checked(path: PathBuf, src: &str) -> Result<Keys, KeysError> {
    load_str(src).map_err(|errors| KeysError::Invalid { path, errors })
}

/// The 1-based line of byte `offset` in `src`.
fn line_of(src: &str, offset: usize) -> usize {
    let before = src.get(..offset).unwrap_or(src);
    before.bytes().filter(|&byte| byte == b'\n').count() + 1
}

/// The context whose table is `path` (`global`, `common`, ...).
fn context_named(path: &str) -> Option<Context> {
    Context::ALL
        .iter()
        .copied()
        .find(|context| context.table() == path)
}

/// `written` as a chord the file may bind (D8 steps 3-4), or the reason it may not. A
/// `CtrlCapital` whose suggestion is `ctrl-c` reports the ctrl-c refusal: never suggest a
/// spelling the next check refuses (B-5).
fn chord_of(written: &str) -> Result<KeyChord, String> {
    match KeyChord::parse_strict(written) {
        Ok(chord) if chord == CTRL_C => Err(CTRL_C_REFUSED.to_owned()),
        Err(ChordError::CtrlCapital { suggestion })
            if KeyChord::parse_strict(&suggestion) == Ok(CTRL_C) =>
        {
            Err(CTRL_C_REFUSED.to_owned())
        }
        Ok(chord) => Ok(chord),
        Err(error) => Err(error.to_string()),
    }
}

/// The shared acts `[context]` may override (MOD-67 M3 D10, PA-3): every shared catalogue row
/// whose act some declared stack's `context` layer offers, in catalogue order. Empty for a
/// context that is not a view's.
fn overridable(context: Context) -> impl Iterator<Item = &'static ActionSpec> {
    CATALOGUE.iter().filter(move |spec| {
        context.is_view()
            && spec.context.is_shared()
            && DECLARED
                .iter()
                .any(|(_, stack)| stack.view_admits(context, spec.act))
    })
}

/// One pass over a parsed file: the keys merged so far and every error found.
struct Loader<'s> {
    src: &'s str,
    keys: Keys,
    errors: Vec<KeyFileError>,
    /// Every `(context, act)` an entry resolved to: `Keys::derive` keeps these rows as written.
    set: Vec<(Context, Act)>,
}

impl Loader<'_> {
    fn line(&self, offset: usize) -> usize {
        line_of(self.src, offset)
    }

    fn push(&mut self, line: usize, message: String) {
        self.errors.push(KeyFileError { line, message });
    }

    /// A top-level entry: a table is a context (or a path to one); `version` is the only value
    /// allowed beside them (D1, D2).
    fn top_level(&mut self, key: &Spanned<DeString<'_>>, value: &Spanned<DeValue<'_>>) {
        let name: &str = key.get_ref();
        let line = self.line(key.span().start);
        match value.get_ref() {
            DeValue::Table(table) => self.walk_table(name, line, table),
            DeValue::Integer(integer)
                if name == "version"
                    && i64::from_str_radix(integer.as_str(), integer.radix()) == Ok(1) => {}
            _ if name == "version" => self.push(
                line,
                "version: write version = 1, the only version htui reads".to_owned(),
            ),
            _ if context_named(name).is_some() => self.push(
                line,
                format!("{name}: write [{name}] as a table, one action per line"),
            ),
            _ => self.push(
                line,
                format!("{name}: only version may sit outside a table"),
            ),
        }
    }

    /// The table at `path` (its header on `line`): a context's entries, or a path whose nested
    /// tables extend it with `.` (D1). A path that is no context and holds a value, or nothing,
    /// is reported once.
    fn walk_table(&mut self, path: &str, line: usize, table: &DeTable<'_>) {
        let context = context_named(path);
        let has_values = table
            .values()
            .any(|value| !matches!(value.get_ref(), DeValue::Table(_)));
        if context.is_none() && (has_values || table.is_empty()) {
            let tables = Context::ALL
                .iter()
                .map(|context| context.table())
                .collect::<Vec<_>>()
                .join(", ");
            self.push(
                line,
                format!("[{path}]: no such table; the tables are {tables}"),
            );
        }
        for (key, value) in table {
            match value.get_ref() {
                DeValue::Table(inner) => {
                    let name: &str = key.get_ref();
                    let line = self.line(key.span().start);
                    self.walk_table(&format!("{path}.{name}"), line, inner);
                }
                _ => {
                    if let Some(context) = context {
                        self.entry(context, key, value);
                    }
                }
            }
        }
    }

    /// One `name = value` in `context` (D7, D8 steps 2-4): resolve the name, check every chord,
    /// and merge the chords that passed (B-8).
    fn entry(
        &mut self,
        context: Context,
        key: &Spanned<DeString<'_>>,
        value: &Spanned<DeValue<'_>>,
    ) {
        let name: &str = key.get_ref();
        let table = context.table();
        let key_line = self.line(key.span().start);
        // The context's own row; else, in a view's table, a shared verb it offers (D10).
        let Some(spec) = CATALOGUE
            .iter()
            .find(|spec| spec.context == context && spec.name == name)
            .or_else(|| overridable(context).find(|spec| spec.name == name))
        else {
            let names = CATALOGUE
                .iter()
                .filter(|spec| spec.context == context)
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
                .join(", ");
            let shared = overridable(context)
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
                .join(", ");
            let message = match (names.is_empty(), shared.is_empty()) {
                (_, true) => format!("[{table}] {name}: no such action; [{table}] has {names}"),
                (true, false) => {
                    format!("[{table}] {name}: no such action; [{table}] may override {shared}")
                }
                (false, false) => format!(
                    "[{table}] {name}: no such action; [{table}] has {names}, and may override \
                     {shared}"
                ),
            };
            self.push(key_line, message);
            return;
        };
        let items: Vec<&Spanned<DeValue<'_>>> = match value.get_ref() {
            DeValue::String(_) => vec![value],
            DeValue::Array(array) => array.iter().collect(),
            _ => {
                self.push(
                    key_line,
                    format!(
                        r#"[{table}] {name}: write a chord such as "q", or a list such as ["q", "x"]"#
                    ),
                );
                return;
            }
        };
        let mut chords: Vec<KeyChord> = Vec::new();
        for item in &items {
            let line = self.line(item.span().start);
            let DeValue::String(written) = item.get_ref() else {
                self.push(
                    line,
                    format!(r#"[{table}] {name}: write each chord as a string, such as "q""#),
                );
                continue;
            };
            let shown = quote(written);
            match chord_of(written) {
                Err(reason) => self.push(line, format!("[{table}] {name} = {shown}: {reason}")),
                Ok(chord) if chords.contains(&chord) => self.push(
                    line,
                    format!("[{table}] {name} = {shown}: already listed for this action"),
                ),
                Ok(chord) => chords.push(chord),
            }
        }
        if spec.act == Act::OverlayClose && items.is_empty() {
            self.push(
                key_line,
                format!(
                    "[{table}] {name}: must keep at least one chord: overlays swallow every other key"
                ),
            );
        }
        self.keys.set(context, spec.act, chords, key_line);
        self.set.push((context, spec.act));
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::path::{Path, PathBuf};

    use super::{FILE_NAME, KeyFileError, KeysError, load_path, load_str, resolve};
    use crate::keys::{Act, CATALOGUE, Context, KeyChord, Keys, views};

    /// The full error vector of `src`, as `(line, message)`.
    fn errors(src: &str) -> Vec<(usize, String)> {
        load_str(src)
            .expect_err("the text has errors")
            .into_iter()
            .map(|error| (error.line, error.message))
            .collect()
    }

    fn one(line: usize, message: &str) -> Vec<(usize, String)> {
        vec![(line, message.to_owned())]
    }

    fn chord(spec: &str) -> KeyChord {
        KeyChord::parse_strict(spec).expect("a valid spec")
    }

    fn no_line_anywhere(keys: &Keys) {
        for spec in CATALOGUE {
            assert_eq!(keys.line(spec.context, spec.act), None, "{:?}", spec.act);
        }
    }

    const TABLES: &str = "the tables are global, overlay, list, pane, confirm, form, common, \
                          settings, settings.agents, settings.hierarchy, settings.kinds, \
                          settings.prompt, settings.connection, settings.qdrant, settings.boxes, \
                          settings.personas, settings.secrets, settings.queue, concepts, \
                          switcher, migration, waiting";

    #[test]
    fn an_empty_file_is_the_defaults() {
        let keys = load_str("").expect("an empty file loads");
        assert_eq!(keys, Keys::defaults());
        no_line_anywhere(&keys);
    }

    #[test]
    fn a_file_with_only_version_is_the_defaults() {
        let keys = load_str("version = 1\n").expect("version alone loads");
        assert_eq!(keys, Keys::defaults());
        no_line_anywhere(&keys);
    }

    #[test]
    fn a_bom_and_crlf_still_give_the_right_lines() {
        let src = "\u{feff}version = 1\r\n\r\n[global]\r\nquit = [\r\n  \"z\",\r\n  \"shift-a\",\r\n]\r\n";
        assert_eq!(
            errors(src),
            one(
                6,
                r#"[global] quit = "shift-a": write a shifted letter as "A""#
            )
        );
    }

    #[test]
    fn a_list_replaces_the_defaults_and_a_string_is_a_list_of_one() {
        let keys = load_str("[global]\nquit = \"z\"\nhelp = [\"f1\"]\n").expect("loads");
        assert_eq!(keys.chords(Context::Global, Act::Quit), [chord("z")]);
        assert_eq!(keys.chords(Context::Global, Act::Help), [chord("f1")]);
        assert_eq!(keys.line(Context::Global, Act::Quit), Some(2));
        assert_eq!(keys.line(Context::Global, Act::Help), Some(3));
        for spec in CATALOGUE
            .iter()
            .filter(|spec| !matches!(spec.act, Act::Quit | Act::Help))
        {
            assert_eq!(
                keys.chords(spec.context, spec.act),
                Keys::compiled().chords(spec.context, spec.act),
                "{:?}",
                spec.act
            );
            assert_eq!(keys.line(spec.context, spec.act), None, "{:?}", spec.act);
        }
    }

    #[test]
    fn an_empty_list_unbinds() {
        let keys = load_str("[global]\nworkspaces = []\n").expect("loads");
        assert!(keys.chords(Context::Global, Act::Workspaces).is_empty());
        assert_eq!(keys.line(Context::Global, Act::Workspaces), Some(2));
    }

    #[test]
    fn a_list_equal_to_the_default_is_not_a_change() {
        let keys = load_str("[list]\ndown = [\"j\", \"down\"]\n").expect("loads");
        assert_eq!(keys, Keys::defaults());
        assert_eq!(keys.line(Context::List, Act::ListDown), None);
        let reordered = load_str("[list]\ndown = [\"down\", \"j\"]\n").expect("loads");
        assert_eq!(reordered.line(Context::List, Act::ListDown), Some(2));
        assert_eq!(
            reordered.chords(Context::List, Act::ListDown),
            [chord("down"), chord("j")]
        );
    }

    #[test]
    fn version_must_be_the_integer_1() {
        for value in ["2", "\"1\"", "1.0", "true", "[1]"] {
            assert_eq!(
                errors(&format!("version = {value}\n")),
                one(1, "version: write version = 1, the only version htui reads"),
                "{value}"
            );
        }
        for value in ["0x1", "+1"] {
            assert_eq!(
                load_str(&format!("version = {value}\n")),
                Ok(Keys::defaults()),
                "{value}"
            );
        }
    }

    #[test]
    fn an_unknown_table_lists_the_tables() {
        for (src, path) in [
            ("[globl]\nquit = \"x\"\n", "globl"),
            ("[settings.nothing]\nreload = \"r\"\n", "settings.nothing"),
            ("[global.extra]\nx = \"y\"\n", "global.extra"),
            ("[nothing]\n", "nothing"),
        ] {
            assert_eq!(
                errors(src),
                one(1, &format!("[{path}]: no such table; {TABLES}")),
                "{src:?}"
            );
        }
    }

    #[test]
    fn an_unknown_name_lists_the_tables_actions() {
        assert_eq!(
            errors("[overlay]\nshut = 5\n"),
            one(2, "[overlay] shut: no such action; [overlay] has close")
        );
    }

    #[test]
    fn a_value_must_be_a_chord_or_a_list_of_chords() {
        assert_eq!(
            errors("[global]\nquit = 5\n"),
            one(
                2,
                r#"[global] quit: write a chord such as "q", or a list such as ["q", "x"]"#
            )
        );
        let element = r#"[global] quit: write each chord as a string, such as "q""#;
        assert_eq!(
            errors("[global]\nquit = [\"z\", 5, [\"y\"]]\n"),
            vec![(2, element.to_owned()), (2, element.to_owned())]
        );
        assert_eq!(
            errors("[global]\nquit = { a = \"x\" }\n"),
            one(2, &format!("[global.quit]: no such table; {TABLES}"))
        );
    }

    #[test]
    fn an_inline_table_at_the_top_is_a_context() {
        // B-12: one rule for every table value.
        let inline = load_str("global = { quit = \"z\" }\n").expect("an inline context loads");
        assert_eq!(inline.chords(Context::Global, Act::Quit), [chord("z")]);
    }

    #[test]
    fn every_chord_is_parsed_strictly_with_the_reason() {
        assert_eq!(
            errors("[global]\nquit = [\"shift-a\", \"ctrl-I\", \"hyper-x\"]\n"),
            vec![
                (
                    2,
                    r#"[global] quit = "shift-a": write a shifted letter as "A""#.to_owned()
                ),
                (
                    2,
                    r#"[global] quit = "ctrl-I": "ctrl-I" arrives as Tab: a terminal cannot tell the two apart"#
                        .to_owned()
                ),
                (
                    2,
                    r#"[global] quit = "hyper-x": "hyper" is not a modifier: write ctrl, alt or shift"#
                        .to_owned()
                ),
            ]
        );
    }

    #[test]
    fn ctrl_c_is_refused_however_it_is_spelled() {
        let src = "[global]\nquit = [\"q\", \"ctrl-c\"]\nfind = \"ctrl-C\"\nwaiting = \" ctrl + shift + c \"\n";
        let refused = "ctrl-c always quits and cannot be bound";
        assert_eq!(
            errors(src),
            vec![
                (2, format!(r#"[global] quit = "ctrl-c": {refused}"#)),
                (3, format!(r#"[global] find = "ctrl-C": {refused}"#)),
                (
                    4,
                    format!(r#"[global] waiting = " ctrl + shift + c ": {refused}"#)
                ),
            ]
        );
        assert_eq!(
            errors("[global]\nfind = \"ctrl-alt-C\"\n"),
            one(
                2,
                r#"[global] find = "ctrl-alt-C": write "ctrl-alt-c": a terminal sends ctrl with the lower-case letter"#
            )
        );
    }

    #[test]
    fn a_chord_listed_twice_in_one_action_is_refused() {
        assert_eq!(
            errors("[global]\nprev_tab = [\"shift-tab\", \"backtab\"]\n"),
            one(
                2,
                r#"[global] prev_tab = "backtab": already listed for this action"#
            )
        );
    }

    #[test]
    fn overlay_close_must_keep_a_chord() {
        assert_eq!(
            errors("[overlay]\nclose = []\n"),
            one(
                2,
                "[overlay] close: must keep at least one chord: overlays swallow every other key"
            )
        );
        assert_eq!(
            errors("[overlay]\nclose = [\"ctrl-c\"]\n"),
            one(
                2,
                r#"[overlay] close = "ctrl-c": ctrl-c always quits and cannot be bound"#
            )
        );
    }

    /// `tests/fixtures/keys/errors.toml` (blueprint §11.2; the fixture itself lands with Task 7).
    const ERRORS_TOML: &str = r#"# MOD-67 M2 fixture: one of each entry error, out of order on purpose.
version = 2

[list]
down = 5
up = ["k", 7]

[globl]
help = "f1"

[global]
quitt = "q"
quit = ["x", "shift-a"]
"#;

    #[test]
    fn every_error_is_reported_and_sorted_by_line() {
        let expected: Vec<(usize, String)> = [
            (2, "version: write version = 1, the only version htui reads"),
            (
                5,
                r#"[list] down: write a chord such as "q", or a list such as ["q", "x"]"#,
            ),
            (6, r#"[list] up: write each chord as a string, such as "q""#),
            (8, &format!("[globl]: no such table; {TABLES}")),
            (
                12,
                "[global] quitt: no such action; [global] has quit, next_tab, prev_tab, \
                 select_tab_1, select_tab_2, select_tab_3, select_tab_4, select_tab_5, \
                 select_tab_6, select_tab_7, select_tab_8, select_tab_9, help, workspaces, find, \
                 waiting",
            ),
            (
                13,
                r#"[global] quit = "shift-a": write a shifted letter as "A""#,
            ),
            // MOD-67 M3: `x` is agents' cancel in Settings > Agents, so the rebound quit would
            // never reach there (ANA-26 §2.2's silent shadowing, now reported).
            (
                13,
                r#"[global] quit = "x": "x" is already settings.agents.cancel (default) in Settings > Agents"#,
            ),
        ]
        .into_iter()
        .map(|(line, message)| (line, message.to_owned()))
        .collect();
        assert_eq!(errors(ERRORS_TOML), expected);
    }

    #[test]
    fn a_syntax_error_is_the_only_error_with_its_line() {
        assert_eq!(
            errors("[global]\nquit = \"x\"\nquit = [\"shift-a\"]\n"),
            one(3, "not valid TOML: duplicate key")
        );
        assert_eq!(
            errors("[global]\nquit = \"x\"\n[global]\n"),
            one(3, "not valid TOML: duplicate key")
        );
        assert_eq!(
            errors("[global\n"),
            one(1, "not valid TOML: unclosed table, expected `]`")
        );
    }

    #[test]
    fn a_top_level_value_is_refused() {
        let table = "global: write [global] as a table, one action per line";
        assert_eq!(errors("global = \"x\"\n"), one(1, table));
        assert_eq!(errors("[[global]]\nquit = \"x\"\n"), one(1, table));
        assert_eq!(
            errors("quit = \"x\"\n"),
            one(1, "quit: only version may sit outside a table")
        );
    }

    const HINT: &str = "Fix the file, or run `htui --default-keys` to start with the default keys.";

    #[test]
    fn the_report_has_ana_7_5s_shape() {
        let invalid = KeysError::Invalid {
            path: PathBuf::from("/home/u/.config/htui/keys.toml"),
            errors: vec![
                KeyFileError {
                    line: 3,
                    message: r#"[global] quit = "ctrl-c": ctrl-c always quits and cannot be bound"#
                        .to_owned(),
                },
                KeyFileError {
                    line: 5,
                    message: r#"[common] edit = "shift-e": write a shifted letter as "E""#
                        .to_owned(),
                },
            ],
        };
        assert_eq!(
            invalid.to_string(),
            format!(
                "/home/u/.config/htui/keys.toml has 2 errors:\n  keys.toml:3: [global] quit = \
                 \"ctrl-c\": ctrl-c always quits and cannot be bound\n  keys.toml:5: [common] \
                 edit = \"shift-e\": write a shifted letter as \"E\"\n{HINT}"
            )
        );
        let single = KeysError::Invalid {
            path: PathBuf::from("/x/keys.toml"),
            errors: vec![KeyFileError {
                line: 1,
                message: "m".to_owned(),
            }],
        };
        assert_eq!(
            single.to_string(),
            format!("/x/keys.toml has 1 error:\n  keys.toml:1: m\n{HINT}")
        );
        let unreadable = KeysError::Unreadable {
            path: PathBuf::from("/x/keys.toml"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        };
        assert_eq!(
            unreadable.to_string(),
            format!("cannot read /x/keys.toml: gone\n{HINT}")
        );
        for error in [&invalid, &single, &unreadable] {
            assert!(error.source().is_none());
            assert_eq!(error.code(), 2);
        }
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write a temp file");
        path
    }

    #[test]
    fn default_keys_ignores_even_a_broken_root_file() {
        let root = tempfile::tempdir().expect("a temp dir");
        let named = write(root.path(), "named.toml", "[globl]\n");
        write(root.path(), FILE_NAME, "[globl]\n");
        let keys = resolve(Some(&named), true, Some(root.path())).expect("the defaults");
        assert_eq!(&keys, Keys::compiled());
    }

    #[test]
    fn a_named_file_is_read_and_a_missing_one_refused() {
        let root = tempfile::tempdir().expect("a temp dir");
        let named = write(root.path(), "mine.toml", "[global]\nquit = \"z\"\n");
        let keys = resolve(Some(&named), false, None).expect("a good file loads");
        assert_eq!(keys.chords(Context::Global, Act::Quit), [chord("z")]);
        assert_eq!(
            load_path(&named).expect("a good file loads"),
            keys,
            "load_path reads the same file"
        );
        let missing = root.path().join("missing.toml");
        match resolve(Some(&missing), false, Some(root.path())) {
            Err(KeysError::Unreadable { path, .. }) => assert_eq!(path, missing),
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_root_file_or_root_is_the_defaults() {
        let root = tempfile::tempdir().expect("a temp dir");
        let keys = resolve(None, false, Some(root.path())).expect("the defaults");
        assert_eq!(&keys, Keys::compiled());
        let keys = resolve(None, false, None).expect("the defaults");
        assert_eq!(&keys, Keys::compiled());
    }

    #[test]
    fn a_broken_root_file_is_invalid_with_its_path() {
        let root = tempfile::tempdir().expect("a temp dir");
        let file = write(root.path(), FILE_NAME, "[global]\nquit = \"ctrl-c\"\n");
        match resolve(None, false, Some(root.path())) {
            Err(KeysError::Invalid { path, errors }) => {
                assert_eq!(path, file);
                assert_eq!(
                    errors,
                    [KeyFileError {
                        line: 2,
                        message:
                            r#"[global] quit = "ctrl-c": ctrl-c always quits and cannot be bound"#
                                .to_owned()
                    }]
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn a_root_file_that_cannot_be_read_is_unreadable() {
        let root = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir(root.path().join(FILE_NAME)).expect("a directory named keys.toml");
        match resolve(None, false, Some(root.path())) {
            Err(KeysError::Unreadable { path, .. }) => {
                assert_eq!(path, root.path().join(FILE_NAME));
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn a_view_table_overrides_a_shared_verb_it_offers() {
        let keys = load_str("version = 1\n[settings.boxes]\nreload = \"f5\"\n").expect("loads");
        assert_eq!(
            keys.chords(Context::SettingsBoxes, Act::Reload),
            [chord("f5")]
        );
        assert_eq!(keys.line(Context::SettingsBoxes, Act::Reload), Some(3));
        assert_eq!(
            keys.actions(views::BOXES_BROWSE, chord("f5")),
            [Act::Reload]
        );
        assert_eq!(keys.actions(views::BOXES_BROWSE, chord("r")), []);
        assert_eq!(
            keys.actions(views::PROMPT_BROWSE, chord("r")),
            [Act::Reload]
        );
        assert_eq!(keys.actions(views::PROMPT_BROWSE, chord("f5")), []);
    }

    #[test]
    fn a_view_table_refuses_what_none_of_its_modes_offer() {
        let boxes = "has edit_tags, edit_quirks, executor, probe, edit_spec, and may override \
                     down, up, yes, no, save, reload, dismiss, next_section, prev_section";
        assert_eq!(
            errors("[settings.boxes]\nedit = \"E\"\n"),
            one(
                2,
                &format!("[settings.boxes] edit: no such action; [settings.boxes] {boxes}")
            )
        );
        assert_eq!(
            errors("[settings.boxes]\nquit = \"Q\"\n"),
            one(
                2,
                &format!("[settings.boxes] quit: no such action; [settings.boxes] {boxes}")
            )
        );
        assert_eq!(
            errors("[switcher]\nclose = \"f2\"\n"),
            one(
                2,
                "[switcher] close: no such action; [switcher] has switch, and may override down, up"
            )
        );
        assert_eq!(
            errors("[settings.prompt]\nprobe = \"p\"\n"),
            one(
                2,
                "[settings.prompt] probe: no such action; [settings.prompt] may override down, \
                 up, edit, reload, dismiss, next_section, prev_section"
            )
        );
    }

    #[test]
    fn a_section_with_no_own_action_has_a_table() {
        let keys = load_str("[settings.prompt]\nreload = \"f5\"\n").expect("prompt loads");
        assert_eq!(
            keys.chords(Context::SettingsPrompt, Act::Reload),
            [chord("f5")]
        );
        let keys = load_str("[settings.queue]\nedit = [\"E\"]\n").expect("queue loads");
        assert_eq!(keys.chords(Context::SettingsQueue, Act::Edit), [chord("E")]);
        assert_eq!(keys.actions(views::QUEUE_BROWSE, chord("enter")), []);
        let keys = load_str("[migration]\nyes = \"a\"\n").expect("migration loads");
        assert_eq!(
            keys.chords(Context::Migration, Act::ConfirmYes),
            [chord("a")]
        );
    }

    #[test]
    fn a_shared_rebind_flows_into_the_view_defaults() {
        let keys = load_str("[form]\nnext_field = [\"ctrl-n\"]\n").expect("loads");
        assert_eq!(
            keys.chords(Context::SettingsSecrets, Act::FormNextField),
            [chord("ctrl-n"), chord("down")]
        );
        assert_eq!(
            keys.line(Context::SettingsSecrets, Act::FormNextField),
            None
        );
        assert_eq!(keys.line(Context::Form, Act::FormNextField), Some(2));
        let keys = load_str("[settings.secrets]\nnext_field = [\"ctrl-n\"]\n").expect("loads");
        assert_eq!(
            keys.chords(Context::SettingsSecrets, Act::FormNextField),
            [chord("ctrl-n")]
        );
        assert_eq!(
            keys.line(Context::SettingsSecrets, Act::FormNextField),
            Some(2)
        );
        assert_eq!(
            keys.chords(Context::SettingsAgents, Act::FormNextField),
            [chord("tab"), chord("down")]
        );
        let keys = load_str("[confirm]\nno = [\"x\"]\n").expect("loads");
        assert_eq!(
            keys.chords(Context::Migration, Act::ConfirmNo),
            [chord("x"), chord("N")]
        );
    }
}

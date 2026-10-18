//! `keys.toml` to [`Keys`] (MOD-67 M2; ANA-26 §7.1, §7.4 steps 1-6, §7.5): find the file, parse
//! it with spans, check names and chords strictly, merge it over the catalogue, and report every
//! error with its line. The merge lets an entry take its chord from an action the file leaves at
//! its default (MOD-12 M3 R1 H1, [`Resolved::notices`]); [`validate()`] adds the checks over the
//! merged keys.

use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use super::{
    Act, ActionSpec, CATALOGUE, CTRL_C, Context, DECLARED, ESC, KeyChord, Keys, quote, validate,
    validate::give_way,
};

/// The key file's name under the config root.
pub const FILE_NAME: &str = "keys.toml";

/// One error in a key file: a line and what is wrong there (ANA-26 §7.5). [`Keys::notices`]
/// uses the same shape for a chord an entry took from a default, which is no error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFileError {
    /// The 1-based line. `0` only for a fault in the compiled defaults, which the validator's
    /// tests rule out.
    pub line: usize,
    /// Everything after `keys.toml:LINE: `, e.g.
    /// `[global] quit = "esc": esc always closes, cancels or goes back and cannot be bound`.
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

/// The refusal of `Esc`, with any modifier (MOD-115 M2 MD7): it is never configurable. Every row
/// whose compiled default holds it keeps it, so a list replaces only that row's other chords.
const ESC_REFUSED: &str = "esc always closes, cancels or goes back and cannot be bound";

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
    let mut keys = Keys::defaults();
    // MOD-115 M2 §1.7: a file that does not say otherwise is from before M2.
    keys.version = 1;
    let mut loader = Loader {
        src,
        keys,
        errors: Vec::new(),
        set: Vec::new(),
    };
    // MOD-115 M2: the version decides how an entry reads (MD7's version-1 `Esc`), and a table
    // may come before it in the parsed order, so it is read first.
    let version_first = |(key, _): &(&Spanned<DeString<'_>>, &Spanned<DeValue<'_>>)| {
        let name: &str = key.get_ref();
        name != "version"
    };
    let mut entries: Vec<_> = root.get_ref().iter().collect();
    entries.sort_by_key(version_first);
    for (key, value) in entries {
        loader.top_level(key, value);
    }
    let Loader {
        mut keys,
        mut errors,
        set,
        ..
    } = loader;
    // MOD-115 M2 (D-9): `ctrl-c` always quit before M2 and no file could list it, so a version-1
    // file's quit keeps it. A version-2 file binds it like any chord.
    if keys.version == 1
        && let Some(quit) = keys.row(Context::Global, Act::Quit)
        && let Some(line) = quit.entry
        && !quit.chords.contains(&CTRL_C)
    {
        let mut chords = quit.chords.clone();
        chords.push(CTRL_C);
        keys.set(Context::Global, Act::Quit, chords, line);
    }
    // MOD-67 M3 PA-1: the view defaults follow the shared rows the file just set.
    keys.derive(&set);
    give_way(&mut keys);
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

/// The keys [`resolve`] found, and the file they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The keys in force.
    pub keys: Keys,
    /// The file read, or `None` for the compiled defaults.
    pub path: Option<PathBuf>,
}

impl Resolved {
    /// The compiled defaults, read from no file.
    fn defaults() -> Self {
        Self {
            keys: Keys::compiled().clone(),
            path: None,
        }
    }

    /// The startup notices (MOD-12 M3 R1 H1): one `PATH:LINE: message` per chord an entry took
    /// from an action the file leaves at its default ([`Keys::notices`]). `lib::run` prints each
    /// on stderr after `htui: `. Empty for the defaults.
    #[must_use]
    pub fn notices(&self) -> Vec<String> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        self.keys
            .notices()
            .into_iter()
            .map(|notice| format!("{}:{}: {}", path.display(), notice.line, notice.message))
            .collect()
    }
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
) -> Result<Resolved, KeysError> {
    if default_keys {
        return Ok(Resolved::defaults());
    }
    let read = |path: PathBuf, keys: Keys| Resolved {
        keys,
        path: Some(path),
    };
    if let Some(path) = keys_flag {
        return load_path(path).map(|keys| read(path.to_owned(), keys));
    }
    let Some(root) = root else {
        return Ok(Resolved::defaults());
    };
    let path = root.join(FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(src) => checked(path.clone(), &src).map(|keys| read(path, keys)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Resolved::defaults()),
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

/// `written` as a chord: [`KeyChord::parse_strict`], with its reason when it fails (D8 step 3).
/// `ctrl-c` binds like any chord since MOD-115 M2 (MD2); `Esc` is the entry's to refuse
/// ([`Loader::entry`]).
fn chord_of(written: &str) -> Result<KeyChord, String> {
    KeyChord::parse_strict(written).map_err(|error| error.to_string())
}

/// Where the compiled row of `act` as `[context]` sees it holds `Esc`, or `None` (MOD-115 M2
/// MD7): the row itself, else (a view's override of a shared verb) the shared row.
fn esc_at(context: Context, act: Act) -> Option<usize> {
    let compiled = Keys::compiled();
    compiled
        .row(context, act)
        .or_else(|| act.spec().and_then(|spec| compiled.row(spec.context, act)))
        .and_then(|row| row.chords.iter().position(|chord| *chord == ESC))
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
    /// allowed beside them (D1, D2). MOD-115 M2 §1.7: `version = 1` (a file from before M2) or
    /// `version = 2`; the keys remember which.
    fn top_level(&mut self, key: &Spanned<DeString<'_>>, value: &Spanned<DeValue<'_>>) {
        let name: &str = key.get_ref();
        let line = self.line(key.span().start);
        let version = match value.get_ref() {
            DeValue::Integer(integer) => {
                match i64::from_str_radix(integer.as_str(), integer.radix()) {
                    Ok(1) => Some(1),
                    Ok(2) => Some(2),
                    _ => None,
                }
            }
            _ => None,
        };
        match value.get_ref() {
            DeValue::Table(table) => self.walk_table(name, line, table),
            _ if name == "version" && version.is_some() => {
                self.keys.version = version.unwrap_or(1);
            }
            _ if name == "version" => self.push(
                line,
                "version: write version = 2 (version = 1 is a file from before MOD-115)".to_owned(),
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
        // MOD-115 M2 MD7: a row whose compiled default holds `Esc` keeps it.
        let esc = esc_at(context, spec.act);
        let keeps_esc = esc.is_some();
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
                // MD7, MD4: a version-1 file listed `Esc` where the row has it anyway (the
                // README's `close = ["esc", "f2"]`); it changes nothing, so it still loads.
                Ok(chord) if chord == ESC && self.keys.version == 1 && keeps_esc => {}
                Ok(chord) if chord.is_esc() => {
                    self.push(line, format!("[{table}] {name} = {shown}: {ESC_REFUSED}"));
                }
                Ok(chord) if chords.contains(&chord) => self.push(
                    line,
                    format!("[{table}] {name} = {shown}: already listed for this action"),
                ),
                Ok(chord) => chords.push(chord),
            }
        }
        if let Some(at) = esc
            && !chords.contains(&ESC)
        {
            chords.insert(at.min(chords.len()), ESC);
        }
        self.keys.set(context, spec.act, chords, key_line);
        self.set.push((context, spec.act));
    }
}

/// Why `act` must keep at least one chord (ANA-26 §7.4 step 6; MOD-115 M2 D7, D-10), or `None`:
/// the validator's safety nets refuse a file that leaves one with none. The rows that hold `Esc`
/// (`overlay.close`, `field.cancel`, `leave.stay`) keep it by construction (MD7).
pub(super) const fn must_keep(act: Act) -> Option<&'static str> {
    match act {
        Act::OverlayClose => Some("overlays swallow every other key"),
        // MOD-57 M2 E9 (a): the one way back from a focused in-pane editor.
        Act::EditorFocus => Some("the editor takes every other key"),
        Act::Quit | Act::LeaveConfirm => Some(
            "global.quit opens the leave dialog and leave.confirm leaves; without both htui \
             cannot be left",
        ),
        Act::FieldCancel => Some("it is the way out of a text field"),
        Act::LeaveStay => Some("it is the way back from the leave dialog"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::path::{Path, PathBuf};

    use super::{ESC_REFUSED, FILE_NAME, KeyFileError, KeysError, load_path, load_str, resolve};
    use crate::keys::{Act, CATALOGUE, CTRL_C, Context, KeyChord, Keys, views};

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

    const TABLES: &str = "the tables are global, overlay, list, pane, confirm, form, common, editor, \
                          settings, settings.agents, settings.hierarchy, settings.kinds, \
                          settings.prompt, settings.connection, settings.qdrant, settings.boxes, \
                          settings.personas, settings.secrets, settings.queue, concepts, \
                          switcher, migration, waiting, skills, skills.library, \
                          skills.templates, skills.attach, skills.help, requirements, backlog, \
                          backlog.body, backlog.runs, backlog.graph, backlog.docs, backlog.notes, \
                          backlog.prompt, backlog.reqs, backlog.filter_form, backlog.item, chat, \
                          queue, picker, layout, history, leave, field, choice, answer";

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
        // MOD-115 M2 (D-9): quit keeps its implicit `ctrl-c`.
        assert_eq!(
            keys.chords(Context::Global, Act::Quit),
            [chord("z"), CTRL_C]
        );
        assert_eq!(keys.chords(Context::Global, Act::Help), [chord("f1")]);
        assert_eq!(keys.line(Context::Global, Act::Quit), Some(2));
        assert_eq!(keys.line(Context::Global, Act::Help), Some(3));
        // MOD-115 M2 D8: the version-1 file's `z` takes `layout.zoom`'s M2 default.
        assert_eq!(keys.chords(Context::Layout, Act::LayoutZoom), []);
        for spec in CATALOGUE
            .iter()
            .filter(|spec| !matches!(spec.act, Act::Quit | Act::Help | Act::LayoutZoom))
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

    /// MOD-115 M2 §1.7: `version = 1` (a file from before M2) or `version = 2`, nothing else;
    /// the keys remember which, and a file without one is version 1.
    #[test]
    fn version_must_be_the_integer_1_or_2() {
        for value in ["3", "0", "\"1\"", "\"2\"", "1.0", "true", "[1]"] {
            assert_eq!(
                errors(&format!("version = {value}\n")),
                one(
                    1,
                    "version: write version = 2 (version = 1 is a file from before MOD-115)"
                ),
                "{value}"
            );
        }
        for (value, version) in [("1", 1), ("0x1", 1), ("+1", 1), ("2", 2), ("0x2", 2)] {
            let keys = load_str(&format!("version = {value}\n")).expect("the version loads");
            assert_eq!(keys, Keys::defaults(), "{value}");
            assert_eq!(keys.version(), version, "{value}");
        }
        assert_eq!(load_str("").expect("an empty file loads").version(), 1);
        assert_eq!(Keys::compiled().version(), crate::keys::VERSION);
        assert_eq!(crate::keys::VERSION, 2);
    }

    /// MOD-115 M2 (D-9): `ctrl-c` always quit before M2 and no file could list it, so a
    /// version-1 file's quit keeps it; a version-2 file binds it like any chord, and no other
    /// row gains it.
    #[test]
    fn a_version_1_quit_keeps_ctrl_c_and_a_version_2_one_binds_it_like_any_chord() {
        let keys = load_str("[global]\nquit = [\"z\"]\nhelp = [\"f1\"]\n").expect("it loads");
        assert_eq!(
            keys.chords(Context::Global, Act::Quit),
            [chord("z"), CTRL_C]
        );
        assert_eq!(keys.chords(Context::Global, Act::Help), [chord("f1")]);
        let unbound = load_str("[global]\nquit = []\n").expect("it loads");
        assert_eq!(unbound.chords(Context::Global, Act::Quit), [CTRL_C]);
        let same = load_str("[global]\nquit = [\"q\"]\n").expect("it loads");
        assert_eq!(
            same.line(Context::Global, Act::Quit),
            None,
            "the default, unmarked"
        );
        let leave = load_str("[leave]\nconfirm = [\"y\"]\n").expect("it loads");
        assert_eq!(
            leave.chords(Context::Leave, Act::LeaveConfirm),
            [chord("y")],
            "only a version-1 quit had ctrl-c implicit"
        );

        let strict = load_str("version = 2\n[global]\nquit = [\"ctrl-y\"]\n").expect("it loads");
        assert_eq!(strict.chords(Context::Global, Act::Quit), [chord("ctrl-y")]);
        let listed =
            load_str("version = 2\n[global]\nquit = [\"ctrl-y\", \"ctrl-c\"]\n").expect("it loads");
        assert_eq!(
            listed.chords(Context::Global, Act::Quit),
            [chord("ctrl-y"), CTRL_C]
        );
        let leave = load_str("version = 2\n[leave]\nconfirm = [\"y\"]\n").expect("it loads");
        assert_eq!(
            leave.chords(Context::Leave, Act::LeaveConfirm),
            [chord("y")]
        );
    }

    /// MOD-115 M2 (MD2): `ctrl-c` binds like any chord: an entry in quit's table takes it from
    /// quit, which keeps `q` (D-10), with the usual notice.
    #[test]
    fn ctrl_c_binds_like_any_chord() {
        let keys = load_str("version = 2\n[global]\nfind = \"ctrl-c\"\n").expect("it loads");
        assert_eq!(keys.chords(Context::Global, Act::Find), [CTRL_C]);
        assert_eq!(keys.chords(Context::Global, Act::Quit), [chord("q")]);
        assert_eq!(
            keys.notices()
                .into_iter()
                .map(|notice| notice.message)
                .collect::<Vec<_>>(),
            [r#"[global] find = "ctrl-c" takes "ctrl-c" from global.quit, which keeps "q""#]
        );
        assert_eq!(
            keys.actions(views::LEAVE, CTRL_C),
            [Act::LeaveConfirm],
            "the leave dialog keeps its own ctrl-c"
        );
    }

    /// MOD-115 M2 D7, D-10: quit and the leave dialog's two answers must keep a chord. An entry
    /// that empties one is refused on its line, naming why; quit gives a chord away only while it
    /// keeps another, so `[global] help = ["q"]` loads and quit keeps `ctrl-c`.
    #[test]
    fn quit_and_the_leave_dialog_must_keep_a_chord() {
        let way_out = "must keep at least one chord: global.quit opens the leave dialog and \
                       leave.confirm leaves; without both htui cannot be left";
        assert_eq!(
            errors("version = 2\n[global]\nquit = []\n"),
            one(3, &format!("[global] quit: {way_out}"))
        );
        assert_eq!(
            errors("version = 2\n[leave]\nconfirm = []\n"),
            one(3, &format!("[leave] confirm: {way_out}"))
        );
        let keys = load_str("version = 2\n[global]\nhelp = [\"q\"]\n").expect("it loads");
        assert_eq!(keys.chords(Context::Global, Act::Quit), [CTRL_C]);
        assert_eq!(keys.chords(Context::Global, Act::Help), [chord("q")]);
        let keys = load_str("version = 2\n[leave]\nconfirm = [\"n\"]\n").expect("it loads");
        assert_eq!(
            keys.chords(Context::Leave, Act::LeaveStay),
            [chord("esc")],
            "stay gives `n` away and keeps Esc"
        );
        // Taking quit's last chord is a collision, not an unbound quit.
        assert!(
            errors("version = 2\n[global]\nhelp = [\"q\", \"ctrl-c\"]\n")
                .iter()
                .any(|(line, message)| *line == 3
                    && message.contains(r#""ctrl-c" is already global.quit (default)"#)),
            "{:?}",
            errors("version = 2\n[global]\nhelp = [\"q\", \"ctrl-c\"]\n")
        );
    }

    /// MOD-115 M2 MD7: a key file cannot list `Esc`, however it is spelled, and a row whose
    /// default holds it keeps it: a list replaces only the row's other chords, `[]` unbinds only
    /// those.
    #[test]
    fn esc_is_never_configurable() {
        for (spec, line) in [("esc", 3), ("Esc", 3), ("escape", 3), ("alt-esc", 3)] {
            assert_eq!(
                errors(&format!("version = 2\n[list]\ntop = [{spec:?}]\n")),
                one(line, &format!("[list] top = {spec:?}: {ESC_REFUSED}")),
                "{spec}"
            );
        }
        assert_eq!(
            errors("version = 2\n[overlay]\nclose = [\"esc\", \"f2\"]\n"),
            one(3, &format!(r#"[overlay] close = "esc": {ESC_REFUSED}"#))
        );
        let keys = load_str("version = 2\n[overlay]\nclose = [\"f2\"]\n[confirm]\nno = []\n")
            .expect("it loads");
        assert_eq!(
            keys.chords(Context::Overlay, Act::OverlayClose),
            [chord("esc"), chord("f2")]
        );
        assert_eq!(
            keys.chords(Context::Confirm, Act::ConfirmNo),
            [chord("esc")]
        );
        let unbound = load_str("version = 2\n[overlay]\nclose = []\n").expect("it loads");
        assert_eq!(
            unbound.chords(Context::Overlay, Act::OverlayClose),
            [chord("esc")]
        );
        assert_eq!(
            unbound.line(Context::Overlay, Act::OverlayClose),
            None,
            "Esc alone is the default"
        );
        // A view's override of a shared verb keeps the shared row's Esc too.
        let keys =
            load_str("version = 2\n[settings.boxes]\ndismiss = [\"x\"]\n").expect("it loads");
        assert_eq!(
            keys.chords(Context::SettingsBoxes, Act::Dismiss),
            [chord("esc"), chord("x")]
        );
    }

    /// MOD-115 M2 MD7 with MD4: a version-1 file that lists `Esc` where the row holds it anyway
    /// (the README's `close = ["esc", "f2"]`) changes nothing, so it still loads; one that lists
    /// it on any other row is refused, as in version 2.
    #[test]
    fn a_version_1_esc_on_a_row_that_holds_it_still_loads() {
        let keys = load_str("[overlay]\nclose = [\"esc\", \"f2\"]\n").expect("it loads");
        assert_eq!(
            keys.chords(Context::Overlay, Act::OverlayClose),
            [chord("esc"), chord("f2")]
        );
        assert_eq!(
            errors("[list]\ntop = [\"g\", \"esc\"]\n"),
            one(2, &format!(r#"[list] top = "esc": {ESC_REFUSED}"#))
        );
    }

    /// MOD-115 M2 D8: a version-1 file's entry takes a `since: 2` default's chord where a
    /// declared stack makes both candidates, with a notice; a version-2 file is refused.
    #[test]
    fn a_version_1_entry_takes_a_new_defaults_chord_and_a_version_2_one_collides() {
        let keys = load_str("[global]\nworkspaces = [\"z\"]\n").expect("version 1 loads");
        assert_eq!(keys.chords(Context::Layout, Act::LayoutZoom), []);
        assert_eq!(
            keys.notices()
                .into_iter()
                .map(|notice| (notice.line, notice.message))
                .collect::<Vec<_>>(),
            [(
                2,
                r#"[global] workspaces = "z" takes "z" from layout.zoom, which is now unbound"#
                    .to_owned()
            )]
        );
        let strict = errors("version = 2\n[global]\nworkspaces = [\"z\"]\n");
        assert!(
            strict
                .iter()
                .any(|(line, message)| *line == 3 && message.contains("layout.zoom (default)")),
            "{strict:?}"
        );
        // A default from before M2 still gives way only in its own context.
        assert!(
            errors("[global]\nworkspaces = [\"j\"]\n")
                .iter()
                .any(|(_, message)| message.contains("list.down (default)"))
        );
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
        assert_eq!(
            inline.chords(Context::Global, Act::Quit),
            [chord("z"), CTRL_C]
        );
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

    /// MOD-115 M2 (MD2): `ctrl-c` is a chord like any other; a capital is still refused with the
    /// spelling to write.
    #[test]
    fn ctrl_c_is_read_like_any_chord() {
        let src = "version = 2\n[global]\nquit = [\"q\", \"ctrl-c\"]\nfind = \"ctrl-C\"\n";
        assert_eq!(
            errors(src),
            one(
                4,
                r#"[global] find = "ctrl-C": write "ctrl-c": a terminal sends ctrl with the lower-case letter"#
            )
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

    /// MOD-57 M2 E9 (a): the one way back from a focused in-pane editor.
    #[test]
    fn the_editor_focus_must_keep_a_chord() {
        assert_eq!(
            errors("[editor]\nfocus = []\n"),
            one(
                2,
                "[editor] focus: must keep at least one chord: the editor takes every other key"
            )
        );
    }

    /// MOD-115 M2 MD7: `overlay.close` keeps `Esc` by construction, so `[]` leaves it `Esc`
    /// alone and the overlay can always be closed.
    #[test]
    fn overlay_close_keeps_esc_by_construction() {
        let keys = load_str("[overlay]\nclose = []\n").expect("it loads");
        assert_eq!(
            keys.chords(Context::Overlay, Act::OverlayClose),
            [chord("esc")]
        );
        for (context, act) in [
            (Context::Field, Act::FieldCancel),
            (Context::Leave, Act::LeaveStay),
            (Context::Overlay, Act::OverlayClose),
        ] {
            let table = context.table();
            let name = act.spec().expect("a row").name;
            let keys =
                load_str(&format!("version = 2\n[{table}]\n{name} = []\n")).expect("it loads");
            assert!(keys.chords(context, act).contains(&chord("esc")), "{name}");
        }
    }

    /// MOD-67 M4 D8.1: `editor.focus` must keep a chord; MOD-115 M2: `ctrl-c` is quit's in a
    /// version-2 file, so focus cannot share it there.
    #[test]
    fn editor_focus_must_keep_a_chord() {
        assert_eq!(
            errors("[editor]\nfocus = []\n"),
            one(
                2,
                "[editor] focus: must keep at least one chord: the editor takes every other key"
            )
        );
        assert!(
            errors("version = 2\n[editor]\nfocus = \"ctrl-c\"\n")
                .iter()
                .all(|(line, message)| *line == 3
                    && message.starts_with(r#"[editor] focus = "ctrl-c": "ctrl-c" is already "#)),
        );
        assert!(
            load_str("[editor]\nabort = []\n").is_ok(),
            "abort may be unbound"
        );
    }

    /// `tests/fixtures/keys/errors.toml` (blueprint §11.2; the fixture itself lands with Task 7).
    const ERRORS_TOML: &str = r#"# MOD-67 M2 fixture: one of each entry error, out of order on purpose.
version = 3

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
            (
                2,
                "version: write version = 2 (version = 1 is a file from before MOD-115)",
            ),
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
                 select_tab_6, select_tab_7, select_tab_8, select_tab_9, help, about, workspaces, find, \
                 waiting, queue, next_pane, prev_pane",
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
            // MOD-67 M4: and `x` detaches in the Library's attachments pane.
            (
                13,
                r#"[global] quit = "x": "x" is already skills.attach.detach (default) in Skills > Library > Attachments"#,
            ),
            // MOD-67 M5: and `x` rejects a step in the Backlog's Runs pane.
            (
                13,
                r#"[global] quit = "x": "x" is already backlog.runs.reject (default) in Backlog > Runs"#,
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
        let resolved = resolve(Some(&named), true, Some(root.path())).expect("the defaults");
        assert_eq!(&resolved.keys, Keys::compiled());
        assert_eq!(resolved.path, None);
    }

    #[test]
    fn a_named_file_is_read_and_a_missing_one_refused() {
        let root = tempfile::tempdir().expect("a temp dir");
        let named = write(root.path(), "mine.toml", "[global]\nquit = \"z\"\n");
        let resolved = resolve(Some(&named), false, None).expect("a good file loads");
        assert_eq!(resolved.path.as_deref(), Some(named.as_path()));
        let keys = resolved.keys;
        // MOD-115 M2 (D-9): quit keeps its implicit `ctrl-c`.
        assert_eq!(
            keys.chords(Context::Global, Act::Quit),
            [chord("z"), CTRL_C]
        );
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
        let resolved = resolve(None, false, Some(root.path())).expect("the defaults");
        assert_eq!(&resolved.keys, Keys::compiled());
        assert_eq!(resolved.path, None);
        let resolved = resolve(None, false, None).expect("the defaults");
        assert_eq!(&resolved.keys, Keys::compiled());
        assert!(resolved.notices().is_empty());
    }

    #[test]
    fn a_broken_root_file_is_invalid_with_its_path() {
        let root = tempfile::tempdir().expect("a temp dir");
        let file = write(root.path(), FILE_NAME, "[global]\nquit = \"esc\"\n");
        match resolve(None, false, Some(root.path())) {
            Err(KeysError::Invalid { path, errors }) => {
                assert_eq!(path, file);
                assert_eq!(
                    errors,
                    [KeyFileError {
                        line: 2,
                        message: format!(r#"[global] quit = "esc": {ESC_REFUSED}"#)
                    }]
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    /// MOD-12 M3 R1 H1: each chord an entry takes from a default is a notice on the entry's
    /// line, naming the file and what the default keeps; a defaults-only resolve has none.
    #[test]
    fn a_chord_taken_from_a_default_is_a_notice_naming_the_file() {
        let root = tempfile::tempdir().expect("a temp dir");
        let file = write(
            root.path(),
            FILE_NAME,
            "[global]\nquit = [\"ctrl-q\"]\n\n[list]\ntop = [\"j\"]\n",
        );
        let resolved = resolve(None, false, Some(root.path())).expect("the file loads");
        assert_eq!(resolved.path.as_deref(), Some(file.as_path()));
        assert_eq!(
            resolved.notices(),
            [
                format!(
                    "{}:2: [global] quit = \"ctrl-q\" takes \"ctrl-q\" from global.queue, which \
                     is now unbound",
                    file.display()
                ),
                format!(
                    "{}:5: [list] top = \"j\" takes \"j\" from list.down, which keeps \"down\"",
                    file.display()
                ),
            ]
        );
        let resolved = resolve(None, true, Some(root.path())).expect("the defaults");
        assert!(resolved.notices().is_empty());
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

    /// MOD-67 M5 §3.1: `[backlog.runs]` overrides a `[backlog]` verb in the Runs pane alone,
    /// `[backlog]` moves it on every sub-tab, and `[chat]` lists what it has and may override.
    #[test]
    fn a_backlog_pane_table_overrides_a_backlog_verb() {
        let keys = load_str("[backlog.runs]\nfilter = \"/\"\n").expect("loads");
        assert_eq!(
            keys.actions(views::RUNS_BROWSE, chord("/")),
            [Act::BacklogOpenFilter]
        );
        assert_eq!(keys.actions(views::RUNS_BROWSE, chord("f")), []);
        assert_eq!(
            keys.actions(views::BACKLOG_BODY, chord("f")),
            [Act::BacklogOpenFilter]
        );
        assert_eq!(keys.actions(views::BACKLOG_BODY, chord("/")), []);
        let keys = load_str("[backlog]\nfilter = \"/\"\n").expect("loads");
        for stack in [
            views::BACKLOG_BODY,
            views::RUNS_BROWSE,
            views::BACKLOG_GRAPH,
            views::BACKLOG_DOCS,
            views::BACKLOG_NOTES,
            views::BACKLOG_PROMPT,
            views::BACKLOG_REQS,
        ] {
            assert_eq!(
                keys.actions(stack, chord("/")),
                [Act::BacklogOpenFilter],
                "{stack:?}"
            );
            assert_eq!(keys.actions(stack, chord("f")), [], "{stack:?}");
        }
        assert_eq!(
            errors("[chat]\nfilter = \"/\"\n"),
            one(
                2,
                "[chat] filter: no such action; [chat] has compose, next_agent, end, thoughts, \
                 top, bottom, and may override down, up, back"
            )
        );
    }

    /// MOD-67 M6 §4.1: `[queue]` and `[picker]` rebind their views' own verbs, and `[picker]`
    /// lists what it has and may override (the plan's `cancel` is `common.back`, PA-10).
    #[test]
    fn a_queue_or_picker_table_rebinds_its_view() {
        let keys = load_str("[queue]\nmove_down = \"D\"\n").expect("loads");
        assert_eq!(
            keys.actions(views::QUEUE_OVERLAY, chord("D")),
            [Act::QueueMoveDown]
        );
        assert_eq!(keys.actions(views::QUEUE_OVERLAY, chord("J")), []);
        let keys = load_str("[picker]\nopen = \"o\"\n").expect("loads");
        assert_eq!(
            keys.actions(views::HIERARCHY_PICKER, chord("o")),
            [Act::PickerOpen]
        );
        assert_eq!(
            errors("[picker]\ncancel = \"c\"\n"),
            one(
                2,
                "[picker] cancel: no such action; [picker] has open, parent, choose, choose_dir, \
                 go_to, hidden, and may override down, up, back"
            )
        );
    }

    /// MOD-67 M4 §3.1: `[skills.library]` overrides a `[skills]` verb in the Library alone;
    /// `[skills.templates]` lists what it has and may override.
    #[test]
    fn a_skills_view_table_overrides_its_shared_verbs() {
        let keys = load_str("[skills.library]\ndiff = \"D\"\n").expect("loads");
        assert_eq!(
            keys.actions(views::LIBRARY_BROWSE, chord("D")),
            [Act::SkillsDiff]
        );
        assert_eq!(keys.actions(views::LIBRARY_BROWSE, chord("d")), []);
        assert_eq!(
            keys.actions(views::TEMPLATES_BROWSE, chord("d")),
            [Act::SkillsDiff]
        );
        assert_eq!(
            errors("[skills.templates]\nimport = \"x\"\n"),
            one(
                2,
                "[skills.templates] import: no such action; [skills.templates] has diff_default, \
                 and may override down, up, scroll_down, scroll_up, page_down, page_up, save, \
                 external_editor, edit, new, reload, switch_view, prev_version, next_version, \
                 base, diff, edit_externally, ask_agent"
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
        // MOD-115 M2 MD7: `no` keeps its Esc, and the migration prompt's row follows it.
        let keys = load_str("[confirm]\nno = [\"x\"]\n").expect("loads");
        assert_eq!(
            keys.chords(Context::Migration, Act::ConfirmNo),
            [chord("x"), chord("esc"), chord("N")]
        );
    }
}

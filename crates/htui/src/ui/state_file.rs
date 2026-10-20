//! The UI-state file (MOD-115 M2 D1, blueprint §5): `<config root>/ui-state.toml`, per machine,
//! versioned and written atomically the moment the about panel or a tab's layout changes. A
//! missing file is the first launch; an unreadable or invalid one falls back to the defaults with
//! a reason, and never stops htui starting. The testkit keeps it in memory.
//!
//! ```toml
//! version = 1
//!
//! [about]
//! visible = false
//!
//! [layout.backlog]            # the tab's id
//! preset = "ide"              # Preset::name
//! list_percent = 55           # exactly one of list_percent / list_cells
//! activity_percent = 30
//! list = true
//! activity = true
//! zoom = false
//!
//! [modal.item_form]           # reserved for R-TUI-12's modal sizes: read and written back
//! width = 80
//! height = 24
//! ```
//!
//! The serde shapes live here, so the shell's types ([`TabLayout`]) carry no serde. Unknown keys
//! are ignored, so a newer htui's additions survive an older one reading the file (they are not
//! written back).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::ui::shell::{ListSize, Preset, TabLayout};

/// The file's name in the config root.
pub const FILE_NAME: &str = "ui-state.toml";

/// The format's version: the only one [`parse`] reads and the one [`render`] writes.
const VERSION: i64 = 1;

/// What the UI-state file holds (D1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiState {
    /// Whether the about panel is shown; `None` leaves it to the first-launch default (on).
    pub about: Option<bool>,
    /// Each tab's layout, keyed by the tab's id (`TabId.0`).
    pub layouts: BTreeMap<String, TabLayout>,
    /// Each modal's size as `(width, height)`, keyed by the modal's name (reserved for `R-TUI-12`:
    /// nothing reads it in M2, it is kept so a later htui's sizes survive).
    pub modals: BTreeMap<String, (u16, u16)>,
}

/// The file as TOML: the tables [`parse`] reads after the version and [`render`] writes.
#[derive(Debug, Serialize)]
struct FileDto {
    version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    about: Option<AboutDto>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    layout: BTreeMap<String, LayoutDto>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    modal: BTreeMap<String, ModalDto>,
}

/// `[about]`.
#[derive(Debug, Serialize, Deserialize)]
struct AboutDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visible: Option<bool>,
}

/// `[layout.<tab>]`.
#[derive(Debug, Serialize, Deserialize)]
struct LayoutDto {
    preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list_cells: Option<u16>,
    activity_percent: u16,
    list: bool,
    activity: bool,
    zoom: bool,
}

/// `[modal.<name>]`.
#[derive(Debug, Serialize, Deserialize)]
struct ModalDto {
    width: u16,
    height: u16,
}

/// Reads the file's text. `Err` is the reason it cannot be used, one line, for the notice.
///
/// # Errors
/// The text is not TOML, its `version` is missing or not 1, or a table does not hold what it
/// should (an unknown preset, both or neither list sizes, a percentage over 100, a wrong type).
pub fn parse(text: &str) -> Result<UiState, String> {
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| {
        let line = error
            .span()
            .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
        let message = one_line(error.message());
        match line {
            Some(line) => format!("not TOML (line {line}): {message}"),
            None => format!("not TOML: {message}"),
        }
    })?;
    match table.get("version") {
        Some(toml::Value::Integer(VERSION)) => {}
        Some(other) => {
            return Err(format!(
                "version: {other} is not one this htui reads (it writes version = {VERSION})"
            ));
        }
        None => {
            return Err(format!(
                "version: missing (this htui writes version = {VERSION})"
            ));
        }
    }
    let mut state = UiState::default();
    if let Some(about) = table.get("about") {
        let about: AboutDto = shaped("about", about)?;
        state.about = about.visible;
    }
    for (name, value) in tables(&table, "layout")? {
        let at = format!("layout.{name}");
        let layout: LayoutDto = shaped(&at, value)?;
        state.layouts.insert(
            name.clone(),
            layout.into_layout().map_err(|why| format!("{at}: {why}"))?,
        );
    }
    for (name, value) in tables(&table, "modal")? {
        let modal: ModalDto = shaped(&format!("modal.{name}"), value)?;
        state
            .modals
            .insert(name.clone(), (modal.width, modal.height));
    }
    Ok(state)
}

/// Writes `state` as the file's text, tables in key order, so the same state always writes the
/// same bytes.
#[must_use]
pub fn render(state: &UiState) -> String {
    let file = FileDto {
        version: VERSION,
        about: state.about.map(|visible| AboutDto {
            visible: Some(visible),
        }),
        layout: state
            .layouts
            .iter()
            .map(|(name, layout)| (name.clone(), LayoutDto::of(layout)))
            .collect(),
        modal: state
            .modals
            .iter()
            .map(|(name, &(width, height))| (name.clone(), ModalDto { width, height }))
            .collect(),
    };
    toml::to_string(&file).expect("plain tables of strings, integers and booleans serialise")
}

impl LayoutDto {
    fn of(layout: &TabLayout) -> Self {
        let (list_percent, list_cells) = match layout.list {
            ListSize::Percent(percent) => (Some(percent), None),
            ListSize::Cells(cells) => (None, Some(cells)),
        };
        Self {
            preset: layout.preset.name().to_owned(),
            list_percent,
            list_cells,
            activity_percent: layout.activity_pct,
            list: layout.list_on,
            activity: layout.activity_on,
            zoom: layout.zoom,
        }
    }

    fn into_layout(self) -> Result<TabLayout, String> {
        let preset = Preset::from_name(&self.preset).ok_or_else(|| {
            let names: Vec<&str> = Preset::ALL.iter().map(|preset| preset.name()).collect();
            format!(
                "unknown preset \"{}\" (one of {})",
                self.preset,
                names.join(", ")
            )
        })?;
        let list = match (self.list_percent, self.list_cells) {
            (Some(percent), None) => ListSize::Percent(within_100("list_percent", percent)?),
            (None, Some(cells)) => ListSize::Cells(cells),
            _ => return Err("write exactly one of list_percent and list_cells".to_owned()),
        };
        Ok(TabLayout {
            preset,
            list,
            activity_pct: within_100("activity_percent", self.activity_percent)?,
            list_on: self.list,
            activity_on: self.activity,
            zoom: self.zoom,
        })
    }
}

/// `percent` if it is a percentage, else why not.
fn within_100(key: &str, percent: u16) -> Result<u16, String> {
    if percent > 100 {
        Err(format!("{key} is {percent}, over 100"))
    } else {
        Ok(percent)
    }
}

/// The sub-tables of `[key]` (`[layout.<tab>]`, `[modal.<name>]`); none when it is absent.
fn tables<'t>(
    table: &'t toml::Table,
    key: &str,
) -> Result<impl Iterator<Item = (&'t String, &'t toml::Value)>, String> {
    match table.get(key) {
        None => Ok(None.into_iter().flatten()),
        Some(toml::Value::Table(inner)) => Ok(Some(inner.iter()).into_iter().flatten()),
        Some(_) => Err(format!("{key}: not a table")),
    }
}

/// `value` read as `T`, or why not, prefixed with where it is (`layout.backlog: …`).
fn shaped<T: serde::de::DeserializeOwned>(at: &str, value: &toml::Value) -> Result<T, String> {
    value
        .clone()
        .try_into()
        .map_err(|error: toml::de::Error| format!("{at}: {}", one_line(error.message())))
}

/// `message` with its line breaks folded, for a one-line notice.
fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What [`UiStore::load`] read: the state to start from, and the header flash when the file
/// could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Loaded {
    /// The state: the file's, or the defaults.
    pub state: UiState,
    /// Why the file was not used, for the header; `None` on a good file or a first launch.
    pub notice: Option<String>,
}

/// Where the UI state is kept (D1).
#[derive(Debug, Clone)]
pub enum UiStore {
    /// `<config root>/ui-state.toml`.
    File(PathBuf),
    /// The file's text in memory (the testkit's, and `App::new`'s): a clone shares it, so two
    /// harnesses over one handle see a restart. `None` is a first launch.
    Memory(Arc<Mutex<Option<String>>>),
    /// No config directory: every launch is the first and saves are dropped.
    Nowhere,
    /// `<config root>/ui-state.toml` behind a latest-wins writer thread (RV2 H1): what `run`
    /// saves through, so no write or `fsync` ever runs on the UI loop. Made by
    /// [`UiStore::background`].
    Background(UiWriter),
}

impl UiStore {
    /// An empty in-memory store: its first load is a first launch.
    #[must_use]
    pub fn memory() -> Self {
        Self::Memory(Arc::new(Mutex::new(None)))
    }

    /// The state to start from. Never fails: a missing file is the first launch (the defaults, no
    /// notice), and a file that cannot be used gives the defaults and the reason.
    #[must_use]
    pub fn load(&self) -> Loaded {
        let text = match self {
            Self::File(path) => match std::fs::read_to_string(path) {
                Ok(text) => Some(text),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Loaded::fallback(&format!("unreadable ({error})")),
            },
            Self::Memory(text) => text.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            Self::Background(writer) => return Self::File(writer.path().to_owned()).load(),
            Self::Nowhere => None,
        };
        match text.as_deref().map(parse) {
            None => Loaded::default(),
            Some(Ok(state)) => Loaded {
                state,
                notice: None,
            },
            Some(Err(reason)) => Loaded::fallback(&reason),
        }
    }

    /// Writes `state`. A file is replaced atomically: the text goes to a temp file of its own in
    /// the same directory (`.ui-state.toml.<random>.tmp`, so two htui processes never share
    /// one), is synced, then renamed over the file, so a crash or a second writer leaves the old
    /// file or the new one, never half of either (RV2 M1). `Background` only renders the text and
    /// hands it to its writer, which reports a failure through
    /// [`take_failure`](Self::take_failure) (RV2 H1). `Nowhere` drops it.
    ///
    /// # Errors
    /// The directory cannot be created or the file cannot be written; the reason names the path.
    /// Never for `Background`: its writes fail later, on its thread.
    pub fn save(&self, state: &UiState) -> Result<(), String> {
        match self {
            Self::File(path) => {
                write_atomically(path, &render(state)).map_err(|error| cannot_save(path, &error))
            }
            Self::Memory(text) => {
                *text.lock().unwrap_or_else(PoisonError::into_inner) = Some(render(state));
                Ok(())
            }
            Self::Background(writer) => {
                writer.post(render(state));
                Ok(())
            }
            Self::Nowhere => Ok(()),
        }
    }

    /// This store with its saves moved off the caller's thread (RV2 H1): a file goes behind a
    /// latest-wins [`UiWriter`]; memory stays synchronous, so a test stays deterministic, and
    /// nowhere stays nowhere. A writer thread that cannot be started leaves the file as it is
    /// (saved in place, as before).
    #[must_use]
    pub fn background(self) -> Self {
        match self {
            Self::File(path) => match UiWriter::spawn(path.clone()) {
                Ok(writer) => Self::Background(writer),
                Err(error) => {
                    tracing::warn!(%error, "the UI-state writer did not start; saving in place");
                    Self::File(path)
                }
            },
            other => other,
        }
    }

    /// Waits until the background writer has written everything handed to it; at once for the
    /// other stores.
    pub fn settle(&self) {
        if let Self::Background(writer) = self {
            writer.settle();
        }
    }

    /// The reason the background writer's last write failed, once; `None` when it succeeded (a
    /// later good write clears an earlier failure: the file is good) and for the other stores,
    /// whose [`save`](Self::save) says so itself.
    #[must_use]
    pub fn take_failure(&self) -> Option<String> {
        match self {
            Self::Background(writer) => writer.take_failure(),
            _ => None,
        }
    }

    /// Writes what is still pending before htui exits (RV2 H1): [`settle`](Self::settle), then
    /// [`take_failure`](Self::take_failure).
    ///
    /// # Errors
    /// The background writer's last write failed; the reason names the path.
    pub fn flush(&self) -> Result<(), String> {
        self.settle();
        self.take_failure().map_or(Ok(()), Err)
    }
}

/// A latest-wins writer of the UI-state file on a thread of its own (RV2 H1): a save hands it the
/// rendered text and returns; a text handed over before the last one was written replaces it, so
/// a burst of layout keys (or a key's auto-repeat) costs one write at the end, not one per key.
/// Clones share the thread; dropping the last one writes what is pending, then joins it.
#[derive(Debug, Clone)]
pub struct UiWriter(Arc<WriterThread>);

/// The thread behind a [`UiWriter`], joined when the last handle goes.
#[derive(Debug)]
struct WriterThread {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// What the UI and the writer thread share.
#[derive(Debug)]
struct Shared {
    path: PathBuf,
    slot: Mutex<Slot>,
    /// Signalled when a text is posted, a write ends, or the writer is closed.
    changed: Condvar,
}

/// The one-text mailbox between the UI and the writer.
#[derive(Debug, Default)]
struct Slot {
    /// The latest text not yet picked up.
    pending: Option<String>,
    /// A write is in progress.
    busy: bool,
    /// The last handle went: write what is pending, then stop.
    closed: bool,
    /// Why the last write failed, until taken.
    failure: Option<String>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The writer thread's loop: the latest pending text, written, until closed with nothing
    /// pending.
    fn run(&self) {
        let mut slot = self.lock();
        loop {
            if let Some(text) = slot.pending.take() {
                slot.busy = true;
                drop(slot);
                let written =
                    write_atomically(&self.path, &text).map_err(|e| cannot_save(&self.path, &e));
                slot = self.lock();
                slot.busy = false;
                slot.failure = written.err();
                self.changed.notify_all();
            } else if slot.closed {
                return;
            } else {
                slot = self
                    .changed
                    .wait(slot)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }
}

impl UiWriter {
    /// Starts the writer thread for `path`.
    fn spawn(path: PathBuf) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            path,
            slot: Mutex::new(Slot::default()),
            changed: Condvar::new(),
        });
        let thread = std::thread::Builder::new()
            .name("htui-ui-state".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                move || shared.run()
            })?;
        Ok(Self(Arc::new(WriterThread {
            shared,
            thread: Some(thread),
        })))
    }

    /// The file it writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0.shared.path
    }

    /// Hands `text` to the thread, replacing a text it has not picked up yet.
    fn post(&self, text: String) {
        self.0.shared.lock().pending = Some(text);
        self.0.shared.changed.notify_all();
    }

    /// Waits until nothing is pending or being written (or the thread is gone).
    fn settle(&self) {
        let shared = &self.0.shared;
        let mut slot = shared.lock();
        while (slot.pending.is_some() || slot.busy) && !self.0.finished() {
            slot = shared
                .changed
                .wait_timeout(slot, std::time::Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn take_failure(&self) -> Option<String> {
        self.0.shared.lock().failure.take()
    }
}

impl WriterThread {
    /// Whether the thread has stopped (it only does once closed, or if it panicked).
    fn finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }
}

impl Drop for WriterThread {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The one-line reason a save of `path` failed.
fn cannot_save(path: &Path, error: &io::Error) -> String {
    format!("cannot save {}: {error}", path.display())
}

impl Loaded {
    /// The defaults, with the notice for a file that could not be used.
    fn fallback(reason: &str) -> Self {
        Self {
            state: UiState::default(),
            notice: Some(format!(
                "{FILE_NAME}: {reason}; using the default layout (the next change rewrites it)"
            )),
        }
    }
}

/// `text` into `path` through a synced temp file beside it and a rename (RV2 M1): the temp file
/// is this write's own (`.ui-state.toml.<random>.tmp`), so a second htui writing at once never
/// writes into it or renames it away, and when any step fails it is removed: only it.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write as _;

    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path names no file"))?;
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            std::fs::create_dir_all(dir)?;
            dir
        }
        _ => Path::new("."),
    };
    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(name);
    prefix.push(".");
    // Dropped on every early return, which deletes it.
    let mut temp = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(dir)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path).map(drop).map_err(|failed| failed.error)
}

#[cfg(test)]
mod tests {
    use super::{FILE_NAME, UiState, parse, render};
    use crate::ui::shell::{ListSize, Preset, TabLayout};

    /// A state with every table: the blueprint's example plus a Skills-shaped layout.
    fn full() -> UiState {
        let mut state = UiState {
            about: Some(false),
            ..UiState::default()
        };
        state.layouts.insert(
            "backlog".to_owned(),
            TabLayout {
                list: ListSize::Percent(55),
                ..TabLayout::DEFAULT
            },
        );
        state.layouts.insert(
            "skills".to_owned(),
            TabLayout {
                preset: Preset::Columns,
                list: ListSize::Cells(32),
                activity_pct: 25,
                list_on: false,
                activity_on: true,
                zoom: true,
            },
        );
        state.modals.insert("item_form".to_owned(), (80, 24));
        state
    }

    /// The blueprint's file, verbatim apart from comments.
    const BACKLOG_FILE: &str = "\
version = 1

[about]
visible = false

[layout.backlog]
preset = \"ide\"
list_percent = 55
activity_percent = 30
list = true
activity = true
zoom = false

[modal.item_form]
width = 80
height = 24
";

    fn reason(text: &str) -> String {
        parse(text).expect_err("the text is invalid")
    }

    #[test]
    fn the_file_is_named_ui_state_toml() {
        assert_eq!(FILE_NAME, "ui-state.toml");
    }

    #[test]
    fn a_state_round_trips_through_its_text() {
        let state = full();
        assert_eq!(parse(&render(&state)), Ok(state));
    }

    #[test]
    fn the_default_state_round_trips_and_writes_only_the_version() {
        assert_eq!(render(&UiState::default()), "version = 1\n");
        assert_eq!(parse("version = 1\n"), Ok(UiState::default()));
    }

    #[test]
    fn the_blueprint_file_reads_and_writes_back_byte_for_byte() {
        let state = parse(BACKLOG_FILE).expect("the blueprint's file reads");
        assert_eq!(state.about, Some(false));
        assert_eq!(
            state.layouts.get("backlog"),
            Some(&TabLayout {
                list: ListSize::Percent(55),
                ..TabLayout::DEFAULT
            })
        );
        assert_eq!(state.modals.get("item_form"), Some(&(80, 24)));
        assert_eq!(render(&state), BACKLOG_FILE);
    }

    #[test]
    fn the_same_state_always_renders_the_same_text_in_key_order() {
        let text = render(&full());
        assert_eq!(text, render(&full()));
        let backlog = text.find("[layout.backlog]").expect("backlog is written");
        let skills = text.find("[layout.skills]").expect("skills is written");
        assert!(backlog < skills, "{text}");
        let (backlog_table, skills_table) = text[backlog..].split_at(skills - backlog);
        assert!(backlog_table.contains("list_percent = 55"), "{text}");
        assert!(!backlog_table.contains("list_cells"), "{text}");
        assert!(skills_table.contains("list_cells = 32"), "{text}");
        assert!(!skills_table.contains("list_percent"), "{text}");
        assert!(skills_table.contains("preset = \"columns\""), "{text}");
    }

    #[test]
    fn every_preset_is_written_by_name_and_read_back() {
        for &preset in Preset::ALL {
            let mut state = UiState::default();
            state.layouts.insert(
                "requirements".to_owned(),
                TabLayout {
                    preset,
                    ..TabLayout::DEFAULT
                },
            );
            let text = render(&state);
            assert!(
                text.contains(&format!("preset = \"{}\"", preset.name())),
                "{text}"
            );
            assert_eq!(parse(&text), Ok(state));
        }
    }

    #[test]
    fn a_file_without_about_leaves_the_panel_to_the_first_launch_default() {
        assert_eq!(parse("version = 1\n").map(|s| s.about), Ok(None));
        assert_eq!(parse("version = 1\n[about]\n").map(|s| s.about), Ok(None));
        assert_eq!(
            parse("version = 1\n[about]\nvisible = true\n").map(|s| s.about),
            Ok(Some(true))
        );
    }

    #[test]
    fn unknown_keys_and_tables_are_ignored() {
        let text = "\
version = 1
theme = \"dark\"

[about]
visible = true
pinned = 3

[layout.backlog]
preset = \"focus\"
list_cells = 40
activity_percent = 20
list = false
activity = false
zoom = false
split = \"vertical\"

[history]
cap = 50
";
        let state = parse(text).expect("unknown keys are ignored");
        assert_eq!(state.about, Some(true));
        assert_eq!(
            state.layouts.get("backlog"),
            Some(&TabLayout {
                preset: Preset::Focus,
                list: ListSize::Cells(40),
                activity_pct: 20,
                list_on: false,
                activity_on: false,
                zoom: false,
            })
        );
        assert!(state.modals.is_empty());
    }

    #[test]
    fn text_that_is_not_toml_says_so_with_its_line() {
        let why = reason("version = 1\n[layout.backlog\n");
        assert!(why.starts_with("not TOML (line 2): "), "{why}");
        assert!(!why.contains('\n'), "one line for the notice: {why}");
    }

    #[test]
    fn a_missing_or_other_version_is_refused() {
        assert_eq!(
            reason(""),
            "version: missing (this htui writes version = 1)"
        );
        assert_eq!(
            reason("version = 2\n"),
            "version: 2 is not one this htui reads (it writes version = 1)"
        );
        assert_eq!(
            reason("version = \"1\"\n"),
            "version: \"1\" is not one this htui reads (it writes version = 1)"
        );
    }

    #[test]
    fn an_unknown_preset_is_refused_with_the_names_it_could_be() {
        let why = reason(
            "version = 1\n[layout.backlog]\npreset = \"wide\"\nlist_percent = 55\n\
             activity_percent = 30\nlist = true\nactivity = true\nzoom = false\n",
        );
        assert_eq!(
            why,
            "layout.backlog: unknown preset \"wide\" (one of ide, ide_activity, columns, focus)"
        );
    }

    #[test]
    fn a_layout_needs_exactly_one_list_size() {
        let tail = "activity_percent = 30\nlist = true\nactivity = true\nzoom = false\n";
        let both = format!(
            "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 55\nlist_cells = 32\n{tail}"
        );
        let neither = format!("version = 1\n[layout.backlog]\npreset = \"ide\"\n{tail}");
        let expected = "layout.backlog: write exactly one of list_percent and list_cells";
        assert_eq!(reason(&both), expected);
        assert_eq!(reason(&neither), expected);
    }

    #[test]
    fn a_percentage_over_100_is_refused() {
        let over_list = "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 101\n\
                         activity_percent = 30\nlist = true\nactivity = true\nzoom = false\n";
        let over_activity = "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 50\n\
                             activity_percent = 150\nlist = true\nactivity = true\nzoom = false\n";
        assert_eq!(
            reason(over_list),
            "layout.backlog: list_percent is 101, over 100"
        );
        assert_eq!(
            reason(over_activity),
            "layout.backlog: activity_percent is 150, over 100"
        );
    }

    #[test]
    fn a_table_of_the_wrong_shape_names_where() {
        let missing = reason(
            "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 50\n\
             activity_percent = 30\nlist = true\nactivity = true\n",
        );
        assert!(missing.starts_with("layout.backlog: "), "{missing}");
        assert!(missing.contains("zoom"), "{missing}");
        let negative = reason("version = 1\n[modal.item_form]\nwidth = -1\nheight = 24\n");
        assert!(negative.starts_with("modal.item_form: "), "{negative}");
        let about = reason("version = 1\n[about]\nvisible = \"yes\"\n");
        assert!(about.starts_with("about: "), "{about}");
        assert_eq!(reason("version = 1\nlayout = 3\n"), "layout: not a table");
        for why in [missing, negative, about] {
            assert!(!why.contains('\n'), "one line for the notice: {why}");
        }
    }

    mod store {
        use super::full;
        use crate::ui::state_file::{FILE_NAME, Loaded, UiState, UiStore, render};
        use std::path::Path;

        const FALLBACK: &str = "; using the default layout (the next change rewrites it)";

        /// The names in `dir`, sorted.
        fn entries(dir: &Path) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .expect("the directory reads")
                .map(|entry| {
                    entry
                        .expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }

        fn first_launch() -> Loaded {
            Loaded {
                state: UiState::default(),
                notice: None,
            }
        }

        #[test]
        fn an_empty_memory_store_is_a_first_launch() {
            assert_eq!(UiStore::memory().load(), first_launch());
        }

        #[test]
        fn the_memory_store_keeps_the_real_text_and_reads_it_back() {
            let store = UiStore::memory();
            store.save(&full()).expect("memory saves");
            let UiStore::Memory(text) = &store else {
                panic!("a memory store");
            };
            assert_eq!(
                text.lock().expect("unpoisoned").as_deref(),
                Some(render(&full()).as_str())
            );
            assert_eq!(
                store.load(),
                Loaded {
                    state: full(),
                    notice: None,
                }
            );
        }

        #[test]
        fn a_clone_of_the_memory_store_sees_the_same_file_like_a_restart() {
            let first = UiStore::memory();
            let second = first.clone();
            first.save(&full()).expect("memory saves");
            assert_eq!(second.load().state, full());
            second.save(&UiState::default()).expect("memory saves");
            assert_eq!(first.load(), first_launch());
        }

        #[test]
        fn invalid_text_in_memory_gives_the_defaults_and_the_reason() {
            let store = UiStore::Memory(std::sync::Arc::new(std::sync::Mutex::new(Some(
                "version = 2\n".to_owned(),
            ))));
            assert_eq!(
                store.load(),
                Loaded {
                    state: UiState::default(),
                    notice: Some(format!(
                        "ui-state.toml: version: 2 is not one this htui reads \
                         (it writes version = 1){FALLBACK}"
                    )),
                }
            );
        }

        #[test]
        fn a_missing_file_is_a_first_launch() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let store = UiStore::File(dir.path().join(FILE_NAME));
            assert_eq!(store.load(), first_launch());
            assert!(entries(dir.path()).is_empty(), "loading writes nothing");
        }

        #[test]
        fn a_save_creates_the_directory_writes_the_text_and_leaves_no_temp_file() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let root = dir.path().join("htui").join("config");
            let path = root.join(FILE_NAME);
            let store = UiStore::File(path.clone());
            store.save(&full()).expect("the file saves");
            assert_eq!(entries(&root), [FILE_NAME]);
            assert_eq!(
                std::fs::read_to_string(&path).expect("the file reads"),
                render(&full())
            );
            assert_eq!(store.load().state, full());
        }

        #[test]
        fn a_second_save_replaces_the_file_and_still_leaves_no_temp_file() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let store = UiStore::File(dir.path().join(FILE_NAME));
            store.save(&full()).expect("the first save");
            let about_on = UiState {
                about: Some(true),
                ..UiState::default()
            };
            store.save(&about_on).expect("the second save");
            assert_eq!(entries(dir.path()), [FILE_NAME]);
            assert_eq!(
                store.load(),
                Loaded {
                    state: about_on,
                    notice: None,
                }
            );
        }

        #[test]
        fn an_invalid_file_gives_the_defaults_and_the_reason_and_is_left_alone() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            std::fs::write(&path, "version = 1\n[layout.backlog\n").expect("the file writes");
            let loaded = UiStore::File(path.clone()).load();
            assert_eq!(loaded.state, UiState::default());
            let notice = loaded.notice.expect("a notice");
            assert!(
                notice.starts_with("ui-state.toml: not TOML (line 2): "),
                "{notice}"
            );
            assert!(notice.ends_with(FALLBACK), "{notice}");
            assert_eq!(
                std::fs::read_to_string(&path).expect("the file reads"),
                "version = 1\n[layout.backlog\n",
                "only the next change rewrites it"
            );
        }

        #[test]
        fn an_unreadable_file_gives_the_defaults_and_the_reason() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("the file writes");
            let loaded = UiStore::File(path).load();
            assert_eq!(loaded.state, UiState::default());
            let notice = loaded.notice.expect("a notice");
            assert!(
                notice.starts_with("ui-state.toml: unreadable ("),
                "{notice}"
            );
            assert!(notice.ends_with(FALLBACK), "{notice}");
        }

        #[test]
        fn a_failed_save_names_the_path_and_leaves_no_temp_file() {
            let dir = tempfile::tempdir().expect("a temp dir");
            // A directory where the file goes: the rename over it fails.
            let path = dir.path().join(FILE_NAME);
            std::fs::create_dir(&path).expect("the directory is made");
            let why = UiStore::File(path.clone())
                .save(&full())
                .expect_err("a directory cannot be replaced by the file");
            assert!(
                why.starts_with(&format!("cannot save {}: ", path.display())),
                "{why}"
            );
            assert_eq!(entries(dir.path()), [FILE_NAME]);
        }

        #[test]
        fn a_save_under_a_file_fails_without_writing() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let blocker = dir.path().join("config");
            std::fs::write(&blocker, "").expect("the file writes");
            let path = blocker.join(FILE_NAME);
            let why = UiStore::File(path.clone())
                .save(&full())
                .expect_err("a file is no directory");
            assert!(
                why.starts_with(&format!("cannot save {}: ", path.display())),
                "{why}"
            );
            assert_eq!(entries(dir.path()), ["config"]);
        }

        #[test]
        fn nowhere_is_always_a_first_launch_and_drops_saves() {
            let store = UiStore::Nowhere;
            assert_eq!(store.save(&full()), Ok(()));
            assert_eq!(store.load(), first_launch());
        }

        /// RV2 M1: a failed save removes the temp file it wrote and nothing else: a temp file
        /// another htui is writing beside it is left alone.
        #[test]
        fn a_failed_save_removes_only_its_own_temp_file() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            std::fs::create_dir(&path).expect("the directory is made");
            let foreign = dir.path().join(".ui-state.toml.tmp");
            std::fs::write(&foreign, "version = 1\n").expect("the other htui's temp file");
            let _ = UiStore::File(path)
                .save(&full())
                .expect_err("a directory cannot be replaced by the file");
            assert_eq!(entries(dir.path()), [".ui-state.toml.tmp", FILE_NAME]);
            assert_eq!(
                std::fs::read_to_string(&foreign).expect("still there"),
                "version = 1\n",
                "the other htui's temp file is untouched"
            );
        }

        /// RV2 M1: two htui processes saving at once (two stores over one path) never fail and
        /// never leave a torn file: every read is one whole state or the other.
        #[test]
        fn two_concurrent_writers_never_yield_a_torn_read() {
            const SAVES: usize = 300;
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            let one = full();
            let mut other = full();
            other.about = Some(true);
            other.modals.insert("leave".to_owned(), (40, 12));
            let writer = |state: UiState| {
                let store = UiStore::File(path.clone());
                std::thread::spawn(move || {
                    (0..SAVES)
                        .filter_map(|_| store.save(&state).err())
                        .collect::<Vec<_>>()
                })
            };
            let first = writer(one.clone());
            let second = writer(other.clone());
            let reader = {
                let store = UiStore::File(path.clone());
                let (one, other) = (one.clone(), other.clone());
                std::thread::spawn(move || {
                    let mut torn = Vec::new();
                    for _ in 0..SAVES {
                        let loaded = store.load();
                        let whole = loaded.notice.is_none()
                            && (loaded.state == one
                                || loaded.state == other
                                || loaded.state == UiState::default());
                        if !whole {
                            torn.push(loaded);
                        }
                    }
                    torn
                })
            };
            let failures: Vec<String> = [first, second]
                .into_iter()
                .flat_map(|writer| writer.join().expect("the writer ran"))
                .collect();
            let torn = reader.join().expect("the reader ran");
            assert!(
                failures.is_empty(),
                "{} failed saves: {failures:?}",
                failures.len()
            );
            assert!(
                torn.is_empty(),
                "{} torn reads: {:?}",
                torn.len(),
                torn.first()
            );
            assert_eq!(entries(dir.path()), [FILE_NAME], "no temp file is left");
        }

        /// RV2 H1: the background writer saves off the caller's thread, the latest state wins,
        /// and `flush` waits for it.
        #[test]
        fn the_background_writer_saves_the_latest_state_and_flush_waits_for_it() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            let store = UiStore::File(path.clone()).background();
            assert!(matches!(store, UiStore::Background(_)), "{store:?}");
            let mut state = UiState::default();
            for width in 1..=50 {
                state.modals.insert("item_form".to_owned(), (width, 24));
                store
                    .save(&state)
                    .expect("a background save never fails at once");
            }
            assert_eq!(store.flush(), Ok(()));
            assert_eq!(
                std::fs::read_to_string(&path).expect("the file reads"),
                render(&state),
                "the last state is the one on disk"
            );
            assert_eq!(store.load().state, state);
            assert_eq!(entries(dir.path()), [FILE_NAME]);
        }

        /// RV2 H1: a write the background writer could not make comes back to the caller, once.
        #[test]
        fn a_background_save_that_fails_is_reported_once() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            std::fs::create_dir(&path).expect("the directory is made");
            let store = UiStore::File(path.clone()).background();
            store.save(&full()).expect("queued");
            store.settle();
            let why = store.take_failure().expect("the failure is reported");
            assert!(
                why.starts_with(&format!("cannot save {}: ", path.display())),
                "{why}"
            );
            assert_eq!(store.take_failure(), None, "once");
            store.save(&full()).expect("queued");
            assert!(store.flush().is_err(), "flush reports it too");
            assert_eq!(store.take_failure(), None);
        }

        /// RV2 H1: a later save that succeeds clears a failure nobody was shown yet: the file
        /// is good.
        #[test]
        fn a_background_save_that_succeeds_clears_an_earlier_failure() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            std::fs::create_dir(&path).expect("the directory is made");
            let store = UiStore::File(path.clone()).background();
            store.save(&full()).expect("queued");
            store.settle();
            std::fs::remove_dir(&path).expect("the directory goes");
            store.save(&full()).expect("queued");
            assert_eq!(store.flush(), Ok(()));
            assert_eq!(store.load().state, full());
        }

        /// RV2 H1: dropping the last handle (htui quitting) writes the state still pending.
        #[test]
        fn dropping_the_background_writer_writes_what_is_pending() {
            let dir = tempfile::tempdir().expect("a temp dir");
            let path = dir.path().join(FILE_NAME);
            let store = UiStore::File(path.clone()).background();
            let copy = store.clone();
            store.save(&full()).expect("queued");
            drop(store);
            drop(copy);
            assert_eq!(UiStore::File(path).load().state, full());
        }

        /// RV2 H1: only a file moves to the background; memory stays synchronous (so a test is
        /// deterministic) and nowhere stays nowhere.
        #[test]
        fn memory_and_nowhere_stay_as_they_are_in_the_background() {
            let memory = UiStore::memory().background();
            assert!(matches!(memory, UiStore::Memory(_)), "{memory:?}");
            memory.save(&full()).expect("memory saves");
            assert_eq!(memory.load().state, full(), "at once");
            assert_eq!(memory.flush(), Ok(()));
            assert_eq!(memory.take_failure(), None);
            assert!(matches!(UiStore::Nowhere.background(), UiStore::Nowhere));
        }
    }
}

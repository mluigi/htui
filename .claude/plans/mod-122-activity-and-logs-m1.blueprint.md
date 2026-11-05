# Blueprint: MOD-122 M1 — Logs tab and logging settings (T1–T3)

**Plan**: `.claude/plans/mod-122-activity-and-logs-m1.plan.md` (D1–D7 settled; not reopened here).
**PRD**: `.claude/prds/mod-122-activity-and-logs.prd.md` (milestone 1).
No source file is written by this blueprint. Line anchors are from `hr/MOD-122` at `d70d4df6`.

## Plan corrections and gaps (read first)

| # | Plan claim / omission | Finding | Consequence |
|---|---|---|---|
| F1 | V9: T2 ∩ T3 = {`app/mod.rs`, `README.md`} | **Incomplete.** Every Settings section has its own key context and browse stack (`Context::SettingsQueue` + `views::QUEUE_BROWSE` `:477`, `SettingsProjectTypes` + `PROJECT_TYPES_BROWSE` `:487`). The Logging section needs `Context::SettingsLogging` + `LOGGING_BROWSE`, so T3 also writes `keys/catalogue.rs`, `keys/views.rs` and `keys/stack.rs` (`DECLARED`), the same three files T2 writes | **B1**: T1 adds both new `Context` variants (the part of `catalogue.rs` that would collide: enum, `ALL`, `table`, `heading`, `is_view`, `context_position` renumbering, two tests). T2 and T3 then touch `views.rs` and `stack.rs` in **disjoint hunks** (below). `catalogue.rs` rows and `docs/htui-keys.md` stay T2-only because T3 adds no row and no view default |
| F2 | T2 "`DELEGATES`" | Not needed for T2: every `logs.*`, `list.*` and `pane.*` act is handled in `ui/tabs/logs/mod.rs`, the file that names `views::LOGS_BROWSE`. An entry whose file handles nothing extra fails `every_act_is_offered_by_a_stack_and_handled_by_a_view` (`views.rs:2104-2106`). T3 **does** need `("LOGGING_BROWSE", &[SETTINGS_TAB_RS])`, as every section stack has | T2: no `DELEGATES` line; T3: one |
| F3 | Plan omits `keys/stack.rs` | `DECLARED` (`stack.rs:226-317`) must list each new stack, or the validator never checks it | T2 appends `("in Logs", views::LOGS_BROWSE)` after `LEAVE`; T3 inserts `("in Settings > Logging", views::LOGGING_BROWSE)` right after `("in Settings > Project types", …)`. Every pair `LOGGING_BROWSE` can produce is already produced by `QUEUE_BROWSE` earlier, so no first-reported phrase moves |
| F4 | Validate lines `cargo test -p htui --all-features logging:: state_file:: -- …` | Cargo takes **one** `TESTNAME`; two positionals is a usage error | Use `cargo test -p htui --all-features --lib -- logging:: state_file:: --test-threads=1` (libtest takes many filters) |
| F5 | "`register_all` passes the buffer/control" | `register_all(app: &mut App)` has them only through `app.logging`. `app.register_tab(Box::new(LogsTab::new(app.logging.buffer().clone())))` compiles (two-phase borrow: a shared read in the argument of a `&mut self` method call), so neither task needs a shared `let` line | T2 and T3 each add exactly their line; no common `let logging = …` to merge |
| F6 | `App::new` needs a `Logging` | Harnesses build `App::new` (`testkit.rs:131`); hundreds of tests do | `Logging::detached()`: buffer + control, **no subscriber at all** (no `Dispatch`, nothing registered with `tracing-core`). D7 holds by construction |
| F7 | Snapshot churn (T2) | 40 `.snap` files show the tab strip ending `⁵Chat` (`integration__demo_shell`, `shell__*`, `requirements__*`, `skills__*`, `templates__*`, `concepts_search__*`, `waiting__*`, `htui__testkit__tests__shell_empty`). The 15 that show the Settings section strip (`hierarchy__*`, `kinds__*`, `prompt_settings__*`) show it windowed around the active section, so T3's append moves none. The two sets are disjoint | T2 owns the 40; T3 owns only its new snapshots. `integration.rs:56-60` reads the header at 160 columns and expects `0 working · 0 waiting · memory`: re-check with six chips |
| F8 | `docs/htui-cli.md` `:55`, `:420` | That page documents `status`/`waiting`/`--search-items`; they never read `ui-state.toml`, so nothing they do changes | T3: one sentence at `:55` (the TUI also keeps its log in memory and Settings › Logging can set a file; neither applies to these commands). `:420` stays |

Verified TRUE, no action: `Action` is matched exhaustively only in `App::apply` (`update.rs:40-74`) and passed through `drain`'s `other =>` arm (`state.rs:1036-1071`); `UiState` has no struct literal outside `state_file.rs` tests (both use `..Default`); `print` skips a context with no rows (`print.rs:31-84`), so T1's two contexts leave `docs/htui-keys.md` byte-identical; `G`/`v`/`F` collide with nothing in `LOGS_BROWSE` once its `list` layer omits `list.bottom` (the validator's stack pass, `validate.rs:438-466`); a version-1 file stays readable (`unknown_keys_and_tables_are_ignored`, `state_file.rs:671`); `on_refresh` reaches the active tab every 4th tick (`update.rs:263-276`); the wheel over a list + detail pane focuses it and presses `list.down`/`up` (`state.rs:1309-1321`).

## Design decisions (blueprint level, on top of D1–D7)

- **B1 — T1 owns the two contexts** (F1): `Context::SettingsLogging` after `SettingsProjectTypes`, `Context::Logs` after `Chat`; no rows, no stacks.
- **B2 — `Logging` carries an optional `Dispatch`.** `build` assembles the subscriber into a `Dispatch` without installing it; `init` = `build` + `try_init` (the same call as today, so the `log` bridge is unchanged); `detached` has none. Tests run events through `tracing::dispatcher::with_default(logging.dispatch().unwrap(), …)`.
- **B3 — one validator for a filter.** `logging::parse_filter` is what the control, the startup apply and the section all call, so a refusal reads the same everywhere.
- **B4 — the tab anchors a paused view on a sequence number**, `Scroll::At(seq of the top line)`, not on a row index: dropping old lines, changing the level or the source never moves what a paused reader sees. Otherwise it is `transcript.rs`'s `Scroll::Tail` machine (`:288-295`, `:552-583`).
- **B5 — an overridden field refuses its edit** ("set by `--log` for this session"): D5 says flag/env win for the session, and a saved value that would not apply is a trap. *Maintainer FYI at review.*
- **B6 — lines are one row each, clipped** (no wrap), so follow and paging count rows exactly; `layout.zoom` gives width.
- **B7 — no `Enter` view default for Settings › Logging** (`e` edits): a view default is a printed row and would put T3 into `docs/htui-keys.md` too.

---

## T1 — capture and control (serial, first)

### Files written

| File | Symbols |
|---|---|
| `crates/htui/src/logging/source.rs` | CREATE: `Source`, `TABLE`, `Source::{ALL, COUNT, of, name, from_name, index}` |
| `crates/htui/src/logging/buffer.rs` | CREATE: `CAPACITY`, `LogLine`, `Counts`, `SourceCounts`, `Since`, `LogBuffer`, `MemoryLayer`, `LineVisitor` |
| `crates/htui/src/logging/mod.rs` | CREATE: `Startup`, `Logging`, `LoggingControl`, `LoggingState`, `LoggingError`, `LoggingPrefs`, `FileSwitch`, `SwitchWriter`, `parse_filter`, `build`, `init`, consts |
| `crates/htui/src/lib.rs` | `pub mod logging;`; `run` (`:107`, `:256-271`); delete `init_tracing` (`:355-381`) and `use std::path::Path` (`:61`, now unused); comments naming `init_tracing` (`:110`, `:113`, `:128`); one source-order test |
| `crates/htui/src/app/state.rs` | `App.logging` field (after `ui_store`, `:542`), `App::new` (`:576-628`) |
| `crates/htui/src/app/action.rs` | `Action::Logging(LoggingAction)`, `LoggingAction` |
| `crates/htui/src/app/mod.rs` | `pub use action::{…, LoggingAction, …}` only (T1 runs first, so this is not a merge point) |
| `crates/htui/src/app/update.rs` | `apply` arm, `App::change_logging`, tests |
| `crates/htui/src/ui/state_file.rs` | `UiState.logging`, `FileDto.logging`, `LoggingDto`, `parse`, `render`, module doc example, tests |
| `crates/htui/src/keys/catalogue.rs` | B1: `Context::{SettingsLogging, Logs}` in the enum (`:61-169`), `ALL` (`:174-224`), `table`, `heading`, `is_view` (`:348-382`), `context_position` (`:2350-2402`, renumber), `every_context_is_listed_once_in_all` (`49` → `51`), `context_tables_and_headings` (`:3188-3312`) |

### `logging/source.rs`

```rust
/// Where a log line comes from (MOD-122 D3), in the requirement's order; `Other` last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source { Ui, Store, Orchestrator, Agents, Mcp, Worker, Secrets, Other }

impl Source {
    pub const ALL: [Self; 8] = [Ui, Store, Orchestrator, Agents, Mcp, Worker, Secrets, Other];
    pub const COUNT: usize = 8;
    pub const fn index(self) -> usize;            // position in ALL
    pub const fn name(self) -> &'static str;      // "ui", "store", …, "other"
    pub fn from_name(name: &str) -> Option<Self>;
    /// The longest prefix of TABLE that `target` equals or continues with `::`; then any
    /// `htui` / `htui::…` target is `Ui`; anything else `Other`.
    pub fn of(target: &str) -> Self;
}

/// D3. Matched by longest prefix, so table order does not matter.
const TABLE: &[(&str, Source)] = &[
    ("htui_secrets", Secrets), ("htui_store::secret", Secrets), ("htui::secrets", Secrets),
    ("htui::secrets_settings", Secrets),
    ("htui_store", Store), ("htui::store_worker", Store), ("sqlx", Store),
    ("htui_orch", Orchestrator),
    ("htui_agent", Agents), ("htui::agent_worker", Agents),
    ("htui_mcp", Mcp), ("htui::mcp_search", Mcp), ("htui::mcp_cmd", Mcp),
    ("htui_worker", Worker), ("htui::run_worker", Worker), ("htui::worker_cmd", Worker),
];
```
Boundary rule: `target == prefix || target.strip_prefix(prefix).is_some_and(|rest| rest.starts_with("::"))`, so `htui_storex` is `Other` and `htui::secrets_settings` never matches `htui::secrets` by accident.

### `logging/buffer.rs`

```rust
/// D2: lines kept; the oldest goes first.
pub const CAPACITY: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub seq: u64,                 // from 0, never reused
    pub at: chrono::DateTime<chrono::Utc>,
    pub level: tracing::Level,
    pub target: String,
    pub source: Source,
    pub text: String,             // message then ` key=value` fields, one line
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts { pub lines: usize, pub warnings: usize }   // warnings = WARN + ERROR

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceCounts([Counts; Source::COUNT]);
impl SourceCounts { pub fn of(&self, source: Source) -> Counts; pub fn all(&self) -> Counts; }

/// What one `since` read under one lock: the plan's `(lines, next_seq)` plus what the tab needs
/// to stay consistent with the ring.
#[derive(Debug, Clone, Default)]
pub struct Since {
    pub lines: Vec<LogLine>,      // every retained line with seq >= the one asked for
    pub next: u64,                // the seq the next line will take: ask from here next time
    pub oldest: u64,              // the oldest retained seq (== next when empty)
    pub counts: SourceCounts,     // counts of retained lines (D2)
}

/// The ring behind a `Mutex`, held only to push or copy (no `tracing` call inside it).
#[derive(Clone)]
pub struct LogBuffer(Arc<Mutex<Ring>>);
struct Ring { lines: VecDeque<LogLine>, next: u64, counts: SourceCounts, capacity: usize }

impl LogBuffer {
    pub fn new() -> Self;                                // CAPACITY
    pub fn push(&self, at: DateTime<Utc>, level: Level, target: &str, text: String) -> u64;
    pub fn since(&self, seq: u64) -> Since;
    pub fn len(&self) -> usize;  pub fn is_empty(&self) -> bool;
}
impl Default for LogBuffer;   impl fmt::Debug for LogBuffer  // `LogBuffer { lines: n }`, never the lines
```
- `push`: source = `Source::of(target)`; at capacity `pop_front` first and decrement that line's counts; then push and increment. Lock with `unwrap_or_else(PoisonError::into_inner)`, as `state_file.rs` does.
- `since(seq)`: `skip = seq.saturating_sub(oldest)` as an index (seqs are contiguous), clone the tail.

```rust
/// D1, D2: every event that passed the global filter, into the buffer.
pub struct MemoryLayer { buffer: LogBuffer }
impl MemoryLayer { pub fn new(buffer: LogBuffer) -> Self }
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for MemoryLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut line = LineVisitor::default();
        event.record(&mut line);                         // formatted before any lock
        let meta = event.metadata();
        let target = line.log_target.take().unwrap_or_else(|| meta.target().to_owned());
        self.buffer.push(chrono::Utc::now(), *meta.level(), &target, line.text());
    }
}

#[derive(Default)]
struct LineVisitor { message: String, fields: String, log_target: Option<String> }
impl tracing::field::Visit for LineVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message.push_str(value),
            "log.target" => self.log_target = Some(value.to_owned()),   // a `log` record (tracing-log)
            name if name.starts_with("log.") => {}
            name => { let _ = write!(self.fields, " {name}={value:?}"); }
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => { let _ = write!(self.message, "{value:?}"); }
            name if name.starts_with("log.") => {}
            name => { let _ = write!(self.fields, " {name}={value:?}"); }
        }
    }
}
impl LineVisitor {
    /// `message` then the fields; without a message the fields alone; `\r`/`\n` folded to a space.
    fn text(&self) -> String;
}
```
`{value:?}` of a `fmt::Arguments` message is unquoted and a `&str` field is quoted, as the `fmt` layer writes them.

### `logging/mod.rs`

```rust
pub mod buffer; pub mod source;
pub use buffer::{CAPACITY, Counts, LogBuffer, LogLine, MemoryLayer, Since, SourceCounts};
pub use source::Source;

pub const DEFAULT_FILTER: &str = "info";
pub const FILTER_VAR: &str = "HTUI_LOG_FILTER";
/// What the section names as the override of each field.
pub const SET_BY_FLAG: &str = "`--log`";
pub const SET_BY_VAR: &str = "`HTUI_LOG_FILTER`";

/// What the process was started with (flag/env). They win over saved values for the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Startup { pub file: Option<PathBuf>, pub filter: Option<String> }
impl Startup {
    /// `run`'s: `--log`/`HTUI_LOG` (clap reads both) and `HTUI_LOG_FILTER`.
    pub fn from_env(log: Option<&Path>) -> Self { Self::from_parts(log, std::env::var(FILTER_VAR).ok()) }
    /// A blank or unparsable variable counts as unset, as today (`lib.rs:368-369` fell back to info).
    pub fn from_parts(log: Option<&Path>, filter_var: Option<String>) -> Self;
}

/// The settings saved in `ui-state.toml`'s `[logging]` (D5). `None` = the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoggingPrefs { pub filter: Option<String>, pub file: Option<PathBuf> }
impl LoggingPrefs { pub fn is_empty(&self) -> bool }

/// D7: what `init` hands back; `App` holds it.
pub struct Logging { buffer: LogBuffer, control: LoggingControl, dispatch: Option<Dispatch>, sentry: bool }
impl Logging {
    /// The subscriber (D1) as a `Dispatch`, not installed.
    /// # Errors  the startup log file cannot be opened.
    pub fn build(startup: Startup) -> anyhow::Result<Self>;
    /// No subscriber: `App::new`'s, the harnesses'. The control records and switches the file.
    pub fn detached() -> Self;
    pub fn buffer(&self) -> &LogBuffer;
    pub fn control(&self) -> &LoggingControl;
    pub fn dispatch(&self) -> Option<&Dispatch>;
    pub(crate) fn sentry(&self) -> bool;   // D1 pin
}
impl fmt::Debug for Logging;   // App derives Debug

/// `build`, then installed process-wide. `run` calls it once; nothing else may.
/// # Errors  the startup log file cannot be opened, or a subscriber is already installed.
pub fn init(startup: Startup) -> anyhow::Result<Logging> {
    use tracing_subscriber::util::SubscriberInitExt as _;
    let logging = Logging::build(startup)?;
    if let Some(dispatch) = logging.dispatch.clone() {
        dispatch.try_init().map_err(|err| anyhow::anyhow!("could not install the log subscriber: {err}"))?;
    }
    Ok(logging)
}
```
`build`:
```rust
let initial = startup.filter.as_deref().unwrap_or(DEFAULT_FILTER);
let (filter, handle) = tracing_subscriber::reload::Layer::new(parse_filter(initial)?);
let file = startup.file.as_deref().map(open_append).transpose()?;   // OpenOptions create+append, as today
let switch = FileSwitch::new(file);
let buffer = LogBuffer::new();
let sentry = startup.file.is_some();                                  // D1: gated as today
let subscriber = tracing_subscriber::registry()
    .with(filter)
    .with(MemoryLayer::new(buffer.clone()))
    .with(tracing_subscriber::fmt::layer().with_writer(switch.clone()).with_ansi(false))
    .with(sentry.then(sentry_tracing::layer));
```
Then `LoggingControl::new(Some(handle), switch, &startup)` and `Dispatch::new(subscriber)`.

```rust
/// D7: the run-time handle the Settings section and `App` share. Clone = same state.
#[derive(Clone)]
pub struct LoggingControl(Arc<ControlInner>);
struct ControlInner {
    reload: Option<reload::Handle<EnvFilter, Registry>>,   // None when detached
    file: FileSwitch,
    state: Mutex<LoggingState>,
}

/// What the section shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingState {
    pub filter: String,                        // the directive in force
    pub file: Option<PathBuf>,                 // the file in force
    pub filter_set_by: Option<&'static str>,   // Some(SET_BY_VAR) when HTUI_LOG_FILTER won
    pub file_set_by: Option<&'static str>,     // Some(SET_BY_FLAG) when --log/HTUI_LOG won
    pub file_error: Option<String>,            // the last file that could not be opened; cleared by a good switch
}

impl LoggingControl {
    pub fn state(&self) -> LoggingState;
    /// # Errors  Overridden, Filter (nothing changes), Gone.
    pub fn set_filter(&self, directive: &str) -> Result<(), LoggingError>;
    /// Some: opened (create, append) **before** the swap; the old file drops after the lock.
    /// None: no file. # Errors  Overridden, File (the previous file stays; `file_error` set).
    pub fn set_file(&self, path: Option<&Path>) -> Result<(), LoggingError>;
    /// D5 startup: each saved field not overridden is applied; the first refusal becomes one
    /// line for the status line, e.g. "ui-state.toml: logging.filter `htui=[` not applied: …;
    /// using info". The other field still applies.
    pub fn apply_saved(&self, saved: Option<&LoggingPrefs>) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggingError {
    Filter { directive: String, why: String },   // "`{directive}` is not a log filter: {why}"
    File { path: PathBuf, why: String },         // "cannot open the log file {path}: {why}; the previous one stays"
    Overridden(&'static str),                    // "set by {flag} for this session"
    Gone,                                        // "the log subscriber is gone"
}
impl fmt::Display for LoggingError;  impl std::error::Error for LoggingError {}

/// B3: the one validator (`EnvFilter::try_new`); `why` folded to one line.
pub fn parse_filter(directive: &str) -> Result<EnvFilter, LoggingError>;
```
Switchable writer (D1):
```rust
#[derive(Clone, Default)]
struct FileSwitch(Arc<Mutex<Option<std::fs::File>>>);
struct SwitchWriter<'a>(MutexGuard<'a, Option<std::fs::File>>);
impl io::Write for SwitchWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.as_mut() { Some(file) => file.write(buf), None => Ok(buf.len()) }   // no file: dropped
    }
    fn flush(&mut self) -> io::Result<()> { self.0.as_mut().map_or(Ok(()), io::Write::flush) }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileSwitch {
    type Writer = SwitchWriter<'a>;
    fn make_writer(&'a self) -> Self::Writer { SwitchWriter(self.0.lock().unwrap_or_else(PoisonError::into_inner)) }
}
```
The `fmt` layer formats into its own buffer and holds the guard for one `write_all`. No `tracing` call happens while `FileSwitch`, `state` or the ring is locked (plan risk "reentrancy").

### `app/action.rs`, `app/state.rs`, `app/update.rs`

```rust
// action.rs, after `History(HistoryAction)`:
/// Apply and save one logging setting (MOD-122 D5): emitted by Settings › Logging.
Logging(LoggingAction),

/// One logging setting (MOD-122 D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggingAction {
    /// A filter directive the section validated (`logging::parse_filter`).
    Filter(String),
    /// A log file, or none.
    File(Option<PathBuf>),
}
```
`App` field (pub, after `ui_store`): `/// MOD-122 D7: the log buffer and control. App::new's is detached; run hands it the installed one. pub logging: Logging,` and `logging: Logging::detached(),` in `App::new`.

`update.rs` `apply`: `Action::Logging(change) => self.change_logging(change),` (before `Error`; `self.dirty = true` follows as for every arm).
```rust
/// MOD-122 D5, D6: applies through the control; on success records it in `self.ui.logging` and
/// saves `ui-state.toml` (latest-wins writer) and logs the directive or the path, never a value
/// read from the keyring or the environment. A refusal goes on the status line; nothing changes.
fn change_logging(&mut self, change: LoggingAction) {
    let control = self.logging.control().clone();
    let applied = match &change {
        LoggingAction::Filter(directive) => control.set_filter(directive),
        LoggingAction::File(path) => control.set_file(path.as_deref()),
    };
    if let Err(err) = applied { self.status = Some(err.to_string()); return; }
    let prefs = self.ui.logging.get_or_insert_with(LoggingPrefs::default);
    match change {
        LoggingAction::Filter(directive) => { tracing::info!(filter = %directive, "log filter set in Settings"); prefs.filter = Some(directive); }
        LoggingAction::File(path) => { tracing::info!(file = ?path, "log file set in Settings"); prefs.file = path; }
    }
    self.save_ui_state();
}
```

### `ui/state_file.rs`

- `UiState` gains `/// MOD-122 D5: [logging]; None = nothing saved. pub logging: Option<LoggingPrefs>,` (import `crate::logging::LoggingPrefs`; the shell type carries no serde).
- `FileDto` gains `#[serde(skip_serializing_if = "Option::is_none")] logging: Option<LoggingDto>,` **between `about` and `layout`**, so `BACKLOG_FILE` still renders byte for byte.
- ```rust
  /// `[logging]` (MOD-122 D5). Strings, so a path never fails to serialise.
  #[derive(Debug, Serialize, Deserialize)]
  struct LoggingDto {
      #[serde(default, skip_serializing_if = "Option::is_none")] filter: Option<String>,
      #[serde(default, skip_serializing_if = "Option::is_none")] file: Option<String>,
  }
  ```
- `parse`: after `[about]`, `if let Some(value) = table.get("logging") { let dto: LoggingDto = shaped("logging", value)?; state.logging = dto.into_prefs(); }`. `into_prefs` maps `file = ""` to `None` and returns `None` when both are `None`. **The filter is not validated here**: a bad directive is kept and refused, with a notice, by `apply_saved` at startup (D5 "a notice, not a refusal of the whole file"). A wrong *type* (`filter = 3`) is refused like any other table (`"logging: …"`).
- `render`: `logging: state.logging.as_ref().filter(|p| !p.is_empty()).map(LoggingDto::of)`.
- Module doc example gains `[logging]` with `filter = "info"` and `file = "/home/me/htui.log"`. `VERSION` stays 1.

### `lib.rs::run` (startup precedence)

- `:107` `init_tracing(args.log.as_deref())?;` becomes
  `let logging = logging::init(logging::Startup::from_env(args.log.as_deref()))?;` with a comment: MOD-122 D1 one subscriber always: memory, a switchable file, Sentry only on a startup `--log` exactly as before. Same place in the order: after the `mcp`/`worker` early returns, before `provision`.
- After `let mut app = App::new(request_tx);` (`:249`): `app.logging = logging;` (D7: before `register_all`, which hands the buffer and the control to the tab and the section).
- `:265-270` become:
  ```rust
  let loaded = ui_store.load();
  // MOD-122 D5: the saved logging settings, once `ui-state.toml` is read; `--log`/`HTUI_LOG` and
  // `HTUI_LOG_FILTER` keep theirs for this session. One that cannot apply is a notice.
  let logging_notice = app.logging.control().apply_saved(loaded.state.logging.as_ref());
  app.apply_ui_state(loaded.state);
  app.ui_store = ui_store.background();
  app.status = loaded.notice.or(logging_notice);
  ```
- `run`'s doc: "logging" in the startup order now reads "logging (always captured in memory; MOD-122)"; `# Errors` unchanged ("the log file cannot be opened").
- The non-TUI verbs (`provision`, `status`, `waiting`, `claude emit`, the keyring/Qdrant flags) keep running after `logging::init`; they never read `ui-state.toml`, so only flag/env apply there, as today.

### `keys/catalogue.rs` (B1)

- Enum: `/// [settings.logging]: the Logging section (no own action; overrides only). SettingsLogging,` after `SettingsProjectTypes`; `/// [logs]: the Logs tab (MOD-122). Logs,` after `Chat`.
- `ALL` in that order; `table`: `"settings.logging"`, `"logs"`; `heading`: `"Logging"`, `"Logs"`; `is_view`: both.
- `context_position`: `SettingsLogging => 20`, `Concepts..Chat` +1 (`21..41`), `Logs => 42`, `Queue..Answer` +2 (`43..50`).
- Tests: `every_context_is_listed_once_in_all` `49` → `51`; `context_tables_and_headings` gains both rows and both contexts in the "views" assertion lists (`Logs` beside `Chat`, `SettingsLogging` beside the `SettingsProjectTypes` pin).

### Tests (TDD order; red first)

| Test | File | Asserts |
|---|---|---|
| `every_row_of_the_table_maps_to_its_source` | `source.rs` | one case per D3 row (all 16) plus a child path of each (`htui_orch::engine` → orchestrator) |
| `the_longest_prefix_wins` | `source.rs` | `htui_store::secret` → secrets; `htui_store::secret::x` → secrets; `htui::store_worker` → store; `htui::agent_worker` → agents |
| `a_prefix_matches_only_at_a_path_boundary` | `source.rs` | `htui_storex` → other; `htui::secretsx` → ui; `sqlxx` → other |
| `any_other_htui_target_is_ui_and_the_rest_is_other` | `source.rs` | `htui`, `htui::app::update` → ui; `hyper::client`, `log` → other |
| `sources_list_in_the_requirement_s_order` | `source.rs` | `ALL` names = `ui store orchestrator agents mcp worker secrets other`; `from_name(name(s)) == Some(s)` |
| `a_pushed_line_keeps_its_time_level_target_source_and_text` | `buffer.rs` | fixed `at`; seq 0 then 1 |
| `since_returns_the_lines_from_the_sequence_asked_and_the_next` | `buffer.rs` | `since(0)` all; `since(next)` none; `since(1)` the tail |
| `the_oldest_line_drops_at_ten_thousand_and_one` | `buffer.rs` | `CAPACITY + 1` pushes: `len == CAPACITY`, `since(0).oldest == 1`, first line gone |
| `counts_follow_a_dropped_line` | `buffer.rs` | one store WARN then `CAPACITY` ui INFO: store `{0, 0}`, all `{CAPACITY, 0}` |
| `only_warn_and_error_count_as_warnings` | `buffer.rs` | trace/debug/info/warn/error → `{5, 2}` |
| `the_memory_layer_puts_the_message_and_fields_on_one_line` | `buffer.rs` | `with_default(registry().with(MemoryLayer::new(b)))`: `warn!(target: "htui_store::cache", rows = 3, path = "x", "cache refreshed")` → text `cache refreshed rows=3 path="x"`, WARN, source store |
| `an_event_without_a_message_keeps_its_fields_and_a_line_break_folds` | `buffer.rs` | `info!(n = 1)` → `n=1`; `"a\nb"` → `a b` |
| `a_log_record_is_sourced_by_its_log_target` | `buffer.rs` | `info!(target: "log", "log.target" = "sqlx::query", "q")` → target `sqlx::query`, source store, text `q` |
| `the_filter_reloads_and_a_refused_directive_changes_nothing` | `mod.rs` | `Logging::build(Startup::default())`, `dispatcher::with_default`: debug hidden at info; `set_filter("debug")` captured; `set_filter("htui=[")` is `Err(Filter)`, `state().filter == "debug"`, debug still captured |
| `the_file_switches_on_moves_and_off` | `mod.rs` | tempdir: on → line in `a`; move → next line only in `b`; off → none in `b`; `state().file` each time |
| `the_old_file_is_closed_when_the_file_moves` | `mod.rs` | `#[cfg(target_os = "linux")]`: no `/proc/self/fd/*` links to `a` after the move |
| `a_file_that_cannot_be_opened_leaves_the_previous_one` | `mod.rs` | `set_file(Some(dir))` → `Err(File)` naming the path; lines still reach `a`; `file_error` set; a good switch clears it |
| `a_startup_file_and_filter_mark_their_overrides` | `mod.rs` | `Startup { file, filter: Some("debug") }`: `file_set_by == Some(SET_BY_FLAG)`, `filter_set_by == Some(SET_BY_VAR)`; both setters `Err(Overridden)` |
| `an_unusable_filter_variable_counts_as_unset` | `mod.rs` | `from_parts(None, Some("["))` and `Some("  ")` → `filter: None` |
| `sentry_rides_only_a_startup_log_file` | `mod.rs` | `build(default).sentry() == false`; with a startup file `true`; `detached().sentry() == false`; a saved file never sets it |
| `saved_settings_apply_unless_overridden` | `mod.rs` | default startup: both applied, `None`; with `Startup.filter`: filter kept, file applied |
| `a_bad_saved_filter_is_a_notice_and_the_file_still_applies` | `mod.rs` | `apply_saved(prefs{filter "htui=[", file a})` → `Some(n)`, `n` names `logging.filter`; file is `a`; filter `info` |
| `a_detached_logging_installs_nothing` | `mod.rs` | `dispatch().is_none()`; `set_filter("debug")` is `Ok` and recorded |
| `the_logging_table_round_trips` | `state_file.rs` | `full()` gains logging; `parse(render)` equal; text has `[logging]` before `[layout.backlog]` |
| `a_file_without_logging_reads_as_no_setting` | `state_file.rs` | `parse("version = 1\n").logging == None`; an empty `[logging]` too; `file = ""` → no file |
| `a_bad_filter_in_the_file_is_kept_not_refused` | `state_file.rs` | `filter = "htui=["` with a layout → `Ok`, layout read, filter kept for the notice |
| `a_logging_table_of_the_wrong_shape_names_where` | `state_file.rs` | `filter = 3` → `Err` starting `logging: `, one line |
| `a_logging_action_applies_and_saves` | `update.rs` | `update(Action::Logging(Filter("debug")))` → control filter `debug`; `ui_store.load().state.logging` holds it; layouts saved beside it unchanged |
| `a_refused_logging_action_says_why_and_saves_nothing` | `update.rs` | `File(Some(dir))` → `status` names the path; store text unchanged |
| `run_installs_logging_first_and_applies_saved_settings_before_the_tabs` | `lib.rs` | source order (as `lib.rs:465-499`): `logging::init(` < `keys::resolve(`; `app.logging = logging;` < `app::register_all(&mut app)`; `ui_store.load()` < `apply_saved(` < `app.apply_ui_state(`; no `init_tracing` left |
| `context_tables_and_headings`, `every_context_is_listed_once_in_all` | `catalogue.rs` | UPDATE as above |

Every existing `state_file` test stays green unchanged (the default still renders `version = 1\n`).

**Validate**:
- `cargo test -p htui --all-features --lib -- logging:: state_file:: app::update:: keys::catalogue:: tests::run_ --test-threads=1`
- `cargo test -p htui --all-features --test keys_file` (the page is unchanged)
- `cargo clippy -p htui --all-targets --all-features -- -D warnings` and `cargo clippy --workspace -- -D warnings`

---

## T2 — Logs tab (after T1; worktree, parallel with T3)

### Files written

| File | Symbols |
|---|---|
| `crates/htui/src/ui/tabs/logs/mod.rs` | CREATE: `LogsTab`, `LOGS_LAYOUT`, `Scroll`, `HINT`, consts, tests |
| `crates/htui/src/ui/tabs/logs/snapshots/htui__ui__tabs__logs__tests__*.snap` | CREATE (6) |
| `crates/htui/src/ui/tabs/mod.rs` | `pub mod logs;` `pub use logs::LogsTab;` |
| `crates/htui/src/keys/catalogue.rs` | `Act::{LogsLevel, LogsFollow, LogsBottom}` appended after `QdrantAdopt`; three rows after `ChatBottom` (`:1990`); test `ALL` (`:2448-2656`) + `position` (`213..215`); `every_act_has_exactly_one_row` `213` → `216` |
| `crates/htui/src/keys/views.rs` | `LOGS_BROWSE` after `CHAT_REPLAY` (`:1015`); `VIEWS` gains `source!("ui/tabs/logs/mod.rs")` after `chat/mod.rs` (its last entry); `VIEW_STACKS` gains `LOGS_BROWSE` after `CHAT_REPLAY`; asserts in `the_view_stacks_resolve_as_designed` |
| `crates/htui/src/keys/stack.rs` | `DECLARED`: `("in Logs", views::LOGS_BROWSE),` appended after `LEAVE` (`:316`) |
| `crates/htui/src/ui/about.rs` | `LOGS`, `about_for` arm, doc `:82-83` ("the reserved workspaces tab before it exists"), `TABS: [&str; 6]` |
| `crates/htui/src/ui/theme.rs` | doc only, `:20`, `:36` ("the logs tab", no longer reserved) |
| `crates/htui/src/app/mod.rs` | **merge point**: `register_all` line + import + doc step 1 |
| `crates/htui/tests/integration.rs` | `:43-48` count 6 and message; `:62` chips assertion |
| `docs/htui-keys.md` | regenerated toml block (`[logs]` after `[chat]`) |
| `README.md` | **merge point**: `:182`, `:184-190`, `:200`, new `### Logs` before `### Settings` (`:423`) |
| 40 existing `.snap` (F7) | reviewed in a full insta run |

### Keys

```rust
// Act, appended after QdrantAdopt:
/// `logs.level`: cycle the Logs tab's minimum level, trace → … → error → trace.
LogsLevel,
/// `logs.follow`: follow the newest line, or pause where the view is.
LogsFollow,
/// `logs.bottom`: the newest line, following again.
LogsBottom,

// CATALOGUE, after ChatBottom's row:
// [logs] (MOD-122 D4). `j`/`k`/`g` are `list.*`; `G` is the view's, as `chat.bottom` is.
// logs/mod.rs: on_key.
row(Act::LogsLevel, Logs, "level", &["v"], "minimum level"),
row(Act::LogsFollow, Logs, "follow", &["F"], "follow"),
row(Act::LogsBottom, Logs, "bottom", &["G"], "newest"),
```
```rust
/// The Logs tab (MOD-122 D4): its own verbs, the list moves **without** `list.bottom` (its `G`
/// is the view's `logs.bottom`, which also jumps there), the pane scroll, then the layout and
/// history layers of a list + detail tab.
pub const LOGS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::Logs, &[Act::LogsLevel, Act::LogsFollow, Act::LogsBottom]),
    Layer::only(Context::List, &[Act::ListDown, Act::ListUp, Act::ListTop]),
    PANE_SCROLL,
    LAYOUT,
    HISTORY,
    GLOBAL,
]);
```
`the_view_stacks_resolve_as_designed` gains: `resolve(LOGS_BROWSE, "G") == [LogsBottom]`, `"v" == [LogsLevel]`, `"F" == [LogsFollow]`, `"end" == []`, `"k" == [ListUp]`, `"K" == [PaneScrollUp]`.

`docs/htui-keys.md`: replace the toml block with `cargo run -q -p htui -- --default-keys --print-keys` (the page's own instruction, pinned by `keys_file.rs:728-744`). Expected new table:
```toml
[logs]
level  = ["v"]  # minimum level
follow = ["F"]  # follow
bottom = ["G"]  # newest
```

### `LogsTab`

```rust
pub const LOGS_LAYOUT: TabLayout = TabLayout { list: ListSize::Cells(30), ..TabLayout::DEFAULT };

/// Where the lines pane is (B4).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Scroll { #[default] Tail, At(u64) }      // At = seq of the top visible line

pub struct LogsTab {
    buffer: LogBuffer,
    lines: VecDeque<LogLine>,     // the tab's copy of the retained lines
    next: u64,                    // ask the buffer from here
    counts: SourceCounts,
    selected: Option<Source>,     // None = `all`
    min_level: Level,             // default TRACE: everything the global filter let through
    scroll: Scroll,
    height: Cell<usize>,          // lines-pane rows as last rendered
}

impl LogsTab {
    pub const ID: TabId = TabId("logs");
    pub fn new(buffer: LogBuffer) -> Self;     // pulls once
    fn pull(&mut self);                         // since(next): extend, drop seq < oldest, adopt counts
    fn rows(&self) -> Vec<Option<Source>>;      // [None] + Source::ALL, `Other` only when counts.of(Other).lines > 0
    fn visible(&self) -> Vec<&LogLine>;         // the selected source (or all), `line.level <= self.min_level` (tracing: ERROR < … < TRACE)
    fn top(&self, len: usize) -> usize;         // Tail: last_page; At(seq): partition_point(seq) clamped to last_page
    fn step_lines(&mut self, delta: isize);     // the state machine below
}
```
`last_page = visible.len().saturating_sub(height)`. State machine (mirrors `transcript.rs:552-583`):

| Input | `Tail` | `At(seq)` (top index `t`) |
|---|---|---|
| down `n` (`list.down` on the lines, `J`, `PgDn`, wheel down) | `Tail` | `t + n >= last_page` → `Tail`, else `At(seq of t+n)` |
| up `n` (`list.up` on the lines, `K`, `PgUp`, wheel up) | `last_page == 0` → `Tail` (nothing above: keep following, as RV4); else `At(seq of last_page - n)` | `At(seq of t - n)` (saturating) |
| `list.top` on the lines | `At(first seq)` | same |
| `logs.bottom` `G` | `Tail` | `Tail` |
| `logs.follow` `F` | `At(seq of last_page)` (paused where it is) | `Tail` |
| `logs.level` `v`, a source move, new lines, dropped lines | unchanged | unchanged (the anchor keeps the same line on top; clamped at render) |

`on_key` (no raw `KeyCode`: `tests/no_hardcoded_keys.rs`):
```rust
let chord = ctx.pressed(&key);
for act in ctx.keys().actions(views::LOGS_BROWSE, chord) {
    // Motion follows the focus (kit::scroll::Step::by_focus): lines pane → step_lines; activity → nothing.
    if Step::by_focus(act, ctx.focus(), |step| self.on_step(step)) { return Handled::Consumed; }
    match act {
        Act::ListDown | Act::ListUp | Act::ListTop => self.move_source(act),   // list focused; keeps follow
        Act::PaneScrollDown => self.step_lines(1),   Act::PaneScrollUp => self.step_lines(-1),
        Act::PanePageDown => self.step_lines(PAGE),  Act::PanePageUp => self.step_lines(-PAGE),
        Act::LogsLevel => self.min_level = next_level(self.min_level),          // TRACE→DEBUG→INFO→WARN→ERROR→TRACE
        Act::LogsFollow => self.toggle_follow(),
        Act::LogsBottom => self.scroll = Scroll::Tail,
        _ => continue,
    }
    return Handled::Consumed;
}
Handled::Pass
```
Trait methods: `id`, `title` "Logs", `wants_requests` → `vec![]`, `on_scope_change` → nothing (the log is the process's), `on_reply` → nothing, `key_stack` → `Some(views::LOGS_BROWSE)`, `panes` → `Panes::ListDetail(LOGS_LAYOUT)`, `on_refresh` → `self.pull()`, `key_hint` → `key_line(keys.hint(LOGS_BROWSE, HINT), theme)`, `location`/`restore` below.

```rust
const HINT: HintSpec = &[
    Hint::One(Act::LogsLevel, "level"), Hint::One(Act::LogsFollow, "follow"),
    Hint::One(Act::LogsBottom, "newest"), Hint::Pair(Act::ListDown, Act::ListUp, "move"),
];
```

### Render

- `let areas = ctx.split(area, LOGS_LAYOUT);` focus per pane through `ctx.pane_focus(PaneId::List | PaneId::Detail)` (Requirements' `focus_of`, `requirements/mod.rs:1482-1488`); hints on the focused pane (`Pane::hints`).
- **List** (`Pane::new("Sources")`): `kit::list::render` over `RowKind::Item` rows, cursor on `selected`, `window(cursor, len, height)`. Row text: `format!("{name:<12} {lines:>5} \u{b7} \u{26a0} {warnings}")`, `all` first; the warning part in `theme.warning` when `warnings > 0`, else `theme.dim`.
- **Lines** pane title (B4/D4 border text): `format!("{source} \u{b7} min {level} \u{b7} {state}")`, source `all` or the name, level lowercase, state `following` / `paused`. Scrollbar `pane.scrollbar(len, top)`.
- **Line**: `{at:%H:%M:%S} {LEVEL:<5} ` + (for `all` only) `{source:<12} ` + `text`, clipped with `cells::clip` to the inner width (B6). Styles: time `theme.dim`; level word and text `theme.error` (ERROR), `theme.warning` (WARN), `theme.base` (INFO), `theme.dim` (DEBUG, TRACE). The level word is always printed, so monochrome loses no information (D4). Time is UTC, as `connection.rs:1043` and `secrets.rs:957` print theirs.
- `self.height.set(inner.height)` every render.
- **Empty states** (`kit::empty::render`, a next step each, MOD-83):
  - buffer empty: `"No log lines yet."` / `"htui's own log appears here as it runs; Settings \u{203a} Logging sets what is kept"`;
  - the source has lines but none at the level: `"No lines at {level} or above."` / `"{v} lowers the minimum level"` (label of `LogsLevel` through `LOGS_BROWSE`; without one, `"lower the minimum level"`);
  - the source has none: `"No lines from {source} yet."` / the first next step.

### `location` / `restore`

`location` → `TabLoc { cursor: Some(format!("source:{}", selected.map_or("all", Source::name))), ..TabLoc::default() }`. `restore` parses `source:<name>`: `all` → `None`, `Source::from_name` → `Some`; sets `selected`, keeps the scroll state, returns `true`; anything else `false`.

### Registration and about

- `app/mod.rs`: `app.register_tab(Box::new(LogsTab::new(app.logging.buffer().clone())));` **after** `ChatTab` (D4: digit 6, no digit moves); import `LogsTab` in `use crate::ui::tabs::{…}`; doc step 1 adds "Chat and, last, Logs (MOD-122 D4)".
- `about.rs`: `const LOGS: About = About { purpose: "htui's own log as it runs: pick a source, set the minimum level, follow or scroll back.", actions: &[(Act::LogsLevel, "level"), (Act::LogsFollow, "follow"), (Act::LogsBottom, "newest")] };` (≤ 90 cells, ends with `.`); `"logs" => Some(&LOGS)`; `TABS` gains `"logs"`; `"workspaces"` stays reserved.

### Tests (TDD order; in `logs/mod.rs` unless named)

Harness: `Harness::empty().with_tab(Box::new(LogsTab::new(buffer.clone())))` over a `LogBuffer` seeded with fixed times (`Utc.with_ymd_and_hms(2026, 11, 5, 9, 30, s)`); no subscriber anywhere. A refresh is four `harness.app().update(Action::Tick)`. Focus moves with `harness.key("tab")` (`global.next_pane`).

| Test | Kind |
|---|---|
| `an_empty_buffer_says_where_lines_come_from` | snapshot `logs_empty` |
| `the_sources_list_counts_lines_and_warnings` | snapshot `logs_sources` (mixed store/agents/ui/other, `all` first, `other` shown) |
| `other_is_listed_only_with_lines` | assert |
| `each_level_takes_its_style_and_its_word` | `render_buffer` cell styles per level; snapshot `logs_levels` for the text |
| `under_monochrome_the_level_word_is_the_signal` | `Theme::monochrome()`: no fg colour, every level word present |
| `a_minimum_level_hides_lower_lines_and_shows_on_the_border` | snapshot `logs_min_warn` |
| `the_border_says_following_or_paused` | snapshot `logs_paused` after `F` (the others show `following`) |
| `a_narrow_terminal_keeps_both_panes_readable` | snapshot `logs_narrow` (60×20) |
| `v_cycles_the_minimum_level_through_every_level` | 5 presses back to trace |
| `a_step_up_in_the_lines_pauses_follow` | > one page of lines, lines pane focused, `k` → `paused`; same for the wheel up |
| `capital_g_resumes_follow_at_the_newest_line` | after `k`, `G` → `following`, last line on screen |
| `capital_f_toggles_follow` | `F` twice |
| `new_lines_arrive_on_refresh_and_stay_in_view_only_while_following` | push, tick ×4: following shows the new last line; paused keeps the same top line |
| `a_paused_view_keeps_its_line_when_old_lines_drop` | B4 with a small fill past `CAPACITY` (push `CAPACITY + k`) |
| `a_source_change_keeps_the_follow_state` | list focused, `j`: still following / still paused |
| `location_and_restore_return_to_the_source` | `location()` round trip, `"source:store"`, unknown → `false` |
| `catalogue`, `views` | `every_act_has_exactly_one_row` (216), `names_are_unique_per_context`, `no_spec_is_shared_in_a_context_unless_state_guarded`, `catalogue_blocks_follow_context_all`, `every_act_is_offered_by_a_stack_and_handled_by_a_view`, `the_view_stacks_resolve_as_designed` (UPDATE) |
| `about.rs` | `every_main_tab_has_a_one_sentence_purpose_and_actions` (6 tabs) |
| `tests/integration.rs` | `the_demo_shell_starts_inside_the_first_workspace_on_the_backlog_tab`: 6 tabs, chips through `⁵Chat  ⁶Logs` if the 100-column strip shows it, header facts at 160 |
| `tests/keys_file.rs` | `the_key_reference_page_is_the_default_print` (after regeneration) |

**Validate**:
- `cargo test -p htui --all-features --lib -- ui::tabs::logs keys:: ui::about --test-threads=1`
- `cargo test -p htui --all-features --test keys_file --test integration --test no_hardcoded_keys -- --test-threads=1`
- `cargo insta test -p htui --all-features` (full run; review every `.snap.new`: the 40 tab-strip snapshots of F7 change only on line 1)

---

## T3 — Settings › Logging (after T1; worktree, parallel with T2)

### Files written

| File | Symbols |
|---|---|
| `crates/htui/src/ui/tabs/settings/logging.rs` | CREATE: `LoggingSection`, `Row`, `Mode`, `Editor`, `Notice`, consts, tests |
| `crates/htui/src/ui/tabs/settings/snapshots/htui__ui__tabs__settings__logging__tests__*.snap` | CREATE (3) |
| `crates/htui/src/ui/tabs/settings/mod.rs` | `/// The logging section (MOD-122). pub mod logging;`, `pub use logging::LoggingSection;`, module doc list, `NO_SECTIONS` doc ("registers ten" → twelve) |
| `crates/htui/src/keys/views.rs` | `LOGGING_BROWSE` after `PROJECT_TYPES_BROWSE` (`:494`); `VIEWS` gains `source!("ui/tabs/settings/logging.rs")` after `project_types.rs`; `VIEW_STACKS` after `PROJECT_TYPES_BROWSE`; `DELEGATES` gains `("LOGGING_BROWSE", &[SETTINGS_TAB_RS]),` after `PROJECT_TYPES_BROWSE`'s (`:1288`) |
| `crates/htui/src/keys/stack.rs` | `DECLARED`: `("in Settings > Logging", views::LOGGING_BROWSE),` after `:270` (F3) |
| `crates/htui/src/app/mod.rs` | **merge point**: one section line + import |
| `README.md` | **merge point**: `:108`, `:188`, a `**Logging**` row at the end of the Settings table (after **Project types**) |
| `docs/htui-cli.md` | `:55` one sentence (F8) |

### Stack

```rust
/// Settings › Logging, browsing (MOD-122 D5): no own action, so `[settings.logging]` holds
/// overrides only; `e` edits the row under the cursor (no `Enter` view default, B7).
pub const LOGGING_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsLogging, &[]),
    SETTINGS,
    Layer::only(Context::Common, &[Act::Edit, Act::Dismiss]),
    LIST,
    HISTORY,
    GLOBAL,
]);
```
Editing uses `views::CAPTURE`, as `queue.rs:599-604` does.

### `LoggingSection` (mirrors `queue.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row { Filter, File }            // ROWS = [Filter, File]

struct Editor { row: Row, input: TextField }    // Debug prints the row only (queue's H-5 habit)
enum Mode { Browse, Editing(Editor) }
enum Notice { Info(String), Error(String) }

pub struct LoggingSection { control: LoggingControl, cursor: usize, mode: Mode, notice: Option<Notice>, scroll: Cell<Option<ScrollSpan>> }

impl LoggingSection {
    pub const ID: SectionId = SectionId("logging");
    pub fn new(control: LoggingControl) -> Self;
}
```
- `title` "Logging"; `wants_requests` → `vec![]`; `on_scope_change` → nothing (per process, not per workspace); `on_reply` → nothing; `captures_input` → `Editing`; `on_paste` into the field; `key_stack` → `LOGGING_BROWSE` / `CAPTURE`; `key_hint` → `key_line(keys.hint(stack, spec), theme)` with `HINT_BROWSE = [Pair(ListDown, ListUp, ""), One(Edit, "edit")]` and `HINT_EDITING = [One(FieldSubmit, "save"), One(FieldCancel, "cancel"), Text("empty resets")]`; `scroll` as `queue.rs`.
- **Current values and overrides** come from `self.control.state()` on every render and every key: the control is the same `Arc` `App` applies through, so the row shows the applied value on the next frame. No copy is kept.
- **Browse keys** (`ctx.keys().actions(views::LOGGING_BROWSE, ctx.pressed(&key))`): `Edit` → refuse when the row's `*_set_by` is `Some(by)` with `overridden(by, row)` (B5) else open `TextField::with_text(current)` (the file row empty when none); `ListDown`/`ListUp` move the cursor; `Dismiss` clears a notice.
- **Submit** (`FieldOutcome::Submit`), text trimmed:
  - Filter: empty → `DEFAULT_FILTER`. `logging::parse_filter(text)`: `Err(e)` → `Notice::Error(e.to_string())`, editor stays open, **nothing emitted**; `Ok` → `ctx.emit(Action::Logging(LoggingAction::Filter(text)))`, back to browse, notice cleared.
  - File: empty → `ctx.emit(Action::Logging(LoggingAction::File(None)))`; else `File(Some(PathBuf::from(text)))`, made absolute first (implementation, `fc4edd66`: `std::path::absolute` against the directory htui runs in, so the path saved to `ui-state.toml` names the same file whichever directory the next session starts from; as typed when there is no current directory). Opening is the control's (via `App`): a refusal shows on the status line and as `state().file_error` under the row.
  - `FieldOutcome::Cancel` → browse; `Pass` → `modal_rest(views::CAPTURE, ctx.pressed(&key))`.
- **Render**: one `kit::form` (`Form::new().label_width(10).focus(cursor).editing(editing)`), then the notice row, as `queue.rs:640-713`:
  - `filter`: `ReadOnly(state.filter)` plus `" \u{b7} set by {by}"` when overridden; help (focused) `FILTER_HELP` or `overridden(by, Filter)`.
  - `log file`: `ReadOnly(path.display())` or `"none"`, plus the same note; help `FILE_HELP` or the override sentence; `.error(file_error)` whenever set (unfocused too).
  - Editing row: `Field::new(label, Value::Text(&input)).help(format!("now {current} \u{b7} empty resets"))`.

```rust
const FILTER_HELP: &str = "which lines htui keeps, as an EnvFilter directive such as info or \
    htui=debug,sqlx=warn; applied at once and kept in ui-state.toml";
const FILE_HELP: &str = "a file htui appends its log to, as --log does; empty writes none; \
    applied at once and kept in ui-state.toml";
/// B5. `by` is SET_BY_FLAG or SET_BY_VAR.
fn overridden(by: &str, row: Row) -> String;   // "set by `--log` for this session: start htui without it to change the log file here"
```

### Registration

`app/mod.rs`: in the `SettingsTab::with_sections(vec![…])` list, after `ProjectTypesSection`:
```rust
        // Last (MOD-122 D5): appending moves no existing section's line.
        Box::new(LoggingSection::new(app.logging.control().clone())),
```
and `LoggingSection` in the `use crate::ui::tabs::settings::{…}` import.

### Tests (TDD order; `SectionBench`, `#[tokio::test]`)

The control comes from `Logging::detached()` (no subscriber), or from `Logging::build(Startup { file: Some(tmp), filter: Some("debug") })` for the override cases (built, never installed). Tempdir paths are masked with insta `filters`.

| Test | Kind |
|---|---|
| `both_fields_render_with_their_values_and_the_focused_help` | snapshot `logging_section` (`info`, `none`) |
| `an_overridden_field_says_what_set_it` | snapshot `logging_section_overridden` |
| `a_file_that_could_not_open_is_said_under_its_row` | `control.set_file(Some(dir))` first; snapshot `logging_section_file_error` |
| `editing_the_filter_to_a_valid_directive_emits_it` | `e`, type `debug`, `enter` → `drained() == [Action::Logging(Filter("debug"))]`, browse |
| `an_invalid_directive_is_refused_and_emits_nothing` | `htui=[` → error notice naming it, editor open, `drained()` empty |
| `an_empty_filter_emits_the_default` | → `Filter("info")` |
| `an_absolute_file_path_is_emitted_trimmed_and_clearing_it_emits_no_file` | `File(Some(p))` (trimmed), then `File(None)` |
| `a_relative_file_path_is_emitted_absolute` | a relative path is made absolute before it is emitted (`fc4edd66`) |
| `an_overridden_field_refuses_the_edit_and_says_why` | `e` on each overridden row → notice, no editor, nothing emitted |
| `the_section_shows_the_applied_value_on_the_next_frame` | `control.set_filter("warn")` directly → render shows `warn` |
| `the_product_registers_logging_last` | `Harness::demo()` + `register_all` + `4` + eleven `l`: frame contains `" Project types  Logging "` and the browse hint |
| `views.rs`, `stack.rs` | `every_act_is_offered_by_a_stack_and_handled_by_a_view` (VIEWS, VIEW_STACKS, DELEGATES) and the validator over `DECLARED` stay green |

**Validate**:
- `cargo test -p htui --all-features --lib -- ui::tabs::settings::logging keys:: --test-threads=1`
- `cargo test -p htui --all-features --test keys_file --test no_hardcoded_keys --test project_types --test secrets_settings --test personas -- --test-threads=1` (the `the_product_registers_*` strip tests)
- `cargo insta test -p htui --all-features` (expected: only T3's new snapshots)

---

## File sets and merge points

| Task | Files |
|---|---|
| T1 | `logging/{mod,buffer,source}.rs` (new), `lib.rs`, `app/{state,action,update,mod}.rs`, `ui/state_file.rs`, `keys/catalogue.rs` (contexts) |
| T2 | `ui/tabs/logs/**` (new), `ui/tabs/mod.rs`, `keys/catalogue.rs` (acts, rows), `keys/views.rs`, `keys/stack.rs`, `ui/about.rs`, `ui/theme.rs` (doc), `app/mod.rs`, `tests/integration.rs`, `docs/htui-keys.md`, `README.md`, 40 `.snap` |
| T3 | `ui/tabs/settings/logging.rs` + snapshots (new), `ui/tabs/settings/mod.rs`, `keys/views.rs`, `keys/stack.rs`, `app/mod.rs`, `README.md`, `docs/htui-cli.md` |

**T2 ∩ T3 = {`app/mod.rs`, `README.md`, `keys/views.rs`, `keys/stack.rs`}** — two more than the plan's V9 (F1).

| File | T2 hunk | T3 hunk | Expected |
|---|---|---|---|
| `app/mod.rs` | `use crate::ui::tabs::{…, LogsTab, …};` (`:27`); `register_tab(LogsTab…)` after `ChatTab` (`:100`); doc step 1 | `use crate::ui::tabs::settings::{…, LoggingSection, …};` (`:22-26`, rustfmt reflows it); section line after `ProjectTypesSection` (`:97`) | **Hand-merge**: the two `use` hunks are adjacent and will conflict; keep both. The register lines are 2 unchanged lines apart: check |
| `README.md` | `:182` six tabs/`6`; row after `:190`; `:200` `1 … 6`; `### Logs` before `### Settings` (`:423`) | `:108` `--log` row; `:188` Settings row; `**Logging**` row at the Settings table's end | Separate hunks (`:188` vs the insert after `:190` have `:189-190` between); hand-check |
| `keys/views.rs` | stack after `CHAT_REPLAY` (`:1015`); `VIEWS` after `chat/mod.rs` (last entry); `VIEW_STACKS` after `CHAT_REPLAY` | stack after `PROJECT_TYPES_BROWSE` (`:494`); `VIEWS` after `project_types.rs`; `VIEW_STACKS` after `PROJECT_TYPES_BROWSE`; `DELEGATES` (`:1288`) | Disjoint; git merges. Re-run `keys::views::` on the merged tree |
| `keys/stack.rs` | append after `LEAVE` (`:316`) | insert after Project types (`:270`) | Disjoint |

Not shared (checked): `keys/catalogue.rs` after T1 (T3 adds no row), `docs/htui-keys.md` (B7), every existing `.snap` (F7), `ui/tabs/mod.rs` (T2) vs `ui/tabs/settings/mod.rs` (T3). Collect order: merge T2, then T3; regenerate nothing; run the plan's gate on the merged tree.

## Commit sequence

Each commit compiles, passes its tests and featureless clippy. Commit with explicit paths.

**T1**
1. `feat(MOD-122): logs and settings.logging key contexts` — `catalogue.rs` (B1) + its two tests. Page unchanged (`--test keys_file`).
2. `feat(MOD-122): log sources and the in-memory buffer` — `logging/{mod.rs (submodules + re-exports only), source.rs, buffer.rs}`, `lib.rs` `pub mod logging;` + their tests.
3. `feat(MOD-122): the logging control: reloadable filter, switchable file` — rest of `logging/mod.rs` + tests (`build` exists; `run` still calls `init_tracing`).
4. `feat(MOD-122): [logging] in ui-state.toml` — `state_file.rs` + tests.
5. `feat(MOD-122): App holds Logging; Action::Logging; run installs it` — `app/{state,action,update,mod}.rs`, `lib.rs` wiring, `init_tracing` and `use std::path::Path` removed, source-order test.

**T2**
1. `feat(MOD-122): Logs tab with its keys` — `logs/mod.rs` + snapshots, `ui/tabs/mod.rs`, `catalogue.rs`, `views.rs`, `stack.rs`, `docs/htui-keys.md` (one commit: `every_act_is_offered_by_a_stack_and_handled_by_a_view` needs the stack's file to handle its acts).
2. `feat(MOD-122): register Logs last (digit 6)` — `app/mod.rs`, `about.rs`, `theme.rs` doc, `tests/integration.rs`, the 40 snapshots (full insta run).
3. `docs(MOD-122): README Logs tab`.

**T3**
1. `feat(MOD-122): Settings › Logging section` — `settings/logging.rs` + snapshots, `settings/mod.rs`, `views.rs`, `stack.rs` (the section's tests use `SectionBench`, so it need not be registered yet).
2. `feat(MOD-122): register Logging last in Settings` — `app/mod.rs` + `the_product_registers_logging_last`.
3. `docs(MOD-122): --log and Settings › Logging` — `README.md`, `docs/htui-cli.md`.

## Risks (added to the plan's)

| Risk | Mitigation |
|---|---|
| A test builds many `Dispatch`es (each `Dispatch::new` rebuilds the callsite interest cache) | Only the T1 control tests and two T3 override tests call `build`; `App::new` is `detached` (F6) |
| `with_default` tests in parallel with other dispatchers see `Interest::sometimes` | Correct by design (per-dispatch `enabled`); the reload rebuilds the interest cache across live dispatchers |
| A paused view's anchor line drops out of the ring | `partition_point` lands on the next retained line, `top` clamps (`a_paused_view_keeps_its_line_when_old_lines_drop`) |
| Six chips crowd the 100-column header | F7: T2 re-reads the header assertion and reviews `integration__demo_shell` |

# Blueprint: MOD-67 milestone 2, "the key file" (`keys.toml`)

**Status**: proposed (2026-10-07). Findings F-1 to F-11 (§0), blueprint decisions B-1 to B-14
(§15) and plan amendments PA-1 to PA-3 (§14) are proposed here. Nothing in D1-D11 is reopened:
§14 lists the three places where the confirmed text, read literally, cannot work or works badly,
with evidence, and says what this blueprint builds meanwhile.

**Plan**: `.claude/plans/mod-67-m2-keys-file.plan.md` (confirmed 2026-10-07). Its D1-D11, Tasks
1-8, file set and Verified-claims table are authoritative except where §0, §11 and §12 refine
them. **Spec**: `docs/ANA-26.md` §6.3-§6.4, §7.1, §7.4, §7.5, §8 (M2 row).

**Verified at**: HEAD `21fe8f9c`, branch `hr/MOD-67`. Every `file:line` below was read at that
HEAD through Gortex (`read`, `search`). **Line numbers are pre-edit.** The toml behaviour in §0
and §3 comes from a compile probe against `toml = "=1.1.5"` (`/tmp/m2probe`, 2026-10-07): spans,
BOM, CRLF, sorted iteration and the exact parser messages quoted below.

**Scope**:
- **Order**: T1 ∥ T2 (disjoint files), then T3 → T4 → T5 → T6 → T7 → T8, serial. After each
  task, its gate runs on the real tree with `--test-threads=1` (memory:
  htui-suite-green-is-scheduling-dependent).
- **New public surface** (all in `crates/htui` unless named): `keys::{load, validate, print}`
  modules; `keys::{KeysError, KeyFileError, FILE_NAME, load_str, load_path, resolve, validate,
  print}` re-exports; `keys::Keys::line`; `keys::ChordError::NotDelivered`; `keys::stack::DECLARED`
  (re-exported as `keys::DECLARED`); `cli::Args::{keys, default_keys, print_keys}`;
  `htui_store::identity::config_root_path`. Crate-private: `Row.line`, `Keys::set`,
  `keys::quote`, `keys::contexts`, `chord::unix_drops`, `lib::print_keys`.
- **No file under `crates/htui/src/ui/` or `crates/htui/src/app/` changes. No snapshot changes.**
  `testkit.rs` does not change (ANA §7.4: `testkit` keeps the compiled defaults; `Ctx::new` uses
  `Keys::compiled()`, `app/state.rs:116`).
- **One new dependency edge**: `crates/htui` gains `toml = { workspace = true }` (1.1.5, already in
  the lock via `htui-store`; features `default` only, `preserve_order` off, verified with
  `cargo tree -e features -i toml@1.1.5`, so `DeTable` iteration is **sorted by key**, not file
  order). `serde` is already a direct dependency (`crates/htui/Cargo.toml`, MOD-66 N5).

**House style (carried from M1)**:
- Lints (`Cargo.toml` `[workspace.lints]`): `unsafe_code = "forbid"`,
  `missing_debug_implementations` and `unused_qualifications` warn, `clippy::all` warns,
  **no `clippy::pedantic`**. rustdoc denies `broken_intra_doc_links`, `private_intra_doc_links`,
  `redundant_explicit_links`. `lib.rs:10` has `#![warn(missing_docs)]` (crate-wide), so every new
  `pub` module needs a `//!` header and every `pub` item, `pub` field, enum variant **and
  struct-variant field** needs a doc comment. Every `pub` type derives `Debug`. No `pub` doc may
  link a private item (`Row`, `quote`, `unix_drops`).
- Gates run clippy with `-D warnings` both `--all-targets --all-features` and featureless
  (`cargo clippy --workspace -- -D warnings`, memory: htui-featureless-clippy-gate). The featureless
  lib build is where a field "never read" outside tests shows up (F-4).
- Inline format args (`format!("{x}")`); clippy's `uninlined_format_args` is on under `all`.
- `unused_qualifications`: once a file imports `std::path::{Path, PathBuf}`, do not also write
  `std::path::Path` in it.
- `std::env::set_var` is `unsafe` in edition 2024 and `unsafe_code` is forbidden: no test may set
  an environment variable in-process. The default-path cases spawn the binary with `env(…)` on the
  child (Task 7).
- `rustfmt.toml`: edition 2024, `max_width = 100`. Toolchain 1.98.1.
- Implementers commit incrementally, stage their own paths only (never `-A`, never `stash`, never
  `--amend`). Every commit compiles and passes clippy both ways.
- The Gortex PreToolUse hook blocks `Read`/`cat`/`grep` on indexed source; read with Gortex
  `read`. If `Edit` is blocked, use an anchored scripted replace (each anchor asserted to match
  once), then `cargo fmt`.

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree / probe at `21fe8f9c` | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (Task 5 tests fail as worded) | D7: `Row.line` is "the file line that set it, `None` for a default". D10 / Task 5: `load_str(print(keys)) == keys` for defaults and for a changed table, and "the default print loads with zero `(changed)` marks". | `Keys` and `Row` derive `PartialEq` (`keys/mod.rs:19`, `:27`). The default print lists **every** action, so loading it back sets every row from the file: every row gets `Some(line)`, so (a) `!= Keys::compiled()` and (b) every line prints `(changed)`. For a changed table the printed file's line numbers differ from the source file's, so `==` fails on `line` alone. | **B-1**: `line` is `Some` only when the file's list **differs** from the catalogue default (order-sensitive). **B-2**: `Row`'s equality ignores `line` (hand-written `PartialEq`/`Eq`). Both tests then hold exactly as the plan words them. See §14 PA-3 for the record. |
| **F-2** | Major (red M1 test on Unix) | D6: `cfg(unix)` refuses any ctrl chord of a character outside `a`-`z`, space, `4`-`7`. | M1's `ctrl_minus_and_ctrl_plus_are_writable` (`keys/chord.rs:521`) asserts `strict("ctrl--")`, `strict("ctrl++")`, `strict("ctrl-+")` are `Ok`. Under D6 they are `NotDelivered` on Unix. | Task 2 rewrites that test: the grammar still reads them (reaching `NotDelivered` proves the parse got past the grammar); `Ok` only under `cfg(not(unix))`. §6.1 test 5. |
| **F-3** | Minor (D11's own example is refused) | D11: "an overlay that handles `Esc` itself still closes on it after `[overlay] close = ["x"]`". | `overlay.close` is `capture_row` (`catalogue.rs:324`), pinned by `only_the_form_and_overlay_close_are_in_capture` (`catalogue.rs:706`). D8 step 6 refuses a printable chord on it, so `close = ["x"]` never loads. | README (Task 8) uses `close = ["f2"]`. §14 PA-1. |
| **F-4** | Major (featureless clippy red after T3) | Task 3 adds `Row.line`; only T4 (validator) and T5 (print) read it. | `#[derive(Debug)]` does not count as a read for `dead_code`. In the featureless lib build of T3's commit nothing outside `#[cfg(test)]` reads `line`, so `cargo clippy --workspace -- -D warnings` fails with "field `line` is never read". | **B-3**: T3 adds `pub fn Keys::line(&self, Context, Act) -> Option<usize>` (a real accessor; T5's print uses it). |
| **F-5** | Major (a panic would reach Sentry) | D10: `--print-keys` prints to stdout. | `print!` panics on `EPIPE` (Rust ignores `SIGPIPE`), so `htui --print-keys \| head` would panic, exit 101 and be captured by Sentry's panic integration (`main.rs` `body` initialises Sentry before `run`). | **B-4**: `lib.rs` writes through `print_keys(&mut dyn Write, &str)`, which treats `ErrorKind::BrokenPipe` as success. Unit-tested. |
| **F-6** | Minor (messages) | D8 step 1: "TOML syntax (one error, with the parser's span → line)". | `toml::de::Error::message()` is terse: `duplicate key`, `unclosed table, expected \`]\``, `invalid basic string, expected \`"\``, `string values must be quoted, expected literal string` (probe). `span()` is `Option<Range<usize>>`, `Some` in every probed case. | The line is `not valid TOML: {message}`; a missing span reports line 1. Tests that pin a parser message pin toml 1.1.5's text; a toml bump may move them (the lock pins 1.1.5). |
| **F-7** | Major (usability; D8 literal) | D8 step 5: a shared pair is allowed only if **both rows** are defaults and the pair is on `STATE_GUARDED`. | `STATE_GUARDED = [(Back, Dismiss)]` (`catalogue.rs:435`), both `esc`. A user who only **adds** a chord to one of them (`[common] back = ["esc", "backspace"]`) makes `back`'s row non-default, so the untouched `esc` share becomes an error they did not create and cannot fix without changing `esc`'s behaviour. | Built as D8 says (B-9); §14 PA-2 proposes a per-chord rule with a one-predicate, one-test delta. |
| **F-8** | Minor (which error wins) | D8 step 4: refuse any `ctrl-c`. | `parse_strict("ctrl-C")` and `("ctrl-shift-c")` fail with `CtrlCapital { suggestion: "ctrl-c" }` (`chord.rs` `refuse_char`), i.e. "write `ctrl-c`", which the next check refuses. | **B-5**: a `CtrlCapital` whose suggestion parses to `CTRL_C` reports the ctrl-c refusal instead. One error per chord. |
| **F-9** | Major (test isolation) | Task 7: "Linux-only case for the default path (`XDG_CONFIG_HOME` on the child process)". | `dirs::config_dir()` reads `XDG_CONFIG_HOME` on Linux only; on macOS it is `$HOME/Library/Application Support`; with a cleared environment and no `XDG_CONFIG_HOME`/`HOME` it falls back to the passwd home, i.e. **the developer's real `keys.toml`**. | Every binary case passes `--keys` or `--default-keys`, or sets `XDG_CONFIG_HOME` **and** `HOME` to a temp dir on the child. Default-path cases are `#[cfg(target_os = "linux")]`. |
| **F-10** | Minor (format edge) | D1: "a nested table extends the context path with `.`". | An inline table value is also `DeValue::Table` (probe: `global = { quit = "x" }` → table). So `[global]` + `quit = { a = "x" }` is the sub-context `global.quit`. | Accepted and pinned: it reports `[global.quit]: no such table; …`. Inline tables as contexts (`global = { quit = "x" }`) load like `[global]`. |
| **F-11** | Minor (signature) | Task 3: `load_str(src, file_name)`, `resolve(…, root: Option<PathBuf>)`. | A `KeyFileError`'s text never contains the file name; only `KeysError`'s report adds it (ANA §7.5 prints `keys.toml:9:` from the path). | **B-6**: `load_str(src: &str)`; `resolve(…, root: Option<&Path>)`. The report takes the name from `KeysError`'s path. |

### 0a. Settled answers to the brief's edge cases

| Case | Behaviour | Pinned by |
|---|---|---|
| Empty file | `Ok(Keys)` equal to the defaults; every `line` is `None`. | T3 test 1 |
| File with only `version = 1` | Same. | T3 test 2 |
| UTF-8 BOM | toml 1.1.5 accepts it; spans are byte offsets into the text **including** the 3 BOM bytes, so counting `\n` before the offset is still right. | T3 test 3 |
| CRLF | `\r\n` contains `\n`; the line count is unchanged. A lone `\r` is a TOML syntax error (`carriage return must be followed by newline, expected newline`). | T3 test 3 |
| A table key that is a value (`global = "x"`) | `global: write [global] as a table, one action per line`. Top-level `[[global]]` (array of tables) gets the same message. | T3 test 17 |
| `[global.extra]` | `[global.extra]: no such table; the tables are …` on the header's line. | T3 test 8 |
| Array with a non-string (`quit = ["x", 5]`) | `[global] quit: write each chord as a string, such as "q"` on the element's line; the string chords are still checked and merged. | T3 test 10 |
| `ctrl-C` (parses as `CtrlCapital`, suggestion is ctrl-c) | The ctrl-c refusal wins (B-5): `[global] find = "ctrl-C": ctrl-c always quits and cannot be bound`. | T3 test 12 |
| Unknown name **and** a bad value | Only the unknown-name error; the value is not inspected. | T3 test 9 |
| Two errors on one line | Stable sort by line keeps walk order: elements left to right; validation errors after load errors. | T3 test 15 |
| `[global]` written twice | toml's own `duplicate key` at the second header. | T3 test 16 (variant) |
| `version` inside a table | An unknown action `version` of that table. | covered by test 9's rule |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (each compiles, clippy clean both ways) | Gate (real tree, `--test-threads=1`) |
|---|---|---|---|
| T1 | `crates/htui-store/src/identity.rs` | 1 | `cargo test -p htui-store --lib identity` |
| T2 | `crates/htui/src/keys/chord.rs` | 1 | `cargo test -p htui --lib keys` |
| T3 | `keys/load.rs` (new), `keys/mod.rs`, `crates/htui/Cargo.toml`, `Cargo.lock` | 1-2 | `cargo test -p htui --lib keys`; `cargo doc -p htui --no-deps` |
| T4 | `keys/validate.rs` (new), `keys/stack.rs`, `keys/load.rs`, `keys/mod.rs` | 1 | `cargo test -p htui --lib keys` |
| T5 | `keys/print.rs` (new), `keys/mod.rs` | 1 | `cargo test -p htui --lib keys`; `cargo doc -p htui --no-deps` |
| T6 | `cli.rs`, `lib.rs`, `main.rs` | 1-2 | `cargo test -p htui --lib cli`; `cargo test -p htui --lib tests::print_keys`; `cargo test -p htui --bin htui` |
| T7 | `tests/keys_file.rs`, `tests/fixtures/keys/{valid,errors,ctrl_c,overlay_close,collision,syntax}.toml` | 1 | `cargo test -p htui --features testkit --test keys_file -- --test-threads=1` |
| T8 | `README.md` | 1 | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | - | - | the plan's full Validation block; zero `.snap.new` |

"Clippy both ways" = `cargo clippy -p htui --all-targets --all-features -- -D warnings` and
`cargo clippy -p htui -- -D warnings` (plus `-p htui-store` for T1).

---

## 2. Public and crate-visible surface (exact signatures)

### 2.1 `crates/htui-store/src/identity.rs` (T1)

```rust
/// `<dirs::config_dir()>/htui`, **without** creating it (MOD-67 M2 D3): `htui` looks for
/// `keys.toml` here before anything is allowed to write. [`config_root`] is this plus
/// `create_dir_all`.
///
/// # Errors
///
/// [`StoreError::Backend`] (`this platform has no config directory`) when `dirs` has none.
pub fn config_root_path() -> Result<PathBuf>;

// unchanged signature; body becomes `let root = config_root_path()?;` + the existing create.
pub fn config_root() -> Result<PathBuf>;
```

### 2.2 `crates/htui/src/keys/chord.rs` (T2)

```rust
pub enum ChordError {
    // … M1's eight variants unchanged …
    /// A chord a Unix terminal in the legacy encoding never delivers as itself (MOD-67 M2 D6):
    /// ctrl with a character other than `a`-`z`, space and `4`-`7`, or ctrl/shift on `Enter`,
    /// `Tab`, `Backspace` or `Esc`. Produced only on Unix; Windows delivers these chords.
    NotDelivered {
        /// The spec as written (trimmed).
        written: String,
    },
}

/// Whether a Unix terminal in the legacy encoding never delivers `chord` as itself (D6).
/// Private; called from `parse_strict` under `cfg!(unix)`, unit-tested on every platform.
fn unix_drops(chord: KeyChord) -> bool;
```

### 2.3 `crates/htui/src/keys/mod.rs` (T3, T4, T5)

```rust
pub mod catalogue;
pub mod chord;
pub mod hint;
pub mod load;      // T3
pub mod print;     // T5
pub mod stack;
pub mod validate;  // T4

pub use catalogue::{Act, ActionSpec, CATALOGUE, Context, STATE_GUARDED};
pub use chord::{CTRL_C, ChordError, KeyChord};
pub use hint::{HelpLine, Hint, HintSpec};
pub use load::{FILE_NAME, KeyFileError, KeysError, load_path, load_str, resolve}; // T3
pub use print::print;                                                         // T5
pub use stack::{DECLARED, Layer, Stack};                                      // T4 adds DECLARED
pub use validate::validate;                                                   // T4

/// One binding row (private: no `pub` doc may link it). Equality ignores `line` (B-2).
#[derive(Debug, Clone)]
struct Row {
    context: Context,
    act: Act,
    help: &'static str,
    chords: Vec<KeyChord>,
    /// The `keys.toml` line of the entry that set `chords`, or `None` while they equal the
    /// catalogue default (B-1). Validation reports on it; `--print-keys` marks it `(changed)`.
    line: Option<usize>,
}
impl PartialEq for Row { /* context, act, help, chords; not line */ }
impl Eq for Row {}

impl Keys {
    /// The `keys.toml` line that set `act`'s chords in exactly `context`, or `None` while they
    /// are the catalogue default (MOD-67 M2 D7). `--print-keys` marks such a row `(changed)`.
    #[must_use]
    pub fn line(&self, context: Context, act: Act) -> Option<usize>;

    /// The loader's merge (D7): replaces the `(context, act)` row's chords; keeps `line` only
    /// when they differ from [`Keys::compiled`]'s for that row (B-1). A missing row is a no-op
    /// (the loader resolves names through `CATALOGUE`, so it never happens).
    fn set(&mut self, context: Context, act: Act, chords: Vec<KeyChord>, line: usize);
}

/// Every context in catalogue order, once each: the key file's tables (M2).
fn contexts() -> Vec<Context>;

/// `text` as a TOML basic string: wrapped in `"`, with `\` → `\\`, `"` → `\"`, and every
/// `char::is_control` character → `\uXXXX` (upper-case hex, 4 digits). The error report and
/// `--print-keys` both spell chords with it.
fn quote(text: &str) -> String;
```

`Keys::defaults` and the test-only `Keys::with_chords` set `line: None` (the only two `Row`
literals, `keys/mod.rs:54`, `:96`). Update `Keys`' doc: "M2 builds one from `keys.toml`
([`load_str`])" (T3 may link `load_str`; it exists in the same commit).

### 2.4 `crates/htui/src/keys/load.rs` (T3; T4 adds the `validate` call)

```rust
//! `keys.toml` to [`Keys`] (MOD-67 M2; ANA-26 §7.1, §7.4 steps 1-6, §7.5): find the file, parse
//! it with spans, check names and chords strictly, merge it over the catalogue, and report every
//! error with its line.

use std::path::{Path, PathBuf};

use toml::de::{DeTable, DeValue};
// `use toml::Spanned;` only if a helper signature names it (else an unused import under -D warnings).

/// The key file's name under the config root.
pub const FILE_NAME: &str = "keys.toml";

/// One error in a key file: a line and what is wrong there (ANA-26 §7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFileError {
    /// The 1-based line. `0` only for a fault in the compiled defaults, which
    /// `the_compiled_defaults_validate` rules out.
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
        /// Why reading failed; part of the message, not a `source()` (B-7).
        source: std::io::Error,
    },
}

impl KeysError {
    /// The exit status `main` ends with: always 2 (a refusal, like `WorkerExit::Refused`).
    #[must_use]
    pub const fn code(&self) -> u8 { 2 }
}
impl std::fmt::Display for KeysError { /* §3.1 */ }
impl std::error::Error for KeysError {} // source() stays None (B-7)

/// Parses and checks a key file's text (D1, D2, D7, D8): `Ok` with the merged keys, or every
/// error sorted by line. A TOML syntax error is the only error then (D8 step 1).
///
/// # Errors
///
/// Every [`KeyFileError`] the text has, sorted by line (stable: same-line errors keep walk order).
pub fn load_str(src: &str) -> Result<Keys, Vec<KeyFileError>>;

/// Reads and checks the file at `path`.
///
/// # Errors
///
/// [`KeysError::Unreadable`] for any read error (missing included: the caller named it), else
/// [`KeysError::Invalid`] with `path`.
pub fn load_path(path: &Path) -> Result<Keys, KeysError>;

/// The keys in force (D3): the defaults with `default_keys`; else the file named by
/// `keys_flag`; else `<root>/keys.toml` when it exists; else the defaults. `root` is `None`
/// when the platform has no config directory. `default_keys` wins if both are given (clap
/// refuses that pair before this runs).
///
/// # Errors
///
/// As [`load_path`] for a named file. For the root's file: a missing file is the defaults, any
/// other read error is [`KeysError::Unreadable`], a bad file is [`KeysError::Invalid`].
pub fn resolve(
    keys_flag: Option<&Path>,
    default_keys: bool,
    root: Option<&Path>,
) -> Result<Keys, KeysError>;
```

Private helpers in `load.rs` (names are suggestions; behaviour is binding):
`fn line_of(src: &str, offset: usize) -> usize` (`src.get(..offset).unwrap_or(src)` then count
`b'\n'` + 1); `fn context_named(path: &str) -> Option<Context>` (over `super::contexts()`);
`fn walk_table(…)`, `fn entry(…)`, `fn check_version(…)`, `fn chord_of(written: &str) ->
Result<KeyChord, String>` (B-5).

### 2.5 `crates/htui/src/keys/validate.rs` (T4)

```rust
//! The key file's semantic checks over the merged keys (MOD-67 M2 D8 steps 5-6; ANA-26 §7.4
//! step 7): collisions in every context and in every declared stack, and printable chords on
//! actions offered while a field captures.

/// Every collision and capture error in `keys`, reported on the user's line (D8 steps 5-6).
/// The compiled defaults give none (`the_compiled_defaults_validate`).
#[must_use]
pub fn validate(keys: &Keys) -> Vec<KeyFileError>;
```

### 2.6 `crates/htui/src/keys/stack.rs` (T4)

```rust
/// Every stack the validator walks (ANA-26 §7.3, MOD-67 M2 D8 step 5), each with the phrase its
/// collision errors end with. M3-M5 append each view mode's stack.
pub static DECLARED: &[(&str, Stack<'static>)] = &[
    ("on every screen", Stack::BASE),
    ("over an overlay", Stack::OVERLAY),
];
```

### 2.7 `crates/htui/src/keys/print.rs` (T5)

```rust
//! `htui --print-keys` (MOD-67 M2 D10; ANA-26 §7.2, §7.5): the keys in force as one complete,
//! commented `keys.toml` that [`load_str`](super::load_str) reads back to the same keys.

/// The keys in force as a complete `keys.toml` (§4 of the blueprint gives the exact layout).
#[must_use]
pub fn print(keys: &Keys) -> String;
```

### 2.8 `crates/htui/src/cli.rs` (T6), three `Args` fields after `dsn_stdin`

```rust
    /// Read key bindings from this file instead of `keys.toml` in htui's config directory
    /// (MOD-67). A missing or invalid file stops htui with status 2.
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = ["default_keys", "set_dsn", "clear_dsn", "index_items", "search_items"]
    )]
    pub keys: Option<PathBuf>,

    /// Ignore `keys.toml` for this run and start with the default keys.
    #[arg(
        long,
        conflicts_with_all = ["keys", "set_dsn", "clear_dsn", "index_items", "search_items"]
    )]
    pub default_keys: bool,

    /// Print the keys in force as a commented `keys.toml` and exit; with an invalid file, print
    /// its errors and exit with status 2.
    #[arg(
        long,
        conflicts_with_all = [
            "set_dsn", "clear_dsn", "index_items", "search_items", "dsn_stdin", "demo", "offline"
        ]
    )]
    pub print_keys: bool,
```

No `env = …` on any of them (`no_tui_argument_but_log_reads_the_environment` stays green).
`args_conflicts_with_subcommands = true` (`cli.rs:12`) already refuses them beside a
subcommand. `Args` derives `Default`; nothing builds it with a struct literal (searched), so the
new fields break no call site.

### 2.9 `crates/htui/src/lib.rs` (T6)

```rust
/// `--print-keys` (MOD-67 M2 D10): the table on stdout. A reader that closed the pipe early
/// (`htui --print-keys | head`) is not a failure (B-4).
fn print_keys(out: &mut dyn std::io::Write, text: &str) -> anyhow::Result<()>;
```

### 2.10 `crates/htui/src/main.rs` (T6)

`exit_code` gains one arm, `reports_to_sentry` one conjunct (§10.3). No new item.

---

## 3. Message catalogue (exact text; tests pin these byte for byte)

`{t}` is the context's table (`Context::table()`), `{n}` the action name as written in the file
(= `ActionSpec::name` once resolved), `{w}` the chord string **as written** (the TOML string's
value, untrimmed), passed through `quote`. `{s}` is `quote(&chord.spec())` (the canonical
spelling, used where the error is about a merged row, not a written string). Every message is
the text after `keys.toml:LINE: `.

| Id | When | Line | Message |
|---|---|---|---|
| S1 | TOML syntax (D8.1) | the error span's start, else 1 | `not valid TOML: {toml message}` |
| V1 | top-level `version` not the integer 1 (D2; any radix/sign that parses to 1 is accepted, `0x1`, `+1`) | key | `version: write version = 1, the only version htui reads` |
| T1 | top-level non-table whose key names a context (`global = "x"`, `[[global]]`) | key | `{key}: write [{key}] as a table, one action per line` |
| T2 | any other top-level non-table but `version` | key | `{key}: only version may sit outside a table` |
| U1 | a table path that is no context and holds a value or nothing at all (D1) | the table key | `[{path}]: no such table; the tables are global, overlay, list, pane, confirm, form, common` |
| U2 | unknown name in a known context (D7) | key | `[{t}] {n}: no such action; [{t}] has {names of {t} in catalogue order, joined by ", "}` |
| W1 | value neither string nor array | key | `[{t}] {n}: write a chord such as "q", or a list such as ["q", "x"]` |
| W2 | array element that is not a string | element | `[{t}] {n}: write each chord as a string, such as "q"` |
| C1 | `parse_strict` refuses (D8.3) | element | `[{t}] {n} = {w}: {ChordError}` |
| K1 | the chord is `ctrl-c`, or a `CtrlCapital` whose suggestion is ctrl-c (D8.4, B-5) | element | `[{t}] {n} = {w}: ctrl-c always quits and cannot be bound` |
| D1 | the chord is already in this entry's list (D8.3) | element | `[{t}] {n} = {w}: already listed for this action` |
| O1 | `[overlay] close = []` (D8.4; only for a written empty list, B-8) | key | `[overlay] close: must keep at least one chord: overlays swallow every other key` |
| X1 | two actions share a chord in one context (D8.5) | the user row's | `[{t}] {n} = {s}: {s} is already {ot}.{on} ({origin}) in [{t}]` |
| X2 | two candidates for one chord in a declared stack (D8.5) | the user row's | `[{t}] {n} = {s}: {s} is already {ot}.{on} ({origin}) {phrase}` |
| P1 | an `in_capture` action has a printable chord (D8.6) | the row's | `[{t}] {n} = {s}: {s} is typed text while a field captures: bind a ctrl or alt chord or a named key` |

`{origin}` is `default` when the other row's `line` is `None`, else `line {N}`. `{phrase}` is the
`DECLARED` phrase (`on every screen`, `over an overlay`). The `[{t}] {n}` subject uses the
resolved table name, so a row reported from validation always has a catalogue name.

New `ChordError::NotDelivered` (T2), in M1's style:

| Variant | Message |
|---|---|
| `NotDelivered { written }` | `"{written}" never reaches htui: a Unix terminal sends it as another key or not at all` |

New `Indistinguishable` arrivals (T2; message is M1's `"{written}" arrives as {label}: a terminal cannot tell the two apart`):
`ctrl-2`, `ctrl-@` → `Ctrl+Space`; `ctrl-3` → `Esc`; `ctrl-8`, `ctrl-?` → `Backspace`;
`ctrl-/` → `Ctrl+7`.

### 3.1 The report (`KeysError` `Display`)

`HINT` = `` Fix the file, or run `htui --default-keys` to start with the default keys. ``

- `Invalid { path, errors }` (`file` = `path.file_name()` lossily, or the whole path if none):

  ```
  {path.display()} has {N} error{s}:
    {file}:{line}: {message}          ← one per error, in stored order (already sorted)
  {HINT}
  ```
  `error` when `N == 1`, else `errors`. Lines joined with `\n`; **no trailing newline** (`main`'s
  `eprintln!("htui: {error:#}")` adds it and prefixes the first line with `htui: `, giving ANA
  §7.5's shape exactly).
- `Unreadable { path, source }`: `cannot read {path.display()}: {source}\n{HINT}`.

`source()` returns `None` for both (B-7): `{error:#}` would otherwise print the io error a second
time after the hint. Example, pinned by T3 test 18:

```
/home/u/.config/htui/keys.toml has 2 errors:
  keys.toml:3: [global] quit = "ctrl-c": ctrl-c always quits and cannot be bound
  keys.toml:5: [common] edit = "shift-e": write a shifted letter as "E"
Fix the file, or run `htui --default-keys` to start with the default keys.
```

---

## 4. `--print-keys` format (exact)

1. The header, five lines, verbatim (line 2 is 99 columns; nothing exceeds 100):

   ```
   # htui key bindings, as `htui --print-keys` prints them. htui reads keys.toml in its config
   # directory: ~/.config/htui on Linux, ~/Library/Application Support/htui on macOS, %APPDATA%\htui
   # on Windows. List only what you change: a list replaces that action's chords, [] unbinds it.
   # ctrl-c always quits and cannot be listed. `htui --default-keys` ignores the file for one run.
   version = 1
   ```
   The header is constant (no path from the run), so the output is the same on every machine
   and the binary test can compare it to `keys::print`.
2. For each context in `contexts()` order that has rows (M2: global, overlay, list, pane,
   confirm, form, common): one blank line, `[{table}]`, then one line per row in row order.
3. A row line is `{left:<L$}  # {help}{mark}` where `left` = `{name:<N$} = {value}`, `N` = the
   longest name in **this table**, `L` = the longest `left` in **this table** (padding by `char`
   count, i.e. Rust's `{:<w$}`), then exactly two spaces, `# `, the row's help, and
   `mark` = ` (changed)` when `row.line.is_some()`, else nothing. No trailing whitespace.
4. `value` = `[` + each chord's `quote(&chord.spec())` joined by `, ` + `]`; unbound is `[]`.
5. The text ends with exactly one `\n` (after the last row).

The default output, all 60 lines (rendered from the catalogue by script; T5 test 1 pins lines
1-26 literally):

```
# htui key bindings, as `htui --print-keys` prints them. htui reads keys.toml in its config
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
close = ["esc"]  # close

[list]
down   = ["j", "down"]  # down
up     = ["k", "up"]    # up
top    = ["g", "home"]  # top
bottom = ["G", "end"]   # bottom
fold   = ["enter"]      # fold

[pane]
scroll_down = ["J"]                # scroll down
scroll_up   = ["K"]                # scroll up
page_down   = ["pgdn"]             # page down
page_up     = ["pgup"]             # page up
next_subtab = ["l", "]", "right"]  # next sub-tab
prev_subtab = ["h", "[", "left"]   # previous sub-tab

[confirm]
yes = ["y"]         # yes
no  = ["n", "esc"]  # no

[form]
next_field      = ["tab"]      # next field
prev_field      = ["backtab"]  # previous field
save            = ["ctrl-s"]   # save
external_editor = ["ctrl-e"]   # $EDITOR

[common]
edit    = ["e"]    # edit
new     = ["n"]    # new
delete  = ["d"]    # delete
clear   = ["c"]    # clear
reload  = ["r"]    # reload
back    = ["esc"]  # back
dismiss = ["esc"]  # dismiss
```

With `[global] quit = ["ctrl-q"]`, `workspaces = []` the global block's changed lines are (the
column stays 26 + 2 because `help`/`prev_tab` are still the widest):

```
quit         = ["ctrl-q"]   # quit (changed)
workspaces   = []           # workspaces (changed)
```
and with `[overlay] close = ["esc", "f2"]`: `close = ["esc", "f2"]  # close (changed)`.

---

## 5. Task 1: `identity::config_root_path()` (D3)

**Tests first** (`crates/htui-store/src/identity.rs` `mod tests`):
1. `config_root_path_is_the_config_dir_joined_with_htui`:
   `assert_eq!(config_root_path().ok(), dirs::config_dir().map(|dir| dir.join("htui")))`. It
   touches no file. **Do not call `config_root()` in a test**: it creates the developer's real
   `~/.config/htui` (memory: parallel-fanout-hidden-file-coupling, home-dir debris).
   Non-creation itself is pinned end to end by T7 test 8 (Linux, `XDG_CONFIG_HOME` on the child).

**Code**: `config_root_path` as §2.1; `config_root` becomes
```rust
pub fn config_root() -> Result<PathBuf> {
    let root = config_root_path()?;
    std::fs::create_dir_all(&root)
        .map_err(|e| StoreError::Backend(format!("cannot create {}: {e}", root.display())))?;
    Ok(root)
}
```
Its doc keeps "created if missing" and gains "[`config_root_path`] without the create". The error
text `this platform has no config directory` is unchanged (it moves into `config_root_path`).

**Commit**: `feat(mod-67): M2 T1 - identity::config_root_path, the non-creating config root`.

---

## 6. Task 2: the Unix legacy rejects (D6)

### 6.1 Tests first (`keys/chord.rs` `mod tests`)

1. `digits_and_symbols_with_a_legacy_byte_are_refused_by_what_arrives` (every platform): for
   `[("ctrl-2", "Ctrl+Space"), ("ctrl-@", "Ctrl+Space"), ("ctrl-3", "Esc"), ("ctrl-8",
   "Backspace"), ("ctrl-?", "Backspace"), ("ctrl-/", "Ctrl+7")]` the result is
   `Indistinguishable { written: spec, arrives_as }` with `arrives_as.label() == label`; and
   `strict("ctrl-2")`'s message is exactly
   `"ctrl-2" arrives as Ctrl+Space: a terminal cannot tell the two apart`.
2. `unix_drops_ctrl_characters_outside_letters_space_and_4_to_7` (every platform; calls the
   private `unix_drops` on chords built with `KeyChord::new`): `true` for ctrl + `1`, `0`, `9`,
   `.`, `,`, `;`, `'`, `=`, `-`, `+`, `é`, and ctrl-alt + `1`; `false` for ctrl + `a`, `z`, `h`,
   `j`, ` `, `4`, `5`, `6`, `7`, ctrl-alt + `x`, alt + `1`, and plain `1`, `q`.
3. `unix_drops_ctrl_and_shift_on_enter_tab_backspace_and_esc` (every platform): `true` for
   `(Enter, CONTROL)`, `(Enter, SHIFT)`, `(Enter, ALT|SHIFT)`, `(Tab, CONTROL)`,
   `KeyChord::new(Tab, CONTROL|SHIFT)` (= ctrl-backtab), `(Backspace, CONTROL)`,
   `(Backspace, SHIFT)`, `(Esc, CONTROL)`, `(Esc, SHIFT)`; `false` for bare `Enter`, `Tab`,
   `Backspace`, `Esc`, `KeyChord::new(Tab, SHIFT)` (= backtab), `(Enter, ALT)`,
   `(Backspace, ALT)`, `(Up, SHIFT)`, `(Up, CONTROL)`, `(F(5), SHIFT)`, `(Delete, CONTROL)`.
4. `#[cfg(unix)] a_chord_a_unix_terminal_never_sends_is_refused_there`: `strict("ctrl-1") ==
   Err(NotDelivered { written: "ctrl-1" })`, message exactly
   `"ctrl-1" never reaches htui: a Unix terminal sends it as another key or not at all`; the
   same variant for `shift-enter`, `ctrl-tab`, `ctrl-shift-tab`, `ctrl-backspace`, `shift-esc`,
   and `" ctrl - . "` (written `ctrl - .`, trimmed as M1 does).
   `#[cfg(not(unix))] a_chord_a_unix_terminal_never_sends_parses_elsewhere`: the same specs are
   `Ok` (`strict("shift-enter") == Ok(KeyChord::new(Enter, SHIFT))`).
5. **Rewrite** `ctrl_minus_and_ctrl_plus_are_writable` (`chord.rs:521`, F-2): keep its doc's point
   (the grammar reads `ctrl--`, `ctrl++`, `ctrl-+`), assert `Ok(ctrl -/+)` under
   `cfg!(not(unix))` and `Err(NotDelivered { written })` with the trimmed spec under
   `cfg!(unix)`; `strict("-")` stays `Ok` everywhere.
6. `the_older_refusals_still_win_over_the_unix_rule` (every platform): `ctrl-A` is
   `CtrlCapital { "ctrl-a" }`, `shift-1` and `ctrl-shift-1` are `ShiftedCharacter`, `ctrl-i` is
   still `Indistinguishable` (Tab).
7. Unchanged and still green: `every_catalogue_default_parses_strictly` (`keys/mod.rs`),
   `strict_reads_every_canonical_spec_as_parse_does`, `spec_round_trips_through_the_strict_parser`
   (no canonical spec is a D6 chord: plan claim 10 re-checked against `catalogue.rs`).

### 6.2 Code

- `legacy_arrival` gains, before `_ => return None`:
  ```rust
  '2' | '@' => KeyChord::new(KeyCode::Char(' '), mods),
  '3' => KeyChord::new(KeyCode::Esc, without_ctrl),
  '8' | '?' => KeyChord::new(KeyCode::Backspace, without_ctrl),
  '/' => KeyChord::new(KeyCode::Char('7'), mods),
  ```
  Its doc cites `crossterm-0.29.0/src/event/sys/unix/parse.rs:92-118` (`0x00` → ctrl-space,
  `0x1B` → Esc, `0x1C..=0x1F` → ctrl-4..7, `0x7F` → Backspace).
- `unix_drops` (§2.2), body:
  ```rust
  let ctrl = chord.mods.contains(KeyModifiers::CONTROL);
  match chord.code {
      KeyCode::Char(c) => ctrl && !(c.is_ascii_lowercase() || c == ' ' || ('4'..='7').contains(&c)),
      KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Esc => {
          chord.mods.intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
      }
      KeyCode::BackTab => ctrl,
      _ => false,
  }
  ```
- `parse_strict`'s last line `Ok(Self::new(code, mods))` becomes
  ```rust
  let chord = Self::new(code, mods);
  if cfg!(unix) && unix_drops(chord) {
      return Err(ChordError::NotDelivered { written: written.to_owned() });
  }
  Ok(chord)
  ```
  `cfg!` (not `#[cfg]`) keeps `unix_drops` used on every target: no `dead_code` on Windows.
- `ChordError::NotDelivered` + its `Display` arm (§3). Update `parse_strict`'s doc ("… and, on
  Unix, a chord the terminal never delivers (D6)"), the module doc and `ChordError`'s doc.
- `parse` (lenient) is untouched.

**Commit**: `feat(mod-67): M2 T2 - Unix legacy-encoding rejects in parse_strict`.

---

## 7. Task 3: the loader (D1-D3, D7, D8 steps 1-4)

### 7.1 Tests first (`keys/load.rs` `mod tests`)

Helper: `fn errors(src: &str) -> Vec<(usize, String)>` = `load_str(src).expect_err(..)` mapped to
`(line, message)`. Every expectation below is the **full** vector.

1. `an_empty_file_is_the_defaults`: `load_str("") == Ok(Keys::defaults())` and
   `Keys::line` is `None` for every catalogue row.
2. `a_file_with_only_version_is_the_defaults`: same for `"version = 1\n"`.
3. `a_bom_and_crlf_still_give_the_right_lines`: `"\u{feff}version = 1\r\n\r\n[global]\r\nquit =
   [\r\n  \"x\",\r\n  \"shift-a\",\r\n]\r\n"` → `[(6, "[global] quit = \"shift-a\": write a
   shifted letter as \"A\"")]`.
4. `a_list_replaces_the_defaults_and_a_string_is_a_list_of_one`: `"[global]\nquit = \"x\"\nhelp =
   [\"f1\"]\n"` → `chords(Global, Quit) == [x]`, `chords(Global, Help) == [F1]`,
   `line(Global, Quit) == Some(2)`, `line(Global, Help) == Some(3)`, every other row as default.
5. `an_empty_list_unbinds`: `"[global]\nworkspaces = []\n"` → `chords(Global, Workspaces)` empty,
   `line == Some(2)`.
6. `a_list_equal_to_the_default_is_not_a_change`: `"[list]\ndown = [\"j\", \"down\"]\n"` →
   `== Keys::defaults()` and `line(List, ListDown) == None`; `"[list]\ndown = [\"down\", \"j\"]\n"`
   (reordered) → `line == Some(2)` (order is meaning: the first chord is the hint's).
7. `version_must_be_the_integer_1`: each of `version = 2`, `version = "1"`, `version = 1.0`,
   `version = true`, `version = [1]` → `[(1, "version: write version = 1, the only version htui
   reads")]`; `version = 0x1` and `version = +1` load.
8. `an_unknown_table_lists_the_tables`: `"[globl]\nquit = \"x\"\n"`, `"[settings.boxes]\nreload
   = \"r\"\n"`, `"[global.extra]\nx = \"y\"\n"`, `"[nothing]\n"` → each one error on line 1:
   `[globl]: no such table; the tables are global, overlay, list, pane, confirm, form, common`
   (and `[settings.boxes]`, `[global.extra]`, `[nothing]` likewise). `"[settings.boxes]"` reports
   once, for the leaf, not for `settings` (which holds only a table).
9. `an_unknown_name_lists_the_tables_actions`: `"[overlay]\nshut = 5\n"` → `[(2, "[overlay]
   shut: no such action; [overlay] has close")]` (the bad value is not inspected).
10. `a_value_must_be_a_chord_or_a_list_of_chords`: `"[global]\nquit = 5\n"` → `[(2, "[global]
    quit: write a chord such as \"q\", or a list such as [\"q\", \"x\"]")]`;
    `"[global]\nquit = [\"x\", 5, [\"y\"]]\n"` → two `(2, "[global] quit: write each chord as a
    string, such as \"q\"")` and `quit` is **not** in error otherwise (B-8: the strings merge);
    `"[global]\nquit = { a = \"x\" }\n"` → `[(2, "[global.quit]: no such table; …")]` (F-10).
11. `every_chord_is_parsed_strictly_with_the_reason`: `"[global]\nquit = [\"shift-a\", \"ctrl-I\",
    \"hyper-x\"]\n"` → three line-2 errors in element order, M1's messages after
    `[global] quit = "<spec>": `.
12. `ctrl_c_is_refused_however_it_is_spelled`: `"[global]\nquit = [\"q\", \"ctrl-c\"]\nfind =
    \"ctrl-C\"\nwaiting = \" ctrl + shift + c \"\n"` → `(2, "[global] quit = \"ctrl-c\": ctrl-c
    always quits and cannot be bound")`, `(3, "[global] find = \"ctrl-C\": …")`,
    `(4, "[global] waiting = \" ctrl + shift + c \": …")` (as written, untrimmed, quoted).
    `ctrl-alt-C` is **not** the ctrl-c refusal: it is `CtrlCapital` "write \"ctrl-alt-c\": …".
13. `a_chord_listed_twice_in_one_action_is_refused`: `"[global]\nprev_tab = [\"shift-tab\",
    \"backtab\"]\n"` → `[(2, "[global] prev_tab = \"backtab\": already listed for this action")]`.
14. `overlay_close_must_keep_a_chord`: `"[overlay]\nclose = []\n"` → `[(2, "[overlay] close: must
    keep at least one chord: overlays swallow every other key")]`; `"[overlay]\nclose =
    [\"ctrl-c\"]\n"` → only the ctrl-c error (B-8: O1 is for a written `[]`).
15. `every_error_is_reported_and_sorted_by_line`: the text of `tests/fixtures/keys/errors.toml`
    (§11.2) → exactly the six `(line, message)` pairs listed there, in that order.
16. `a_syntax_error_is_the_only_error_with_its_line`: `"[global]\nquit = \"x\"\nquit =
    [\"shift-a\"]\n"` → `[(3, "not valid TOML: duplicate key")]`; `"[global]\nquit =
    \"x\"\n[global]\n"` → `[(3, "not valid TOML: duplicate key")]`; `"[global\n"` →
    `[(1, "not valid TOML: unclosed table, expected `]`")]` (toml 1.1.5 texts, F-6).
17. `a_top_level_value_is_refused`: `"global = \"x\"\n"` → `[(1, "global: write [global] as a
    table, one action per line")]`; `"[[global]]\nquit = \"x\"\n"` → the same on line 1;
    `"quit = \"x\"\n"` → `[(1, "quit: only version may sit outside a table")]`.
18. `the_report_has_ana_7_5s_shape`: `KeysError::Invalid { path: "/home/u/.config/htui/keys.toml",
    errors: [(3, …ctrl-c…), (5, …shift-e…)] }.to_string()` is §3.1's example exactly; one error
    gives `has 1 error:`; `Unreadable { path: "/x/keys.toml", source:
    io::Error::new(NotFound, "gone") }.to_string() == "cannot read /x/keys.toml: gone\nFix the
    file, or run `htui --default-keys` to start with the default keys."`; `source()` is `None`
    for both; `code() == 2` for both.
19. `resolve` (each its own test, over a `tempfile::tempdir()` root, never the user's):
    a. `default_keys_ignores_even_a_broken_root_file` → `Ok(defaults)`;
    b. `a_named_file_is_read_and_a_missing_one_refused` → `Ok` for a good file;
       `Unreadable { path }` with the named path for a missing one;
    c. `a_missing_root_file_or_root_is_the_defaults` (empty root; and `root = None`);
    d. `a_broken_root_file_is_invalid_with_its_path` → `Invalid { path: root/keys.toml, .. }`;
    e. `a_root_file_that_cannot_be_read_is_unreadable` (`keys.toml` is a directory).

### 7.2 Code

- `crates/htui/Cargo.toml`, after `serde`:
  ```toml
  # MOD-67 M2 D1: keys.toml is parsed with spans (`toml::de::DeTable`). 1.1.5 is already in the
  # lock through htui-store, so nothing new is compiled.
  toml               = { workspace = true }
  ```
  Then `cargo build -p htui` updates `Cargo.lock` (one line in `htui`'s dependency list). Stage
  `Cargo.lock` with the commit.
- `keys/mod.rs`: `Row.line`, hand-written `PartialEq`/`Eq` (B-2), `Keys::line`, `Keys::set`,
  `contexts`, `quote`, `pub mod load;`, the `load` re-exports (§2.3). Both `Row` literals get
  `line: None`.
- `keys/load.rs`, `load_str`, in this order:
  1. `DeTable::parse(src)`; on `Err(e)` return `vec![KeyFileError { line:
     e.span().map_or(1, |span| line_of(src, span.start)), message: format!("not valid TOML: {}",
     e.message()) }]` (S1). Nothing else runs (D8.1).
  2. `let mut keys = Keys::defaults(); let mut errors = Vec::new();`
  3. For each top-level `(key, value)` (sorted iteration): `DeValue::Table(t)` → `walk_table(key,
     t)`; else if `key == "version"` → V1 unless the value is `DeValue::Integer(i)` with
     `i64::from_str_radix(i.as_str(), i.radix()) == Ok(1)`; else T1 if `context_named(key)` is
     `Some`, else T2.
  4. `walk_table(path, key_span, table)`: let `has_values` = any entry that is not a table.
     If `context_named(path)` is `None` and (`has_values` or `table.is_empty()`) → U1 once, on
     `key_span`'s line. Then for each entry: a table → recurse with `format!("{path}.{name}")`;
     a value in a known context → `entry`; a value in an unknown one → nothing more.
  5. `entry(context, key, value)`: resolve `CATALOGUE.iter().find(|s| s.context == context &&
     s.name == key)`, else U2 (names of `context` in catalogue order) and return. Items: a
     `String` is `[value]`, an `Array` its elements, anything else W1 and return. For each item:
     non-string → W2; else `chord_of(written)` → C1 / K1; a chord already pushed → D1; else push.
     If the act is `OverlayClose` and the **written** list is empty → O1. Finally
     `keys.set(context, act, chords, line_of(key.span().start))` (B-8: always merge what passed).
  6. `chord_of(written)` (B-5):
     ```rust
     match KeyChord::parse_strict(written) {
         Ok(chord) if chord == CTRL_C => Err(CTRL_C_REFUSED.to_owned()),
         Err(ChordError::CtrlCapital { suggestion })
             if KeyChord::parse_strict(&suggestion) == Ok(CTRL_C) =>
         {
             Err(CTRL_C_REFUSED.to_owned())
         }
         Ok(chord) => Ok(chord),
         Err(err) => Err(err.to_string()),
     }
     ```
     with `const CTRL_C_REFUSED: &str = "ctrl-c always quits and cannot be bound";`.
  7. `errors.sort_by_key(|error| error.line)` (stable). `Ok(keys)` iff `errors.is_empty()`.
- `load_path`: `std::fs::read_to_string(path)` → any `Err` is `Unreadable` (non-UTF-8 included);
  `load_str` → `Invalid { path: path.to_owned(), errors }`.
- `resolve`: as §2.4; the root branch matches `ErrorKind::NotFound` → `Ok(defaults)` (a dangling
  symlink is `NotFound` too, and so the defaults; noted in §13).
- In T3 **no doc links to `validate` or `print`** (they do not exist yet; name them in plain
  backticks; rustdoc denies broken links).

**Commit** (1, or 2 with `KeysError` + `resolve` second):
`feat(mod-67): M2 T3 - keys.toml loader with spans and the KeysError report`.

---

## 8. Task 4: the validator (D8 steps 5-6)

### 8.1 Tests first

`keys/stack.rs`:
1. `declared_walks_every_screen_then_over_an_overlay`: `DECLARED == [("on every screen",
   Stack::BASE), ("over an overlay", Stack::OVERLAY)]`.

`keys/validate.rs` (user rows come from `load_str`, so the messages and lines are the real ones):
2. `the_compiled_defaults_validate`: `validate(Keys::compiled()).is_empty()` (D8: "the compiled
   defaults pass 2-6"; steps 2-4 are file-only and pass vacuously, T5 test 2 runs them through a
   file).
3. `two_actions_sharing_a_chord_in_one_context_are_refused_on_the_users_line`:
   `"[list]\ntop = [\"j\"]\n"` → `[(2, "[list] top = \"j\": \"j\" is already list.down (default)
   in [list]")]`.
4. `a_shared_context_no_view_offers_yet_is_still_checked`: `"[confirm]\nyes = [\"n\"]\n"` →
   `[(2, "[confirm] yes = \"n\": \"n\" is already confirm.no (default) in [confirm]")]`.
5. `two_user_rows_report_on_the_later_line`: `"[global]\nquit = [\"x\"]\nhelp = [\"x\"]\n"` →
   `[(3, "[global] help = \"x\": \"x\" is already global.quit (line 2) in [global]")]`.
6. `a_collision_over_an_overlay_is_found_in_the_overlay_stack`: `"[global]\nhelp =
   [\"esc\"]\n"` → `[(2, "[global] help = \"esc\": \"esc\" is already overlay.close (default)
   over an overlay")]`.
7. `a_collision_seen_by_two_checks_is_reported_once`: `"[global]\nquit = [\"w\"]\n"` → exactly
   `[(2, "[global] quit = \"w\": \"w\" is already global.workspaces (default) in [global]")]`
   (the BASE stack sees the same pair; deduplicated).
8. `a_state_guarded_default_pair_is_allowed`: `"[common]\nback = [\"esc\"]\n"` (restates the
   default, so `line` stays `None`, B-1) → `Ok`.
9. `a_state_guarded_pair_the_user_changes_is_refused` (**D8 literal; its first case flips under
   PA-2**): `"[common]\nback = [\"esc\", \"backspace\"]\n"` → `[(2, "[common] back = \"esc\":
   \"esc\" is already common.dismiss (default) in [common]")]`; and `"[common]\ndismiss =
   [\"x\"]\nback = [\"x\"]\n"` → `[(3, "[common] back = \"x\": \"x\" is already common.dismiss
   (line 2) in [common]")]` (`back` is on the later line, so it is the reported row; an error under
   PA-2 too).
10. `an_in_capture_action_refuses_a_printable_chord`: `"[form]\nsave = [\"s\"]\n"` → `[(2,
    "[form] save = \"s\": \"s\" is typed text while a field captures: bind a ctrl or alt chord
    or a named key")]`; `"[overlay]\nclose = [\"x\"]\n"` → the same shape for `[overlay] close`
    (F-3); `"[form]\nsave = [\"alt-s\", \"f2\"]\nnext_field = [\"down\"]\n"` → `Ok`.

### 8.2 Code

- `stack.rs`: `DECLARED` (§2.6); module doc gains one sentence.
- `validate.rs`, `validate(keys)`: three passes, results appended in this order, then returned
  (`load_str` sorts):
  1. **Contexts**: for every pair of rows `i < j` with the same `context`, for each chord of row
     `i` (in order) that row `j` also binds → `report(i_row, j_row, chord, "in [{t}]")`.
  2. **Stacks**: for each `(phrase, stack)` in `DECLARED`: collect the distinct chords of the rows
     the stack admits (layer order, row order, first occurrence); for each, `let acts =
     keys.actions(stack, chord)`; for each pair `a < b` of `acts`, rows via
     `keys.resolve_row(stack, act)` → `report(…, phrase)`.
  3. **Capture**: each row whose `act.spec()` has `in_capture`, each printable chord → P1 on
     `row.line.unwrap_or(0)`.
  `report(first, second, chord, place)`: skip if the unordered pair `((ctx, act), (ctx, act))`
  with this chord was already reported (a `Vec`, checked both ways); skip if **allowed** (B-9:
  `first.line.is_none() && second.line.is_none() && STATE_GUARDED` holds the pair either way);
  else the reported row is the one with the greater `line` (`Option` order, `None < Some`), the
  second on a tie; message X1/X2 with the other row's origin; line `reported.line.unwrap_or(0)`.
- `load.rs`: `errors.extend(validate(&keys))` just before the sort, and the module doc may now
  link [`validate`](super::validate).
- `mod.rs`: `pub mod validate;`, `pub use validate::validate;`, `DECLARED` in the stack re-export.

**Commit**: `feat(mod-67): M2 T4 - validator over every context and declared stack`.

---

## 9. Task 5: `--print-keys` (D10)

### 9.1 Tests first (`keys/print.rs`)

1. `the_default_print_starts_with_the_header_global_and_overlay`: the first 26 lines of
   `print(Keys::compiled())` equal §4's lines 1-26 (a literal `const` in the test).
2. `the_default_print_reads_back_as_the_defaults`: `load_str(&print(Keys::compiled())) ==
   Ok(Keys::compiled().clone())`, and the re-print contains no `(changed)` (this is D8's "the
   compiled defaults pass 2-6" through a real file).
3. `a_changed_table_round_trips_and_keeps_its_marks`: `keys = load_str("[global]\nquit =
   [\"ctrl-q\"]\nworkspaces = []\n\n[overlay]\nclose = [\"esc\", \"f2\"]\n")`; `printed =
   print(&keys)`; `load_str(&printed) == Ok(keys)` (B-2 makes `==` binding-only) **and**
   `print(&load_str(&printed)?) == printed` (the marks survive); `printed` contains §4's three
   changed lines verbatim and `printed.matches("(changed)").count() == 3`.
4. `quote_escapes_what_toml_needs`: `quote("\\") == r#""\\""#`, `quote("\"") == r#""\"""#`,
   `quote("\u{1}") == "\"\\u0001\""`, `quote("ctrl-f") == "\"ctrl-f\""`; and
   `"[global]\nquit = [\"\\\\\"]\n"` (the chord `\`) prints and reads back to the same keys.

### 9.2 Code

`print(keys)` as §4: the header `const`, then per context of `contexts()`: rows of that context
(`keys.rows`, row order), widths, lines; `Keys::line` (or `row.line`) for the mark. `pub mod
print;` and `pub use print::print;` in `mod.rs` (a module and a function may share the name:
different namespaces).

**Commit**: `feat(mod-67): M2 T5 - --print-keys renders the keys as commented TOML`.

---

## 10. Task 6: flags, the load point, exit 2 (D4, D5, D9)

### 10.1 Tests first

`cli.rs`:
1. `the_key_flags_parse_alone_and_beside_the_tui_flags`: `--keys k.toml` (→ `Some("k.toml")`),
   `--default-keys`, `--print-keys`, `--print-keys --keys k.toml`, `--print-keys --default-keys`,
   `--keys k.toml --demo`, `--keys k.toml --offline`, `--keys k.toml --dsn-stdin`,
   `--default-keys --demo` all parse.
2. `the_key_flags_refuse_flags_that_never_read_keys`: refused: `--keys k --default-keys`; each
   of `[--keys k]`, `[--default-keys]`, `[--print-keys]` beside each of `--set-dsn`,
   `--clear-dsn`, `--index-items`, `--search-items q`; `--print-keys` beside `--dsn-stdin`,
   `--demo`, `--offline`; `--print-keys worker` and `--keys k worker`.
3. `no_tui_argument_but_log_reads_the_environment` (`cli.rs`, existing) stays green unchanged.

`lib.rs` `mod tests`:
4. `print_keys_writes_the_text_and_ignores_a_closed_pipe`: into a `Vec<u8>` → the bytes; into a
   writer whose `write` returns `ErrorKind::BrokenPipe` → `Ok(())`; one returning
   `ErrorKind::Other` → `Err`.

`main.rs` `mod tests`:
5. `a_key_file_refusal_exits_2_and_never_reaches_sentry`: for `KeysError::Invalid { .. }` and
   `KeysError::Unreadable { .. }`: `exit_code(&err) == 2`; `!reports_to_sentry(&err, 2)` **and**
   `!reports_to_sentry(&err, 1)` (refused by type, not only by code, D9); the same for
   `err.context("while starting")`.

### 10.2 Code: `lib.rs` (`run`)

Between the `--search-items` early exit (ends `lib.rs:128`) and `let started = if args.demo`
(`lib.rs:130`):
```rust
    // MOD-67 M2 D5: the key file, after every flag that exits without the TUI and before the
    // backend, `--demo` included (ANA-26 §6.3), so a bad file stops htui before the store or the
    // terminal is touched (§7.4 step 8). `--print-keys` ends here.
    let keys = keys::resolve(
        args.keys.as_deref(),
        args.default_keys,
        identity::config_root_path().ok().as_deref(),
    )?;
    if args.print_keys {
        return print_keys(&mut std::io::stdout().lock(), &keys::print(&keys));
    }
```
and after `let mut app = App::new(request_tx, Keymap::default_global());` (`lib.rs:182`), before
`app::register_all`:
```rust
    app.keys = keys;
```
`print_keys` body:
```rust
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(err) if err.kind() != std::io::ErrorKind::BrokenPipe => Err(err.into()),
        _ => Ok(()),
    }
```
`run`'s doc: the startup-order paragraph gains "then the key file (MOD-67), which
`--print-keys` prints and exits on"; `# Errors` gains "when the key file cannot be read or is
invalid ([`keys::KeysError`], exit 2)". (The local `keys` and the module `keys` live in different
namespaces; `keys::resolve`/`keys::print` are paths, `&keys` the value.)

### 10.3 Code: `main.rs`

- `exit_code` (`main.rs:100`): one more `.or_else(|| error.downcast_ref::<htui::keys::KeysError>()
  .map(htui::keys::KeysError::code))` before `.unwrap_or(1)`; its doc names `KeysError`.
- `reports_to_sentry` (`main.rs:124`): `&& error.downcast_ref::<htui::keys::KeysError>().is_none()`;
  its doc gains "MOD-67 M2 D9: no key-file refusal either: the report carries the user's home
  path".
- `main`'s doc: the exit-2 list gains "a key file htui refuses (MOD-67)".

**Commits**: `feat(mod-67): M2 T6 - --keys, --default-keys, --print-keys` (cli.rs), then
`feat(mod-67): M2 T6 - load keys.toml at startup; KeysError exits 2 outside Sentry` (lib.rs,
main.rs). Each compiles alone: the new `Args` fields are `pub`, so not dead before `lib.rs` reads
them.

---

## 11. Task 7: integration tests over fixtures

### 11.1 `crates/htui/tests/keys_file.rs` layout

No file-level `cfg`: the binary cases need no `testkit`. Three modules:
- `#[cfg(unix)] mod binary` (std `Command`, synchronous): `const HTUI: &str =
  env!("CARGO_BIN_EXE_htui");` a helper `run(args, home: &Path) -> Output` with `env_clear()`,
  `PATH` from the test's env, and `HOME` and `XDG_CONFIG_HOME` both set to `home` (a
  `tempfile::tempdir()`; F-9), so no case can read the developer's file; fixtures by
  `Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keys").join(name)`.
- `#[cfg(target_os = "linux")] mod default_path` (same helper).
- `#[cfg(feature = "testkit")] mod app` (tokio, `Harness`).

Expected stderr is built with `format!("htui: {} has …", path.display())` and compared with
`assert_eq!` on `String::from_utf8(output.stderr)`; every report ends with `\n` (from
`eprintln!`).

### 11.2 Fixtures (exact content; line numbers matter)

`valid.toml`:
```toml
# MOD-67 M2 fixture (tests/keys_file.rs): a valid key file. Quit moves to ctrl-q; F2 also closes
# an overlay.
version = 1

[global]
quit = ["ctrl-q"]

[overlay]
close = ["esc", "f2"]
```

`errors.toml` → `has 6 errors:`
```toml
# MOD-67 M2 fixture: one of each entry error, out of order on purpose.
version = 2

[list]
down = 5
up = ["k", 7]

[globl]
help = "f1"

[global]
quitt = "q"
quit = ["x", "shift-a"]
```
Expected lines (after `  errors.toml:`):
```
2: version: write version = 1, the only version htui reads
5: [list] down: write a chord such as "q", or a list such as ["q", "x"]
6: [list] up: write each chord as a string, such as "q"
8: [globl]: no such table; the tables are global, overlay, list, pane, confirm, form, common
12: [global] quitt: no such action; [global] has quit, next_tab, prev_tab, select_tab_1, select_tab_2, select_tab_3, select_tab_4, select_tab_5, select_tab_6, select_tab_7, select_tab_8, select_tab_9, help, workspaces, find, waiting
13: [global] quit = "shift-a": write a shifted letter as "A"
```

`ctrl_c.toml` → `has 2 errors:`
```toml
# MOD-67 M2 fixture: ctrl-c, however it is spelled.
[global]
quit = ["q", "ctrl-c"]
find = "ctrl-C"
```
```
3: [global] quit = "ctrl-c": ctrl-c always quits and cannot be bound
4: [global] find = "ctrl-C": ctrl-c always quits and cannot be bound
```

`overlay_close.toml` → `has 1 error:`
```toml
# MOD-67 M2 fixture: the overlay close unbound.
[overlay]
close = []
```
```
3: [overlay] close: must keep at least one chord: overlays swallow every other key
```

`collision.toml` → `has 4 errors:`
```toml
# MOD-67 M2 fixture: collisions in one context and over an overlay, and typed text in a form.
[global]
quit = ["w"]
help = ["esc"]

[form]
save = ["s"]

[list]
top = ["j"]
```
```
3: [global] quit = "w": "w" is already global.workspaces (default) in [global]
4: [global] help = "esc": "esc" is already overlay.close (default) over an overlay
7: [form] save = "s": "s" is typed text while a field captures: bind a ctrl or alt chord or a named key
10: [list] top = "j": "j" is already list.down (default) in [list]
```

`syntax.toml` → `has 1 error:` (the `shift-a` is never checked: D8.1)
```toml
# MOD-67 M2 fixture: a TOML syntax error stops every other check.
[global]
quit = "x"
quit = ["shift-a"]
```
```
4: not valid TOML: duplicate key
```

### 11.3 Tests

`binary` (Unix):
1. `print_keys_prints_a_valid_file`: `--keys valid.toml --print-keys` → status 0, empty stderr,
   stdout `== htui::keys::print(&htui::keys::load_path(&valid)?)`, and contains
   `quit         = ["ctrl-q"]   # quit (changed)`.
2. `print_keys_with_default_keys_prints_the_defaults`: `--default-keys --print-keys` → 0, stdout
   `== print(Keys::compiled())`, no `(changed)`.
3. `each_bad_fixture_exits_2_with_its_exact_report_and_nothing_on_stdout`: for `errors`,
   `ctrl_c`, `overlay_close`, `collision`, `syntax`: `--keys F --print-keys` → `code() ==
   Some(2)`, empty stdout, stderr `== format!("htui: {} has {N} error{s}:\n{lines}Fix the file,
   or run `htui --default-keys` to start with the default keys.\n", path.display())` with §11.2's
   lines each as `  {name}.toml:{line}: {message}\n`.
4. `a_bad_file_also_stops_the_tui_before_the_terminal`: `--keys errors.toml --offline` (no
   `--print-keys`) → status 2 and the same stderr: the refusal precedes `connect::start` and
   `terminal::init`. (Runs with no TTY; if the load point were late it would fail differently.)
5. `a_missing_named_file_exits_2`: `--keys <tmp>/missing.toml --print-keys` → 2; stderr starts
   with `htui: cannot read {path}: ` and ends with `\nFix the file, or run \`htui
   --default-keys\` to start with the default keys.\n` (the OS text between is not pinned).
6. `keys_and_default_keys_are_refused_together`: `--keys valid.toml --default-keys` → status 2
   (clap's usage error), stderr contains `cannot be used with`.

`default_path` (Linux, `XDG_CONFIG_HOME=<tmp>`):
7. `the_config_directorys_file_is_read`: write `<tmp>/htui/keys.toml` = `"[global]\nquit =
   \"ctrl-c\"\n"`; `--print-keys` → 2, stderr `== "htui: <tmp>/htui/keys.toml has 1 error:\n
   keys.toml:2: [global] quit = \"ctrl-c\": ctrl-c always quits and cannot be bound\nFix …\n"`.
8. `no_file_is_the_defaults_and_creates_nothing`: empty `<tmp>`; `--print-keys` → 0, stdout `==
   print(Keys::compiled())`, and `!<tmp>/htui` exists afterwards (T1's non-creation, end to end).
9. `default_keys_ignores_a_broken_file`: the file of test 7 + `--default-keys --print-keys` → 0.

`app` (`testkit`):
10. `a_rebound_quit_and_close_work_end_to_end` (`#[tokio::test]`): `let keys =
    htui::keys::load_path(&valid)?;` `Harness::demo().with_agent_runtime(AgentRuntime::new(
    DriverFactory::new()))`, `harness.app().keys = keys;` **then** `register_all(harness.app())`
    (D5's order), `drive_to_end`. Assert: `key("q")` leaves `should_quit` false; the status line
    starts with `Ctrl+q quit · Tab next tab`; `key("?")` opens a box containing `Ctrl+q/Ctrl+c
    quit`; `key("?")` closes it; `key("w")` opens the switcher, `key("f2")` closes it
    (`WorkspaceSwitcher::on_key` passes every code but `j`/`k`/arrows/`Enter`,
    `workspace_switcher.rs:161-174`); `key("w")` then `key("?")` shows `Overlay: Esc/F2 close`;
    `key("esc")` still closes; finally `key("ctrl-q")` sets `should_quit` (the Backlog tab passes
    CONTROL chords, ANA §2.6, `backlog/mod.rs:231`).

**Commit**: `test(mod-67): M2 T7 - keys.toml fixtures end to end`.

---

## 12. Task 8: README

- **Command-line options** table (`README.md:87-104`), after `--dsn-stdin`:
  - `` | `--keys <PATH>` | Read key bindings from this file instead of `keys.toml`. See [Changing keys](#changing-keys). | ``
  - `` | `--default-keys` | Ignore `keys.toml` for this run and use the default keys. | ``
  - `` | `--print-keys` | Print every key binding in force as a commented `keys.toml` and exit. With an invalid file, print its errors and exit with status 2. | ``
- **New `### Changing keys`** under "Using htui", right after "### Everywhere" (`README.md:185`):
  the file is `keys.toml` in the config directory (link "Where htui keeps its files"); list only
  what you change, a list replaces the action's keys and `[]` unbinds it; `htui --print-keys`
  prints every action (start from it); a short example (`[global] quit = ["ctrl-q"]`,
  `[overlay] close = ["esc", "f2"]`); htui refuses to start on an invalid file and names each
  line, `--default-keys` starts anyway; `ctrl-c` always quits and cannot be bound; and **the D11
  sentence**: "Until every screen reads its keys from this file, a key a screen uses itself wins
  there: with `[global] quit = ["e"]`, `e` still edits in Settings instead of quitting." (Not the
  `close = ["x"]` example: the validator refuses it, F-3.)
- **Where htui keeps its files** table (`README.md:437-440`): `` | `keys.toml` | Your key
  bindings, if you change any. htui only reads it. | ``
- The "Everywhere" table is not changed (no default changes).

**Commit**: `docs(mod-67): M2 T8 - README: keys.toml and the three key flags`.

---

## 13. Hazards and edge cases

| # | Hazard | Where | Handling |
|---|---|---|---|
| H-1 | `DeTable` iterates **sorted by key** (`preserve_order` off in this workspace's feature set), not file order. | toml 1.1.5 `src/de/parser/detable.rs` doc; `cargo tree -e features` | Nothing depends on order but the tie-break of same-line errors (walk order). Sort is stable by line only. If a future dependency turns on `preserve_order`, only same-line ties could reorder; no fixture has one across keys. |
| H-2 | toml's parser messages are pinned by T3 test 16 and the `syntax.toml` fixture. | F-6 | A toml bump may move them; the lock pins 1.1.5. Re-baseline those two expectations only. |
| H-3 | `KeysError` is converted to `anyhow::Error` by `?` in `run`; `main` prints `{error:#}`, which appends every `source()`. | `main.rs` `body` | `source()` is `None` (B-7), the io error is inside `Display`. Pinned by T3 test 18 and T7 test 5. |
| H-4 | `exit_code` / `reports_to_sentry` downcast through `.context(…)`: anyhow supports it, the provision test relies on it. | `main.rs:100`, `:124` | T6 test 5 covers the wrapped case. |
| H-5 | A dangling `keys.toml` symlink reads as `NotFound` → the defaults, silently. | `resolve` | Accepted (D3 "missing → defaults"); documented in `resolve`'s doc. |
| H-6 | A non-UTF-8 `keys.toml` is `Unreadable` ("stream did not contain valid UTF-8"), not a line error. | `load_path`, `resolve` | Accepted; exit 2 with the hint. |
| H-7 | `Row` equality ignores `line` (B-2): `assert_eq!(a, b)` on two `Keys` no longer proves the same provenance. | `keys/mod.rs` | Documented on `Row`; provenance is asserted through `Keys::line` or `print`. `compiled_is_built_once` (`keys/mod.rs`) still holds. |
| H-8 | `unix_drops` changes what `ctrl--`/`ctrl-+` mean on Unix. | F-2 | T2 rewrites the M1 test; README never advertises those chords. |
| H-9 | The binary cases spawn `htui`, which initialises Sentry with the real DSN (`main.rs` `body`). | T7 | Every spawned case ends in exit 0 or a refusal (2): nothing is captured. Do not add a case that exits 1. |
| H-10 | Spawned binaries could read the developer's `keys.toml` or write into their config dir. | F-9 | `env_clear` + `HOME` + `XDG_CONFIG_HOME` = temp dir on every child; cases that look up the default path are Linux-only. T7 test 8 also proves `--print-keys` creates nothing. |
| H-11 | `htui --print-keys \| head` | F-5 | `print_keys` ignores `BrokenPipe`. |
| H-12 | The load point must precede `Started::detached` (`--demo`, `lib.rs:131`) and `connect::start` (`lib.rs:146`), and `app.keys` must be set before `register_all` (`lib.rs:186`). | D5 | §10.2; T7 test 4 proves "before the terminal". |
| H-13 | `missing_docs` covers struct-variant fields (`KeysError::Invalid { path, errors }`, `Unreadable { path, source }`, `NotDelivered { written }`). | `lib.rs:10` | §2 gives a doc for each. |
| H-14 | Intra-doc links across tasks: T3 must not link `validate`/`print`; `Row` is private. | rustdoc deny list | §7.2 last bullet; `cargo doc -p htui --no-deps` in the T3 and T5 gates. |
| H-15 | `Row.line` dead in the featureless lib build until T4/T5. | F-4 | `Keys::line` (B-3). |
| H-16 | `with_chords` (test-only) builds rows with `line: None`, i.e. "default": tests that need a **user** row must go through `load_str`. | `keys/mod.rs:82-102` | T4's tests all use `load_str`. |
| H-17 | `STATE_GUARDED` literal rule (D8.5) refuses a user who only extends one of the pair. | F-7 | Built literally (B-9); §14 PA-2. |
| H-18 | Windows: the `cfg(not(unix))` test and `cfg!(unix)` branch never run in the sandbox. | T2 | Optional: `cargo check -p htui --tests --target x86_64-pc-windows-gnu` with `CC`/`AR` set to the host gcc/ar (memory: windows-cross-check-needs-cc-env). |
| H-19 | `htui-store`'s `identity` doc says `config_root` is the config root "in production, a temporary directory under test"; `keys` reads `config_root_path` without a root override. | `identity.rs` `cache_dir` doc | Tests pass `root` to `resolve` directly (T3 test 19) or set `XDG_CONFIG_HOME` on a child; `lib.rs` is the only production caller. |
| H-20 | Snapshot drift. | plan Risks | None expected: no default, view or `testkit` change. `cargo insta test -p htui --all-features --check` in the close gate; zero `.snap.new`. |

---

## 14. Plan amendments

Each needs the maintainer's word before it replaces the confirmed text. Until then the
implementer builds what the "Meanwhile" column says.

| # | Decision | Why the text cannot stand as written | Evidence | Proposed text | Meanwhile |
|---|---|---|---|---|---|
| **PA-1** | D11 (README sentence) | Its example `[overlay] close = ["x"]` never loads: `overlay.close` is an `in_capture` action, and D8 step 6 refuses a printable chord on one. | `catalogue.rs:324` `capture_row(Act::OverlayClose, …)`; `catalogue.rs:706` `only_the_form_and_overlay_close_are_in_capture`; plan D8 item 6. | "… an overlay that handles `Esc` itself still closes on it after `[overlay] close = ["f2"]`." | README uses `f2` (§12). No code impact. |
| **PA-2** | D8 step 5, allow-list rule | "Allowed only if both rows are defaults" turns the existing, reviewed `back`/`dismiss` `esc` share into an error as soon as the user adds any chord to either action (`[common] back = ["esc", "backspace"]`), an error they did not create and can only fix by changing `esc`. | `catalogue.rs:435` `STATE_GUARDED = [(Back, Dismiss)]`, both default `esc` (`catalogue.rs` `[common]` block). | "A pair is allowed for a chord only if the pair is on `STATE_GUARDED` and the chord is a catalogue default of **both** actions; a chord the user adds to either side is checked like any other." | Built literally (B-9; T4 test 9 pins the literal outcome). If accepted: in `report`, replace `first.line.is_none() && second.line.is_none()` with "`chord` is in both acts' `Keys::compiled()` chords", and flip T4 test 9's first case to `Ok` (its second case stays an error). |
| **PA-3** | D7 `Row.line` / D10 round-trip test | Read literally ("the file line that set it", derived equality), D10's two tests fail: the default print sets every row from a file line, so it neither equals the defaults nor prints without `(changed)`, and a changed table's printed line numbers differ from its source's. | `keys/mod.rs:19`, `:27` `#[derive(… PartialEq, Eq)]`; §4 (every action is printed). | "`line` is the file line of an entry whose list differs from the default (`None` otherwise); equality of `Keys` compares bindings, not lines." | Built this way (B-1, B-2): it is a reading of "`None` for a default", not a new behaviour, and it makes D10's tests pass as the plan words them. Listed here so the maintainer sees the interpretation. |

No confirmed decision is otherwise unworkable. D1-D6, D9 and D10 are built as written; F-11's
signature change (`load_str(src)` without `file_name`, `root: Option<&Path>`) is a refinement of
Task 3's sketch, not of a decision.

---

## 15. Blueprint decisions

| # | Decision | Reason |
|---|---|---|
| B-1 | `Row.line = Some(key line)` only when the merged list differs from the catalogue default, order-sensitive. | F-1; a restated default is not a change (print, validation). |
| B-2 | `Row`'s `PartialEq`/`Eq` ignore `line`. | F-1; D10's round trip as worded. |
| B-3 | `pub fn Keys::line(&self, Context, Act) -> Option<usize>`. | F-4; also the natural API for M3+'s `?` box. |
| B-4 | `--print-keys` writes through `print_keys`, which treats `BrokenPipe` as success. | F-5. |
| B-5 | A `CtrlCapital` whose suggestion is `ctrl-c` reports K1, not the capital hint. | F-8: never suggest a spelling the next check refuses. |
| B-6 | `load_str(src)`; `resolve(Option<&Path>, bool, Option<&Path>)`. | F-11. |
| B-7 | `KeysError::source()` is `None`; the io error is in `Display`, the hint is last. | `main` prints `{error:#}`; no duplicate text, hint stays the last line. |
| B-8 | An entry merges the chords that passed every per-chord check, even when another element failed; O1 fires only for a written `[]`. | Fewer cascaded collision errors; one error for `close = ["ctrl-c"]`. |
| B-9 | Allow-list rule exactly as D8.5 (both rows default, pair on `STATE_GUARDED`). | Confirmed text; PA-2 offers the alternative. |
| B-10 | A collision is reported on the row with the later line (`None < Some`), the second of the pair on a tie, once per unordered pair and chord across all checks. | D8.5 "reported on the user's line"; dedup between the context and stack passes (T4 test 7). |
| B-11 | Entry-level errors (U2, W1, O1, V1, T1/T2) report the key's line; chord-level ones (W2, C1, K1, D1) the element's line; U1 the table key's line; S1 the parser's span. | Multi-line arrays point at the offending element. |
| B-12 | An inline-table value is a sub-context (`global = { … }` = `[global]`; `quit = { … }` under `[global]` = `[global.quit]`). | F-10; one rule for every `DeValue::Table`. |
| B-13 | `--print-keys`' header is constant (no run-specific path). | Machine-independent output; the binary test compares with `keys::print`. |
| B-14 | `KeyFileError` has no `Display`; only `KeysError` renders, adding the file name. | F-11; one renderer for §7.5. |

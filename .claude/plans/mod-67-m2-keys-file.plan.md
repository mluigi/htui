# Plan: MOD-67 M2 — the key file (`keys.toml`)

**Source**: `HANDOFF.md` MOD-67; spec `docs/ANA-26.md` §6.3-§6.4, §7.1, §7.4, §7.5, §8 (M2 row);
the M1 deferral in `.claude/plans/mod-67-m1-catalogue-resolver.plan.md` "Review gate"
**Routing**: routed as **plan** by maintainer override (2026-10-07): the rule gave PRD (C2 + C4),
but ANA-26 §6-§9 already is the requirements document (same call as M1). Ultracode for
review-finding verification only; `rust-reviewer` stays the gate. Implement: not needed (~10
files, mostly a serial chain).
**Selected milestone**: M2 only. M3-M6 are later runs.
**Complexity**: Medium (~11 files, two new modules, no snapshot churn intended)
**Status**: confirmed by the maintainer (2026-10-07); implementing

## Summary

htui reads `<config_root>/keys.toml` (or `--keys PATH`) before the backend starts, parses it with
spans, checks names and chords strictly, merges it over the compiled catalogue, validates the
result over every context and every declared stack, and either hands the `Keys` to `App` or
refuses to start with every error as `keys.toml:LINE: …` and exit status 2, kept out of Sentry.
`--default-keys` skips the file and `--print-keys` prints the resolved table as commented TOML
(or the errors). The strict parser gains the Unix legacy-encoding rejects M1 deferred. Global and
overlay keys become configurable end to end. No default changes; no view file is touched.

## Decisions (proposed; confirmed at the CONFIRM gate)

- **D1 — File walk.** Parse with `toml::de::DeTable::parse` (spans on every key and value;
  verified claim 1). A TOML table whose values are chords is a context; a nested table extends the
  context path with `.` (`[settings.boxes]` → `settings.boxes`), so M3-M5's view contexts need no
  loader change. A value is one chord string or an array of chord strings; anything else is an
  error. The top-level `version` key is the only non-table key allowed.
- **D2 — `version`.** Optional. When present it must be the integer `1`; any other value is an
  error naming the supported version. (ANA §7.1 writes it in the example; making it mandatory
  would refuse a two-line file for no gain.)
- **D3 — Which file.** `--default-keys` → no file. `--keys PATH` → that file; missing or unreadable
  is an error (exit 2), since the user named it. Otherwise `config_root_path()/keys.toml`: missing
  → the defaults, silently; present but unreadable → error. No config directory on the platform →
  the defaults (keys are not where that failure should surface; `connect::start` reports it).
  `htui-store` gains `identity::config_root_path()`, non-creating; `config_root()` calls it and
  then creates the directory. `crates/htui` does not take `dirs`.
- **D4 — Flags.** `--keys <PATH>`, `--default-keys` (conflict with each other), `--print-keys`.
  All three conflict with `--set-dsn`, `--clear-dsn`, `--index-items`, `--search-items` (those exit
  before keys are read, so the flag would be silently ignored); `--print-keys` also conflicts with
  `--dsn-stdin`, `--demo` and `--offline` (it never starts the TUI). None reads the environment
  (`no_tui_argument_but_log_reads_the_environment` stays green). `--demo` reads the file (ANA §6.3).
- **D5 — Load point.** In `lib::run`, after the concepts early exits and before `connect::start`
  (and before `--demo`'s `Started::detached`); `--print-keys` exits right there. `App::new` keeps
  its signature: `lib.rs` sets `app.keys = keys` (the field is `pub`, verified claim 6) before
  `register_all`, so every `Ctx` built by `App::ctx` carries them (`with_keys(&self.keys)`).
  `testkit` keeps the compiled defaults (ANA §7.4).
- **D6 — The Unix legacy rejects (M1 deferral).** crossterm 0.29's Unix parser (verified claim 3)
  and htui never pushing keyboard-enhancement flags (claim 4) mean these arrive as another key:
  - **Unconditional errors** (like M1's `ctrl-i`/`ctrl-m`/`ctrl-[`/`ctrl-\`…): `ctrl-2` and
    `ctrl-@` → `ctrl-space`; `ctrl-3` → `esc`; `ctrl-8` and `ctrl-?` → `backspace`; `ctrl-/` →
    `ctrl-7`. Same `ChordError::Indistinguishable` shape.
  - **`cfg(unix)` errors** (a new `ChordError::NotDelivered`): any other `ctrl-` with a character
    that is not `a`-`z`, space or `4`-`7` (`ctrl-1`, `ctrl-0`, `ctrl-9`, `ctrl-.`, `ctrl-,`,
    `ctrl-;`, `ctrl-'`, …), and `ctrl-`/`shift-` on `enter`, `tab`, `backspace` and `esc`
    (`shift-tab` stays `backtab`). These never reach htui in a Unix terminal, so a binding to one
    is a silently dead action, which is what the validator exists to prevent; Windows delivers
    them, so they stay legal there. A warning was rejected: before the alternate screen nothing on
    stderr survives, and `--print-keys` is not run on every start.
  `parse` (lenient, test harness) is unchanged.
- **D7 — Lookup and merge.** `(table, name)` resolves exactly against the catalogue
  (`Context::table()` + `ActionSpec::name`). An unknown table or name is an error that lists the
  table's valid names (for an unknown table, the valid tables). The file's list **replaces** that
  action's defaults; `[]` unbinds. The **narrower-context override** of a shared verb (ANA §7.4
  step 5, `[settings.boxes] reload = …`) needs a view context, and M2 has none: it lands with M3's
  first view context. `Keys`' `Row` gains `line: Option<usize>` (the file line of an entry whose list
  differs from the default, `None` otherwise; `Keys` equality compares bindings, not lines: PA-3): validation and `--print-keys` both read it.
- **D8 — Checks (ANA §7.4 steps 3-7), all collected, none short-circuiting:**
  1. TOML syntax (one error, with the parser's span → line; nothing else is checked after it);
  2. `version`; unknown table; unknown name; a value that is not a chord or a list of chords;
  3. every chord strictly (`parse_strict`, D6); a chord listed twice in one action;
  4. any `ctrl-c` (refused: "ctrl-c always quits and cannot be bound"); `[overlay] close = []`
     (refused: "must keep at least one chord: overlays swallow every other key");
  5. collisions: two actions sharing a chord **in one context** (every context, including the
     shared ones no view consumes yet), and two candidates for one chord **in any declared stack**
     (`stack::DECLARED`, new: `Stack::BASE` "on every screen", `Stack::OVERLAY` "over an overlay";
     M3-M5 append theirs). A pair is allowed for a chord only if the pair is on `STATE_GUARDED` and the
     chord is a catalogue default of **both** actions (amended by PA-2); a chord the user adds to
     either side is checked like any other, so a pair the user creates is always an error, reported
     on the user's line. The shadowing allow-list kind (ANA §7.4) lands with M3's first form stack; M2 has no
     stack that needs it;
  6. an `in_capture` action bound to a printable chord.
  A unit test asserts the compiled defaults pass 2-6 with no error.
- **D9 — `KeysError` and exit 2.** `keys::KeysError { path, errors: Vec<KeyFileError { line,
  message }> }` (plus `Unreadable { path, source }`). `Display` is the ANA §7.5 report: header
  `<path> has N error(s):`, one `  <file name>:<line>: [table] name = "spec": reason` per error
  sorted by line, then `Fix the file, or run \`htui --default-keys\` to start with the default
  keys.` `main.rs` prints `htui: {error:#}` as today, `exit_code` downcasts `KeysError` → 2, and
  `reports_to_sentry` refuses it explicitly (not only through `code != 2`): the report carries the
  user's home path.
- **D10 — `--print-keys` format.** stdout, one complete TOML file that the loader reads back to the
  same `Keys` (a round-trip test pins it): a header comment (where the file lives, "list only what
  you change", "ctrl-c always quits"), `version = 1`, then one `[table]` per context in catalogue
  order, one line per action, `name = ["spec", …]` with the `KeyChord::spec` spelling, padded, then
  `# help`; a line the file changed ends `# help (changed)`; an unbound action prints `name = []`.
  With an invalid file it prints nothing on stdout and fails with the `KeysError` (exit 2).
- **D11 — Known limit, stated, not fixed in M2.** `App::on_key` tries the overlay, then the tab,
  then the global layer. A chord a view or an overlay still matches on `KeyCode` (all of them until
  M3-M5) is taken there first, so `[global] quit = ["e"]` is eaten by Settings' `e`, and an overlay
  that handles `Esc` itself still closes on it after `[overlay] close = ["f2"]` (PA-1: `x` is refused, `overlay.close` is `in_capture`). The validator cannot
  see hand-written arms; M3-M5 remove them. README says so in one sentence.

## Amendments (maintainer, 2026-10-07, from blueprint §14)

- **PA-1** (D11): the README example is `[overlay] close = ["f2"]`; `["x"]` never loads.
- **PA-2** (D8 step 5): the allow-list is per chord: a `STATE_GUARDED` pair may share a chord only
  when it is a catalogue default of both actions. Replaces "both rows are defaults".
- **PA-3** (D7, D10): `line` is set only when the list differs from the default; `Row` equality
  ignores `line`.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Typed exit error | `crates/htui/src/provision/mod.rs:43` `ProvisionExit`, `main.rs` `exit_code`/`reports_to_sentry` | enum with `code()`; `main` downcasts; refusals (2) stay out of Sentry |
| Early-exit flag | `lib.rs` `--set-dsn`/`--index-items` branches | print to the shell, return before `connect::start` and `terminal::init` |
| CLI flag + conflicts | `cli.rs` `dsn_stdin` (`conflicts_with_all`), tests `the_tui_dsn_stdin_flag_parses_alone…` | one `#[arg(long, conflicts_with_all = […])]`, a parse/refuse test table |
| Non-creating path | `htui-store/src/identity.rs:50` `config_root` | `dirs::config_dir()…join("htui")`, `StoreError::Backend` |
| Strict chord errors | `keys/chord.rs` `refuse_char`, `legacy_arrival`, `ChordError` | one variant per reason, `Display` is the tail of a `keys.toml:LINE:` line |
| Private rows, child module | `keys/mod.rs` `Row`, `keys/stack.rs` `impl Keys` | new files are children of `keys`, so they reach `Keys::rows` without widening visibility |
| Temp config root under test | `htui-store/src/connect.rs:207-210` (`config_root` is a temp dir under test) | tests pass a root; never the user's config |
| Binary under test | `tests/mcp_stdio.rs:26` `env!("CARGO_BIN_EXE_htui")` | spawn the built binary, assert status and stderr |
| Integration test gate | `tests/backlog.rs:6` `#![cfg(feature = "testkit")]` | the App-level cases need `testkit` |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-store/src/identity.rs` | UPDATE | `config_root_path()` (non-creating); `config_root` uses it |
| `crates/htui/src/keys/chord.rs` | UPDATE | D6 rejects + tests |
| `crates/htui/src/keys/load.rs` | CREATE | D1-D3, D7, D8 1-4: file → `Keys` with lines, `KeysError` |
| `crates/htui/src/keys/validate.rs` | CREATE | D8 5-6 over contexts and `DECLARED` stacks |
| `crates/htui/src/keys/print.rs` | CREATE | D10 |
| `crates/htui/src/keys/mod.rs` | UPDATE | `Row.line`, `pub mod load/validate/print`, re-exports |
| `crates/htui/src/keys/stack.rs` | UPDATE | `DECLARED` (name + stack) |
| `crates/htui/src/cli.rs` | UPDATE | three flags, conflict tests |
| `crates/htui/src/lib.rs` | UPDATE | D5 load point, `--print-keys` exit, `app.keys = …` |
| `crates/htui/src/main.rs` | UPDATE | D9 exit 2, kept out of Sentry; doc |
| `crates/htui/Cargo.toml`, `Cargo.lock` | UPDATE | `toml = { workspace = true }` (already in the lock, 1.1.5: an edge, no new crate) |
| `crates/htui/tests/keys_file.rs`, `crates/htui/tests/fixtures/keys/*.toml` | CREATE | integration tests over fixture files |
| `README.md` | UPDATE | the three flags in the flag table; a short "Keys" paragraph (path, format, D11 limit) |

## Tasks

TDD per task: tests first, red, then code.

### Task 1: `config_root_path()` (htui-store) — independent
- **Files**: `crates/htui-store/src/identity.rs`
- **Action**: split `config_root` into `config_root_path()` (no I/O) and the creating wrapper.
- **Validate**: `cargo test -p htui-store --lib identity`

### Task 2: Unix legacy rejects (D6) — independent
- **Files**: `crates/htui/src/keys/chord.rs`
- **Action**: extend `legacy_arrival`; add `ChordError::NotDelivered` behind `cfg(unix)` checks;
  tests split by `cfg`. Defaults still parse (`every_catalogue_default_parses_strictly`).
- **Validate**: `cargo test -p htui --lib keys::chord`

### Task 3: loader (D1-D3, D7, D8 1-4) — after Task 2
- **Files**: `keys/load.rs`, `keys/mod.rs` (`Row.line`, module wiring), `crates/htui/Cargo.toml`,
  `Cargo.lock`
- **Action**: `load_str(src, file_name) -> Result<Keys, Vec<KeyFileError>>`,
  `load_path(path) -> Result<Keys, KeysError>`, `resolve(keys_flag, default_keys, root:
  Option<PathBuf>) -> Result<Keys, KeysError>`; `KeysError` and its `Display`.
- **Validate**: `cargo test -p htui --lib keys::load`

### Task 4: validator (D8 5-6) — after Task 3
- **Files**: `keys/validate.rs`, `keys/stack.rs` (`DECLARED`), `keys/load.rs` (calls it)
- **Action**: collisions per context and per declared stack with the default-only allow-list
  rule; `in_capture` printable check; the compiled-defaults-pass test.
- **Validate**: `cargo test -p htui --lib keys::validate`

### Task 5: `--print-keys` (D10) — after Task 3
- **Files**: `keys/print.rs`
- **Action**: renderer + round-trip test (`load_str(print(keys)) == keys` for defaults and for a
  changed table) + a test that the default print loads with zero `(changed)` marks.
- **Validate**: `cargo test -p htui --lib keys::print`

### Task 6: CLI, startup, exit code (D4, D5, D9) — after Tasks 1, 3-5
- **Files**: `cli.rs`, `lib.rs`, `main.rs`
- **Action**: flags + conflict tests; load point; `app.keys = keys`; `exit_code`/`reports_to_sentry`
  arms + tests.
- **Validate**: `cargo test -p htui --lib cli`, `cargo test -p htui --bin htui`

### Task 7: integration tests over fixtures — after Task 6
- **Files**: `tests/keys_file.rs`, `tests/fixtures/keys/{valid,errors,ctrl_c,overlay_close,
  collision,syntax}.toml`
- **Action**: binary cases (`--keys F --print-keys`: exit 0 + TOML / exit 2 + the exact §7.5 report
  on stderr; `--keys missing.toml` → 2; `--keys F --default-keys` refused by clap); an App-level
  case under `testkit`: a harness with `app.keys` from `valid.toml` quits on the rebound chord and no
  longer on `q`, and the status line and `?` box show the new chord. Linux-only case for the default
  path (`XDG_CONFIG_HOME` on the child process).
- **Validate**: `cargo test -p htui --features testkit --test keys_file`

### Task 8: docs
- **Files**: `README.md`
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`

Tasks 1 and 2 have disjoint file sets ({identity.rs} ∩ {chord.rs} = ∅) and may run in parallel;
everything after is a chain through `keys/` (shared `load.rs`/`mod.rs`), so it runs serial.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate (memory: htui-featureless-clippy-gate)
cargo test -p htui --all-features -- --test-threads=1
cargo test -p htui-store --lib identity
cargo insta test -p htui --all-features --check   # no snapshot may change
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A rebound global/overlay chord is eaten by a not-yet-converted view (D11) | High (until M5) | Stated in README; M3-M5 convert the views; the validator walks every declared stack as they land |
| The `cfg(unix)` rejects refuse a chord some Unix terminal does deliver (kitty protocol) | Low | htui does not enable the protocol (claim 4); revisit if it ever pushes enhancement flags |
| `DeTable` iteration order is not file order (claim 1: sorted) | Certain | errors are sorted by line before printing; nothing depends on iteration order |
| A user's file breaks a scripted `htui --index-items` | None | D4: those flags exit before keys load and refuse the key flags |
| Snapshot drift | Low | no default and no view changes; `insta --check` is a gate |

## Acceptance

- [ ] All tasks complete, TDD order kept
- [ ] Validation passes; no snapshot changed
- [ ] A broken file exits 2 with every error as `keys.toml:LINE:` and reaches no Sentry
- [ ] `--print-keys` output round-trips through the loader
- [ ] Patterns mirrored, not reinvented

## Verified claims (plan fact-check, 2026-10-07)

| # | claim | verdict | evidence |
|---|---|---|---|
| 1 | `toml` 1.1.5 exposes `toml::de::DeTable::parse` with spans on keys and values, nested tables for `[a.b]`, a duplicate-key error with a span; iteration is sorted, not file order | ✓ | compile probe `/tmp/tomlprobe` against `toml = "=1.1.5"`: `global @13..19`, `settings`→`boxes`, `dup: Some(20..24) duplicate key`; printed order global, settings, version |
| 2 | `toml` 1.1.5 and `serde` are already in the lock / in `htui`'s deps; only an edge is added | ✓ | `Cargo.lock:6758-6759`; workspace `Cargo.toml:96` `toml = "1.1"`; `htui-store/Cargo.toml` `toml = { workspace = true }`; `crates/htui/Cargo.toml` has `serde`, no `toml` |
| 3 | crossterm 0.29 Unix: `0x00` → `ctrl-space`, `0x1C..0x1F` → `ctrl-4..7`, `0x7F` → `backspace`, `\t` → `tab`, `\r` → `enter`, `0x1B` → `esc` | ✓ | `crossterm-0.29.0/src/event/sys/unix/parse.rs:92-118` |
| 4 | htui never pushes keyboard-enhancement flags (legacy encoding only) | ✓ | text search `KeyboardEnhancement` over the repo: 0 hits |
| 5 | `identity::config_root()` creates the directory | ✓ | `htui-store/src/identity.rs:50-57` `create_dir_all` |
| 6 | `App.keys` is a `pub Keys` field, `App::new` fills it from `Keys::compiled()`, and `App::ctx`/`render`/`on_key` read `self.keys` | ✓ | `app/state.rs:174`, `:265`, `:829` `.with_keys(&self.keys)`, `:862`, `:763` `apply_keys(Stack::OVERLAY, …)` |
| 7 | `lib::run` early exits (`mcp`, `worker`, `provision`, `--set-dsn`, `--clear-dsn`, concepts) all precede `connect::start` / `--demo` | ✓ | `lib.rs` `run` body |
| 8 | `main.rs` maps unknown errors to 1 and reports every code-≠2 error to Sentry; downcast pattern exists | ✓ | `main.rs` `exit_code`, `reports_to_sentry` |
| 9 | No test fixtures directory exists yet; integration tests spawn the binary via `CARGO_BIN_EXE_htui` and gate on `testkit` | ✓ | `tests/fixtures` absent; `tests/mcp_stdio.rs:26`; `tests/backlog.rs:6` |
| 10 | No catalogue default is refused by D6 | ✓ | `catalogue.rs` defaults: ctrl chords are `ctrl-f/w/s/e` only; no ctrl/shift on enter/tab/backspace/esc |
| 11 | `Keys::rows`/`Row` are private to `keys`; child modules can read them | ✓ | `keys/mod.rs` `struct Row`, `stack.rs` `impl Keys { fn actions … self.rows }` |
| 12 | `Ctx::new` uses `Keys::compiled()`, so testkit stays on defaults | ✓ | `app/state.rs:116` |
| 13 | Tasks 1 and 2 are independent | ✓ | file sets {`htui-store/src/identity.rs`} ∩ {`htui/src/keys/chord.rs`} = ∅ |

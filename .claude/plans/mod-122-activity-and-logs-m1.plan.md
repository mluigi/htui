# Plan: MOD-122 M1 — Logs tab and logging settings

**Source PRD**: `.claude/prds/mod-122-activity-and-logs.prd.md`
**Selected Milestone**: 1 — Logs tab and logging settings
**Complexity**: Medium
**Status**: implemented on `hr/MOD-122` (`96cc49ba`..`83dd23c0`, 2026-11-05), review round R1 applied; changed decisions recorded in the blueprint's R1 amendments.
**Routing**: PRD path by `/handoff-run MOD-122`; ultracode accepted for the implement phase.
**Grounded at**: `1c50e30c` (branch `hr/MOD-122`). **Blueprint**: `.claude/plans/mod-122-activity-and-logs-m1.blueprint.md` (its F1–F8 amend this plan; the blueprint wins where they differ).

## Summary

htui always installs one `tracing` subscriber in the TUI process: a global level filter that can be
reloaded at run time, an in-memory ring buffer layer that every passing event lands in, and a file
layer whose target can be switched on, off or moved at run time. A new **Logs** tab reads the buffer:
sources with line and warning counts on the left, the selected source's lines on the right,
coloured by level, with a minimum level, follow, and scroll-up-pauses-follow. A new **Logging**
section in Settings edits the filter and the log file; both apply at once and are saved in
`ui-state.toml`. Requirements: `R-TUI-15`, `R-TUI-8`; PRD milestone 1.

## Decisions

- **D1 — one subscriber, always.** `init_tracing` stops returning early without `--log`. It installs
  `registry` + `reload::Layer<EnvFilter>` (global) + `MemoryLayer` (the buffer) + a `fmt` layer
  writing through a switchable writer (no file = writes dropped). Probe-verified (claim V1). The
  Sentry layer is installed **only when a log file was given at startup (`--log`/`HTUI_LOG`)**, as
  today: widening what reaches Sentry is MOD-99's call (the hard-coded DSN, unscrubbed events), not
  this item's. `htui mcp` still installs nothing (stdout is the protocol); `htui worker` keeps its
  own subscriber — its log reaches the main instance in M2. *Maintainer check at CONFIRM.*
- **D2 — the buffer.** A bounded ring of **10 000 lines** (a constant, not a setting, in M1),
  oldest dropped first. A line keeps: sequence number, wall-clock time, level, target, source, and
  the message with its fields formatted on one line. Per-source counts (lines, warnings = `WARN` +
  `ERROR`) are counts of **retained** lines, so a count always matches what the list can show. The
  buffer sits behind a `Mutex` held only to push or copy; the tab copies lines newer than the last
  sequence it saw. *(PRD open question "in-memory cap" answered: constant, 10 000.)*
- **D3 — sources by target.** A fixed table maps a module-path target to a source, longest prefix
  first:
  - **secrets**: `htui_secrets`, `htui_store::secret`, `htui::secrets`, `htui::secrets_settings`
  - **store**: `htui_store`, `htui::store_worker`, `sqlx`
  - **orchestrator**: `htui_orch`
  - **agents**: `htui_agent`, `htui::agent_worker`
  - **mcp**: `htui_mcp`, `htui::mcp_search`, `htui::mcp_cmd`
  - **worker**: `htui_worker` (this also covers the TUI's in-process run runtime), `htui::run_worker`,
    `htui::worker_cmd`
  - **ui**: any other `htui::…` target
  - **other**: anything left, mostly third-party crates. Shown only when it has lines.

  The list shows `all` first, then the sources in the requirement's order.
- **D4 — the tab.** `TabId("logs")`, title "Logs", `Panes::ListDetail(LOGS_LAYOUT)` mirroring
  Requirements. Registered **last**, after Chat (digit `6`), so no existing tab's digit moves. The
  accent and monochrome style already exist (`Theme::tab_accent("logs")`); `about_for("logs")` gets
  its text. Keys go in a new `logs` context, each one configurable in `keys.toml`:
  - `logs.level` `v` cycles the minimum level `trace → debug → info → warn → error`.
  - `logs.follow` `F` toggles follow.
  - `logs.bottom` `G` goes to the newest line and resumes follow, mirroring `chat.bottom`.
  - Any step up (`k`, `↑`, `PgUp`, the wheel) pauses follow.

  New lines are pulled in `on_refresh` (once a second, active tab only). Counts render as
  `name  lines · ⚠ warnings`. Level colours: `error` → `theme.error`, `warn` → `theme.warning`,
  `info` → base, `debug`/`trace` → `theme.dim`. Under monochrome, the level word itself is the
  signal, so colour is never the only cue. The detail border shows the active minimum level and
  `following` / `paused`. History `location`/`restore` keep the selected source.
- **D5 — the settings section.** A Logging section, appended last in Settings with the existing
  form pattern (`queue.rs`). It has two fields:
  - **Filter**: an `EnvFilter` directive, default `info`. It is validated with `EnvFilter::try_new`
    before it applies, and a refused directive says why and changes nothing.
  - **Log file**: a path; empty means no file.

  A saved edit emits `Action::Logging(..)`. `App` applies it through the `LoggingControl` handle and
  saves it in a new `[logging]` table of `ui-state.toml` through the existing latest-wins writer.
  The table is optional and older htui ignores it, so the version stays 1 (claim V7).

  At startup the saved values are applied once `ui-state.toml` is read. `--log`/`HTUI_LOG` and
  `HTUI_LOG_FILTER` override them for that session, and the section shows "set by `--log`" or "set
  by `HTUI_LOG_FILTER`" beside an overridden field. A log file that cannot be opened is said in the
  section and on the status line, and the previous file stays.
- **D6 — never log secrets.** M1 adds log calls only for setting changes. Each one logs the filter
  directive or the file path, never a value read from the keyring or the environment.
- **D7 — handles, not globals.** `init_tracing` returns `Logging { buffer: LogBuffer, control:
  LoggingControl }`. `App` holds it (`App.logging`), and `register_all` passes the buffer to
  `LogsTab::new` and the control to `LoggingSection::new`. Tests build a standalone
  `LogBuffer`/`MemoryLayer` under `tracing::subscriber::with_default`, so nothing process-global
  enters a test (memory: the suite's green depends on scheduling).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| List + detail tab | `crates/htui/src/ui/tabs/requirements/mod.rs:66` (`REQUIREMENTS_LAYOUT`), `:1278` (`impl Tab`), `:1455` (`ctx.split`), `:1527`/`:1550` (`location`/`restore`) | Layout const, split per frame, history location by cursor |
| Tab trait | `crates/htui/src/ui/tabs/registry.rs:39`, `on_refresh` `:117`, `panes` `:149` | Required methods; per-second pull for the active tab |
| Follow/tail | `crates/htui/src/ui/tabs/chat/transcript.rs:290-292, 588-613` | `Scroll::Tail`; a step up leaves the tail; bottom re-arms it |
| Keys | `crates/htui/src/keys/catalogue.rs` (`row(Act::ChatBottom, Chat, "bottom", &["G"], …)` `:1990`; `Context` `:61`); stacks `keys/views.rs` (`CHAT_LIVE` `:988`, `DELEGATES` `:1230-1310`) | New context + stack: view layer, then LIST/LAYOUT/HISTORY/GLOBAL |
| Settings section | `crates/htui/src/ui/tabs/settings/queue.rs`; trait `settings/mod.rs:136`; bench `testkit.rs:741` (`SectionBench`) | `kit::form` rows, `TextField` editor, emits through `Ctx` |
| UI-state persistence | `crates/htui/src/ui/state_file.rs` (`UiState` `:46`, `FileDto` `:57`, `VERSION` `:42`); `App::save_ui_state` `app/state.rs:644` | Optional table, latest-wins background writer |
| Logging | `crates/htui/src/lib.rs:359-381` (`init_tracing`); `worker_cmd.rs:281` | `HTUI_LOG_FILTER`, default `info`, never stdout, no ANSI in files |
| Errors | `settings/mod.rs` `is_error`/notices; `anyhow` at startup | A refused value is a one-line notice, nothing changes |
| Tests | `testkit::Harness` (`with_tab` `:538`, `key` `:642`, `render` `:674`); insta `snapshots/htui__…__tests__<name>.snap`; subscriber in tests `concepts_worker.rs:1736` (`with_default`) | Harness render + snapshot; scoped subscriber per test |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/logging/mod.rs` | CREATE | T1 | `Logging`, `LoggingControl` (reload handle + switchable writer), `init` |
| `crates/htui/src/logging/buffer.rs` | CREATE | T1 | `LogBuffer`, `LogLine`, ring, counts, `MemoryLayer` |
| `crates/htui/src/logging/source.rs` | CREATE | T1 | `Source` enum and the target table (D3) |
| `crates/htui/src/lib.rs` | UPDATE | T1 | `init_tracing` → `logging::init`; pass `Logging` to `App`; apply saved settings after `ui-state.toml` loads; `mod logging` |
| `crates/htui/src/app/state.rs` | UPDATE | T1 | `App.logging` field |
| `crates/htui/src/app/action.rs` | UPDATE | T1 | `Action::Logging(LoggingAction)` |
| `crates/htui/src/app/update.rs` | UPDATE | T1 | apply + save on `Action::Logging` |
| `crates/htui/src/ui/state_file.rs` | UPDATE | T1 | `[logging]` table (`filter`, `file`), parse/render + tests |
| `crates/htui/src/ui/tabs/logs/mod.rs` | CREATE | T2 | `LogsTab` |
| `crates/htui/src/ui/tabs/logs/snapshots/*` | CREATE | T2 | render snapshots |
| `crates/htui/src/ui/tabs/mod.rs` | UPDATE | T2 | export `LogsTab` |
| `crates/htui/src/keys/catalogue.rs` | UPDATE | T2 | `Context::Logs`, `Act::Logs*` rows |
| `crates/htui/src/keys/views.rs` | UPDATE | T2 | `LOGS` stack, `DELEGATES` |
| `crates/htui/src/ui/about.rs` | UPDATE | T2 | `"logs"` about text; "reserved" doc/tests |
| `crates/htui/tests/integration.rs` | UPDATE | T2 | tab count 5 → 6 |
| `docs/htui-keys.md` | UPDATE | T2 | regenerated (`keys_file.rs:723-726` pins it) |
| `crates/htui/src/ui/tabs/settings/logging.rs` | CREATE | T3 | `LoggingSection` |
| `crates/htui/src/ui/tabs/settings/mod.rs` | UPDATE | T3 | `pub mod logging`, export |
| `crates/htui/src/app/mod.rs` | UPDATE | T2, T3 | register the tab (T2) and the section (T3), one line each |
| `README.md` | UPDATE | T2, T3 | tab table / `1`…`6` / key table (T2); `--log` row and Settings (T3) |
| `docs/htui-cli.md` | UPDATE | T3 | `--log` (`:55`, `:420`): memory always, file optional, Settings |

## Tasks

T1 runs first. T2 and T3 both depend on T1's API. Their file sets intersect on `app/mod.rs` and
`README.md` (claim V9), so the plan does **not** mark them independent. They run in **isolated
worktrees**, and the two one-line `register_all` additions plus the two README hunks are merged by
hand at collect. Running them in sequence is the fallback.

### Task 1: Capture and control (serial, first)
- **Action**: tests first.
  - Source mapping table: one case per row of D3, plus longest-prefix order (`htui_store::secret`
    maps to secrets, not store).
  - Ring cap: the oldest line drops at 10 001, and counts follow the drop.
  - Warning count: `WARN` + `ERROR` only.
  - `MemoryLayer` under `with_default`: message and fields on one line, target, level.
  - `LoggingControl`:
    - The filter reloads and a refused directive changes nothing.
    - The file is switched on, moved and switched off, the old file is closed, and a file that
      cannot be opened leaves the previous one.
  - `ui-state.toml`: the `[logging]` table round-trips, a file without it reads as no setting, and
    a bad filter in the file is a notice, not a refusal of the whole file.
  - `Action::Logging` applies and saves.

  Then implement `logging/`, swap `init_tracing`, apply startup precedence (flag/env over the saved
  values), keep Sentry gated on the startup `--log` (D1), and add the `App` field, the action and
  the `update` arm.
- **Mirror**: `init_tracing` shape and its doc comment style; `state_file.rs` DTO tables and
  parse-error notices; `with_default` tests.
- **Validate**: `cargo test -p htui --all-features logging:: state_file:: -- --test-threads=1`;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`.

### Task 2: Logs tab (after T1; worktree, parallel with T3)
- **Action**: tests first, through `testkit::Harness` with the tab built over a test `LogBuffer`
  that holds seeded lines.
  - Snapshots:
    - empty buffer (empty state with the next step)
    - mixed sources with counts
    - lines coloured by level
    - the minimum level hides lower lines and shows the level on the border
    - `following` vs `paused` on the border
    - narrow width
  - Key tests:
    - `v` cycles the level
    - a step up pauses follow
    - `G` resumes it and jumps to the newest line
    - `F` toggles follow
    - new lines arrive on `on_refresh` and keep the view at the bottom only while following
    - a source change keeps the follow state
    - `location`/`restore` returns to the source
  - Key tables: catalogue invariants (`every_act_has_exactly_one_row`,
    `names_are_unique_per_context`, `no_spec_is_shared_in_a_context_unless_state_guarded`,
    `context_tables_and_headings`) and the regenerated `docs/htui-keys.md`.

  Then implement the tab, the context and stack, the about text, registration last, and the
  integration count of 6.
- **Mirror**: Requirements tab structure; chat transcript tail logic; `CHAT_LIVE` stack shape.
- **Validate**: `cargo test -p htui --all-features ui::tabs::logs keys:: about:: -- --test-threads=1`;
  `cargo test -p htui --all-features --test keys_file --test integration`;
  `cargo insta test -p htui --all-features` (full run: memory, snapshot impact needs a full insta run).

### Task 3: Logging settings section (after T1; worktree, parallel with T2)
- **Action**: tests first, with `SectionBench`.
  - Renders both fields with help beneath and an "overridden by" note when set by flag or env.
  - Editing the filter to a valid directive emits `Action::Logging` with it.
  - An invalid directive shows the notice and emits nothing.
  - Clearing the file emits "no file".
  - Snapshots of the section.

  Then implement and register it last in Settings, and update the `--log` docs.
- **Mirror**: `queue.rs` form/editor; settings notices.
- **Validate**: `cargo test -p htui --all-features ui::tabs::settings::logging -- --test-threads=1`;
  `cargo insta test -p htui --all-features`.

## Validation

```bash
bash scripts/gate.sh                                   # the repo gate
cargo clippy --workspace -- -D warnings                # featureless (memory: --all-features hides dead code)
cargo test -p htui --all-features -- --test-threads=1  # memory: htui suite green is scheduling-dependent
cargo insta test -p htui --all-features                # full snapshot run, review every .snap.new
```
Manual check, on the host or in a PTY:
1. Start `htui --demo` without `--log`. Lines appear in the Logs tab.
2. Set Filter to `debug` in Settings › Logging. Debug lines start arriving.
3. Set a file. Lines land in that file.
4. Restart. Both settings persist.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A process-global subscriber leaks into tests and makes them order-dependent | Medium | D7: handles, `with_default` in tests; `init` only from `run` |
| The mutex on the hot logging path slows busy code (agent streams log per frame) | Low | Push is O(1) with no allocation beyond the line; the tab copies by sequence; measure if `agent_worker` debug floods |
| An `on_event` reentrancy deadlock (logging while holding the buffer lock) | Low | No `tracing` call inside the buffer code; lock scope is the push/copy only |
| Logs tab snapshots churn with timestamps | High | Lines in tests carry fixed times; render through a time formatter fed by the line |
| `G` already means `list.bottom` in the LIST layer | — | The logs view layer sits above LIST, as `chat.bottom` does; the catalogue's shared-spec test confirms it |
| Sentry behaviour changes by accident | Low | D1 keeps it gated on startup `--log`; a test pins that the layer is absent without it |
| T2/T3 merge on `app/mod.rs`/`README.md` | Certain | One line or one hunk each, merged by hand; the gate runs on the merged tree |

## Verified claims

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `tracing-subscriber` 0.3.23 allows a reloadable global `EnvFilter`, plus a custom layer and a `fmt` layer through a switchable `MakeWriter` | ✓ | Compile-and-run probe `/tmp/probe122` with exact versions `--offline`: a debug line was hidden at `info` and captured after reload to `debug`; the file switched on live |
| V2 | Workspace deps: `tracing` 0.1, `tracing-subscriber` 0.3 with `env-filter`; htui has `sentry-tracing`, `chrono` | ✓ | `Cargo.toml:78-79`; `crates/htui/Cargo.toml` (read) |
| V3 | No subscriber without `--log`; the Sentry layer only rides that path | ✓ | `crates/htui/src/lib.rs` `init_tracing` (read): `let Some(path) = path else { return Ok(()) }`, then `.with(sentry_tracing::layer())` |
| V4 | `sentry::init` runs on every start in `main.rs` `body`; MOD-99 owns its scope | ✓ | `crates/htui/src/main.rs:66-82` (read); `HANDOFF.md:679-690` |
| V5 | A `"logs"` accent and its monochrome style already exist | ✓ | `ui/theme.rs:251`, tests `:327`, `:468` |
| V6 | Integration test pins 5 tabs | ✓ | `crates/htui/tests/integration.rs:43-48` (read) |
| V7 | `ui-state.toml` is version 1 with optional tables; unknown keys ignored | ✓ / partly | `ui/state_file.rs:42-57` (read): optional, `skip_serializing_if` tables; "unknown keys ignored" from the grounding sweep, re-checked in T1's first test |
| V8 | Sections can emit actions through `Ctx` (`emit: &Emit`) | ✓ | `app/state.rs:88-124` (read) |
| V9 | T2 ∩ T3 = {`app/mod.rs`, `README.md`}; T1 ∩ T2 = ∅; T1 ∩ T3 = ∅ | ✗ **falsified by the blueprint (F1, F3)**, amended | Every Settings section owns a key context and browse stack, so T3 also writes `keys/catalogue.rs`, `keys/views.rs`, `keys/stack.rs`. Amended: T1 adds both new `Context` variants (`Logs`, `SettingsLogging`) up front; T2 ∩ T3 is then {`app/mod.rs`, `README.md`, `keys/views.rs`, `keys/stack.rs`} in disjoint hunks (blueprint F1/F3); T2/T3 stay worktree-parallel, merged by hand, `keys::` tests re-run on the merged tree |
| V13 | The validate lines pass two test filters to cargo | ✗ falsified (blueprint F4), amended | cargo takes one TESTNAME; use `cargo test -p htui --all-features --lib -- logging:: state_file:: --test-threads=1` |
| V10 | `v`, `F` unbound in the global, list, layout and history layers; `G` is `list.bottom` and `chat.bottom` shadows it in Chat | ✓ (sweep) | Grounding sweep: `keys/catalogue.rs:1069`, `:1990`; re-checked by the catalogue tests in T2 |
| V11 | `on_refresh` runs for the active tab only, every 4 ticks of 250 ms | ✓ (sweep) | `app/update.rs:24`, `:260-278`; `event_loop.rs:23` |
| V12 | `htui-secrets` crate has no `tracing` dependency today | ✓ (sweep) | Grounding sweep; the D3 table maps it anyway for later calls |

## Acceptance
- [ ] All tasks complete, TDD order
- [ ] Gate green on the merged tree; featureless clippy clean; full insta run reviewed
- [ ] Logs tab works without `--log`; Settings › Logging applies live and persists
- [ ] Sentry unchanged without `--log` (pinned)
- [ ] Patterns mirrored, not reinvented

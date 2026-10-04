# Plan: MOD-76 orchestrator carried risks after MOD-37

**Source**: HANDOFF `MOD-76` (from MOD-37, `docs/decisions/mod/mod-37.md` "Carried"; `R-ORCH-3`, `R-ORCH-8`, `R-TUI-4`)
**Routed**: plan path via `/handoff-run` (1 criterion fired: C3, the options R-32 and R-44 name), accepted by the
maintainer 2026-10-04. Sandbox run `hr/MOD-76`.
**Scope**: chosen by the maintainer per risk, 2026-10-04: R-32 closed as accepted, R-44 abbreviate the model, R-53
closed as accepted, R-55 fixed now.
**Complexity**: Small (one pure function in the Runs pane, one comparison in the worker runtime's parts cache, docs)
**Status**: done 2026-10-04 (`docs/decisions/mod/mod-76.md`)

## Summary

MOD-37 closed with four risks re-deferred. Two of them close with no code: R-32 (a crash-cut D93 park loses its
refusal reason) and R-53 (`ItemActions` is as of the last `Runs` reply). Two get small fixes: R-44 (the Runs pane
cuts `agent/model` at 43 columns), fixed by abbreviating the model id, and R-55 (`command_limits` is read once per
process), fixed by re-reading the limits wherever the repo map is already re-read and rebuilding the verifier
when they changed and no walk is live. No migration, no `.sqlx`, no store or engine change.

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: R-32 closes as accepted, no code.** The lost detail is the refusal reason of a D93 park whose sweep crashed
  between `interrupt_step` and `park_interrupted` (`engine.rs` `never_reset` → `settle_failed`). The recovered note
  already says so (`PARK_CUT_SHORT`, or `DIRTY_AT_START` for a dirty tree) and still lists every tree's path and
  `before_hash`. So nothing is lost for recovery, only the sentence explaining it. Keeping that sentence needs a
  `run_step` column (migration 0015 plus a cache migration, `.sqlx`, and the `interrupt_step` signature across two trait
  declarations and nine implementations) or a change to `gate_note`, which `settle_failed` and four tests match exactly. Either costs far
  more than a diagnostic in a two-write crash window is worth, and 0015 would race the migrations of branches in flight.
- **D2: R-53 closes as accepted, no code.** The engine re-checks every action with the same admission function
  (D184) and the pane re-reads after each action (D171). The only window left is between a `Runs` reply and the key
  press, and D184 already guards it.
- **D3: R-44, the model is abbreviated before it is fitted.** A pure `who(agent, model) -> String` replaces the
  `format!` in `tail` (`runs.rs` `tail`):
  1. A trailing `-YYYYMMDD` (a `-` then exactly eight ASCII digits) is dropped from the model:
     `claude-sonnet-4-5-20250929` → `claude-sonnet-4-5`.
  2. A model that starts with `{agent}-` loses that prefix, because the agent name in front already says it:
     agent `claude`, model `claude-sonnet-4-5` → `claude/sonnet-4-5`.
  3. Either step that would leave an empty model is skipped (the model is kept as it was).
  4. A missing agent or model is still `—`; the rules run only when both are present.
  The result is still fitted with `cells::fit(.., TAIL_WIDTH)`, so an id that is still too long is cut with `…` as
  today. The agent name is never dropped: agents are told apart by name, and `agent/model` keeps its shape.
  `claude/sonnet-4-5` is 17 columns, so it fits beside the widest indicator the snapshots show (`~36k ! `, 7) in the
  24-column tail exactly. Row height, the indent, and D197's columns are unchanged.
- **D4: R-55, the parts cache remembers the limits it built the verifier with.** `Built` gains
  `limits: Option<BTreeMap<String, u32>>`. In `singletons`, past the early return that serves cached parts on a
  non-`start_run` call (the path that already re-reads the repo map: a walking `StartRun`, and the worker's sweep),
  the limits are re-read with `command_limits`. When they differ from `built.limits`:
  - no walk of this process is live → `built.verifier = None`, so the existing block rebuilds it from the fresh
    limits;
  - a walk is live → the cached verifier stays and nothing is refused. Limits are a throughput setting, not a
    correctness one, and the next call with no walk live applies them. Two verifiers live at once would mean two
    `verify` semaphores, so the class could run more than its limit (verify.rs `ShellVerifier`), which is why the
    swap waits.
  The repo-map rebuild keeps its own D202 refusal (`REPOS_MOVED`) unchanged. A read failure is passed up like the
  reads beside it (D216), so a verifier is never cached from a failed read. The TUI applies an edit at its next
  `StartRun`, and `htui worker` at its next sweep with no walk live, with no restart either way.
- **D5: no editor is added.** R-55's trigger ("whoever adds an editor") had not arrived. D4 means a future editor
  needs nothing more than its write.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Runs pane cells | `crates/htui/src/ui/tabs/backlog/detail/runs.rs` `tail`, `gate`, `indicator` | small pure `fn(&RunStepSummary) -> String` helpers, fitted by `cells::fit` at the call site |
| Runs pane tests | same file, `every_step_row_fits_forty_three_columns`, `extremes()` | fixture steps with extreme values; assert per-cell text with `cell(from, to)` |
| Parts cache | `crates/htui-worker/src/runtime.rs` `singletons`, `Built` | rebuild-on-change behind `!self.any_live()`; reads passed up as `String` errors |
| Runtime tests | same file, the MOD-41 review R-1 tests around `isolator_builds` / `parts()` | a sweep or `StartRun` against `Backend`, then the cached parts read back through `parts()` |
| Errors | `command_limits` doc (D216) | a failed read is not the default; nothing cached from it |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | D3: `who` helper, `tail` uses it; unit tests + an `extremes()` case |
| `crates/htui-worker/src/runtime.rs` | UPDATE | D4: `Built.limits`, compare and rebuild in `singletons`; doc comments on `singletons` and `command_limits` (drop "R-55 … next process"); tests |
| `crates/htui-core/src/store/mem.rs` | UPDATE | D4 tests: a `#[cfg(feature = "test-support")]` `MemStore::set_box_setting(id, key, value)` seam, since no `BoxEdit` field writes `command_limits` (mirrors `relay_rows` / `command_rows`, via `self.write`) |
| `docs/htui-worker.md` | UPDATE | D4: lines 45-51, a limits change alone now reaches the worker at its next sweep with no walk live, not at restart |
| `docs/decisions/mod/mod-76.md`, `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-37.md` (pointer only if the law asks) | CREATE / UPDATE | close-out per `lifecycle.md` P2 |

## Tasks

T1 and T2 are independent (file sets `{htui/.../detail/runs.rs}` and `{htui-worker/src/runtime.rs, htui-core/src/store/mem.rs, docs/htui-worker.md}` do not intersect). T3 runs after both.

### Task 1: R-44 abbreviate the model (D3)
- **Test first**: unit tests on `who`: the date suffix is dropped; the agent prefix is dropped; both together
  (`claude` + `claude-sonnet-4-5-20250929` → `claude/sonnet-4-5`); no change for `claude/sonnet`, `agy/default`,
  `scripted/sonnet`; a model equal to `{agent}-` or to `-20250929` alone keeps its text (D3.3); a 7-digit or
  9-digit tail, or a non-digit tail, is kept; `None` on either side is `—`. Add an `extremes()` step (or a dedicated
  test) proving `~36k ! claude/sonnet-4-5` renders uncut in the 43-column row.
- **Action**: add `fn who(agent: Option<&str>, model: Option<&str>) -> String` beside `tail`, and have `tail` call it.
- **Mirror**: `gate` / `indicator` style; doc comment cites MOD-76 D3 and R-44.
- **Validate**: `cargo test -p htui --all-features --lib runs` plus the `backlog` and `replay` snapshot tests
  unchanged (`cargo test -p htui --all-features --test backlog --test replay`).

### Task 2: R-55 re-read `command_limits` (D4)
- **Test first** (in `runtime.rs` tests, beside the R-1 tests):
  1. With no walk live, a change to the box's `settings.command_limits` alone (repo map unchanged) gives a new
     verifier at the next sweep (`Arc::ptr_eq` on `parts()` is false), and `isolator_builds()` is unchanged.
  2. With unchanged limits, a sweep keeps the same verifier (`Arc::ptr_eq` true).
  3. While a walk is live, a limits change keeps the cached verifier and refuses nothing; the first sweep after the
     walk rests rebuilds it.
  4. The `StartRun` path (`start_run && Tails::Walk`) applies the change the same way as the sweep (this is the TUI's
     only re-read).
- **Action**: the `MemStore::set_box_setting` test seam (`test-support`); `Built.limits`; in `singletons`, read `command_limits` after the repo-map block, compare, clear
  `built.verifier` when they changed and `!self.any_live()`; the rebuild stores the limits it used. Update the two
  doc comments.
- **Mirror**: the repo-map rebuild just above; D216 error passing.
- **Validate**: `cargo test -p htui-worker --all-features runtime` and `cargo test -p htui --all-features run_worker`.

### Task 3: docs and close-out
- `docs/htui-worker.md` lines 45-51 (part of T2's file set, written by T2's implementer).
- Close-out per `references/lifecycle.md` P2: write-up `docs/decisions/mod/mod-76.md` (D1-D5, R-32/R-53 closing
  reasons), DECISIONS index, HANDOFF item `[x]` with status/summary, plan status `done`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
cargo test -p htui-worker --all-features
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The abbreviation hides a real difference (two models that differ only in date) | Low | Two dated snapshots of one model on one item are rare; the full id is still in the replay and the store. Recorded in the write-up. |
| An extra `box_row` read per sweep tick and per walking `StartRun` | Low | The same path already reads the box row and the repo map each time; one more small read. |
| Verifier rebuilt while a command is mid-flight outside a walk | Low | `any_live` counts walks only; a non-walk command holds its own `Arc` to the old verifier, as with today's repo-map rebuild. |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Line 2 is `INDENT` 8 + `GATE_WIDTH` 10 + 1 + `TAIL_WIDTH` 24 = `PANE` 43 | true | `runs.rs:131`, `:150-154` |
| `tail` is the only place the Runs pane renders `agent/model` | true | `runs.rs:1093-1103`; the only `.model` read under `detail/` is `runs.rs:1097` |
| No snapshot changes: every snapshot's `agent/model` is `claude/sonnet`, `claude/opus`, `agy/default`, `scripted/sonnet`, which D3 leaves alone | true | `crates/htui/tests/snapshots/backlog__*.snap`, `replay__runs_step_selected.snap` |
| `extremes()`'s `LONG_AGENT` / `LONG_MODEL` are untouched by D3 (no prefix match, no date tail), so the width test keeps cutting them | true | `runs.rs:2203-2206` |
| `claude/sonnet-4-5` beside `~36k ! ` fits the 24-column tail | true | 17 + 7 = 24; indicator text from `backlog__detail_runs.snap:18` |
| No existing model-abbreviation helper to reuse | true | symbol search ("short model abbreviate"): none in `htui` UI |
| `singletons` serves cached parts only when `!start_run`; otherwise it re-reads the repo map | true | `runtime.rs:387-391` |
| The verifier is rebuilt only when the repo map moved or none is cached | true | `runtime.rs:393`, `:424-438` |
| `Built` holds no limits today | true | `runtime.rs:169-175` |
| Callers: walking `StartRun` passes `start_run && tails == Walk`; the sweep passes `true`, worker role only | true | `runtime.rs:900`, `:1876-1878` |
| So the TUI re-reads only at a walking `StartRun` | true | `:1876` gates the sweep call on `Role::Worker` |
| `any_live` counts walks only | true | `runtime.rs:448-450`, `:706-710` |
| Each `verify` class semaphore lives in its `ShellVerifier`, so two verifiers mean two semaphores | true | `htui-orch/src/verify.rs:160` |
| A failed limits read is passed up, not defaulted (D216) | true | `runtime.rs:797-821`, `:431-433` |
| Nothing in production writes `settings.command_limits`; `BoxEdit` has no such field | true | `htui-core/src/model/box_.rs:162-170`; text search: writes only in tests |
| The tests need a write seam; MemStore has a `test-support` pattern and a `write` closure | true (plan amended: `mem.rs` added to T2) | `mem.rs:846-849`, `:885-898`; `htui-worker/Cargo.toml:32` enables `test-support` |
| `Backend::memory(store.clone())` shares rows with `store`, so a test can edit after building the runtime | true | `mem.rs:864-866` (rows shared with every clone); `runtime.rs:3768-3800` does it |
| The worker doc says a limits change alone needs a restart | true | `docs/htui-worker.md:45-51` |
| R-32's alternative touches `interrupt_step`: two trait declarations, nine implementations | amended (was "12 implementations") | text search `fn interrupt_step`: 12 hits, one is a conformance test |
| Latest migration is 0014; a column would be 0015 + cache 0006 | true | `ls crates/htui-store/migrations`, `cache_migrations` |
| T1 ∩ T2 file sets = ∅ | true | `{runs.rs}` vs `{runtime.rs, mem.rs, docs/htui-worker.md}` |

## Acceptance
- [ ] T1, T2, T3 complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented

# Plan: CLEAN-11 - CLEAN-9 deferred review residuals

**Source**: HANDOFF item CLEAN-11 (`docs/decisions/clean/clean-9.md`), `R-AGT-7`. Routed as plan (C3 only fired).
**Complexity**: Small

## Summary

Three residuals from CLEAN-9. (1) Pin the `info`/`debug` level `Shared::note_batch_stop` and `note_bad_cap`
log, which the existing test cannot see because it asserts only the returned `bool`. (2) Warn once per
project, not on every resolve, when `project.settings` does not decode. (3) Settle the `null` project-settings
disagreement between reader and writers.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Tests (log capture) | `crates/htui/src/agent_worker.rs:9022-9070` | `LogBuffer` (`Arc<Mutex<Vec<u8>>>` as `io::Write`) behind `tracing_subscriber::fmt().with_writer(..).with_ansi(false).finish()`, scoped with `tracing::subscriber::set_default` |
| Tests (bool pin to extend) | `crates/htui-worker/src/runtime.rs:3664-3691` | `a_batch_says_its_first_stop_and_its_first_bad_cap_once_each` |
| Once-per-key state | `crates/htui-worker/src/runtime.rs:333-353` (`first_of`) | a `StdMutex<Option<BatchId>>` remembering the last key |
| Dev-dependency | `crates/htui/Cargo.toml:67` | `tracing-subscriber = { workspace = true }` |

## Decisions for CONFIRM

**D1 - `null` project settings.** My earlier lean (make `clear_setting` accept `null`) is withdrawn after reading
the code. In Pg, `'null'::jsonb - 'k'` raises, so accepting `null` there needs a `CASE` in `clear_setting` and
`clear_queue_setting`, a new `.sqlx` entry for each, and a matching Mem branch. Every *writer* already refuses a
non-object (`set_*` carry `jsonb_typeof(settings) = 'object'`, pg/write.rs:3986, 4248, 4281, 4356, 4388).
**Recommended: refuse `null` everywhere** by making `ProjectCaps::from_settings` (`htui-core/src/model/quota.rs:371`)
return `CapError::document` for `null` too, with the doc comment rewritten. Smaller, no SQL, no `.sqlx`.
Cost: a project whose blob is a JSON `null` stops being admitted to the queue and a run on it is refused at
`graph::resolve` until fixed. Whether any row or fixture holds one is a claim to verify (V3) before this is built.
Alternative: keep the status quo and only add the test that pins the disagreement (documentation-only close).

**D2 - Dedupe scope.** Dedupe by `ProjectId` with a process-wide `Mutex<HashSet<ProjectId>>` in `graph.rs`
(`OnceLock`), warn once per project per process, `debug` after. Alternative: dedupe on `(ProjectId, updated_at)` so
an edit that stays malformed warns again; costs a clone of the timestamp, and `project_settings` receives `&Project`
which carries it. **Recommended: `(ProjectId, updated_at)`** - a fixed-then-rebroken document is news.

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-worker/Cargo.toml` | UPDATE | dev-dependency `tracing-subscriber = { workspace = true }` |
| `crates/htui-worker/src/runtime.rs` | UPDATE | `LogBuffer` in `mod tests`; level assertions beside the existing test |
| `crates/htui-orch/src/graph.rs` | UPDATE | once-per-`(project, updated_at)` warn; tests |
| `crates/htui-orch/Cargo.toml` | UPDATE (if absent) | same dev-dependency for the warn test |
| `crates/htui-core/src/model/quota.rs` | UPDATE (D1 recommended) | `null` is a `CapError::document`; doc comment; test |
| `docs/decisions/clean/clean-11.md`, `DECISIONS.md`, `HANDOFF.md` | CREATE/UPDATE | close-out per `workflow-docs.md` |

## Tasks (TDD: test first, seen red)

### Task 1: Pin the batch log levels
- **Action**: add `LogBuffer`; in a new test, run `note_batch_stop` and `note_bad_cap` twice under one batch with a
  scoped subscriber; assert the first line carries `INFO`/`WARN` and the second `DEBUG`. The subscriber must have
  `with_max_level(Level::DEBUG)` so the `debug` line is captured.
- **Mirror**: `agent_worker.rs:9047-9100`.
- **Validate**: `cargo test -p htui-worker --features test-support -- --test-threads=1 batch`. Seen red by swapping
  the two level macros in `note_batch_stop`.

### Task 2: Dedupe the decode warn
- **Action**: add a process-wide once-set keyed per D2; keep the `empty` check; second resolve logs at `debug`.
  Test: two `project_settings` calls on one malformed project produce one `WARN` line; a changed `updated_at`
  warns again.
- **Validate**: `cargo test -p htui-orch --lib graph::`. Tests that reuse a `ProjectId` across cases share the
  global set, so each test mints its own id.

### Task 3: `null` (D1)
- **Action**: per the confirmed option. Recommended path: failing `from_settings(&Value::Null)` test first, then the
  one-line change, then update every caller test that relied on `null` = unbounded.
- **Validate**: `cargo test -p htui-core --lib quota` and the workspace gate.

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Verified claims (step 3.5)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `note_batch_stop` is `info` first, `debug` after; `note_bad_cap` is `warn` first, `debug` after | verified | `runtime.rs:333-353` |
| V2 | The only existing pin is the returned `bool` | verified | `runtime.rs:3664-3691` |
| V3 | A `null` `project.settings` reaches `from_settings` in production or a fixture | **unverified** | to check before Task 3: callers of `from_settings` and `Value::Null` fixtures |
| V4 | `htui-worker` has no `tracing-subscriber` | verified | `htui-worker/Cargo.toml` has `tracing` only |
| V5 | `tracing-subscriber` is a workspace dependency | verified | root `Cargo.toml:79` |
| V6 | `htui-orch` already has `tracing-subscriber` as a dev-dependency | **falsified**: it has none, so Task 2 adds it (the `htui-orch/Cargo.toml` row is UPDATE, not conditional) | `crates/htui-orch/Cargo.toml:55-63` |
| V7 | `jsonb_typeof(settings) = 'object'` guards the Pg `set_*` and `clear_*` writers | verified | `pg/write.rs:3986, 4104, 4248, 4281, 4356, 4388` |
| V8 | `'null'::jsonb - 'k'` raises in Postgres | verified from Postgres semantics (scalar delete); not run here, check with `psql -h localhost -p 5439` | - |
| V9 | Tasks touch disjoint files, so 1/2/3 may be parallel | **fails the rule**: Task 1 and 2 are disjoint, Task 3 shares `quota.rs` tests with no one, but a plan this small runs serial | file sets above |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Global once-set leaks across tests in one process | Medium | per-test project ids; assert on the delta |
| `tracing` callsite interest is cached per process, so a `debug` line may be dropped if another test installed a stricter default | Medium | `set_default` is thread-local and rebuilds interest; run with `--test-threads=1` as the gate does |
| D1 refusal strands a project on a stored `null` | Low | V3 before building; D1's alternative is documentation only |

## Acceptance
- [ ] Level pins fail when the levels are swapped
- [ ] A malformed project warns once per `(project, updated_at)`
- [ ] D1 settled and documented on `ProjectCaps::from_settings`
- [ ] Gate green, validator 0/0

# Plan: CLEAN-9 - MOD-12 M2 review residuals

**Source**: `HANDOFF.md` CLEAN-9 (R-AGT-7, R-TUI-8); parent plan `.claude/plans/mod-12-m2-spend-guard.plan.md`
**Routed as**: plan (CLEAN prefix, 1 of 4 criteria fired, C3 weak). Sandbox run `hr/CLEAN-9`, no push.
**Complexity**: Medium (four residuals, about 10 files, one `.sqlx` regeneration)

## Summary
Four behaviour-preserving-or-tightening residuals from MOD-12 M2: the recorder's `cap_exceeded` row
words its figure as the project's full run cap when it is the session's remaining allowance; the
engine's snapshot drops both caps when an unrelated project key is malformed; Pg `clear_setting`
accepts non-object `project.settings` blobs that Mem refuses; and two tests under-assert.

## Decisions (maintainer may amend at CONFIRM)
- **D1 (wording)**: `RunCap` gains `basis: CapBasis { Run, Batch(BatchId) }`. Row reads
  "session allowance reached: an estimated $X (n micros) spent against an allowance of $Y (m micros),
  the remainder of project.settings.per_token_cap_run" (or "of batch <id>'s per_token_cap_batch").
  Both existing substring pins (`per_token_cap_run`, `estimated`) still hold on the run basis.
  `run_chat` (no batch) passes `Run`; `Engine::open_recorder` maps `Allowance.batch`.
- **D2 (uncapping)**: the snapshot reads the two caps through `ProjectCaps::from_settings` on the raw
  blob. A bad *cap* key fails the run closed (the worker's D4 stance); a bad *unrelated* key still
  defaults the other `ProjectSettings` fields but logs a `warn` naming the blob's project. Caps are
  never taken from the defaulted struct again.
- **D3 (non-object blob)**: Pg `clear_setting` refuses a non-object blob with
  `project_settings_not_an_object` (same text and `StoreError` class as Pg `set_setting` and Mem).
  `ProjectCaps::from_settings` returns `CapError` for a non-object, non-null blob; `null`/absent stay
  "no caps".
- **D4 (tests)**: assert the batch is still open before the second sweep; pin the once-per-batch
  `info` then `debug` with a tracing capture layer if one is already a dev-dependency, else pin the
  returned `bool` only and defer the level pin as a new CLEAN item (no new dependency for a log level).

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Errors | `htui-core/src/store/traits.rs:2337` `project_settings_not_an_object` | one refusal sentence shared by Mem and Pg |
| Pg refusal | `htui-store/src/pg/write.rs:3913-4054` `set_setting` | how the Pg merge path refuses a non-object blob |
| Mem refusal | `htui-core/src/store/mem.rs:4238-4325`, test `:10733` | the behaviour Pg must match |
| Caps read | `htui-core/src/model/quota.rs:326` `ProjectCaps::from_settings` | strict cap decode, first bad key named |
| Allowance | `htui-orch/src/engine.rs:7204` `session_allowance` | the term (run vs batch) already computed; basis is just exposing it |
| Tests | `htui-agent/tests/recorder.rs:2607`, `htui-store/tests/pg_criteria.rs:5467` | breach row pins; non-object blob pin |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-agent/src/record.rs` | UPDATE | `RunCap.basis`, `record_cap_breach` wording (T1) |
| `crates/htui-orch/src/engine.rs` | UPDATE | `open_recorder` sets basis; caps via `from_settings` at snapshot (T1, T2) |
| `crates/htui/src/agent_worker.rs` | UPDATE | `run_chat` `RunCap` literal (T1) |
| `crates/htui-agent/src/conformance.rs`, `tests/relay.rs`, `tests/recorder.rs` | UPDATE | `RunCap` literals; new wording pins (T1) |
| `crates/htui-orch/src/graph.rs` | UPDATE | `resolve` reads caps strictly; warn on a defaulted blob (T2) |
| `crates/htui-core/src/model/quota.rs` | UPDATE | non-object blob is a `CapError` (T3) |
| `crates/htui-store/src/pg/write.rs` + `.sqlx/` | UPDATE | `clear_setting` refuses non-object (T3) |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE | clear on array/scalar blob (T3) |
| `crates/htui-worker/tests/auto_queue.rs`, `src/runtime.rs` (test) | UPDATE | test hardening (T4) |

## Tasks (TDD: test first in each)
### T1: allowance basis in the cap-breach row
Files: `record.rs`, `engine.rs` (`open_recorder`), `agent_worker.rs` (`run_chat`), `conformance.rs`, `tests/relay.rs`, `tests/recorder.rs`.
Validate: `cargo test -p htui-agent --all-features`, `cargo test -p htui-orch --all-features -- --test-threads=1`.
### T2: caps survive an unrelated malformed key
Files: `graph.rs`, `engine.rs` (`project_settings`/snapshot only). **Shares `engine.rs` with T1, so T1 and T2 run serial.**
Validate: `cargo test -p htui-orch --all-features -- --test-threads=1`.
### T3: non-object `project.settings`
Files: `quota.rs`, `pg/write.rs`, `.sqlx/`, `pg_criteria.rs`. Disjoint from T1/T2/T4.
Validate: `cargo test -p htui-core`, Pg gate `cargo test -p htui-store --all-features -- --test-threads=1` on the sandbox DB; `cargo sqlx prepare` against a migrated scratch DB.
### T4: test hardening
Files: `auto_queue.rs`, `runtime.rs` test module. Disjoint from the rest.
Validate: `cargo test -p htui-worker --all-features -- --test-threads=1`.

Order: T3 and T4 parallel-safe with T1; T1 then T2 serial. Full gate afterwards.

## Validation
```bash
cargo fmt --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1
cargo insta test --workspace --all-features   # snapshot impact needs the full run
```

## Verified claims (step 3.5)
| Claim | Verdict | Evidence |
|---|---|---|
| `record_cap_breach` words the cap as `project.settings.per_token_cap_run = <n> micros` | verified | `record.rs:1703-1713` |
| `RunCap` is `{micros, grace}`, `Copy`, 5 construction sites | verified | `record.rs:172-177`; usages: `play_capped`, `relay.rs`, `open_recorder`, `run_chat`, `recorder.rs::run_cap` |
| `Engine::project_settings` and `graph.rs::project_settings` use `unwrap_or_default()` | verified | `engine.rs:6825`, `graph.rs:655` |
| Caps in the snapshot come from the defaulted struct | verified | `graph.rs:359` `per_token_cap_run: settings.per_token_cap_run` |
| `allowance()` already yields the run-vs-batch term and batch id | verified | `engine.rs:6212`, `session_allowance` `:7204` |
| Pg `clear_setting` is `settings - $2::text` with no object check | verified | `pg/write.rs:4067-4137` |
| Mem refuses a non-object blob on clear | verified | `mem.rs:10733` test name; refusal fn `traits.rs:2337` |
| `note_batch_stop` is `info` first, `debug` after, returns the bool | verified | `runtime.rs:333-341` |
| Existing breach tests pin only `per_token_cap_run` + `estimated` substrings | verified | `recorder.rs:2669`, `agent_worker.rs:9943` |
| A tracing capture layer is available to the worker tests | **unverified**: decide at T4 (D4 fallback covers it) | not checked |
| `ProjectCaps::from_settings` reads a non-object as no caps; `null` fixtures exist | **unverified** | stated by HANDOFF; `cap_at` body not read, fixtures using `Null` settings not enumerated |
| Changing the Pg SQL needs a new `.sqlx` entry | verified by rule | `sqlx-offline-hash-is-literal-query` (hash = literal query) |
| T1 and T2 are independent | **falsified** | both touch `engine.rs`; marked serial |

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| D2 failing closed turns an existing run with a bad cap into a failed run | Low | worker D4 already fails closed; add a test with the run's failure text |
| D3 breaks fixtures using `Value::Null` settings | Medium | enumerate before editing; `null` stays "no caps" |
| `.sqlx` regeneration needs a migrated scratch DB | Medium | sandbox Postgres on 5439; follow `docs/hr-sandbox.md` |
| Snapshot (insta) text changes from the new row wording | Medium | full `cargo insta test`, not a `.snap` grep |
| Stack depth in `htui-orch` tests | Low | run gates `--no-fail-fast --test-threads=1`, grep SIGABRT |

## Acceptance
- [ ] Row names its basis (run or batch) and an allowance, not the project's full cap
- [ ] A malformed unrelated key no longer drops either cap; a malformed cap key fails closed
- [ ] Pg `clear_setting` refuses array and scalar blobs like Mem
- [ ] Batch-open assertion added; log-level pin added or deferred explicitly
- [ ] Validation passes; HANDOFF/DECISIONS close-out; validator green

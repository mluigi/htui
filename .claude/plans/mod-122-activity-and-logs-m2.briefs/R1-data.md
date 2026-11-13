# MOD-122 M2 — review fix round R1, data path (worker shipper)

Source: the configured reviewer (`rust-reviewer`, data-path instance) over `a05deeac..2499049b`.
Confirm each finding against the code before acting; if one is wrong, say so in `rejected_findings`
and leave it. Tests first for every behaviour change.

Context (the plan's decisions that bind here): D3 buffer cap 20 000 lines, oldest non-`error` line
dropped first, drops shipped as one `warn` line; D7 `LogShipLevel {Off, Error, Warn, Info, Debug}`,
`Off` ships nothing; D8 per-layer filter, the ship layer's own `filter_fn`, `rebuild_interest_cache`
on a level change, `sqlx`/`sqlx::…`/`sqlx_…` events below WARN ignored; D9 every line scrubbed on the
worker, a refused line becomes the withheld text with level and target kept. "htui never writes a
secret into a log line — memory, file or shipped."

## Files owned

`crates/htui/src/worker_log/mod.rs`, `crates/htui/src/worker_log/ship.rs`,
`crates/htui/src/worker_cmd.rs`, `crates/htui/tests/worker_log_pg.rs`, `docs/htui-worker.md`,
`crates/htui-core/src/store/worker_log.rs` (doc comment only).

## Findings to apply

**M1 — `off`, or a lowered level, does not reach lines already buffered** (`ship.rs:93-103`,
`mod.rs:179-197`). The handle starts at `Warn`; startup `warn`/`error` lines are buffered before the
flusher's first cycle reads `off` from the box row, and `flush` ships them anyway. Fix: lines the
current level would not ship are not shipped — filter by `level().ships(..)` when the batch is taken
(or drop them when `set_level` lowers the level). The drop notice is unaffected. Test: buffer a
`warn`, set `Off`, flush → the fake store receives nothing; `info` buffered then level `Error` →
not shipped.

**M2 — shutdown drops the last lines** (`worker_cmd.rs:154` aborts the flusher with no final
flush). Fix: before aborting, one bounded final `flush()` (e.g. a shutdown `Notify`/`oneshot` handled
in `run_flusher`'s `select!`, then `tokio::time::timeout(2 s, …)`). Test against the fake store: a
line pushed just before shutdown is shipped; a store that hangs does not block shutdown past the
bound.

**M3 — two planned tests are missing or prove less than claimed.**
- Plan V18 / risk row: "a test counts the lines one flush adds at `debug` and requires zero". Add it
  in `tests/worker_log_pg.rs`: on a `current_thread` runtime with the ship layer installed via
  `set_default` around the flusher (so the real `sqlx` events of the flush are seen), level `debug`,
  run one real flush against Postgres, assert no `sqlx` line entered the buffer.
- `worker_log_a_level_change_reaches_a_callsite_seen_while_refused` (`mod.rs:440`) runs under a
  scoped dispatcher, where (plan probe V2) the case works even without `rebuild_interest_cache`.
  Make it prove the call: a global-subscriber case confined to a child process (re-exec the test
  binary with an env var selecting the child body, the usual pattern), or another construction
  that fails if `set_level` drops `rebuild_interest_cache`. Check by temporarily removing the call
  that the test goes red (do not commit that).

**M4 — a failing sweep retries every cycle** (`ship.rs:107-122` leaves `next_sweep` unchanged on
error). Fix: on failure set `next_sweep = now + FLUSH_EVERY * 30` (one minute) or similar; log on
transition only, as now. Test with the fake store failing the prune.

**L1 — buffer is bounded by lines, not bytes** (`mod.rs:292`). Cap the text in `on_event` right
**after** `scrubbed(...)` (never before: cutting first can shorten a token below a rule's minimum and
let part of a secret through), to `WORKER_LOG_TEXT_MAX` with the same `…` marker `NewWorkerLog::stored_text`
uses (reuse it if it fits). Test: a 1 MiB field is held at ≤ the cap.

**L2 — cap edge in `push`** (`mod.rs:155-170`): when `in_flight == cap` and nothing is held, an
incoming error is counted dropped but `pop_front()` removes nothing → buffer `cap + 1`, notice
over-counts. Fix: take that branch only when lines are held; otherwise drop the newcomer (counted).
Test the edge.

**L3 — ambiguous commit duplicates rows.** Accept; document it in the `WorkerLogStore::append_worker_logs`
doc (`htui-core/src/store/worker_log.rs`): a batch whose commit outcome is unknown is re-sent, so a
reader may see a line twice; rows are never lost.

**L4 — wording drifted from the plan.** Use the plan's strings: withheld text
`[line withheld: it matched a credential rule]` (D9); drop notice
`N log lines were dropped while the database was unreachable` with the singular
`1 log line was dropped while the database was unreachable` (D3). Update tests and
`docs/htui-worker.md` together.

**L5 — targets are not scrubbed** (`mod.rs:285-288`). Pass the target (including a bridged
`log.target`) through the same scrubber; a refused target becomes a fixed placeholder (e.g.
`withheld`). Test.

**L7 — early return leaks the flusher** (`worker_cmd.rs:138`, `McpHost::new(..)?` after the spawn).
Make the flusher's handle abort on drop (a small guard) or move the spawn after the fallible setup,
whichever keeps the existing worker tests green unchanged. Do not change `index_job`'s pattern.

Deferred (do NOT do): L6 (adding the Qdrant key to the scrubber), L8 (prune cost/deadlock note),
L9 (per-chunk copies), L10 (`user_id` index).

## Check

`cargo test -p htui --all-features --lib -- worker_log worker_cmd --test-threads=1`;
`cargo test -p htui --all-features --test worker_log_pg --test worker_pg -- --test-threads=1`
(worker_pg takes ~3 min: run it in the background and poll); `cargo test -p htui-core --all-features --lib -- worker_log`;
`cargo clippy -p htui --all-targets --all-features -- -D warnings`; `cargo clippy -p htui -- -D warnings`;
`cargo fmt -p htui -p htui-core`.

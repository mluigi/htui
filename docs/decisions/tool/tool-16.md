# TOOL-16 - A gate that fits in a session: template databases, parallel tests, shorter long poles (done, 2026-11-13)

From TOOL-15. After TOOL-15 the gate took 30 min, 29 of them the test step: `cargo test --workspace
--all-features --no-fail-fast -- --test-threads=1`, 1732 s (htui-store `it` 928 s, htui `it` 376 s, htui unit
tests 156 s). The maintainer's constraint: a full-suite hour is unacceptable, and an item run must fit in a
session. No plan file: the work was driven from measurements in the session that closed TOOL-15. Branch
`tool-16` (on `tool-15`).

## Result

| | TOOL-15 gate (`fe802740`) | TOOL-16 gate (`1dd0d52f`) |
|---|---|---|
| whole gate | 30 min | **8 min 11 s** |
| test step | 1732 s, `--test-threads=1` | **457 s** (414 s on `14025c0a`), default threads |
| htui-store `it`, default threads | 1022 s | **221 s** with Qdrant live, 169 s without |
| `pg_conformance::` | ~1000 s (one test) | 4 shards of ~93 s side by side |
| failures | 2 base + the cancel/interrupt pair | the 3 base failures (`worker_crash_pg` passed in the TOOL-15 gate) |

The three base failures, unchanged from `ae910789`: `claude_plugin::the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude`
(the installed `claude` 2.1.323's strict validate), `worker_pg::the_worker_binary_never_exposes_its_dsn` (28P01 for
the worker's own connect) and `worker_crash_pg::a_cancel_picked_but_not_applied_survives_the_kill` (the same assertion
as the base log).

## Built

1. **Template database** (`c9108fb1`, `htui-store/src/testkit.rs`). `fresh_db` and so `demo_db` were `CREATE
   DATABASE` + connect + `apply_migrations` (30 migrations) per test. Now each copies `htui_tmpl_<16 hex>`
   (`CREATE DATABASE … TEMPLATE`): every migration applied and `raise_target` run, nothing seeded, no box
   registered, so the copy's own `PgStore::connect` seeds and registers its test's identity exactly as the bootstrap
   in `apply_migrations` did. The name hashes every migration's version, description and checksum plus
   `HTUI_VERSION`, so an edited migration gets a new template. Built once per server under a Postgres advisory lock
   (any test binary or worktree), under a temporary `htui_tmplbuild_` name renamed into place when complete, so a
   template that exists is a finished one. The once-per-process sweep drops dead builds after an hour and other
   migration sets' templates after a week. The one visible difference: `migrations_at_connect` is `UpToDate`; the
   four tests that read it start from `bare_db`, which is unchanged. `pg::raise_target` became `pub(crate)`.
   `pg_criteria::` 82 s to 30 s, `cache::` 11.4 s to 4.1 s, `pg_conformance::` ~1000 s to 81 s.
2. **Live Qdrant cases take turns** (`c9108fb1`, `14025c0a`). `testkit::qdrant_turn()` is a process-wide async
   lock; `qdrant_live` (32 cases), htui's `qdrant_worker`, `state_cli_pg` and the `concepts_worker` unit tests hold
   it for the whole case. Each case already owned its collection; what they share is the server, and a dozen
   collection creates at once time out (`create collection: … Timeout expired`). Alone and serial the 32 pass in
   87 s. Possible only because TOOL-15 put each crate's tests in one process.
3. **`pg_store_conformance` sharded** (`c9108fb1`): `CASES` dealt round-robin across four tests.
4. **The gate at default threads** (`14025c0a`): `gate.sh` and `gate.ps1` drop `--test-threads=1`;
   `docs/hr-sandbox.md`'s full-suite command too.
5. **No write handle on a fixture executable** (`14025c0a`, `1dd0d52f`). A test that writes a script and then
   execs it races every `fork` of the other tests in its process: a child forked while the write handle is open
   inherits it, and `execve` answers `ETXTBSY`. Since TOOL-15 the other test can be any test of the crate.
   `tests/rust/write_executable.rs` (one source, included by the htui-agent, htui and htui-orch `it` binaries
   through `#[path]`, no feature) has a `/bin/sh` child write the file. Used by `box_probe` (whose exec happens
   inside the production probe, so no retry can wrap it), `cli_driver`, `install`, `relay`, `backlog`, `provision`
   and `gix_isolator`; `install` also stopped opening the cmd for writing to set its mtime. `probe`, `auth` and
   `acp_driver` keep their retries.
6. **`gate.sh --detach` keeps SIGINT** (`1dd0d52f`, `0629cc71`). See below.
7. **Qdrant creates retried in tests** (after the merge with main). A collection create takes about 0.13 s on an
   idle Qdrant but went past qdrant-client's default 5 s deadline under the parallel gate, so
   `concepts_worker::tests::a_tui_search_of_another_embedders_collection_is_refused_before_the_model` failed in 2
   of 4 parallel runs; the load is every other test, so `qdrant_turn` cannot help. `testkit::qdrant_connect_to`
   retries `QdrantStore::connect_to` (idempotent) on `Timeout expired`, up to three tries; used by `qdrant_live`,
   `qdrant_worker`, `state_cli_pg` and that test. The product side, a longer deadline for collection admin calls
   only, is MOD-175 (maintainer: both).

`AGENTS.md` "Verify" states the conventions: parallel tests, `qdrant_turn`, `write_executable`, `bare_db` for
pending-migration tests.

## The "load flakes" were ignored SIGINT

`htui-agent`'s `cli_driver::a_cancel_drains_a_late_result_and_reports_its_done_rather_than_a_second_one` and
`launch::an_interrupt_lets_the_child_exit_on_its_own_terms` failed in every detached gate (MOD-122, both MOD-141
M2 gates, TOOL-15) and passed in every foreground run; they were written down as load flakes. Five orphaned
fixtures (`sh -c "trap 'exit 42' INT; … while :; do sleep 0.05; done"`) were still running, all with
`SigIgn: 0x6`. `gate.sh --detach` launched `setsid bash … &` from a non-interactive shell, whose background jobs
start with SIGINT and SIGQUIT ignored; everything under it inherits that, and a shell that starts with a signal
ignored cannot trap it. Reproduced both ways (plain detach: 2 failed; with the signals reset: 2 passed in 0.2 s).
The first fix, `set -m`, made the job a process-group leader, so `setsid` forked and the printed PID was a parent
that exited at once; the launch is now `setsid env --default-signal bash …` (bare `--default-signal` is the form GNU
coreutils 9.7 and the host's uutils 0.10 agree on), whose children start with `SigIgn: 0`. The TOOL-15 write-up
and the memory note are corrected.

## Considered and dropped

- **mold or `split-debuginfo`:** `[profile.dev]` already has `debug = "line-tables-only"` and
  `split-debuginfo = "unpacked"`, and Rust 1.100 links `x86_64-unknown-linux-gnu` with the bundled `rust-lld`.
  mold would gain little over lld and need an install on the host and in the hr image.
- **A change-scoped gate** (test only the changed crates and their dependents): htui depends on every crate, so any
  change below it is the whole suite; it saves about 3.5 of 8 min on htui-only changes. Dropped by the maintainer.
- **Templates for the `migrations::` tests** (15-30 s each, migrating step by step from `bare_db`): not the floor
  any more.
- **cargo-nextest:** filed as TOOL-17 (one pool across binaries, a process per test, retries, test groups).

## Watch

- **The dev Postgres's load-related crash** hit the first default-thread gate: a backend `exited with exit code
  2`, the postmaster reinitialised, and 97 Postgres cases failed `57P03` while it recovered (disk at 65%). It did
  not recur in the next two gates and predates TOOL-16 (seen 2026-09-16); it struck again during a 20-test run of
  `state_cli_pg::`/`qdrant_worker::`, so it is not load alone. No cause found: the backend logs nothing before
  `exited with exit code 2` (Postgres's SIGQUIT exit), `htui-postgres` runs as uid 999 so the tests cannot signal
  it, and the server had `log_connections = off`, `log_min_messages = warning`. Three crashes on 2026-11-13
  (12:19, 14:41, 14:46 UTC), two of them about a minute into the gate's test step (the htui unit binary runs
  first), none in the cgroup's `oom_kill`. Not reproduced by 180 parallel template copies with forced drops, nor
  by four repeats of htui's Postgres modules. For the gate on `732b75af` the dev server logged connections,
  disconnections, DDL and `%d %a` in its line prefix (`ALTER SYSTEM`): no crash, red only on `claude_plugin` and
  `worker_pg`. The maintainer had that logging reverted at close (`ALTER SYSTEM RESET` of the four, then
  `SELECT pg_reload_conf()`); to chase the next crash, set it again the same way. The gate now drives more concurrent
  create/drop-database work, so a recurrence is more likely than it was; the tell is `57P03` across many cases and
  `exited with exit code` in `docker logs htui-postgres`. Re-run before suspecting the change.
- `gate.ps1` was not run (no Windows here); its change is the same one-line flag removal, and Windows has no
  POSIX signal inheritance to fix.

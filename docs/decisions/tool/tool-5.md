# TOOL-5 - Dev Postgres Crash Fix

**Date:** 2026-09-14
**Status:** Accepted and implemented

## Context
Concurrent `cargo test` runs would crash the dev Postgres instance into a recovery loop because killed runs leaked `htui_test_*` databases, slowing down recovery due to fsync overhead.

## Decision
Implemented an eager sweep of stale test databases at harness start-up in `crates/htui-store/src/testkit.rs`.

1. A `tokio::sync::OnceCell` ensures the sweep runs only once per test process.
2. We filter stale databases using `(pg_stat_file('base/' || oid, true)).modification < now() - interval '1 hour'`, utilizing `pg_stat_file` with `missing_ok = true` to gracefully handle databases that are dropped concurrently.
3. The stale databases are aggressively dropped using `DROP DATABASE IF EXISTS ... WITH (FORCE)`.

This mirrors the staging sweep pattern from `htui-agent::install`, preventing `htui_test_*` accumulation across killed suites without racing against databases actively being used by other running suites.

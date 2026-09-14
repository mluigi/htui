# Plan: TOOL-5 Postgres Test Crash Fix

## Context
TOOL-5 tracks an issue where concurrent `cargo test` suites crash the dev Postgres instance into a recovery loop. This happens because killed test suites leak their `htui_test_*` throwaway databases, which accumulate and slow down Postgres's crash recovery until the system falls over. The proposed fix is to introduce an eager sweep of stale test databases at harness start-up, similar to the existing `install` staging sweep.

## Proposed Changes

### 1. Eager Sweep of Stale Test Databases
In `crates/htui-store/src/testkit.rs`:
- Introduce a static `tokio::sync::OnceCell<()>` to ensure the sweep only runs once per test process (harness start-up).
- Inside `bare_db()`, call `SWEEP.get_or_init(...)` to perform the sweep asynchronously.
- The sweep will open a temporary connection to `maint_url` and identify stale databases using the following SQL query:
  ```sql
  SELECT datname 
  FROM pg_database 
  WHERE datname LIKE 'htui_test_%' 
    AND (pg_stat_file('base/' || oid)).modification < now() - interval '1 hour'
  ```
- Iterate over the returned `datname` values and execute `DROP DATABASE IF EXISTS "{datname}" WITH (FORCE)` for each stale database.
- Close the temporary connection.

### 2. Rationale & Safety
- **Why `OnceCell`?** Test cases within a suite run concurrently in the same process. Running the sweep unconditionally on every `bare_db()` call would create extreme lock contention and could accidentally race against a recently created database.
- **Why 1 hour?** A 1-hour threshold ensures that databases actively being used by other concurrent `cargo test` processes are untouched, while genuinely leaked databases from killed runs are safely removed. The 1-hour limit mirrors the `install` module's staging sweep max age (`config.staging_max_age`).
- **Why `pg_stat_file`?** Postgres does not natively expose database creation time. However, checking the modification time of `base/<oid>` is a robust proxy, as a leaked database receives no updates.

## Execution
Once this plan is confirmed, we will proceed to implementation in `crates/htui-store/src/testkit.rs`.

### Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| Postgres 16 supports `pg_stat_file('base/' || oid)` and returns a modification time. | Pass | Verified directly against the dev Postgres instance using `docker compose exec postgres psql`. |
| `DROP DATABASE ... WITH (FORCE)` works for cleaning up connections. | Pass | Already used in `testkit.rs` for database drop (`testkit.rs:167`). |
| `tokio::sync::OnceCell` is available in the crate. | Pass | `tokio` is a direct dependency of `htui-store` and `sync` features are heavily used across the workspace. |

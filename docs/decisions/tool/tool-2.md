# TOOL-2 - Demo fixture's app_user.name collides with the OS username (concluded, 2026-09-14)

## Context
`crates/htui-core/src/fixtures.rs` previously seeded `app_user.name = "luigi"`, and `PgStore::seed_if_empty` derives the same name from the OS `USERNAME`. This meant that `common::demo_db()` hit a `duplicate key value violates unique constraint "app_user_name_key"` if the test ran on a machine where the OS user was named `luigi`. 
Furthermore, `crates/htui-store/src/testkit.rs` skipped database tests silently (by returning early and printing a skip message) when `HTUI_TEST_DATABASE_URL` was unset, which allowed CI environments to potentially report a false positive pass if the database was unavailable.

## Decisions
1. **Fixture Username Update**: Changed the seeded username in `crates/htui-core/src/fixtures.rs` from `"luigi"` to `"htui-demo-user"` to avoid collisions with realistic OS usernames.
2. **CI Skip Path Distinction**: Modified `bare_db()` in `crates/htui-store/src/testkit.rs` to check for the `CI` environment variable (`std::env::var("CI").is_ok()`). If `CI` is set and `HTUI_TEST_DATABASE_URL` is empty, it now panics to explicitly fail the test, ensuring tests cannot be skipped silently in CI environments.

## Results
- The Postgres tests can now safely run on a host regardless of the developer's OS username.
- CI pipelines are protected against silently skipped database tests.

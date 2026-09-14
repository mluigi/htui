# Plan: TOOL-2 Demo fixture username collision

## Tasks

### 1. Fix the username collision in the demo fixture
- In `crates/htui-core/src/fixtures.rs`, change the seeded `app_user.name` in `users()` from `"luigi"` to `"htui-demo-user"`.
- Update the associated email to `"htui-demo-user@example.invalid"`.
- Update the doc comment on `fixtures::users()` or other places that mention `"luigi"` to reflect the new name.
- This ensures `demo_db()`'s `load_demo` call will not hit a duplicate key constraint on `app_user.name` if the host OS user running the tests happens to be named `"luigi"`.

### 2. Make the database skip path distinguishable from a real pass in CI
- In `crates/htui-store/src/testkit.rs`, modify `bare_db()` (which both `fresh_db()` and `demo_db()` rely on) to check if the `CI` environment variable is set (e.g. `std::env::var("CI").is_ok()`).
- If `CI` is set and `HTUI_TEST_DATABASE_URL` is missing or empty, `panic!` instead of returning `None`. 
- This prevents CI runs from silently skipping database tests and reporting a false green when the database is unavailable, while keeping local development builds convenient (they will continue to silently skip).

## Verification
- Run `cargo test -p htui-store --features test-support` with `HTUI_TEST_DATABASE_URL` set and `USERNAME=luigi` (or `USER=luigi`) to verify the collision is gone.
- Run `cargo test -p htui-store --features test-support` with `HTUI_TEST_DATABASE_URL` unset and `CI=1` to verify it panics and fails the test.

### Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| `fixtures.rs` sets `app_user.name` to `"luigi"` in `users()` | Verified | `crates/htui-core/src/fixtures.rs:392` |
| `demo_db()` uses `fresh_db()` which uses `bare_db()` | Verified | `testkit.rs:165` calls `fresh_db()`, `testkit.rs:147` calls `bare_db()` |
| `bare_db()` skips tests when `HTUI_TEST_DATABASE_URL` is missing | Verified | `testkit.rs:74` returns `None` and prints `SKIP` |
| Test suite tasks are independent | Verified | Tasks touch different files (`fixtures.rs` vs `testkit.rs`) |

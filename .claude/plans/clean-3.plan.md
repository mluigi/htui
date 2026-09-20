# CLEAN-3: `cargo doc --workspace --no-deps` has never been green

## Goal
Decide which feature set the workspace doc gate documents, then fix intra-doc link errors in `htui-store`, `htui-core`, and `htui-agent` to make `cargo doc` green on that feature set.

## Findings from investigation
1. `cargo doc --workspace --no-deps` fails with unresolved links to `testkit` items (`FAKE`, `mock_keyring`) because those are guarded behind the `testkit` feature.
2. `cargo doc --workspace --no-deps --all-features` fails with "public documentation links to private item" because `testkit` items are not public.
3. CLEAN-2 removed the offline buffered-write path (`BufferedWriter`, `Writer::Buffered`, `pending::seal_orphaned`), adding about 12 more dead links in `htui-store`'s `backend.rs`, `writer.rs`, and `cache/mod.rs`.

## Decision: Feature Set
The workspace doc gate will document with **`--all-features`** (`cargo doc --workspace --no-deps --all-features`).
Reasoning: Workspace docs should cover the whole tree, including `testkit` structures, as they are part of the internal API surface used across crates. To fix the "public documentation links to private item" errors, we will replace the intra-doc links `[`...`]` to private items with plain code formatting `` `...` ``.

## Tasks
1. **Fix `htui-store` dead links (from CLEAN-2 removal)**
   - `crates/htui-store/src/backend.rs`: Remove references to `Writer::Buffered` and `BufferedWriter`.
   - `crates/htui-store/src/writer.rs`: Remove or update references to `BufferedWriter`, `Writer::Buffered`, `pending::seal_orphaned`, `StoreError::NotFound`, `StoreError::Unreachable`.
   - `crates/htui-store/src/cache/mod.rs`: Remove references to `pending::seal_orphaned`.
2. **Fix `htui-store` private/unresolved links**
   - `crates/htui-store/src/connect.rs`: Fix `reconnect_over` link in `reconnect_for` docs.
   - `crates/htui-store/src/dsn.rs`: Fix `Dsn::as_str` and `scan` links.
   - `crates/htui-store/src/secret.rs`: Fix `FAKE` and `mock_keyring` links.
   - `crates/htui-store/src/testkit.rs`: Fix `KEYRING` link.
3. **Fix `htui-core` private links**
   - `crates/htui-core/src/prompt/fixtures.rs`: Fix `handoff_events` link in `handoff_basic` docs.
4. **Fix `htui-agent` private links**
   - `crates/htui-agent/src/conformance.rs`: Fix `open_case` links in `caps` docs.
5. **Verify**
   - Run `cargo doc --workspace --no-deps --all-features` to ensure no errors.

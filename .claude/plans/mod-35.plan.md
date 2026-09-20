# Plan for MOD-35: Add Qdrant connection settings

## Goal
Add connection settings for Qdrant (URL and optional API key) mirroring the Postgres DSN structure. This provides the configuration for `htui-store`'s `QdrantStore` (introduced in MOD-34).

## Architecture

1.  **Secret Management (`crates/htui-store/src/secret.rs`)**:
    *   Add `get_qdrant_dsn()`, `set_qdrant_dsn(dsn: &str)`, and `clear_qdrant_dsn()` to store the Qdrant settings in the OS keyring under the service `htui` and user `qdrant-dsn`.

2.  **Model & Validation (`crates/htui-store/src/qdrant_dsn.rs`)**:
    *   Create a `QdrantDsn` newtype wrapping `Zeroizing<String>`, similar to `Dsn`.
    *   `QdrantDsn::parse(text: &str)` will validate the format. We will accept a URL optionally followed by a space and the API key (e.g., `http://localhost:6334` or `https://cluster.qdrant.io:6334 my_api_key`).
    *   `QdrantDsn::summary()` returns a redacted summary (e.g., `http://localhost:6334 [with API key]`).
    *   `QdrantDsn::url_and_key()` returns `(&str, Option<&str>)` for building the `QdrantStore`.

3.  **Requests & State (`crates/htui/src/store_worker.rs` and `crates/htui/src/qdrant_dsn_info.rs`)**:
    *   In `StoreRequest`, add `QdrantInfo`, `SetQdrantDsn(QdrantDsn)`, and `ClearQdrantDsn`.
    *   Create a new file `crates/htui/src/qdrant_dsn_info.rs` containing `QdrantSnapshot` with `qdrant_state` (Stored, NotStored, Unreadable) and `qdrant_summary`.
    *   Implement these requests in `store_worker.rs` (calling the new `secret::` functions on blocking threads).

4.  **UI Section (`crates/htui/src/ui/tabs/settings/qdrant.rs`)**:
    *   Create a `QdrantSection` implementing `SettingsSection` (registered in `mod.rs`).
    *   It will display the `Qdrant` status (stored/not stored, summary).
    *   Typing `e` opens a masked `TextField` where the user types the Qdrant URL (and optional API key separated by space).
    *   Typing `c` clears the Qdrant settings.

5.  **Vector Store Integration (`crates/htui-store/src/vector.rs`)**:
    *   Update `QdrantStore::new(collection_name: &str)` to fetch the `QdrantDsn` directly from `secret::get_qdrant_dsn()` (since `QdrantStore::new` has no callers yet, changing signature is safe).
    *   Apply `Qdrant::from_url(url)` and `.api_key(key)` if an API key is present.
    *   If no Qdrant config is found, `QdrantStore::new` can return `StoreError::Backend("no Qdrant DSN stored".into())`.

## Verified Claims Table
| Claim | Verdict | Evidence |
|---|---|---|
| `QdrantStore::new` caller count | Verified | `rg 'QdrantStore::new'` returns 0 callers in `crates/` |
| `QdrantBuilder` accepts API key | Verified | `qdrant-client` 1.19 source has `.api_key(key)` |
| Task independence | Verified | Task 1: `crates/htui-store/src/secret.rs`, `crates/htui-store/src/qdrant_dsn.rs`, `crates/htui-store/src/vector.rs`, `crates/htui-store/src/lib.rs`. Task 2: `crates/htui/src/store_worker.rs`, `crates/htui/src/qdrant_dsn_info.rs`, `crates/htui/src/lib.rs`. Task 3: `crates/htui/src/ui/tabs/settings/qdrant.rs`, `crates/htui/src/ui/tabs/settings/mod.rs`. File sets are disjoint. |

## Tasks (Independent)
1.  **htui-store/secrets**: Implement `secret.rs` additions and `qdrant_dsn.rs` model. Update `QdrantStore::new()` to read these secrets.
2.  **htui/worker**: Implement `StoreRequest` variants and `store_worker.rs` handlers for Qdrant. Create `QdrantSnapshot`.
3.  **htui/ui**: Implement `QdrantSection` and register it in the settings tab.

Status: COMPLETE

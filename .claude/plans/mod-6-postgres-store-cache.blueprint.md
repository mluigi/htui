# Blueprint: MOD-6 Postgres store + cache

**Binding inputs**: `.claude/plans/mod-6-postgres-store-cache.plan.md` (D1-D14, T1-T4, V1-V12),
`docs/ANA-9.md` (design authority: 3 conventions, 4.1-4.4, 5 DDL, 6.1-6.3, 7.1-7.5, 11 criteria),
`docs/REQUIREMENTS.md` (`R-STO-1..6`, `R-ENT-1..12`, `R-USR-2`, `R-NF-2`, `R-NF-3`),
`.claude/plans/mod-1-tui-scaffold.blueprint.md` (the shell this item extends).

**ANA-9 wins over the plan where they disagree.** Decisions the plan does not spell out are marked
**`[blueprint]`** and carry their reason inline. Anything that contradicts ANA-9, the code as it
stands, or the sqlx 0.9.0 API is in H. Errata with the fix. Implementers must not invent anything
outside this file; if something is missing, ask, do not improvise.

Toolchain and crate versions are already verified (plan V1-V12) against rustc 1.98.1 / cargo 1.98.0
on this box: sqlx 0.9.0, keyring 3.6.3, dirs 6.0, gethostname 1.1, sha2 0.10, toml 1.1. **Do not
re-probe, and never cite a sqlx 0.8 API from memory** - every signature below was read out of
`~/.cargo/registry/src/*/sqlx-{core,postgres,sqlite,cli}-0.9.0/`.

Section numbering follows the task brief: A crate/workspace, B migration, C htui-store modules,
D htui changes, E test harness, F .sqlx workflow, G build order, H errata.

---

## A. Crate and workspace

### A.1 File tree (new and changed)

```
Cargo.toml                                   UPDATE  T1  member + workspace deps
.cargo/config.toml                           CREATE  T1  SQLX_OFFLINE
compose.yaml                                 exists (pre-T1)
crates/htui-core/
  Cargo.toml                                 UPDATE  T1  optional sqlx, feature `sqlx`
  src/model/ids.rs                           UPDATE  T1  cfg_attr derives
  src/model/mod.rs                           UPDATE  T1  str_enum! cfg_attr derives
  src/store/conformance.rs                   UPDATE  T1  run_case + iteration-driven run_all
  src/store/backend.rs                       DELETE  T4  moved to htui-store
  src/store/mod.rs, src/lib.rs               UPDATE  T4  drop the Backend re-export
crates/htui-store/
  Cargo.toml                                 CREATE  T1
  migrations/0001_init.sql                   CREATE  T1  ANA-9 sec 5 verbatim, FK order
  cache_migrations/0001_mirror.sql           CREATE  T3  ANA-9 sec 4.4 mirror
  .sqlx/query-*.json                         CREATE  T2, T3  committed offline data
  src/lib.rs                                 CREATE  T1
  src/error.rs                               CREATE  T1
  src/identity.rs                            CREATE  T1
  src/pg/mod.rs                              CREATE  T1
  src/pg/demo.rs                             CREATE  T1  feature `demo`
  src/pg/rows.rs                             CREATE  T2
  src/pg/read.rs                             CREATE  T2
  src/pg/write.rs                            CREATE  T2
  src/cache/mod.rs                           CREATE  T3
  src/cache/read.rs                          CREATE  T3
  src/cache/refresh.rs                       CREATE  T3
  src/cache/pending.rs                       CREATE  T3
  src/secret.rs                              CREATE  T4
  src/connect.rs                             CREATE  T4
  src/backend.rs                             CREATE  T4
  tests/common/mod.rs                        CREATE  T1
  tests/migrations.rs                        CREATE  T1
  tests/pg_conformance.rs                    CREATE  T2
  tests/pg_criteria.rs                       CREATE  T2
  tests/cache.rs                             CREATE  T3
crates/htui/
  Cargo.toml                                 UPDATE  T4  dep htui-store
  src/cli.rs                                 UPDATE  T4  --set-dsn --clear-dsn --offline
  src/lib.rs                                 UPDATE  T4  startup order
  src/store_worker.rs                        UPDATE  T4  select! loop, StoreState, ApplyMigrations
  src/testkit.rs                             UPDATE  T4  Backend import only
  src/app/state.rs, src/app/update.rs        UPDATE  T4  StoreState observe, prompt gate
  src/ui/overlay/migration_prompt.rs         CREATE  T4
  src/ui/overlay/mod.rs, src/app/mod.rs      UPDATE  T4  register the overlay
  tests/shell.rs (+ snapshots)               UPDATE  T4
README.md                                    UPDATE  T4
```

### A.2 `crates/htui-store/Cargo.toml` (exact)

```toml
[package]
name        = "htui-store"
version     = "0.1.0"
description = "Postgres store, per-box SQLite cache and the Backend the TUI holds"

edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
publish.workspace      = true

[features]
default = []
# Forwards to htui-core so `PgStore::load_demo(&DemoData)` and `fixtures::ids` exist.
demo    = ["htui-core/demo"]

[dependencies]
htui-core   = { workspace = true, features = ["sqlx"] }
sqlx        = { workspace = true }
keyring     = { workspace = true }
dirs        = { workspace = true }
gethostname = { workspace = true }
sha2        = { workspace = true }
toml        = { workspace = true }
uuid        = { workspace = true }
chrono      = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tokio       = { workspace = true }
tracing     = { workspace = true }

[dev-dependencies]
htui-core = { workspace = true, features = ["test-support"] }
tokio     = { workspace = true, features = ["test-util", "rt-multi-thread"] }
tempfile  = "3"
futures   = { workspace = true }

[lints]
workspace = true
```

Notes.
- `demo` forwards only to `htui-core/demo`; `pg/demo.rs` is `#[cfg(feature = "demo")]`.
- The dev-dependency on `htui-core/test-support` (which implies `demo`) gives the test targets
  `conformance::{run_all, run_case, CASES}` and `fixtures::ids` without the lib carrying them.
- `tempfile` is the only new dev dependency (temp cache dirs in `tests/cache.rs`); dev-only, so it
  is not a workspace dependency.

### A.3 Workspace `Cargo.toml` additions (exact)

```toml
[workspace]
members  = ["crates/htui-core", "crates/htui", "crates/htui-store"]
resolver = "3"
```

Appended to `[workspace.dependencies]`, keeping the existing alignment:

```toml
htui-store         = { path = "crates/htui-store" }
sqlx               = { version = "0.9.0", default-features = false, features = [
                       "runtime-tokio", "postgres", "sqlite", "migrate", "macros",
                       "chrono", "uuid", "json", "tls-rustls-ring-native-roots"] }
keyring            = { version = "3.6.3", default-features = false, features = [
                       "windows-native", "apple-native", "sync-secret-service", "crypto-rust"] }
dirs               = "6"
gethostname        = "1.1"
sha2               = "0.10"
toml               = "1.1"
```

`default-features = false` on sqlx drops the `any` driver (sqlx's default is
`["any", "macros", "migrate", "json"]`); every feature needed is listed explicitly. The `sqlite`
feature implies `sqlite-bundled`, so no system SQLite is required (`R-NF-2`).

`tokio` gains no new features: `sync` covers `mpsc` / `watch` / `Notify`, `time` covers
`interval` / `sleep`, `rt-multi-thread` is already there.

### A.4 `.cargo/config.toml` (CREATE, T1)

```toml
# sqlx compile-time query checking runs against the committed `crates/htui-store/.sqlx/` data, so
# `cargo build` / `clippy` / `doc` never need a database (plan D2). `cargo sqlx prepare` sets
# SQLX_OFFLINE=false in the child `cargo check` it spawns, and a non-forced `[env]` entry does not
# override an already-set variable, so regeneration still works from here.
[env]
SQLX_OFFLINE = "true"
```

Do **not** write `SQLX_OFFLINE = { value = "true", force = true }`: `force` would win over
sqlx-cli's own `SQLX_OFFLINE=false` and `cargo sqlx prepare` would silently produce nothing
(`sqlx-cli-0.9.0/src/prepare.rs::run_prepare_step`).

### A.5 `crates/htui-core/Cargo.toml` (UPDATE, T1)

```toml
[features]
default      = []
demo         = []
test-support = ["demo"]
sqlx         = ["dep:sqlx"]

[dependencies]
sqlx = { version = "0.9.0", default-features = false, features = [
         "derive", "postgres", "sqlite", "uuid"], optional = true }
```

Reasons in order: `derive` supplies `#[derive(sqlx::Type)]` (in sqlx 0.9 that one derive expands
`Encode` + `Decode` + `Type` together -
`sqlx-macros-core-0.9.0/src/derives/mod.rs::expand_derive_type_encode_decode`); `postgres` and
`sqlite` gate the per-database arms inside that expansion; `uuid` is needed for the
`Uuid: Type<DB>` bound the transparent newtype derive emits. No runtime, no `macros`, no `migrate`
in the domain crate: `htui-core` is written against the sqlx derives only, never against its API,
so nothing else in the crate changes.

Spell the feature `sqlx = ["dep:sqlx"]`, not implicit, so the optional dependency does not also
create a second same-named feature.

### A.6 `crates/htui-core/src/model/ids.rs` - the exact cfg_attr lines (D3)

Inside `macro_rules! id_newtype!`, between `$(#[$meta])*` and `pub struct $name(pub Uuid);`:

```rust
            $(#[$meta])*
            #[derive(
                Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize,
                Deserialize,
            )]
            #[serde(transparent)]
            #[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
            #[cfg_attr(feature = "sqlx", sqlx(transparent))]
            pub struct $name(pub Uuid);
```

`#[sqlx(transparent)]` is not optional for a tuple struct: without it the derive falls through to
the named-type branch and emits `PgTypeInfo::with_name("ItemId")`, which no column matches
(`derives/type.rs::expand_derive_has_sql_type_transparent`, the `if attr.transparent` branch). With
it the impl is `impl<DB: Database> Type<DB> for $name where Uuid: Type<DB>`, plus a
`PgHasArrayType` delegating to the `Uuid` array type - one derive, both databases.

### A.7 `crates/htui-core/src/model/mod.rs` - `str_enum!` (D3)

Two additions to the macro body; nothing else in the file changes:

```rust
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
        #[cfg_attr(feature = "sqlx", sqlx(type_name = "text"))]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                #[serde(rename = $text)]
                #[cfg_attr(feature = "sqlx", sqlx(rename = $text))]
                $variant,
            )+
        }
```

- The per-variant `sqlx(rename = $text)` is driven by the same literal as `serde(rename)` and
  `as_str`, so the database text has exactly one definition per variant (D3). One macro edit covers
  all 15 `str_enum!` sites (V10).
- `#[sqlx(type_name = "text")]` is **required**, and is the fix for a plan gap: an enum with no
  `#[repr]` takes sqlx's *strong enum* path, whose Postgres `Type` impl is
  `PgTypeInfo::with_name(<type_name, defaulting to the Rust ident>)`. Without `type_name = "text"`
  that is `PgTypeInfo::with_name("Status")`, which no `TEXT` column matches: `PgType::eq_impl`
  falls through to `name_eq(self.name(), other.name())`. With it the impl compares equal to
  `PgType::Text`, and the generated `PgHasArrayType` is `PgTypeInfo::array_of("text")`, equal to
  `_text` through the `try_array_element` branch.
- `Encode` / `Decode` for a strong enum go through `&str` on every database (`derives/encode.rs`,
  `derives/decode.rs`), so one derive covers a Postgres `TEXT` column and a SQLite `TEXT` column.
- Do not add `#[sqlx(no_pg_array)]`; the array impl is correct and costs nothing.

### A.8 The one binding rule for every Postgres query **`[blueprint]`**

> **Parameters are bound as primitives; result columns use type overrides.**
>
> - Bind `Uuid` (`id.as_uuid()`), `&str` (`status.as_str()`), `i32`, `i16`, `bool`,
>   `Vec<String>`, `Vec<Uuid>`, `DateTime<Utc>`, `serde_json::Value`. Never bind an ID newtype or
>   a `str_enum!` enum directly.
> - Read them back with the type-override syntax: `id as "id: ItemId"`,
>   `status as "status: Status"`, forcing nullability where a CTE defeats inference:
>   `key as "key!"`, `closed_at as "closed_at?"`.

Reason: `sqlx::query!` type-checks *parameters* against the Postgres-inferred type through
`sqlx::ty_match` (`sqlx-macros-core/src/query/args.rs::quote_args`) and there is no override syntax
for a parameter, only an `expr as _` cast that disables the check entirely. Binding primitives
keeps the check on and keeps every `as _` out of the codebase. Result columns do have override
syntax, generated as `try_get_unchecked::<T, _>`, so they only need `Decode`, which A.7 supplies.

Inside a `WITH ... RETURNING` CTE sqlx marks almost every output column nullable; expect a `!` on
most of them. That is noise, not a smell.

---

## B. `crates/htui-store/migrations/0001_init.sql` (T1)

The DDL **bodies are copied verbatim from ANA-9 section 5** (plus `item_key_counter` from 4.1 and
`session_event` from 4.3) - column for column, `CHECK` list for `CHECK` list, index for index. They
are not reproduced here. What this section fixes is the **order**, which ANA-9 groups by topic and
5.0 only sketches.

### B.1 Table creation order (32 statements, FK order)

| # | Table | Depends on |
|---|---|---|
| 1 | `app_user` | - |
| 2 | `capability_tag` | - |
| 3 | `box` | `app_user` |
| 4 | `box_tool` | `box` |
| 5 | `agent` | - |
| 6 | `agent_box` | `agent`, `box` |
| 7 | `workspace` | `app_user` |
| 8 | `project` | `app_user` |
| 9 | `workspace_project` | `workspace`, `project` |
| 10 | `workspace_box_path` | `workspace`, `box` |
| 11 | `repo` | `project` |
| 12 | `repo_box_path` | `repo`, `box` |
| 13 | `step_graph` | `project` |
| 14 | `step_graph_phase` | `step_graph` |
| 15 | `phase_agent` | `step_graph_phase`, **`agent`** |
| 16 | `prompt_template` | `project`, `app_user` |
| 17 | `item_kind` | `project`, **`step_graph`** |
| 18 | `item_key_counter` | `project` |
| 19 | `item` | `project`, `item_kind`, `step_graph`, `app_user` |
| 20 | `item_revision` | `item`, `app_user`, `box` |
| 21 | `item_link` | `item` (+ deferred `run_step`) |
| 22 | `item_note` | `item`, `app_user`, `box` (+ deferred `run_step`) |
| 23 | `document` | `item`, `app_user` (+ deferred `run_step`) |
| 24 | `skill` | `app_user` |
| 25 | `skill_version` | `skill`, `app_user` |
| 26 | `skill_binding` | `skill`, `project`, `step_graph_phase` |
| 27 | `run` | `project`, `item`, `box`, `app_user` |
| 28 | `run_step` | `run`, `agent` |
| 29 | `run_step_commit` | `run_step`, `repo` |
| 30 | `session_event` | `run_step` |
| 31 | `command_run` | `run_step`, `box` |
| 32 | `app_setting` | - |

Ordering constraints that are easy to get wrong, because ANA-9 declares the two halves apart:

- **`agent` (5) before `phase_agent` (15)**: `phase_agent.agent_id REFERENCES agent(id)`, yet
  ANA-9 declares `agent` in 5.7 and `phase_agent` in 5.4.
- **`step_graph` (13) before `item_kind` (17)**: `item_kind.default_graph_id` is
  `NOT NULL REFERENCES step_graph(id)`.
- **`step_graph_phase` (14) before `skill_binding` (26)**.
- **`repo` (11) before `run_step_commit` (29)**.
- **`run_step` (28) before the three forward FKs** of B.3.

`item.key` (`GENERATED ALWAYS AS (key_prefix || '-' || key_number::text) STORED`),
`item_link.deleted_at` and the `UNIQUE NULLS NOT DISTINCT` on `skill_binding` are copied as
written; the last one is why Postgres 16 is the floor (V11).

Every index of section 5 is created immediately after its own table: `idx_box_tags`,
`idx_workspace_project_project`, `uq_repo_primary`, `idx_item_project_status`,
`idx_item_required_tags`, `idx_item_updated_at`, `idx_item_link_to`, `idx_item_note_item`,
`idx_run_item`, `idx_run_project_status`, `idx_session_event_tool`, `idx_command_run_queue`.

`CREATE FUNCTION set_updated_at()` (section 5.1) goes **first**, before statement 1, so the trigger
loop at the end can reference it.

### B.2 Trigger loop (ANA-9 5.1, uncommented)

Placed after statement 32 and before the `ALTER TABLE`s:

```sql
DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['app_user','box','workspace','project','repo','repo_box_path',
    'workspace_box_path','item_kind','item','item_link','step_graph','step_graph_phase',
    'prompt_template','skill','skill_binding','agent','agent_box','run','run_step','app_setting']
  LOOP EXECUTE format('CREATE TRIGGER trg_%s_updated_at BEFORE UPDATE ON %I
                       FOR EACH ROW EXECUTE FUNCTION set_updated_at()', t, t);
  END LOOP; END $$;
```

Exactly twenty tables - the ones that have an `updated_at` column. `item_note`, `document`,
`item_revision`, `session_event`, `run_step_commit`, `command_run`, `box_tool`, `phase_agent`,
`workspace_project`, `item_key_counter`, `capability_tag` and `skill_version` are append-only or
timestamp-free and are **not** in the list (ANA-9 5.5: notes and documents are append-only, so they
carry `created_at` alone). Do not add any of them.

The trigger is `BEFORE UPDATE` only, so an `INSERT` that supplies an explicit `updated_at` (the
demo loader, C.5) keeps it, while every `UPDATE` gets `clock_timestamp()` - which is what the cache
cursor of 4.4 rides on, and why no write path may set `updated_at` by hand.

### B.3 The three deferred forward references (last three statements)

```sql
ALTER TABLE item_link
    ADD CONSTRAINT fk_item_link_step FOREIGN KEY (proposed_by_step_id)
        REFERENCES run_step(id) ON DELETE SET NULL;

ALTER TABLE item_note
    ADD CONSTRAINT fk_item_note_step FOREIGN KEY (via_step_id)
        REFERENCES run_step(id) ON DELETE SET NULL;

ALTER TABLE document
    ADD CONSTRAINT fk_document_step FOREIGN KEY (produced_by_step_id)
        REFERENCES run_step(id) ON DELETE SET NULL;
```

The three columns are declared in statements 21-23 as plain `UUID` **without** their `REFERENCES`
clause and gain it here. `ON DELETE SET NULL` is ANA-9 5.5 verbatim.

### B.4 What the migration does not do

No seed rows. Seeding is `PgStore::seed_if_empty` (C.4), because the 5.10 set depends on the process
environment (the user name) and must be idempotent across reconnects, which a migration is not.

---

## C. `htui-store` module map

`src/lib.rs`:

```rust
//! Postgres store, per-box SQLite mirror and the `Backend` the TUI holds (`docs/ANA-9.md` 4.4, 6).
#![warn(missing_docs)]

pub mod backend;
pub mod cache;
pub mod connect;
pub mod error;
pub mod identity;
pub mod pg;
pub mod secret;

pub use backend::Backend;
pub use cache::CacheStore;
pub use connect::{ConnEvent, StartOptions, Started};
pub use identity::Identity;
pub use pg::{Connected, MigrationState, PgStore};

/// The embedded Postgres schema (ANA-9 section 5), applied by `PgStore::apply_migrations`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The embedded SQLite mirror schema (ANA-9 section 4.4), applied by `CacheStore::open`.
pub static CACHE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./cache_migrations");
```

Two `migrate!` invocations in one crate are fine (V1, probe printed `pg migrator versions [1],
cache 1`). Paths are relative to `CARGO_MANIFEST_DIR`.

### C.1 `src/error.rs` (T1)

```rust
use htui_core::store::StoreError;

/// Maps a `sqlx::Error` onto the one `StoreError` every store returns.
///
/// SQLSTATE class `23` (integrity constraint violation) is `Constraint`, a missing row is
/// `NotFound`, everything else is `Backend` (ANA-9 6.1, MOD-1 blueprint B.5).
#[must_use]
pub fn map_sqlx(err: sqlx::Error) -> StoreError {
    match err {
        sqlx::Error::RowNotFound => StoreError::NotFound { entity: "row", id: String::new() },
        sqlx::Error::Database(db) => {
            let code = db.code().unwrap_or_default().into_owned();
            let text = match db.constraint() {
                Some(name) => format!("{name}: {}", db.message()),
                None => db.message().to_owned(),
            };
            match code.as_str() {
                // 23000 restrict, 23001 dependent-fields, 23502 not-null, 23503 foreign key,
                // 23505 unique, 23514 check, 23P01 exclusion.
                c if c.starts_with("23") => StoreError::Constraint(text),
                _ => StoreError::Backend(format!("{code}: {text}")),
            }
        }
        other => StoreError::Backend(other.to_string()),
    }
}

/// `map_sqlx` with a caller-supplied entity name for the `NotFound` arm.
#[must_use]
pub fn map_sqlx_for(entity: &'static str, id: impl core::fmt::Display, err: sqlx::Error)
    -> StoreError;
```

`DatabaseError::code() -> Option<Cow<'_, str>>`, `constraint() -> Option<&str>` and
`message() -> &str` are the 0.9 signatures (`sqlx-core-0.9.0/src/error.rs`). Do not use
`ErrorKind`: it collapses `23502` into `NotNullViolation` but leaves `23000`/`23001` as `Other`,
and the SQLSTATE prefix is the rule ANA-9 and the MOD-1 errata name.

`23503` on `item.created_by` / `item_revision.author_id` is what makes the conformance case
`nil_author_rejected` pass on `PgStore` (MOD-1 errata, "authorship").

### C.2 `src/identity.rs` (T1, D6)

```rust
use std::path::{Path, PathBuf};

use htui_core::model::BoxId;
use htui_core::store::Result;

/// This box, as `<config_root>/box.toml` records it (ANA-9 4.4 layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// `box.id`, minted as a UUIDv7 on first launch and never re-minted.
    pub box_id: BoxId,
    /// `box.hostname` as of the last launch; a hostname change does not change `box_id`.
    pub hostname: String,
}

/// `<dirs::config_dir()>/htui`, created if missing.
///
/// `%APPDATA%\htui` on Windows, `~/.config/htui` on Linux,
/// `~/Library/Application Support/htui` on macOS (V4).
pub fn config_root() -> Result<PathBuf>;

/// Reads `<root>/box.toml`, or mints and writes it.
///
/// Minting takes `BoxId::new()` (UUIDv7) and `gethostname()`. An existing file whose `hostname`
/// differs from the current one is rewritten with the new hostname and the **same** `box_id`.
pub fn load_or_mint(root: &Path) -> Result<Identity>;

/// Overwrites `<root>/box.toml`; used by the adopt-DB-id rule of `PgStore::register_box`.
pub fn store(root: &Path, identity: &Identity) -> Result<()>;

/// Lowercase hex sha256 of `host:port/dbname`, the cache directory name (ANA-9 4.4).
///
/// Never includes credentials. The DSN is parsed with `PgConnectOptions::from_str`; `dbname` is
/// `get_database().unwrap_or_default()`. A DSN that does not parse hashes its own text, so a
/// broken DSN still gets a stable, non-colliding directory instead of a panic.
#[must_use]
pub fn db_fingerprint(dsn: &str) -> String;
```

On-disk form, `toml` 1.1 round-trips it (V4):

```toml
box_id = "0199f3c0-1111-7000-8000-000000000000"
hostname = "DESKTOP-JBKR0TS"
```

serialised from a private `#[derive(Serialize, Deserialize)] struct BoxToml { box_id: Uuid,
hostname: String }` through `toml::to_string_pretty` / `toml::from_str`.

Cache directory for a DSN: `config_root()?.join("cache").join(db_fingerprint(dsn))`, holding
`cache.sqlite` and `pending/`.

### C.3 `src/secret.rs` (T4, D7)

```rust
use htui_core::store::Result;

/// Keyring service name; the entry is `("htui", "postgres-dsn")` (D7).
pub const SERVICE: &str = "htui";
/// Keyring user name.
pub const USER: &str = "postgres-dsn";

/// The stored DSN, or `None` when nothing is stored.
///
/// `keyring::Error::NoEntry` is `Ok(None)`, not an error: a first launch has no DSN yet. Every
/// other keyring error is `StoreError::Backend`.
pub fn get_dsn() -> Result<Option<String>>;

/// Stores the DSN, replacing whatever was there.
pub fn set_dsn(dsn: &str) -> Result<()>;

/// Removes the entry. A missing entry is `Ok(())`.
pub fn clear_dsn() -> Result<()>;
```

keyring 3.6.3 API: `Entry::new(SERVICE, USER)?`, `entry.get_password() -> Result<String>`,
`entry.set_password(&str)`, `entry.delete_credential()` (**not** `delete_password`, which is the
2.x name). `Error::NoEntry` is the variant to match (V3).

There is **no env-var fallback in the binary** (`R-STO-1`). `HTUI_TEST_DATABASE_URL` is read by the
test harness only (E), never by `htui-store` at run time. TLS is whatever the DSN's `sslmode=`
says, handled by `PgConnectOptions::from_str` (`R-STO-2`, V1).

### C.4 `src/pg/mod.rs` (T1)

```rust
use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, UserId};
use htui_core::store::Result;
use sqlx::PgPool;

/// The Postgres store: the only `WriteStore` in the product (ANA-9 6.1).
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
    this_box: BoxId,
    this_user: UserId,
}

/// What `connect` found: a usable store plus the state of its schema.
#[derive(Debug, Clone)]
pub struct Connected {
    /// The store, usable for reads even while migrations are pending.
    pub store: PgStore,
    /// Whether the embedded set is fully applied.
    pub migrations: MigrationState,
}

/// The schema check of ANA-9 5.0 / `R-STO-5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// Every embedded migration is applied and its checksum matches.
    UpToDate,
    /// This many embedded migrations are not applied yet.
    Pending(usize),
}
```

```rust
impl PgStore {
    /// Connects, checks the schema, seeds if empty and registers this box.
    ///
    /// Order, and it matters: open the pool, compare `_sqlx_migrations` against `MIGRATOR`
    /// (5.0), and only then - when the state is `UpToDate` - seed and register. With
    /// `Pending(n)` the tables may not exist yet, so seeding is deferred to
    /// `apply_migrations`, which finishes the same three steps.
    ///
    /// # Errors
    ///
    /// `StoreError::Backend("schema is newer than this htui: migration <v> is applied but not
    /// embedded")` when an applied version is not in the embedded set (`MigrateError::
    /// VersionMissing`); `StoreError::Backend("migration <v> was applied with a different
    /// checksum")` on drift (`VersionMismatch`); `Backend` on a dirty version
    /// (`MigrateError::Dirty`). All three refuse rather than repair (D4, `R-STO-5`).
    pub async fn connect(dsn: &str) -> Result<Connected>;

    /// Runs the pending migrations, then seeds and registers as `connect` would have.
    pub async fn apply_migrations(&self) -> Result<()>;

    /// Seeds ANA-9 5.10's global part if it is not there, and returns the single `app_user`.
    ///
    /// Idempotent: every insert is `ON CONFLICT DO NOTHING`. Seeds one `app_user` (name from
    /// `USERNAME` / `USER`, fallback `htui`), the ten `capability_tag` rows with `seeded = true`
    /// (`gpu`, `vulkan`, `msvc`, `mingw`, `clang`, `cmake`, `vcpkg`, `docker`, `rust`,
    /// `heavy_build`), and the `app_setting` defaults `cache_refresh_seconds` = 30 and
    /// `cache_overlap_seconds` = 300. **No `agent` rows** (D5; see H.2).
    pub async fn seed_if_empty(&self) -> Result<UserId>;

    /// Upserts this box on `(user_id, hostname)` and bumps `last_seen_at`.
    ///
    /// Writes what `std::env::consts` and `gethostname` know: `os_family` from
    /// `std::env::consts::OS` mapped onto `OsFamily`, `arch` from `std::env::consts::ARCH`,
    /// `htui_version` from `env!("CARGO_PKG_VERSION")`, `os_version` empty. The full probe
    /// (`box_tool`, tags, RAM, GPU) is MOD-7 and must not be attempted here.
    ///
    /// **Adopt-DB-id rule**: the returned `id` is the row's, which is `identity.box_id` on a
    /// first registration but the *existing* id when this hostname already has a row under a
    /// different id. The caller (`PgStore::connect`) then rewrites `box.toml` through
    /// `identity::store` so the two agree from that point on (D6).
    pub async fn register_box(&self, identity: &Identity) -> Result<BoxId>;

    /// The pool, for the refresh task (`cache::refresh`) and for the tests.
    #[must_use]
    pub fn pool(&self) -> &PgPool;

    /// The highest version in the embedded set: `MIGRATOR.iter().map(|m| m.version).max()`.
    ///
    /// This is the number `cache_meta.schema_version` is compared against (D8).
    #[must_use]
    pub fn schema_version() -> i64;

    /// `box.id` of this box.
    #[must_use]
    pub fn this_box(&self) -> BoxId;

    /// `app_user.id` of the seeded user.
    #[must_use]
    pub fn this_user(&self) -> UserId;
}
```

The migration check, in sqlx 0.9 terms (V8, read from
`sqlx-core-0.9.0/src/migrate/{migrator,migrate,error}.rs`):

```rust
let mut conn = pool.acquire().await.map_err(map_sqlx)?;
// `Migrate::list_applied_migrations` takes the table name in 0.9 (it took none in 0.8).
let applied = conn.list_applied_migrations("_sqlx_migrations").await?;  // Vec<AppliedMigration>
```

- `AppliedMigration { version: i64, checksum: Cow<'static, [u8]> }`.
- An applied `version` that `MIGRATOR.version_exists(version)` denies -> refuse ("schema is newer").
- A version present in both whose `checksum` differs from the embedded `Migration::checksum` ->
  refuse.
- `MigrationState::Pending(MIGRATOR.iter().filter(|m| !m.migration_type.is_down_migration()
  && !applied_versions.contains(&m.version)).count())`, `UpToDate` when that count is zero.
- `apply_migrations` is `MIGRATOR.run(&self.pool).await` mapped through `map_sqlx`; `Migrator::run`
  re-checks and raises `VersionMissing` / `VersionMismatch` / `Dirty` itself, so the two paths
  cannot disagree.
- Pool: `PgPoolOptions::new().max_connections(8).acquire_timeout(Duration::from_secs(10))
  .connect_with(PgConnectOptions::from_str(dsn)?)`. `from_str` accepts `sslmode=` (V1).

### C.5 `src/pg/demo.rs` (T1, feature `demo`)

```rust
#[cfg(feature = "demo")]
impl PgStore {
    /// Loads a `DemoData` fixture with its own ids and timestamps, in FK order.
    ///
    /// One transaction. Explicit ids and explicit `created_at` / `updated_at` on every row, so the
    /// fixture that MOD-1's conformance suite asserts against is byte-identical on Postgres and in
    /// `MemStore` (the `BEFORE UPDATE` trigger of B.2 does not touch an `INSERT`).
    pub async fn load_demo(&self, data: &DemoData) -> Result<()>;
}
```

Insert order and the `DemoData` field that feeds each table (the struct is
`crates/htui-core/src/fixtures.rs:271`, twenty fields):

| # | Table | `DemoData` field | Note |
|---|---|---|---|
| 1 | `app_user` | `users: Vec<AppUser>` | |
| 2 | `box` | `boxes: Vec<BoxRow>` | `cpu`, `ram_mb`, `gpu_*`, `probed_tags`, `declared_tags`, `quirks`, `settings` all come from the row |
| 3 | `workspace` | `workspaces: Vec<Workspace>` | |
| 4 | `project` | `projects: Vec<Project>` | |
| 5 | `workspace_project` | `workspace_projects: Vec<WorkspaceProject>` | |
| 6 | `agent` | `agents: Vec<Agent>` | `launch` and `settings` are `serde_json::Value` |
| 7 | `step_graph` | `graphs: Vec<StepGraph>` | |
| 8 | `step_graph_phase` | `phases: Vec<StepGraphPhase>` | `isolation` is `Option<Isolation>` |
| 9 | `prompt_template` | `templates: Vec<PromptTemplate>` | |
| 10 | `item_kind` | `kinds: Vec<ItemKind>` | after `step_graph`: `default_graph_id` |
| 11 | `item_key_counter` | `item_key_counter: HashMap<(ProjectId, String), i32>` | key -> `(project_id, prefix)`, value -> `last_value` |
| 12 | `item` | `items: Vec<Item>` | never insert `key`: it is `GENERATED ALWAYS` |
| 13 | `item_revision` | `revisions: Vec<ItemRevision>` | |
| 14 | `item_link` | `links: Vec<ItemLink>` | tombstones included, `deleted_at` as recorded |
| 15 | `item_note` | `notes: Vec<Note>` | |
| 16 | `document` | `documents: Vec<Document>` | bodies included |
| 17 | `run` | `runs: Vec<Run>` | |
| 18 | `run_step` | `steps: Vec<RunStep>` | |
| 19 | `session_event` | `events: Vec<SessionEvent>` | |

`DemoData` has **no** `repo`, `repo_box_path`, `workspace_box_path`, `capability_tag`,
`phase_agent`, `agent_box`, `skill*`, `run_step_commit`, `command_run` or `app_setting` field, so
`load_demo` writes none of those tables. Do not invent rows for them.

`this_box`: `data.this_box: Option<BoxId>` is the box the top bar points at. `load_demo` writes it
onto the store - `self.this_box = data.this_box.unwrap_or(self.this_box)` through a `&mut self`
setter or by rebuilding the `PgStore` - and the test harness (E) hands back the updated store, so
`box_info()` answers `DESKTOP-HTUI` exactly as `MemStore::from_demo` does. Likewise
`this_user` becomes `data.users.first().map(|u| u.id)` when the fixture carries one, so
`ids::USER` is the author every conformance case mints under.

Insert style: one multi-row `INSERT ... SELECT * FROM UNNEST($1::uuid[], $2::text[], ...)` per
table where the row count is large (`item`, `session_event`), a plain parameterised `INSERT` in a
loop otherwise. Either is fine; what is not fine is `ON CONFLICT DO NOTHING` - `load_demo` runs
against a database created seconds earlier and a conflict is a bug that must surface.

### C.6 `src/pg/rows.rs` (T2)

`FromRow` structs only where a query cannot decode straight into a model type:

```rust
/// `runs` result row before its steps are attached: `run` plus the joined `box.hostname`.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct RunRow { /* every RunSummary field except `steps` */ }

/// One `run_step` projected for `RunSummary::steps`.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct StepRow { /* every RunStepSummary field, plus `run_id: RunId` for grouping */ }

/// One node of the `links` traversal, with `project_slug` joined and `depth` from the CTE.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct LinkNodeRow { /* LinkNode fields, `depth: i32` narrowed to u8 by the caller */ }
```

Everything else (`Item`, `ItemSummary`, `DocumentHead`, `Note`, `SessionEvent`, `ItemRevision`,
`ProjectRef`, `WorkspaceSummary` pieces, `BoxInfo`) is produced by `query_as!` with column
overrides straight into the model type, so `rows.rs` stays this short.

### C.7 `src/pg/read.rs` (T2) - the seven `ReadStore` methods

`impl ReadStore for PgStore`. Ordering rules are `MemStore`'s, because the conformance suite is the
spec (`crates/htui-core/src/store/mem.rs`).

**`items(scope, filter)`** - every `ItemFilter` field in SQL, ordered by scope position, then
`key_prefix`, then `key_number` (MOD-1 errata, "list rows sort by `(key_prefix, key_number)`"). The
scope order is carried in as an array and recovered with `array_position`:

```sql
SELECT i.id            AS "id: ItemId",
       i.project_id    AS "project_id: ProjectId",
       i.kind_id       AS "kind_id: ItemKindId",
       i.key           AS "key!",
       i.key_prefix, i.key_number, i.title,
       i.status        AS "status: Status",
       i.priority, i.required_tags, i.updated_at
  FROM item i
 WHERE i.project_id = ANY($1)                                  -- scope.project_ids, Vec<Uuid>
   AND ($2::uuid[]  IS NULL OR i.project_id = ANY($2))          -- filter.project_ids
   AND ($3::text[]  IS NULL OR i.status     = ANY($3))          -- filter.statuses, as_str()
   AND ($4::text[]  IS NULL OR i.required_tags @> $4)           -- filter.tags, all of them
   AND ($5::text    IS NULL OR i.key ILIKE '%' || $5 || '%'
                            OR i.title ILIKE '%' || $5 || '%')  -- filter.text
   AND ($6::bool    IS NULL OR $6 = (
            i.status = 'open'
        AND NOT EXISTS (
            SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id
             WHERE l.from_item_id = i.id AND l.kind = 'blocked_by'
               AND l.deleted_at IS NULL AND t.status NOT IN ('done','closed'))))  -- filter.ready
 ORDER BY array_position($1, i.project_id), i.key_prefix, i.key_number;
```

The readiness predicate is ANA-9 7.4's `NOT EXISTS` half plus `status = 'open'`, exactly
`MemStore::State::is_ready`; the capability half stays the caller's `tags` filter. `$6` compares
the predicate to the wanted boolean, so `ready: Some(false)` selects the not-ready items.
`@>` is the `TEXT[]` containment operator and uses `idx_item_required_tags`.

**`item(id)`** - `Option<Item>`, every column of `item` including `key`, `touched_paths`,
`step_graph_id`, `version`, `closed_at`:

```sql
SELECT id AS "id: ItemId", project_id AS "project_id: ProjectId",
       kind_id AS "kind_id: ItemKindId", key_prefix, key_number, key AS "key!",
       title, body, status AS "status: Status", priority, required_tags, touched_paths,
       step_graph_id AS "step_graph_id: StepGraphId", version,
       created_by AS "created_by: UserId", created_at, updated_at, closed_at
  FROM item WHERE id = $1;
```

**`links(id, hops)`** - a recursive CTE over live edges in **both** directions, crossing projects
by UUID, with `depth` and `project_slug`. `hops = 0` returns the root alone; a tombstoned edge is
never traversed and never returned (the conformance case asserts both).

```sql
WITH RECURSIVE walk(item_id, depth) AS (
        SELECT $1::uuid, 0
    UNION
        SELECT CASE WHEN l.from_item_id = w.item_id THEN l.to_item_id ELSE l.from_item_id END,
               w.depth + 1
          FROM walk w
          JOIN item_link l ON (l.from_item_id = w.item_id OR l.to_item_id = w.item_id)
         WHERE l.deleted_at IS NULL AND w.depth < $2::int
),
node AS (SELECT item_id, MIN(depth) AS depth FROM walk GROUP BY item_id)
SELECT n.item_id AS "item_id!: ItemId",
       i.project_id AS "project_id!: ProjectId",
       p.slug   AS "project_slug!",
       i.key    AS "key!",
       i.title  AS "title!",
       i.status AS "status!: Status",
       n.depth  AS "depth!"
  FROM node n
  JOIN item i    ON i.id = n.item_id
  JOIN project p ON p.id = i.project_id
 ORDER BY n.depth, i.key;
```

`UNION` (not `UNION ALL`) plus the `depth < $2` guard is what terminates the walk on a cycle;
`MIN(depth)` per node is the breadth-first depth `MemStore` produces. The root not existing is
`StoreError::NotFound { entity: "item", .. }`, checked by a separate `SELECT 1 FROM item WHERE
id = $1` when the node set comes back empty.

Edges are a second statement over the reached set:

```sql
SELECT l.from_item_id AS "from_item_id: ItemId", l.to_item_id AS "to_item_id: ItemId",
       l.kind AS "kind: LinkKind"
  FROM item_link l
 WHERE l.deleted_at IS NULL
   AND l.from_item_id = ANY($1) AND l.to_item_id = ANY($1);   -- the reached item ids
```

**`documents(id)`** - heads only, **no `body` column in the SELECT list**, ordered `(kind, version)`:

```sql
SELECT id AS "id: DocumentId", item_id AS "item_id: ItemId", kind, version, title,
       produced_by_step_id AS "produced_by_step_id: StepId",
       created_by AS "created_by: UserId", created_at
  FROM document WHERE item_id = $1 ORDER BY kind, version;
```

**`notes(id)`** - ascending `created_at`, with `id` as the tiebreaker so two notes minted in the
same transaction keep a stable order across reads (the conformance case asserts stability):

```sql
SELECT id AS "id: NoteId", item_id AS "item_id: ItemId", body,
       created_by AS "created_by: UserId", box_id AS "box_id: BoxId",
       via_step_id AS "via_step_id: StepId", created_at
  FROM item_note WHERE item_id = $1 ORDER BY created_at, id;
```

**`runs(id)`** - newest run first (`idx_run_item` is `(item_id, queued_at DESC)`), `box_hostname`
joined from the executing box or, when nothing is executing, the target box:

```sql
SELECT r.id AS "id: RunId", r.item_id AS "item_id: ItemId", r.project_id AS "project_id: ProjectId",
       r.kind AS "kind: RunKind", r.mode AS "mode: RunMode", r.status AS "status: RunStatus",
       r.target_box_id AS "target_box_id: BoxId",
       r.executing_box_id AS "executing_box_id: BoxId",
       COALESCE(b.hostname, '') AS "box_hostname!",
       r.queued_at, r.started_at, r.finished_at, r.failure
  FROM run r
  LEFT JOIN box b ON b.id = COALESCE(r.executing_box_id, r.target_box_id)
 WHERE r.item_id = $1
 ORDER BY r.queued_at DESC;
```

and the steps in one second statement, ordered `(position, attempt, fanout_index)`, grouped in Rust
by `run_id`:

```sql
SELECT s.run_id AS "run_id: RunId", s.id AS "id: StepId", s.position, s.attempt, s.fanout_index,
       s.phase_name, s.agent_id AS "agent_id: AgentId", s.model,
       s.status AS "status: StepStatus", s.gate_outcome AS "gate_outcome: GateOutcome",
       s.started_at, s.finished_at
  FROM run_step s
 WHERE s.run_id = ANY($1)
 ORDER BY s.run_id, s.position, s.attempt, s.fanout_index;
```

Two statements, not a `LEFT JOIN` with a row per step: a run with no steps must still appear, and
the `RunSummary` shape is a nested one.

**`step_events(step)`** - ANA-9 7.5 verbatim, plus the columns `SessionEvent` carries.
`Ok(None)` when the step has no rows at all (6.1: `None` = not cached), `Ok(Some(rows))` otherwise:

```sql
SELECT run_step_id AS "run_step_id: StepId", seq, turn,
       kind AS "kind: EventKind", role AS "role: EventRole",
       tool_call_id, payload, raw, at
  FROM session_event WHERE run_step_id = $1 ORDER BY seq;
```

### C.8 `src/pg/read.rs` - the four inherent reads

Not `ReadStore` methods: ANA-9 6.1 is quoted verbatim and has none of them (MOD-1 blueprint B.7).
Same four signatures `MemStore` and `CacheStore` carry, so `Backend` can dispatch over three arms.

```rust
impl PgStore {
    /// Every workspace with its projects, ordered by name then by position.
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>>;
    /// This box's row, projected for the top bar.
    pub async fn box_info(&self) -> Result<Option<BoxInfo>>;
    /// How many runs of the scope are active (`RunStatus::is_active`).
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize>;
    /// The scope's projects, ordered by `workspace_project.position`.
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>>;
}
```

```sql
-- workspaces: one statement, grouped in Rust by workspace_id.
SELECT w.id AS "workspace_id: WorkspaceId", w.slug, w.name,
       p.id AS "project_id?: ProjectId", p.slug AS "project_slug?",
       p.name AS "project_name?", wp.position AS "position?"
  FROM workspace w
  LEFT JOIN workspace_project wp ON wp.workspace_id = w.id
  LEFT JOIN project p            ON p.id = wp.project_id
 ORDER BY w.name, wp.position;

-- box_info
SELECT id AS "box_id: BoxId", hostname, os_family AS "os_family: OsFamily"
  FROM box WHERE id = $1;                                    -- self.this_box

-- active_runs
SELECT COUNT(*) AS "count!" FROM run
 WHERE project_id = ANY($1)
   AND status IN ('queued','running','awaiting_approval');   -- RunStatus::is_active

-- projects
SELECT p.id AS "project_id: ProjectId", p.slug, p.name, wp.position
  FROM workspace_project wp JOIN project p ON p.id = wp.project_id
 WHERE wp.workspace_id = $1 AND p.id = ANY($2)
 ORDER BY wp.position;
```

The `IN (...)` list in `active_runs` is written out rather than derived from `RunStatus::ALL`
because `query!` needs a literal; a unit test asserts the two agree
(`RunStatus::ALL.iter().filter(|s| s.is_active())`).

### C.9 `src/pg/write.rs` (T2) - `impl WriteStore for PgStore`

**`mint_item(new)`** - ANA-9 7.1, one statement, with the kind-belongs-to-project guard folded
into the counter CTE so an invalid kind burns no key number:

```sql
WITH c AS (
    INSERT INTO item_key_counter (project_id, prefix, last_value)
    SELECT $2, k.prefix, 1 FROM item_kind k WHERE k.id = $3 AND k.project_id = $2
    ON CONFLICT (project_id, prefix)
    DO UPDATE SET last_value = item_key_counter.last_value + 1
    RETURNING prefix, last_value
), i AS (
    INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body,
                      priority, required_tags, touched_paths, step_graph_id, created_by)
    SELECT $1, $2, $3, c.prefix, c.last_value, $4, $5, $6, $7, $8, $9, $10 FROM c
    RETURNING id, project_id, kind_id, key_prefix, key_number, key, title, body, status,
              priority, required_tags, touched_paths, step_graph_id, version,
              created_by, created_at, updated_at, closed_at
), r AS (
    INSERT INTO item_revision (item_id, version, title, body, required_tags,
                               author_id, box_id, reason)
    SELECT id, version, title, body, required_tags, $10, $11, 'created' FROM i
    RETURNING item_id
)
SELECT i.id AS "id!: ItemId", i.project_id AS "project_id!: ProjectId",
       i.kind_id AS "kind_id!: ItemKindId", i.key_prefix AS "key_prefix!",
       i.key_number AS "key_number!", i.key AS "key!", i.title AS "title!", i.body AS "body!",
       i.status AS "status!: Status", i.priority AS "priority!",
       i.required_tags AS "required_tags!", i.touched_paths AS "touched_paths!",
       i.step_graph_id AS "step_graph_id?: StepGraphId", i.version AS "version!",
       i.created_by AS "created_by!: UserId", i.created_at AS "created_at!",
       i.updated_at AS "updated_at!", i.closed_at AS "closed_at?"
  FROM i, r;
```

Rules this encodes, each one a conformance case:

- **Kind guard.** An unknown `kind_id`, or one belonging to another project, makes the `SELECT`
  feeding `c` empty, so nothing is inserted anywhere and the outer `SELECT` returns zero rows.
  Zero rows is `StoreError::Constraint("item_kind `<id>` does not exist in project `<id>`")`
  (`mint_unknown_kind_rejected`). ANA-9 5.5 has no composite `(project_id, id)` FK on `item_kind`,
  so this guard has to be written (MOD-1 errata, `B.8 update_item`).
- **No burned number on a refused mint.** The counter upsert is in the same statement as the guard.
- **Prefix from the kind, never from the caller.** `NewItem` has no `key_prefix` field (4.1).
- **Revision 1** with `reason = 'created'` in the same statement, so the 4.2 ancestor always exists.
- **`created_by` nil** is FK error 23503 -> `StoreError::Constraint` (`nil_author_rejected`).
- **Duplicate `item.id`** is 23505 -> `StoreError::Constraint`.
- `r` is joined into the final `SELECT` (`FROM i, r`) purely so Postgres is obliged to execute the
  revision CTE; a CTE whose output nothing references is still executed for a data-modifying
  statement, but the join documents the dependency and costs nothing.

**`update_item(id, expected_version, patch)`** - ANA-9 7.2, extended to all seven spec columns of
4.2 with `COALESCE` patch semantics:

```sql
WITH u AS (
    UPDATE item SET
        title         = COALESCE($3, title),
        body          = COALESCE($4, body),
        kind_id       = COALESCE($5, kind_id),
        required_tags = COALESCE($6, required_tags),
        priority      = COALESCE($7, priority),
        touched_paths = COALESCE($8, touched_paths),
        step_graph_id = CASE WHEN $9 THEN $10 ELSE step_graph_id END,
        version       = version + 1
     WHERE id = $1
       AND version = $2
       AND ($5::uuid IS NULL OR EXISTS (
               SELECT 1 FROM item_kind k
                WHERE k.id = $5 AND k.project_id = item.project_id))
    RETURNING id, project_id, kind_id, key_prefix, key_number, key, title, body, status,
              priority, required_tags, touched_paths, step_graph_id, version,
              created_by, created_at, updated_at, closed_at
), r AS (
    INSERT INTO item_revision (item_id, version, title, body, required_tags,
                               author_id, box_id, reason)
    SELECT id, version, title, body, required_tags, $11, $12, $13 FROM u
    RETURNING item_id
)
SELECT u.* FROM u, r;   -- spelled out with the same overrides as mint
```

Parameter map, in order:

| `$n` | Rust | Note |
|---|---|---|
| `$1` | `id.as_uuid()` | |
| `$2` | `expected_version: i32` | |
| `$3` | `patch.title: Option<String>` | `COALESCE` -> `None` keeps the column |
| `$4` | `patch.body: Option<String>` | |
| `$5` | `patch.kind_id.map(ItemKindId::as_uuid)` | also drives the kind guard |
| `$6` | `patch.required_tags: Option<Vec<String>>` | |
| `$7` | `patch.priority: Option<i16>` | |
| `$8` | `patch.touched_paths: Option<Vec<String>>` | |
| `$9` | `patch.step_graph_id.is_some(): bool` | outer `Option` = "change it" |
| `$10` | `patch.step_graph_id.flatten().map(StepGraphId::as_uuid)` | inner `Option` = the value |
| `$11` | `patch.author_id.as_uuid()` | |
| `$12` | `patch.box_id.map(BoxId::as_uuid)` | |
| `$13` | `patch.reason.as_str()` | |

The **`step_graph_id` double-`Option`** is exactly why `$9` and `$10` are two parameters:
`COALESCE($10, step_graph_id)` cannot express "clear the override back to the kind default",
because `None` and `Some(None)` would both arrive as SQL `NULL`. `CASE WHEN $9` splits them.

Zero rows means one of three things, disambiguated by **one follow-up read** rather than by three
statements up front:

```rust
// zero rows from the CAS
let head = self.item(id).await?;                       // one SELECT
match head {
    None => Err(StoreError::NotFound { entity: "item", id: id.to_string() }),
    Some(head) if head.version != expected_version => {
        let ancestor = /* SELECT ... FROM item_revision WHERE item_id = $1 AND version = $2 */;
        Ok(UpdateOutcome::Diverged { head, ancestor })   // 4.2
    }
    Some(_) => Err(StoreError::Constraint(format!(
        "item_kind `{kind}` does not exist in project `{project}`"))),  // the guard fired
}
```

The version still matching after a zero-row CAS can only mean the kind guard rejected it - which is
`MemStore`'s order of judgements (compare-and-set first, kind check second) and therefore what
`update_kind_keeps_key_and_project` and `update_cas_diverged` expect. `key_prefix`, `key_number`
and `key` are never touched, even across a kind change (4.1).

No explicit transaction is needed: the CTE is one statement and therefore atomic, and under
`READ COMMITTED` the loser of a race blocks on the row lock, re-evaluates `version = $2` against the
committed value and matches nothing - one `Updated`, one `Diverged` (criterion 11.3).

`updated_at` is **not** in the `SET` list: the `BEFORE UPDATE` trigger of B.2 owns it, and
`RETURNING` sees the trigger-modified `NEW` row.

**`transition(id, from, to)`** - the status compare-and-set of 4.2. Never touches `version`, never
writes a revision:

```sql
UPDATE item
   SET status    = $3,
       closed_at = CASE WHEN $3 IN ('done','closed') THEN clock_timestamp() ELSE NULL END
 WHERE id = $1 AND status = $2;
```

`closed_at` follows the *current* status in both directions - set on a move to a terminal status,
**cleared** on a move back to a live one. The one-armed form leaves a stale `closed_at` on a
reopened item; this is the MOD-1 errata rule (`closed_at = to.is_terminal().then_some(now)`) written
in SQL, and `status_cas_keeps_version` carries the reopen leg. `$2` and `$3` are
`from.as_str()` / `to.as_str()`.

`rows_affected() == 1` -> `Ok(true)`. Zero rows -> `SELECT 1 FROM item WHERE id = $1`; no row is
`StoreError::NotFound`, a row is `Ok(false)` (the status did not match), matching `MemStore`.

There is **no `DELETE FROM item`** anywhere in this file, and none may be added: 4.1's "keys are
never reused" is enforced by the absence of the path, and `no_delete_path` is the case that pins it.

### C.10 `src/cache/mod.rs` (T3, D8)

```rust
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use htui_core::store::Result;
use sqlx::SqlitePool;

/// The per-box read-only mirror (ANA-9 4.4). `ReadStore` only: it can never accept a write.
#[derive(Debug, Clone)]
pub struct CacheStore {
    pool: SqlitePool,
    dir: PathBuf,
}

/// What `cache_meta` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMeta {
    /// The Postgres migration version the mirror was built from.
    pub schema_version: i64,
    /// `sha256(host:port/dbname)` of the server this mirror belongs to.
    pub db_fingerprint: String,
    /// When the file was created (or last rebuilt).
    pub built_at: DateTime<Utc>,
    /// When the last full (cursor-at-zero) pass finished; `None` until one has.
    pub last_full_refresh_at: Option<DateTime<Utc>>,
}

impl CacheStore {
    /// Opens `<root>/cache/<fingerprint>/cache.sqlite`, creating and migrating it if needed.
    ///
    /// Creates `<root>/cache/<fingerprint>/` and its `pending/` subdirectory. Connect options:
    /// `create_if_missing(true)`, `journal_mode(SqliteJournalMode::Wal)`,
    /// `busy_timeout(Duration::from_secs(5))`, `foreign_keys(false)` (the mirror is fed in FK
    /// order by one writer and a partial mirror must not refuse a row whose parent is not
    /// mirrored). Then `CACHE_MIGRATOR.run(&pool)`.
    ///
    /// **Rebuild** (delete the file, recreate, re-migrate, empty `cache_cursor`) when
    /// `cache_meta.schema_version` differs from `schema_version`, when `cache_meta.db_fingerprint`
    /// differs from `fingerprint`, or when the file exists but has no `cache_meta` row. A
    /// `last_full_refresh_at` older than seven days does **not** rebuild: it clears
    /// `cache_cursor` so the next pass is a full one (D8).
    pub async fn open(root: &Path, fingerprint: &str, schema_version: i64) -> Result<CacheStore>;

    /// Drops every mirrored row and every cursor, keeping the file and `cache_meta`.
    ///
    /// This is `Settings > Rebuild cache`; the next pass refills from zero.
    pub async fn rebuild(&self) -> Result<()>;

    /// The `cache_meta` row set.
    pub async fn meta(&self) -> Result<CacheMeta>;

    /// `<root>/cache/<fingerprint>`; `dir().join("pending")` is the offline chat buffer (4.3).
    #[must_use]
    pub fn dir(&self) -> &Path;

    /// The pool, for `cache::refresh` (the only writer) and for the tests.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool;
}
```

`CacheStore` is `Clone` (an `SqlitePool` is an `Arc` inside), which is what lets the store worker
move the cache from an `Offline` `Backend` into an `Online` one and hand a second handle to the
refresher.

### C.11 `cache_migrations/0001_mirror.sql` (T3)

The fifteen mirrored tables of ANA-9 4.4 (`app_user`, `box`, `workspace`, `workspace_project`,
`project`, `repo`, `item_kind`, `item`, `item_link`, `item_note`, `document`, `run`, `run_step`,
`run_step_commit`, `session_event`) plus `cache_meta` and `cache_cursor`. Same table names and same
column names as Postgres. Type mapping, 4.4 verbatim: `UUID` -> `TEXT`, `TIMESTAMPTZ` -> `INTEGER`
(microseconds since the epoch), `TEXT[]` -> `TEXT` (a JSON array), `JSONB` -> `TEXT`, `INTEGER` and
`SMALLINT` -> `INTEGER`, `BOOLEAN` -> `INTEGER` (0/1), `TEXT` -> `TEXT`.

```sql
CREATE TABLE cache_meta   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE cache_cursor (project_id TEXT NOT NULL, table_name TEXT NOT NULL,
                           high_water INTEGER NOT NULL,
                           PRIMARY KEY (project_id, table_name));

CREATE TABLE app_user (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, email TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE box (
    id TEXT PRIMARY KEY, user_id TEXT NOT NULL, hostname TEXT NOT NULL,
    os_family TEXT NOT NULL, os_version TEXT NOT NULL, arch TEXT NOT NULL,
    cpu TEXT NOT NULL DEFAULT '', ram_mb INTEGER, gpu_present INTEGER NOT NULL DEFAULT 0,
    gpu_vendor TEXT, htui_version TEXT NOT NULL,
    probed_tags TEXT NOT NULL DEFAULT '[]', declared_tags TEXT NOT NULL DEFAULT '[]',
    quirks TEXT NOT NULL DEFAULT '', settings TEXT NOT NULL DEFAULT '{}',
    registered_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
    last_probed_at INTEGER, updated_at INTEGER NOT NULL);

CREATE TABLE workspace (
    id TEXT PRIMARY KEY, slug TEXT NOT NULL, name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE workspace_project (
    workspace_id TEXT NOT NULL, project_id TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, project_id));
CREATE INDEX idx_cache_workspace_project_project ON workspace_project(project_id);

CREATE TABLE project (
    id TEXT PRIMARY KEY, slug TEXT NOT NULL, name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', secret_provider TEXT, secret_scope TEXT,
    settings TEXT NOT NULL DEFAULT '{}', created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE repo (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, remote_url TEXT,
    default_branch TEXT NOT NULL DEFAULT 'main', is_primary INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_repo_project ON repo(project_id);

CREATE TABLE item_kind (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, prefix TEXT NOT NULL, name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', default_graph_id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_item_kind_project ON item_kind(project_id);

CREATE TABLE item (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, kind_id TEXT NOT NULL,
    key_prefix TEXT NOT NULL, key_number INTEGER NOT NULL, key TEXT NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'open',
    priority INTEGER NOT NULL DEFAULT 0,
    required_tags TEXT NOT NULL DEFAULT '[]', touched_paths TEXT NOT NULL DEFAULT '[]',
    step_graph_id TEXT, version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, closed_at INTEGER);
CREATE INDEX idx_cache_item_project_status ON item(project_id, status);
CREATE INDEX idx_cache_item_order          ON item(project_id, key_prefix, key_number);

CREATE TABLE item_link (
    from_item_id TEXT NOT NULL, to_item_id TEXT NOT NULL, kind TEXT NOT NULL,
    proposed_by_step_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    PRIMARY KEY (from_item_id, to_item_id, kind));
CREATE INDEX idx_cache_item_link_to ON item_link(to_item_id);

CREATE TABLE item_note (
    id TEXT PRIMARY KEY, item_id TEXT NOT NULL, body TEXT NOT NULL, created_by TEXT NOT NULL,
    box_id TEXT, via_step_id TEXT, created_at INTEGER NOT NULL);
CREATE INDEX idx_cache_item_note_item ON item_note(item_id, created_at);

CREATE TABLE document (
    id TEXT PRIMARY KEY, item_id TEXT NOT NULL, kind TEXT NOT NULL, version INTEGER NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL, produced_by_step_id TEXT,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX idx_cache_document_item ON document(item_id, kind, version);

CREATE TABLE run (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, item_id TEXT, kind TEXT NOT NULL,
    mode TEXT NOT NULL, status TEXT NOT NULL, target_box_id TEXT NOT NULL,
    executing_box_id TEXT, graph_snapshot TEXT, started_by TEXT NOT NULL,
    queued_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER, failure TEXT,
    updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_run_item    ON run(item_id, queued_at DESC);
CREATE INDEX idx_cache_run_project ON run(project_id, status);

CREATE TABLE run_step (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL, position INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1, fanout_index INTEGER NOT NULL DEFAULT 0,
    phase_name TEXT NOT NULL, agent_id TEXT, model TEXT, status TEXT NOT NULL,
    gate_outcome TEXT, gate_note TEXT, selected INTEGER, exit_code INTEGER,
    prompt_digest TEXT, trim_record TEXT, usage TEXT, isolation_path TEXT,
    started_at INTEGER, finished_at INTEGER, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_run_step_run ON run_step(run_id, position, attempt, fanout_index);

CREATE TABLE run_step_commit (
    run_step_id TEXT NOT NULL, repo_id TEXT NOT NULL, before_hash TEXT NOT NULL,
    after_hash TEXT, PRIMARY KEY (run_step_id, repo_id));

CREATE TABLE session_event (
    run_step_id TEXT NOT NULL, seq INTEGER NOT NULL, turn INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL, role TEXT NOT NULL, tool_call_id TEXT, payload TEXT NOT NULL,
    raw TEXT, at INTEGER NOT NULL, PRIMARY KEY (run_step_id, seq));
```

Deliberate differences from Postgres, each one required:

- **No `deleted_at` on `item_link`**: a tombstone in Postgres is a *deletion* in the mirror
  (4.4, "the mirror drops the row"). The refresher deletes; the reads never filter.
- **No `CHECK` constraints and no foreign keys.** The mirror is fed by one writer from a database
  that already enforced them, and a partially mirrored parent must not refuse a child row.
- **`item.key` is a plain column**, not generated: it is copied from the server.
- **Indexes are the ones C.12's reads need**, prefixed `idx_cache_` so a grep never confuses them
  with the Postgres ones.
- Not mirrored, and no table for them: revisions, skills, templates, graphs, phases, agents,
  counters, settings, capability tags, command queue (4.4).

### C.12 `src/cache/read.rs` (T3)

`impl ReadStore for CacheStore` - the same seven methods - plus the same four inherent reads
(`workspaces`, `box_info`, `active_runs`, `projects`). **Same ordering rules as C.7 and C.8**: the
11.4 equality criterion compares a mirror read with a Postgres read row for row.

The SQL is the C.7 text with three mechanical substitutions, and nothing else:

1. `ANY($1)` becomes `IN (<n placeholders>)`, built at run time (SQLite has no array type).
2. `i.required_tags @> $4` becomes an `EXISTS` over `json_each(i.required_tags)` per wanted tag.
3. `item_link ... AND l.deleted_at IS NULL` loses the clause: a tombstone is already gone (C.11).

Everything else is identical, including the recursive CTE (SQLite supports `WITH RECURSIVE` and
`UNION` the same way), `ORDER BY kind, version`, `ORDER BY created_at, id`, `ORDER BY queued_at
DESC` and `ORDER BY position, attempt, fanout_index`.

Queries here are **runtime-checked** `sqlx::query` / `sqlx::query_as` with `.bind(..)`, never
`query!` (D2: one crate has one `DATABASE_URL` for the macros, and it is the Postgres one). Nothing
in `cache/read.rs` contributes a `.sqlx/query-*.json` file.

**Decoding helpers** - these are not optional, and each one exists because the sqlx 0.9 SQLite
driver would otherwise get it silently wrong:

```rust
/// TEXT -> a UUID newtype. `Uuid: Type<Sqlite>` is a **BLOB** in sqlx 0.9
/// (`sqlx-sqlite-0.9.0/src/types/uuid.rs`: `Decode` calls `value.blob_borrowed()`), so a TEXT
/// UUID column can never be decoded straight into `Uuid` or into a transparent newtype over it.
fn uuid_col<T: From<uuid::Uuid>>(text: &str) -> Result<T>;

/// INTEGER microseconds -> `DateTime<Utc>`. sqlx 0.9 decodes an INTEGER datetime as **seconds**
/// (`sqlx-sqlite-0.9.0/src/types/chrono.rs::decode_datetime_from_int` -> `timestamp_opt(v, 0)`),
/// and encodes a `DateTime` as RFC-3339 **text**, so both directions must be explicit.
fn ts_col(micros: i64) -> Result<DateTime<Utc>>;
fn ts_bind(at: DateTime<Utc>) -> i64;        // at.timestamp_micros()

/// TEXT holding a JSON array -> `Vec<String>` (`required_tags`, `touched_paths`, `probed_tags`).
fn strings_col(text: &str) -> Result<Vec<String>>;

/// TEXT holding JSON -> `serde_json::Value` (`payload`, `raw`, `settings`, `graph_snapshot`,
/// `trim_record`, `usage`).
fn json_col(text: &str) -> Result<serde_json::Value>;

/// INTEGER 0/1 -> bool.
fn bool_col(v: i64) -> bool;
```

A bad value in any of them is `StoreError::Backend("cache: <column> is not <shape>")`, never a
panic: a corrupt mirror must degrade to an error the TUI can show, and the rebuild path exists for
exactly that. Enums decode natively (`str_enum!` gets a SQLite `Type` impl over `str`, A.7), so
`status`, `kind`, `role`, `os_family` need no helper.

`step_events(step)` on the mirror returns `Ok(None)` for a step outside the cached last N: no rows
in `session_event` is "not cached", which is the 6.1 contract (`None` = not cached) and what D12
names. It is never an error and never an empty `Vec`.

### C.13 `src/cache/refresh.rs` (T3, D9)

```rust
use std::sync::Arc;
use std::time::Duration;

use htui_core::model::ProjectId;
use htui_core::store::Result;
use sqlx::PgPool;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

/// Cursor-pass tuning, read from `app_setting` at connect (ANA-9 4.4, 6.2).
#[derive(Debug, Clone, Copy)]
pub struct RefreshSettings {
    /// `app_setting.cache_refresh_seconds`, default 30 s.
    pub interval: Duration,
    /// `app_setting.cache_overlap_seconds`, default 300 s: the visibility window of 4.4.
    pub overlap: Duration,
    /// `project.settings.cached_transcript_steps`, default 20.
    pub transcript_steps: i64,
}

impl Default for RefreshSettings { /* 30 s, 300 s, 20 */ }

/// What one pass did; logged at `info` and asserted on in `tests/cache.rs`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    /// Rows upserted, per mirrored table, in pass order.
    pub tables: Vec<(&'static str, u64)>,
    /// `item_link` tombstones removed from the mirror.
    pub tombstones: u64,
    /// `session_event` rows copied.
    pub events: u64,
    /// `session_event` rows trimmed for steps outside the last N.
    pub trimmed: u64,
    /// `pending/*.jsonl` files uploaded.
    pub uploaded: usize,
}

/// The one task that writes `cache.sqlite` (6.3).
#[derive(Debug)]
pub struct Refresher {
    handle: JoinHandle<()>,
    wake: Arc<Notify>,
}

impl Refresher {
    /// Spawns the task: a first pass immediately, then one every `settings.interval`.
    ///
    /// `projects` is the scope the store worker updates on every `Items` request; the task reads
    /// `*projects.borrow()` at the top of each pass, so a scope change is picked up on the next
    /// one without a restart. Nothing here ever touches the UI task (`R-NF-3`).
    pub fn spawn(
        pool: PgPool,
        cache: CacheStore,
        projects: watch::Receiver<Vec<ProjectId>>,
        settings: RefreshSettings,
    ) -> Refresher;

    /// The task handle, for tests and for shutdown.
    #[must_use]
    pub fn handle(&self) -> &JoinHandle<()>;

    /// Stops the task. Called when the backend leaves `Online`.
    pub fn abort(&self);

    /// Forces a pass now instead of at the next tick (a `Notify` the loop selects on).
    pub fn trigger(&self);
}

/// One pass, as a free function so `tests/cache.rs` can run it without a task.
pub async fn run_pass(
    pool: &PgPool,
    cache: &CacheStore,
    projects: &[ProjectId],
    settings: &RefreshSettings,
) -> Result<PassReport>;
```

The loop body is `tokio::select! { _ = ticker.tick() => .., _ = wake.notified() => .. }` around
`run_pass`; a failed pass is `tracing::warn!` and the loop continues, because a transient Postgres
error must not kill the mirror.

**`run_pass`, in order** (ANA-9 6.2 verbatim, with the two gaps of H.3 and H.4 closed):

1. **Unscoped, full replace** - `app_user`, `workspace`, `workspace_project`: small tables with no
   usable cursor column (`workspace_project` has no timestamp at all). One transaction per table:
   `DELETE FROM <t>` then insert everything. No `cache_cursor` row is written for them.
2. **Own `box` row**, unscoped: `SELECT ... FROM box WHERE id = $1` (this box), PK upsert.
3. **Per project in `projects`, per table in FK order**, cursor-driven:

| order | table | scope predicate | `ts_col` |
|---|---|---|---|
| 1 | `project` | `id = $p` | `updated_at` |
| 2 | `repo` | `project_id = $p` | `updated_at` |
| 3 | `item_kind` | `project_id = $p` | `updated_at` |
| 4 | `item` | `project_id = $p` | `updated_at` |
| 5 | `item_link` | `from_item_id IN (SELECT id FROM item WHERE project_id = $p) OR to_item_id IN (...)` | `updated_at` |
| 6 | `item_note` | `item_id IN (SELECT id FROM item WHERE project_id = $p)` | **`created_at`** |
| 7 | `document` | `item_id IN (SELECT id FROM item WHERE project_id = $p)` | **`created_at`** |
| 8 | `run` | `project_id = $p` | `updated_at` |
| 9 | `run_step` | `run_id IN (SELECT id FROM run WHERE project_id = $p)` | `updated_at` |
| 10 | `run_step_commit` | `run_step_id IN (SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id WHERE r.project_id = $p)` | **`run_step.updated_at`** (H.3) |

   `ts_col` is `updated_at`, or `created_at` for the two append-only tables (6.2). `run_step_commit`
   has no timestamp column of its own, so it rides its parent step's - the query joins `run_step`
   and selects `s.updated_at AS ts` alongside the commit columns.

   Per `(project, table)`:

```
hw    = cache_cursor[project, table] or 0                     -- INTEGER micros
since = micros_to_utc(hw) - settings.overlap                  -- the 4.4 overlap window
rows  = SELECT <cols> FROM <t> WHERE <scope> AND <ts_col> > $since ORDER BY <ts_col>
sqlite: INSERT INTO <t> (..) VALUES (..) ON CONFLICT (<pk>) DO UPDATE SET <every non-pk column>
cache_cursor[project, table] = max(ts_col of rows)  if rows else hw   -- server time, never local
```

   One SQLite transaction per `(project, table)` batch, so a crash mid-pass leaves whole tables
   consistent and the cursor un-advanced.

   **Tombstones**: the `item_link` fetch selects `deleted_at` too; a row with `deleted_at IS NOT
   NULL` is `DELETE FROM item_link WHERE from_item_id = ? AND to_item_id = ? AND kind = ?` in the
   mirror instead of an upsert, and still advances the cursor (4.4: "removals are tombstones and
   ride the same cursor; the mirror drops the row").

4. **Last-N transcripts**, per project. `n = project.settings.cached_transcript_steps` as an
   integer, falling back to `settings.transcript_steps`:

```sql
SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id
 WHERE r.project_id = $1
 ORDER BY s.finished_at DESC NULLS LAST, s.updated_at DESC
 LIMIT $2;
```

   For each id **not already present** in the mirror's `session_event`, copy every event of that
   step (7.5's column list) and upsert on `(run_step_id, seq)`. Then trim, scoped to this project so
   another project's cached steps survive:

```sql
DELETE FROM session_event
 WHERE run_step_id IN (<every mirrored step of this project>)
   AND run_step_id NOT IN (<the N ids just selected>);
```

5. **`pending/*.jsonl` upload** - `cache::pending::upload_pending(pool, cache.dir(), this_box,
   this_user)`, once per pass, after the tables (6.2's last line, 4.3).
6. **Meta**. `cache_meta.built_at` is written by `CacheStore::open` and never here.
   `cache_meta.last_full_refresh_at` is set to `Utc::now()` only when every `(project, table)`
   cursor was `0` at the top of the pass, i.e. this was a full pull (D8's seven-day rule).

Logging (plan Patterns/Logging): `debug` per table with the row count, `info` once per pass with
the `PassReport`, `warn` on a failed pass.

### C.14 `src/cache/pending.rs` (T3, ANA-9 4.3)

```rust
/// Uploads every `pending/*.jsonl` chat buffer and returns how many files landed.
///
/// One transaction per file: `run`, its `run_step`s and every event, then the file is deleted.
/// A file that fails to parse is left in place and logged at `warn`, never deleted, never fatal.
pub async fn upload_pending(
    pool: &PgPool,
    dir: &Path,                 // the cache dir; the buffer is `dir.join("pending")`
    this_box: BoxId,
    this_user: UserId,
) -> Result<usize>;
```

**File name**: `pending/<project_id>.<run_id>.jsonl` - both UUIDs hyphenated, separated by a dot.
ANA-9 4.3 says `<run_id>.jsonl` only, which cannot work: `run.project_id`, `run.target_box_id` and
`run.started_by` are all `NOT NULL` and the line format holds only `session_event` columns (H.5).
`project_id` comes from the name, `target_box_id` / `started_by` from the two arguments.

**Line shape**: one JSON object per line, exactly the serde form of `htui_core::model::SessionEvent`
- and that is not a coincidence to be re-derived, it is `serde_json::from_str::<SessionEvent>(line)`:

```json
{"run_step_id":"0199...","seq":0,"turn":0,"kind":"prompt","role":"htui",
 "tool_call_id":null,"payload":{"text":"...","digest":"...","sections":[]},
 "raw":null,"at":"2026-09-04T10:11:12Z"}
```

`kind` and `role` are the `str_enum!` texts (4.3's vocabulary), `at` is RFC-3339, `payload` and
`raw` are objects. Lines are appended in `seq` order but the loader sorts by `seq` anyway.

**Synthesis rules** (4.3: "the store inserts `run` (`kind = 'chat'`, `item_id NULL`), `run_step`
(`phase_name = 'chat'`) and the events in one transaction"):

```sql
INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, executing_box_id,
                 started_by, queued_at, started_at, finished_at)
VALUES ($run, $project, NULL, 'chat', 'manual', 'done', $box, $box, $user, $first_at, $first_at,
        $last_at)
ON CONFLICT (id) DO NOTHING;

INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, status,
                      started_at, finished_at)
VALUES ($step, $run, $position, 1, 0, 'chat', 'done', $step_first_at, $step_last_at)
ON CONFLICT (id) DO NOTHING;

INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id, payload, raw, at)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (run_step_id, seq) DO NOTHING;
```

- One `run_step` per **distinct `run_step_id`** in the file, ordered by that step's smallest `seq`
  then its earliest `at`; `position` is the index in that order (0, 1, 2, ...), so the
  `UNIQUE (run_id, position, attempt, fanout_index)` of 5.8 cannot collide.
- `queued_at` / `started_at` are the earliest `at` in the file, `finished_at` the latest.
- **Idempotency** is the three `ON CONFLICT DO NOTHING` clauses on top of
  `session_event`'s `PRIMARY KEY (run_step_id, seq)` (4.3's own argument). Running the upload twice
  inserts nothing the second time and still deletes the file - criterion 11.7.
- The file is deleted **after** the transaction commits, never before. A crash between commit and
  delete re-uploads on the next pass, which is a no-op.

### C.15 `src/connect.rs` (T4, D10)

```rust
/// What the connect task reports back, exactly once per attempt.
#[derive(Debug)]
pub enum ConnEvent {
    /// Connected, schema up to date. The worker swaps in `Backend::Online` and spawns a refresher.
    Online(PgStore),
    /// Connected, but `n` embedded migrations are not applied (`R-STO-5`). The store is usable for
    /// reads; the TUI asks before `apply_migrations`.
    MigrationsPending(PgStore, usize),
    /// The attempt failed: no DSN, unreachable server, refused schema. The text is what the top
    /// bar's status line shows.
    Failed(String),
}

/// How the shell wants to start.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// `--offline`: open the cache, never attempt a connection, no reconnect ticker.
    pub offline: bool,
    /// Overrides the keyring DSN. `None` in the binary; the tests set it.
    pub dsn: Option<String>,
    /// Overrides `identity::config_root()`. `None` in the binary; the tests set it.
    pub root: Option<PathBuf>,
}

/// Everything the store worker needs to own.
#[derive(Debug)]
pub struct Started {
    /// The backend to move into the worker: `Offline` over the opened cache, always.
    pub backend: Backend,
    /// One `ConnEvent` per attempt. Empty and closed when `offline` is set.
    pub events: mpsc::Receiver<ConnEvent>,
    /// The scope the worker publishes on every `Items` request; the refresher reads it.
    pub projects: watch::Sender<Vec<ProjectId>>,
    /// A second handle on the same mirror, for building `Backend::Online`.
    pub cache: CacheStore,
    /// Cursor-pass tuning; the worker hands it to `Refresher::spawn`.
    pub settings: RefreshSettings,
}

/// How often an `Offline` worker retries (D10).
pub const RECONNECT: Duration = Duration::from_secs(30);

/// Opens the cache and starts the connect task.
///
/// Never blocks on the network: the DSN read, the cache open and the return happen on the caller's
/// task; the connection itself is a spawned task that reports through `events` (4.4, "open the
/// cache, render immediately, connect off the UI thread").
///
/// Steps: `identity::config_root` -> `identity::load_or_mint` -> `secret::get_dsn` ->
/// `identity::db_fingerprint(dsn)` (or the literal `"offline"` when there is no DSN) ->
/// `CacheStore::open(root, fingerprint, PgStore::schema_version())` ->
/// `Backend::Offline { cache, since: None }` -> spawn the connect task unless `offline`.
pub async fn start(opts: StartOptions) -> Result<Started>;

/// One connection attempt, spawned by `start` and re-run by the worker's 30 s ticker.
///
/// `secret::get_dsn()? == None` is `ConnEvent::Failed("no DSN stored; run `htui --set-dsn`")`,
/// not an error: a first launch has no DSN and must still open offline.
pub async fn attempt(dsn: Option<String>, root: PathBuf) -> ConnEvent;
```

The refresher is **not** created here. It is created by the store worker on `ConnEvent::Online`
(and after a successful `ApplyMigrations`), because the worker is the only thing that owns both the
`PgStore` and the `CacheStore` at that moment:

```rust
let refresher = Refresher::spawn(
    pg.pool().clone(), cache.clone(), projects.subscribe(), settings);
```

and it is `abort()`ed whenever the backend leaves `Online`.

### C.16 `src/backend.rs` (T4, D1, D12)

Moved from `htui_core::store::backend`; `htui-core` loses the module, the `pub use backend::Backend`
in `store/mod.rs` and the `[store::Backend]` doc link in `src/lib.rs`. Five files outside the module
mention it (V7): `htui/src/{app/state.rs, lib.rs, store_worker.rs, testkit.rs}` and the doc lines in
`htui-core/src/store/{error,mem,traits}.rs`.

```rust
/// The store the application runs against (ANA-9 6.1).
///
/// An enum, not a `Box<dyn ReadStore>`: native `async fn` in traits is not object safe, and a
/// concrete type keeps the spawned worker's futures `Send`-inferable (MOD-1 D2). `WriteStore` is
/// implemented by `PgStore` alone, so an offline write is a **compile error**, not a runtime flag.
#[derive(Debug, Clone)]
pub enum Backend {
    /// Everything in process memory (`--demo`, tests).
    Memory(MemStore),
    /// Postgres reachable: reads go to Postgres, writes are available, the refresher is running.
    Online {
        /// The writable store.
        pg: PgStore,
        /// The mirror, kept warm by the refresher.
        cache: CacheStore,
    },
    /// Postgres unreachable: reads come from the mirror, there is no write path at all.
    Offline {
        /// The mirror.
        cache: CacheStore,
        /// When the process gave up. `None` means "the first attempt has not answered yet".
        since: Option<DateTime<Utc>>,
    },
}

impl Backend {
    /// Wraps an in-memory store.
    #[must_use] pub fn memory(store: MemStore) -> Self;

    /// Store-state text for the top bar (D11).
    ///
    /// - `Memory`            -> `"memory"`
    /// - `Online { .. }`     -> `"online"`
    /// - `Offline { since: None }`      -> `"connecting"`
    /// - `Offline { since: Some(t) }`   -> `"offline · <age>"`, where `<age>` is
    ///   `Utc::now() - t` rendered as `<n>s` under a minute, `<n>m` under an hour, `<n>h`
    ///   otherwise, truncating (so 3 min 40 s reads `offline · 3m`).
    #[must_use] pub fn label(&self) -> String;

    /// Whether write paths are reachable: `Offline` is the first backend to answer `false`.
    #[must_use] pub fn is_writable(&self) -> bool;

    /// The writable store, or `None`. The **only** way a caller reaches `WriteStore` (D1).
    #[must_use] pub fn writable(&self) -> Option<&PgStore>;

    /// The mirror, when there is one; `None` for `Memory`.
    #[must_use] pub fn cache(&self) -> Option<&CacheStore>;

    // The four inherent reads, one `match` arm per variant each (MOD-1 blueprint B.7).
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>>;
    pub async fn box_info(&self) -> Result<Option<BoxInfo>>;
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize>;
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>>;
}

impl ReadStore for Backend { /* seven methods, three arms each */ }
```

Dispatch rule (D12): `Memory` -> `MemStore`; `Online` -> **`pg`**, never the cache (live reads go
to the source of truth while it is reachable, 4.4's last paragraph); `Offline` -> `cache`.

`"connecting"` is carried by `Offline { since: None }` rather than by a fourth variant
**`[blueprint]`**: the plan asks for the label (D10) and for three variants (D1), and "no attempt
has answered yet" is genuinely a property of being offline, not a fourth kind of store. The worker
sets `since = Some(Utc::now())` on the first `ConnEvent::Failed`; `--offline` starts at
`Some(Utc::now())` directly, so it reads `offline · 0s` and never `connecting`.

There is no `Backend::write*` method and no `impl WriteStore for Backend`. A write path takes
`&PgStore` from `writable()` and is unreachable in the other two variants at the type level - which
is the D1 claim, documented in the module header and **not** covered by a test (a compile-fail test
is not worth a `trybuild` dependency).

---

## D. `htui` changes (T4)

### D.1 `store_worker.rs` - new request and reply variants

```rust
pub enum StoreRequest {
    // ... the nine MOD-1 variants, unchanged ...
    /// The top bar's store field and the pending-migration count (D11).
    StoreState,
    /// Apply the pending migrations (`R-STO-5`), after the user answered `y`.
    ApplyMigrations,
}

impl StoreRequest {
    pub const fn name(&self) -> &'static str {
        match self {
            // ...
            Self::StoreState      => "store_state",
            Self::ApplyMigrations => "apply_migrations",
        }
    }
}

pub enum StoreReply {
    // ... the nine MOD-1 variants, unchanged ...
    /// Answer to `StoreRequest::StoreState`.
    StoreState {
        /// `Backend::label()`.
        label: String,
        /// How many migrations are pending; `None` when the question does not apply
        /// (`Memory`, `Offline`, or an `Online` backend whose schema is up to date).
        migrations_pending: Option<usize>,
    },
    /// Answer to `StoreRequest::ApplyMigrations`.
    MigrationsApplied {
        /// How many were pending before the run; `0` when there was nothing to do.
        applied: usize,
    },
    /// The store failed. `request` is `StoreRequest::name`.
    Failed { request: &'static str, message: String },
}
```

Both new replies are addressed like every other one (`seq` + `Origin`, MOD-1 blueprint C.2); the
event loop does not change, and neither does `serve`'s shape - it gains two arms.

### D.2 `store_worker.rs` - the loop (pseudocode)

`serve(&Backend, &StoreRequest) -> StoreReply` stays a pure function and gains two arms; what
changes is `spawn`, which becomes a `select!` over three sources and owns the swap.

```rust
pub fn spawn(
    mut backend: Backend,
    mut rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    mut events: mpsc::Receiver<ConnEvent>,
    projects: watch::Sender<Vec<ProjectId>>,
    settings: RefreshSettings,
    reconnect: Option<Duration>,          // None with --offline and with --demo
) -> JoinHandle<()> {
  tokio::spawn(async move {
    let mut pending: Option<usize> = None;      // migrations pending on the live backend
    let mut refresher: Option<Refresher> = None;
    let mut ticker = interval(reconnect.unwrap_or(FOREVER));

    loop { select! {
      Some(env) = rx.recv() => {
          // Publish the scope so the refresher follows what the user is looking at (D9).
          if let StoreRequest::Items { scope, .. } = &env.request {
              let _ = projects.send_replace(scope.project_ids.clone());
          }
          let reply = match &env.request {
              StoreRequest::StoreState => StoreReply::StoreState {
                  label: backend.label(),
                  migrations_pending: pending,
              },
              StoreRequest::ApplyMigrations => match backend.writable() {
                  Some(pg) => match pg.apply_migrations().await {
                      Ok(()) => {
                          let applied = pending.take().unwrap_or(0);
                          // The schema is now current: start mirroring.
                          refresher = spawn_refresher(&backend, &projects, settings);
                          StoreReply::MigrationsApplied { applied }
                      }
                      Err(err) => failed("apply_migrations", &err),
                  },
                  None => failed_msg("apply_migrations", "this backend is read-only"),
              },
              other => serve(&backend, other).await,
          };
          if tx.send(ReplyEnvelope { seq: env.seq, origin: env.origin, reply }).is_err() { break }
      }

      Some(event) = events.recv() => match event {
          ConnEvent::Online(pg) => {
              let cache = backend.cache().cloned().expect("offline backend owns the mirror");
              backend  = Backend::Online { pg, cache };
              pending  = None;
              refresher = spawn_refresher(&backend, &projects, settings);
              tracing::info!("store online");
          }
          ConnEvent::MigrationsPending(pg, n) => {
              let cache = backend.cache().cloned().expect("offline backend owns the mirror");
              backend  = Backend::Online { pg, cache };
              pending  = Some(n);
              // No refresher yet: the mirror must not be filled from a schema we refuse to use.
              tracing::warn!(pending = n, "schema has pending migrations");
          }
          ConnEvent::Failed(why) => {
              if let Backend::Offline { since: since @ None, .. } = &mut backend {
                  *since = Some(Utc::now());          // "connecting" becomes "offline · 0s"
              }
              tracing::warn!(%why, "connect failed");
          }
      },

      _ = ticker.tick(), if reconnect.is_some() && !backend.is_writable() => {
          let events_tx = /* the sender kept alive next to `events` */;
          tokio::spawn(async move { let _ = events_tx.send(connect::attempt(..).await).await; });
      }
    } }

    if let Some(r) = refresher { r.abort(); }
  })
}
```

Rules the pseudocode encodes:

- The `Backend` is **moved into the worker** and swapped in place; no other task ever holds one
  (MOD-1 D4, `R-NF-3`).
- The reconnect arm is guarded by `!backend.is_writable()`, so an `Online` worker never dials.
- The refresher is spawned exactly on the two transitions that make the schema usable
  (`Online`, and `ApplyMigrations` succeeding) and aborted when the loop ends.
- `projects.send_replace` on **every** `Items` request, not only on a scope change: the send is a
  pointer swap, and a missed update would leave the mirror refreshing the wrong project.
- `--demo` passes `reconnect: None` and an already-closed `events` receiver, so the two new arms
  never fire and the worker behaves exactly as it did in MOD-1.

### D.3 `cli.rs`

```rust
pub struct Args {
    /// Load the demo fixture instead of connecting. Implies offline.
    #[arg(long)] pub demo: bool,

    /// Write logs to this file. Never stdout: stdout is the TUI.
    #[arg(long, env = "HTUI_LOG", value_name = "PATH")] pub log: Option<PathBuf>,

    /// Read a Postgres DSN from stdin, store it in the OS keyring and exit (`R-STO-1`).
    #[arg(long, conflicts_with_all = ["clear_dsn", "demo"])] pub set_dsn: bool,

    /// Remove the stored DSN from the OS keyring and exit.
    #[arg(long, conflicts_with_all = ["set_dsn", "demo"])] pub clear_dsn: bool,

    /// Open from the cache and never attempt a connection (demos, tests).
    #[arg(long)] pub offline: bool,
}
```

`--set-dsn` reads one line from stdin **before the terminal is put into raw mode**, so the DSN never
appears in a shell history and never in `argv`. Echo suppression is not attempted (no extra
dependency); the prompt says so: `paste the DSN and press Enter (it will be visible):`.

### D.4 `lib.rs` startup order

1. `init_tracing(args.log)` - unchanged.
2. `if args.set_dsn` -> read stdin, `secret::set_dsn`, print `DSN stored in the OS keyring.`,
   `return Ok(())`. **Before** any terminal work.
3. `if args.clear_dsn` -> `secret::clear_dsn`, print `DSN removed.`, `return Ok(())`.
4. Build the backend:
   - `args.demo` -> `Backend::memory(MemStore::demo())`, a closed `ConnEvent` channel,
     `reconnect: None`.
   - otherwise -> `connect::start(StartOptions { offline: args.offline, ..Default::default() })`.
5. `let label = started.backend.label();` **before** the move.
6. `store_worker::spawn(started.backend, request_rx, reply_tx, started.events, started.projects,
   started.settings, (!args.offline && !args.demo).then_some(connect::RECONNECT))`.
7. `App::new`, `app.top_bar.store = label`, `app::register_all(&mut app)`, `app.start()`.
8. `terminal::init()`, `event_loop::run`, `term.restore()`, `worker.abort()` - unchanged.

Step 4's failure mode: `connect::start` returning `Err` (no config directory, unwritable cache) is
fatal and surfaces through `anyhow` in `main`. A missing DSN is not an error (C.15).

### D.5 `App` changes (`app/state.rs`, `app/update.rs`)

`TopBarState` is unchanged - `store: String` already exists and its doc comment already promises
`"online"` / `"offline · 3m"`. Three additions:

```rust
// app/state.rs, on App:
/// Whether the migration prompt has already been offered this session, so answering `n` is not
/// re-asked every four ticks.
pub(super) migration_prompt_shown: bool,
```

```rust
// app/state.rs, App::start(): one more read, so the top bar is right on the first frame.
self.dispatch(Origin::App, StoreRequest::Workspaces);
self.dispatch(Origin::App, StoreRequest::BoxInfo);
self.dispatch(Origin::App, StoreRequest::StoreState);
```

```rust
// app/update.rs, App::on_tick(): the same fourth tick that re-reads the run count.
if self.ticks % TICKS_PER_REFRESH == 0 {
    self.dispatch(Origin::App, StoreRequest::StoreState);   // NOT gated on the scope
    if !self.scope.is_empty() {
        self.dispatch(Origin::App, StoreRequest::ActiveRuns { scope: self.scope.clone() });
    }
    self.dirty = true;
}
```

`StoreState` is not gated on a non-empty scope: `offline · 3m` must keep ticking on an empty shell.

```rust
// app/update.rs, App::observe_reply(): one more arm.
StoreReply::StoreState { label, migrations_pending } => {
    self.top_bar.store = label.clone();
    if let Some(n) = migrations_pending
        && *n > 0
        && !self.migration_prompt_shown
        && let Some(id) = /* MigrationPrompt::ID */
        && !self.overlays.iter().any(|o| o.id() == id)
    {
        self.migration_prompt_shown = true;
        self.update(Action::Overlay(OverlayAction::Open(id)));
    }
}
StoreReply::MigrationsApplied { applied } => {
    self.status = Some(format!("applied {applied} migration(s)"));
}
```

`observe_reply` is the read-only top-bar hook of MOD-1 blueprint C.2 and runs **above** the
staleness gate, which is right here: the label must follow the newest answer whatever it addresses.
Opening the overlay from `observe_reply` is the one exception to "read-only" and is why
`migration_prompt_shown` exists - one prompt per session, and `n` closes it for good.

`App` must not name `MigrationPrompt` directly (MOD-1 keeps concrete views out of the shell). The
id arrives the same way the switcher's does: `register_all` sets a new field
`pub migration_overlay: Option<OverlayId>` next to `startup_overlay`, and `observe_reply` reads it.

### D.6 `ui/overlay/migration_prompt.rs` (T4)

Shaped exactly like `workspace_switcher.rs`.

```rust
/// The `R-STO-5` confirmation: pending migrations are never applied without a human (D11).
#[derive(Debug, Default)]
pub struct MigrationPrompt {
    /// From the `StoreState` reply; `0` while it has not landed.
    pending: usize,
    /// Whether a reply has landed: "reading the store" and "0 pending" are different screens.
    loaded: bool,
}

impl MigrationPrompt {
    /// Identity of the prompt; `register_all` registers the factory under it.
    pub const ID: OverlayId = OverlayId("migration_prompt");
    #[must_use] pub fn new() -> Self;
}

impl Overlay for MigrationPrompt {
    fn id(&self) -> OverlayId { Self::ID }
    fn title(&self) -> &str { "Schema" }
    fn is_modal(&self) -> bool { true }
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> { vec![StoreRequest::StoreState] }
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled { /* below */ }
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) { /* StoreState -> pending */ }
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) { /* centred box */ }
}
```

`wants_requests` returning `StoreState` is how the overlay learns the count without the shell
handing it constructor arguments - the factory signature is `Fn() -> Box<dyn Overlay>` and must
stay that way.

Key handling:

| Key | Effect |
|---|---|
| `y` | `ctx.emit(Action::Store(StoreRequest::ApplyMigrations))` then `Action::Overlay(OverlayAction::Close)`; `Handled::Consumed` |
| `n` | `Action::Overlay(OverlayAction::Close)`; `Handled::Consumed`. The backend stays as it is (schema pending, no refresher) |
| `Esc` | `Handled::Pass` - it falls through to the wildcard `OverlayId::ANY` binding that already closes overlays; same as `n` in effect |
| anything else | `Handled::Pass`, swallowed by `is_modal` |

Body text: `3 schema migrations are pending. Apply them now?` (singular `1 schema migration is
pending.`), then a blank line, then `y apply · n / Esc stay offline`. Never a blank box (MOD-1 D11).

`register_all` gains three lines next to the switcher's:

```rust
app.overlay_factories.register(MigrationPrompt::ID, || Box::new(MigrationPrompt::new()));
app.migration_overlay = Some(MigrationPrompt::ID);
```

and `ui/overlay/mod.rs` gains `pub mod migration_prompt;` plus
`pub use migration_prompt::MigrationPrompt;`. No global key binding: the prompt is opened by the
shell, not by the user.

### D.7 `testkit.rs` - behaviour unchanged

The only edit is the import:

```rust
use htui_core::store::MemStore;
use htui_store::Backend;
```

`Harness::over(store)` still builds `Backend::memory(store)`, `settle()` still calls
`store_worker::serve(&self.backend, ..)` inline, and `top_bar.store` still reads `"memory"`. No
harness gains a Postgres or a cache path: every MOD-1 snapshot must stay byte-identical, and the
existing assertion `assert_eq!(top_bar.store, "memory")` is the guard.

`crates/htui/Cargo.toml` gains `htui-store = { workspace = true }`. The `testkit` feature does not
change.

### D.8 `store/conformance.rs` - iteration-driven (T1)

`CASES` keeps its fifteen names, in order, and no name changes: they are the suite's API and MOD-6
reports per case.

```rust
/// Runs one case by name against an already-loaded store.
///
/// **Panics** - on the first failed assertion, naming the case, and on an unknown `name`. The
/// return type is `()` and not `Result<(), String>` deliberately: the cases are written as
/// `assert_eq!` chains that carry their own messages, and turning fifteen bodies into
/// error-returning functions would rewrite the whole suite to gain a string the panic already
/// prints. `pg_conformance.rs` gets its per-case reporting from the loop, not from a `Result`.
pub async fn run_case<S: WriteStore>(name: &str, store: &S) {
    match name {
        "mint_consecutive_keys"            => mint_consecutive_keys(store).await,
        "mint_prefix_isolation"            => mint_prefix_isolation(store).await,
        "mint_writes_revision_v1"          => mint_writes_revision_v1(store).await,
        "mint_unknown_kind_rejected"       => mint_unknown_kind_rejected(store).await,
        "update_cas_success"               => update_cas_success(store).await,
        "update_kind_keeps_key_and_project"=> update_kind_keeps_key_and_project(store).await,
        "update_cas_diverged"              => update_cas_diverged(store).await,
        "status_cas_keeps_version"         => status_cas_keeps_version(store).await,
        "no_delete_path"                   => no_delete_path(store).await,
        "links_hops_1_vs_2"                => links_hops_1_vs_2(store).await,
        "filter_status_project_tags_ready" => filter_status_project_tags_ready(store).await,
        "documents_ordered_by_version"     => documents_ordered_by_version(store).await,
        "notes_ordered_by_created_at"      => notes_ordered_by_created_at(store).await,
        "events_ordered_by_seq"            => events_ordered_by_seq(store).await,
        "nil_author_rejected"              => nil_author_rejected(store).await,
        other => panic!("unknown conformance case `{other}`; CASES and run_case disagree"),
    }
}

/// Runs every case in `CASES`, each against a store `make` produced fresh.
pub async fn run_all<S, F, Fut>(make: F)
where S: WriteStore, F: Fn() -> Fut, Fut: Future<Output = S>
{
    for name in CASES {
        run_case(name, &make().await).await;
    }
}
```

The fifteen case functions themselves are untouched. The existing
`case_names_are_unique` unit test stays; add one that `run_case` accepts every name in `CASES`
(it panics on a typo, which is the whole point of the `match`).

---

## E. Test harness `crates/htui-store/tests/common/mod.rs` (T1, D13)

```rust
/// A throwaway database and the store that owns it.
pub struct TestDb {
    /// A `PgStore` connected to the fresh database, migrated and box-registered.
    pub store: PgStore,
    /// The same pool, for raw assertions and for a second connection in the race tests.
    pub pool: PgPool,
    /// `htui_test_<12 hex>`.
    pub name: String,
    /// The maintenance DSN the database was created through, for `drop_db`.
    pub maint_url: String,
}

/// A fresh, migrated, empty database - or `None` when `HTUI_TEST_DATABASE_URL` is not set.
///
/// Steps: read `HTUI_TEST_DATABASE_URL`; connect a single `PgConnection` to it; `CREATE DATABASE
/// "htui_test_<12 hex>"` (12 hex characters of `Uuid::now_v7().simple()`, so the name is short,
/// unique and a legal identifier); rewrite the DSN's path to the new name; `PgStore::connect`;
/// `apply_migrations`. The maintenance connection is closed before `PgStore::connect` runs.
///
/// Prints `skipped: HTUI_TEST_DATABASE_URL not set` and returns `None` when the variable is
/// absent, so `cargo test --workspace --all-features` is green on a box without Postgres (D13).
pub async fn fresh_db() -> Option<TestDb>;

/// `fresh_db` plus `store.load_demo(&fixtures::demo_data())`.
pub async fn demo_db() -> Option<TestDb>;

impl TestDb {
    /// Closes the pool and drops the database. Every test calls this on its last line.
    pub async fn drop_db(self);
}
```

Skip message, byte for byte, so a reader of `cargo test -- --nocapture` recognises it:

```
skipped: HTUI_TEST_DATABASE_URL not set
```

**Drop strategy (both halves, deliberately)**:

1. **`TestDb::drop_db(self).await` at the end of every test** is the normal path: it takes `self`,
   `self.pool.close().await`, opens a maintenance connection and runs
   `DROP DATABASE IF EXISTS "<name>" WITH (FORCE)` (Postgres 13+; `WITH (FORCE)` terminates any
   connection the test leaked). This is the only path that reports an error.
2. **`impl Drop for TestDb`** is the panic net: a test that fails mid-way never reaches its last
   line, so `drop` spawns a **detached, best-effort** cleanup and ignores every error -
   `std::thread::spawn` with its own single-threaded `tokio::runtime::Runtime` (the ambient runtime
   may already be shutting down, so `tokio::spawn` from `Drop` is not reliable), running the same
   `DROP DATABASE ... WITH (FORCE)`. It never blocks the test thread beyond the join of that thread
   and never panics.

`drop_db` sets a private `dropped: bool` so the `Drop` impl is a no-op on the normal path; a
leaked database is therefore only possible if the whole process is killed, and the `htui_test_`
prefix plus the README note is the manual cleanup story (plan risk row).

`fresh_db` uses a raw `CREATE DATABASE` on a maintenance connection rather than
`Postgres::create_database(url)` (`sqlx::migrate::MigrateDatabase`) because the tests need the
maintenance DSN afterwards for the drop, and because two tests racing on `create_database` against
the same URL is an avoidable failure mode.

Every test file opens with `mod common;` and the guard

```rust
let Some(db) = common::fresh_db().await else { return };
```

### E.1 `tests/migrations.rs` (T1) - ANA-9 criterion 1

| test fn | asserts |
|---|---|
| `migrations_apply_on_a_clean_database` | after `apply_migrations`, `_sqlx_migrations` holds every embedded version and `information_schema.tables` holds all 32 table names of B.1 |
| `a_second_run_is_a_no_op` | `apply_migrations` twice leaves the same `_sqlx_migrations` row count and no error |
| `connect_reports_up_to_date_after_apply` | `PgStore::connect` on the migrated database answers `MigrationState::UpToDate` |
| `connect_reports_pending_on_a_bare_database` | `connect` before `apply_migrations` answers `MigrationState::Pending(1)` |
| `a_newer_applied_version_is_refused` | inserting a fake `_sqlx_migrations` row with `version = 9999` makes `connect` return `StoreError::Backend` whose text contains `schema is newer` |
| `a_checksum_mismatch_is_refused` | corrupting the stored `checksum` of version 1 makes `connect` return `StoreError::Backend` |
| `the_trigger_bumps_updated_at_on_update_only` | an inserted `item` keeps its explicit `updated_at`; an `UPDATE` moves it forward |
| `seed_is_idempotent` | `seed_if_empty` twice yields one `app_user`, ten `capability_tag` rows, two `app_setting` rows and the same `UserId` |
| `register_box_upserts_and_adopts` | two `register_box` calls with the same hostname and different `box_id` values return the **same** id, and `box.toml` is rewritten to it (D6) |
| `load_demo_round_trips_a_count_per_table` | after `demo_db()`, `COUNT(*)` per table equals the `DemoData` vector length, table by table |
| `box_toml_mint_is_stable_across_two_reads` | `load_or_mint` twice on a temp root returns the same `box_id` (no database needed; this one does not skip) |

### E.2 `tests/pg_conformance.rs` (T2)

| test fn | asserts |
|---|---|
| `pg_store_conformance` | for every `name` in `conformance::CASES`, a **fresh** `demo_db()` and `conformance::run_case(name, &db.store).await`; the loop prints `case <name>` before each one, so a panic names the case in the output as well as in the message. Fifteen databases, dropped as it goes. |
| `case_list_matches_mem_store` | `conformance::CASES.len() == 15`, the same guard `crates/htui-core/tests/mem_store.rs` carries |

### E.3 `tests/pg_criteria.rs` (T2) - ANA-9 criteria 2, 3, 4

| test fn | asserts |
|---|---|
| `concurrent_mints_produce_consecutive_numbers` | two independent pools mint 50 items each under the same `(project, prefix)`; the 100 `key_number` values are exactly `1..=100`, no gap, no duplicate (criterion 2) |
| `a_rolled_back_mint_leaves_no_gap` | mint inside a transaction, roll it back, mint again: the second mint reuses the number the first one took (`item_key_counter.last_value` went back) and no `item` row was left behind |
| `the_importer_variant_keeps_the_counter_above_max` | `ON CONFLICT ... GREATEST(last_value, $n)` with an explicit `key_number = 500` leaves `last_value >= 500`, and the next ordinary mint yields 501 (criterion 2's third clause; the importer statement itself is MOD-8, this test pins the counter rule) |
| `two_edits_from_one_version_diverge_exactly_once` | two `update_item` calls with `expected_version = v` on two pools: exactly one `Updated`, one `Diverged`, and `ancestor.version == v` (criterion 3) |
| `a_diverged_edit_writes_no_revision` | after the race, `item_revision` holds exactly `v + 1` rows |
| `status_cas_never_bumps_version` | `transition` moves `status` and `closed_at` and leaves `version` alone; a move back from `done` to `open` clears `closed_at` |
| `five_thousand_events_replay_in_seq_order` | insert 5 000 `session_event` rows for one step in shuffled insert order; `step_events` returns them ordered `0..5000` with identical `payload` content (criterion 4, Postgres half) |

### E.4 `tests/cache.rs` (T3) - ANA-9 criteria 4, 6, 7

Each test takes a `demo_db()` plus a `tempfile::TempDir` root.

| test fn | asserts |
|---|---|
| `a_pass_mirrors_the_demo_projects` | one `run_pass` over the demo scope leaves the mirror's row counts equal to Postgres, table by table |
| `mirror_reads_equal_postgres_reads` | `items`, `item`, `links`, `documents`, `notes`, `runs` return **equal values** from `CacheStore` and `PgStore` for the demo fixture, ordering included (this is what pins C.12's "same ordering rules") |
| `five_thousand_events_replay_equal_to_postgres` | criterion 4, mirror half: the same 5 000 events, mirrored, compare equal to the Postgres read |
| `a_long_transaction_is_caught_by_the_overlap` | criterion 6 with `overlap = 2 s`: open a transaction, `UPDATE item`, run a pass, commit, run another pass; the second pass has the row |
| `a_tombstoned_link_disappears_from_the_mirror` | mirror a live link, tombstone it in Postgres, run a pass: the mirror row is gone and `links` no longer returns the edge |
| `steps_beyond_n_lose_their_events` | with `transcript_steps = 1`, `step_events` is `Some` for the newest step and `None` for the older one |
| `a_schema_version_change_rebuilds` | write `cache_meta.schema_version = 0`, reopen: the file is rebuilt, every mirrored table is empty and `cache_cursor` is empty |
| `a_fingerprint_change_rebuilds` | same, through `db_fingerprint` |
| `rebuild_clears_rows_but_keeps_the_file` | `CacheStore::rebuild` empties the tables and the cursors and leaves `cache_meta` |
| `pending_upload_lands_ordered` | criterion 7: a `pending/<project>.<run>.jsonl` with 20 events lands as one `run` (`kind = 'chat'`), one `run_step` (`phase_name = 'chat'`) and 20 events in `seq` order; the file is gone |
| `pending_upload_is_idempotent` | criterion 7's second clause: re-creating the same file and uploading again inserts nothing new (`COUNT(*)` unchanged) and still deletes the file |
| `a_malformed_pending_file_is_left_alone` | a file with one unparsable line is not deleted, nothing is inserted, and `upload_pending` returns `Ok(0)` |

Criterion 5 (warm start under one second) has no test: it is a manual measurement on this box,
recorded in the close-out (plan Validation, 11 mapping).

### E.5 `crates/htui/tests/shell.rs` additions (T4)

| test fn | asserts |
|---|---|
| `the_top_bar_shows_the_offline_age` | an `App` fed a synthetic `StoreReply::StoreState { label: "offline · 3m".into(), migrations_pending: None }` renders `<ws> · <box> · offline · 3m · N runs`; snapshot `shell__offline_label` |
| `a_pending_migration_opens_the_prompt` | the same, with `migrations_pending: Some(3)`, opens `MigrationPrompt`; snapshot `shell__migration_prompt` |
| `answering_n_does_not_reopen_the_prompt` | `n` closes it and a second `StoreState` reply with `Some(3)` does not reopen it (`migration_prompt_shown`) |
| `answering_y_emits_apply_migrations` | `y` puts a `StoreRequest::ApplyMigrations` on the request channel and closes the overlay |

These go through `Harness` and a hand-fed `Action::Reply`, so they need no database. Worker-level
unit tests for the swap (`Offline` -> `Online` on a fake `ConnEvent`, and `StoreState` reporting the
new label) live in `crates/htui/src/store_worker.rs`'s own `mod tests`.

---

## F. `.sqlx` workflow

`crates/htui-store/.sqlx/` is **committed**. It is what makes `cargo build`, `cargo clippy` and
`cargo doc` hermetic (D2, V12); a query changed without regenerating it fails the build with
`set DATABASE_URL to use query macros online, or run cargo sqlx prepare`.

Install the CLI once, if it is not on `PATH`:

```bash
cargo install sqlx-cli --no-default-features --features postgres,sqlite
```

The prepare database must have the migrations applied **first** - `sqlx prepare` type-checks every
query against a live schema, and an empty database fails on the first `FROM item`:

```bash
docker compose up -d
psql postgres://postgres:htui@localhost:5433/postgres -c 'CREATE DATABASE htui_sqlx;'
DATABASE_URL=postgres://postgres:htui@localhost:5433/htui_sqlx \
  cargo sqlx migrate run --source crates/htui-store/migrations
```

Then regenerate, **from inside the crate directory**:

```bash
cd crates/htui-store
DATABASE_URL=postgres://postgres:htui@localhost:5433/htui_sqlx \
  cargo sqlx prepare -- --all-targets --all-features
cd ../..
git add crates/htui-store/.sqlx
```

Notes, each one checked against `sqlx-cli-0.9.0/src/{opt,prepare}.rs`:

- **`cargo sqlx prepare` has no `-p` flag.** Its options are `--check`, `--all`, `--workspace`,
  `--database-url/-D`, `--connect-timeout`, `--config`, and everything after `--` is forwarded to
  the `cargo check` it spawns. The plan's `cargo sqlx prepare -p htui-store -- ...` does not parse
  (H.6). Running from the crate directory is what scopes it, because the output directory is
  `manifest_dir()/.sqlx`.
- Do **not** pass `--workspace`: that writes a single `.sqlx` at the repo root, and the plan's file
  table, the task split (G) and `.gitignore` all assume `crates/htui-store/.sqlx`.
- `--all-targets --all-features` is required so the queries inside `#[cfg(test)]`, inside
  `tests/*.rs` and inside `#[cfg(feature = "demo")] pg/demo.rs` are prepared too.
- `prepare` sets `SQLX_OFFLINE=false` in the child `cargo check`, which beats the non-forced
  `[env]` entry of A.4. Do not add `force = true` there.
- `cargo sqlx prepare --check` is the CI form; every task's validate step runs the plain form and
  then `SQLX_OFFLINE=true cargo clippy -p htui-store --all-targets --all-features -- -D warnings`.
- Only Postgres queries produce `.sqlx` files. The SQLite mirror is runtime-checked (`sqlx::query`
  with `.bind`), so `cache/read.rs` and the SQLite half of `cache/refresh.rs` add none (D2).
- `.sqlx/query-<hash>.json` files are per query hash and additive, so T2 and T3 both writing there
  merges cleanly (V6). T4 runs one final `prepare` to garbage-collect stale entries.

---

## G. Build order and task boundaries

Waves: **A = {T1}** serial, then **B = {T2 || T3}** in parallel worktrees, then **C = {T4}** serial.
TDD per task: the tests named in E first, then the code.

```
T1 ──► T2 ──┐
   └──► T3 ──┴──► T4
```

### T1 - crate, migration, connect, seed, identity, demo loader, test harness

**Owns**: `Cargo.toml`, `.cargo/config.toml`, `crates/htui-core/Cargo.toml`,
`crates/htui-core/src/model/{ids,mod}.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-store/{Cargo.toml, migrations/0001_init.sql}`,
`crates/htui-store/src/{lib,error,identity}.rs`, `crates/htui-store/src/pg/{mod,demo}.rs`,
`crates/htui-store/tests/{common/mod.rs, migrations.rs}`.

**Exposes to T2/T3**: `MIGRATOR`, `CACHE_MIGRATOR`, `PgStore` (incl. `pool()`, `this_box()`,
`this_user()`, `schema_version()`, `load_demo`), `Connected`, `MigrationState`, `error::map_sqlx`,
`identity::{Identity, config_root, load_or_mint, store, db_fingerprint}`,
`tests/common::{TestDb, fresh_db, demo_db}`, `conformance::{run_all, run_case, CASES}`.

**Validate**: `cargo build -p htui-store`,
`cargo clippy -p htui-store --all-targets --all-features -- -D warnings`,
`HTUI_TEST_DATABASE_URL=... cargo test -p htui-store --all-features --test migrations`.

### T2 - PgStore reads and writes (parallel with T3)

**Owns**: `crates/htui-store/src/pg/{rows,read,write}.rs`,
`crates/htui-store/tests/{pg_conformance,pg_criteria}.rs`, plus the `.sqlx/query-*.json` its
queries generate.

**Validate**: `cargo sqlx prepare` per F, then
`HTUI_TEST_DATABASE_URL=... cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria`
and clippy with `SQLX_OFFLINE=true`.

### T3 - CacheStore, refresh, pending upload (parallel with T2)

**Owns**: `crates/htui-store/cache_migrations/0001_mirror.sql`,
`crates/htui-store/src/cache/{mod,read,refresh,pending}.rs`, `crates/htui-store/tests/cache.rs`,
plus the `.sqlx/query-*.json` its Postgres-side fetches generate.

**Validate**: `cargo sqlx prepare` per F, then
`HTUI_TEST_DATABASE_URL=... cargo test -p htui-store --all-features --test cache` and clippy.

**T2 and T3 must not edit a single T1 file.** Their file sets are disjoint from each other and from
T1's (V6); `src/pg/mod.rs`, `src/pg/demo.rs` and `tests/common/mod.rs` are **read-only** for both,
and `src/lib.rs` already declares every module either will need. The one shared directory is
`.sqlx/`, where both may **add** `query-*.json` files - one file per query hash, never a shared
file, so the two worktrees cherry-pick cleanly. If T2 or T3 believes it needs a change in a T1
file, that is a hand-back to the maintainer, not an edit.

### T4 - Backend, keyring, shell wiring, README

**Owns**: `crates/htui-store/src/{backend,secret,connect}.rs`,
`crates/htui-core/src/store/backend.rs` (DELETE), `crates/htui-core/src/store/mod.rs`,
`crates/htui-core/src/lib.rs`, `crates/htui/Cargo.toml`,
`crates/htui/src/{cli,lib,store_worker,testkit}.rs`, `crates/htui/src/app/{mod,state,update}.rs`,
`crates/htui/src/ui/overlay/{mod,migration_prompt}.rs`, `crates/htui/tests/shell.rs` and its
snapshots, `README.md`, and one final `cargo sqlx prepare`.

**Validate**: the full block, plus a manual run: `htui --set-dsn`, `htui` against the local server
(top bar `online`), stop the server, `htui` again (renders from the cache, `offline · ...`).

README sections to add: DSN setup (`--set-dsn` / `--clear-dsn`, keyring, no env-var fallback), the
cache location (`<config_dir>/htui/cache/<fingerprint>/`), `--offline`, the test environment
variable and the `htui_test_` cleanup note, the `cargo sqlx prepare` workflow of F, and
**Postgres 16 minimum**.

---

## H. Errata

Found while writing this blueprint against ANA-9, the code as it stands and the sqlx 0.9.0 sources.
The implementation follows the corrected reading.

- **H.1 - "Thirty tables" (ANA-9 section 3).** The entity map and section 5 declare **32** tables,
  not 30: the prose count omits `app_setting` and `capability_tag`, both of which are drawn in the
  same map. `migrations/0001_init.sql` creates 32 (B.1). No design change; the count in ANA-9's
  prose is wrong and the close-out should say so.

- **H.2 - Seeding agents (ANA-9 5.10 vs plan D5).** 5.10 seeds `agent` rows `claude` and `agy`;
  D5 seeds none, because `agent.launch` is `JSONB NOT NULL` and its shape is ANA-4's, unconcluded.
  **Keep D5.** This is the one place the plan knowingly overrides the design authority and it says
  so with a reason; seeding a guessed launch shape would create a row ANA-4 then has to migrate
  away. Record it as a deviation in the close-out, not as a defect.

- **H.3 - `run_step_commit` has no cursor column (ANA-9 6.2).** The refresh loop lists
  `run_step_commit` among the cursor-driven tables, but the table
  (`run_step_id, repo_id, before_hash, after_hash`) has no `updated_at` and no `created_at`, so
  `ts_col > hw - overlap` cannot be written. **Fix**: the fetch joins its parent and uses
  `run_step.updated_at` as `ts_col` (C.13, row 10). Smallest possible change; no DDL edit, and
  correct because a commit row is only ever written while its step is being updated.

- **H.4 - `workspace_project` has no cursor column either (ANA-9 4.4).** 4.4 mirrors it and calls
  the unscoped tables "small". **Fix**: `app_user`, `workspace` and `workspace_project` are a full
  delete-and-insert replace once per pass, with no `cache_cursor` row (C.13, step 1). `workspace`
  does have `updated_at`, but replacing all three together is simpler and is what "small" licenses.

- **H.5 - `pending/<run_id>.jsonl` cannot reconstruct the run (ANA-9 4.3).** The file name carries
  only `run_id` and the lines carry only `session_event` columns, but `run.project_id`,
  `run.target_box_id` and `run.started_by` are all `NOT NULL`. **Fix**: the file is named
  `pending/<project_id>.<run_id>.jsonl`; `target_box_id` and `started_by` come from
  `upload_pending`'s `this_box` / `this_user` arguments (C.14). Filename-only change, no new
  format, no sidecar.

- **H.6 - `cargo sqlx prepare -p htui-store` does not parse (plan D2 and every Validate step).**
  sqlx-cli 0.9's `Prepare` subcommand accepts `--check`, `--all`, `--workspace`, `--database-url`,
  `--connect-timeout`, `--config`, and forwards only what follows `--`; there is no `-p`.
  **Fix**: run it from `crates/htui-store` (F), which is also what puts `.sqlx` in the crate rather
  than at the repo root.

- **H.7 - `Migrate::list_applied_migrations` takes the table name in 0.9 (plan D4, V8).** The
  signature is `list_applied_migrations(&mut self, table_name: &str)`; in 0.8 it took none. It is
  on `PgConnection`, not on `PgPool`, so acquire a connection first. `Migrator::table_name` is a
  `Cow<'static, str>` whose default is `_sqlx_migrations` (C.4).

- **H.8 - a `str_enum!` enum needs `#[sqlx(type_name = "text")]` (plan D3).** D3 calls the derive a
  "weak-enum" derive with per-variant renames. In sqlx those are two different paths: a *weak* enum
  is `#[repr(i32)]` and has no per-variant renames; a *strong* enum is the string one, and
  per-variant `rename` belongs to it. The strong-enum Postgres `Type` impl is
  `PgTypeInfo::with_name(<ident>)`, which does not match a `TEXT` column, so `type_name = "text"`
  is required (A.7). The terminology slip is harmless; the missing attribute is a real defect that
  would have failed at run time, not at compile time.

- **H.9 - a UUID cannot be decoded from a SQLite TEXT column (plan D8, ANA-9 4.4).** 4.4 stores
  UUIDs as TEXT, but `impl Decode<Sqlite> for Uuid` in sqlx 0.9 reads `value.blob_borrowed()` - it
  is a **BLOB** mapping. The transparent ID newtypes therefore cannot decode a mirror column
  directly. **Fix**: mirror reads go through `uuid_col` (C.12), a `String` column parsed with
  `Uuid::parse_str`. Keep TEXT: it is what 4.4 specifies and what makes the file readable with the
  `sqlite3` shell.

- **H.10 - a timestamp cannot be decoded from a SQLite INTEGER-microseconds column (same).** 4.4
  stores microseconds; sqlx 0.9 decodes an INTEGER datetime as **seconds**
  (`decode_datetime_from_int` calls `timestamp_opt(v, 0)`) and encodes a `DateTime` as RFC-3339
  text. **Fix**: `ts_col(i64)` / `ts_bind(DateTime<Utc>)` in C.12; never bind or decode a
  `DateTime` against the mirror directly. This one is silent - a wrong-by-a-million timestamp
  decodes without an error - so it is the single most important line in C.12.

- **H.11 - ANA-9 7.1 and 7.2 return too few columns.** Both sketches end with `RETURNING item_id`
  and `RETURNING version`, but `mint_item` and `update_item` return a full `Item` (6.1). **Fix**:
  the `RETURNING` lists are extended to every `item` column and the statement ends with a `SELECT`
  over the CTE (C.9). The CTE structure, the counter upsert and the revision insert are 7.1 / 7.2
  verbatim.

- **H.12 - ANA-9 4.2 does not say what happens to `closed_at`.** The status CAS is stated without
  it. **Fix**: `closed_at = CASE WHEN $new IN ('done','closed') THEN clock_timestamp() ELSE NULL
  END`, i.e. `closed_at` tracks the current status in both directions. This is the MOD-1 blueprint
  errata rule carried over so `MemStore` and `PgStore` pass the same case (C.9).

- **H.13 - the kind guard on `update_item` has no FK to lean on.** ANA-9 5.5 has no composite
  `(project_id, id)` unique key on `item_kind`, so "the patched kind belongs to the item's project"
  must be an explicit `EXISTS` (MOD-1 blueprint errata, B.8 update_item). It is folded into the CAS
  `WHERE`, and the zero-row disambiguation of C.9 keeps a bad kind (`Constraint`) distinguishable
  from a lost race (`Diverged`).

- **H.14 - plan D10's `start` signature cannot build a `Refresher`.** D10 writes
  `start(args) -> (Backend, mpsc::Receiver<ConnEvent>, Option<Refresher>)`. The refresher needs a
  `PgStore`, which does not exist until a `ConnEvent::Online` arrives, and a `watch::Receiver` the
  store worker owns. **Fix**: `start` returns `Started` (C.15) carrying the `watch::Sender` and the
  settings, and the worker spawns the refresher on the `Online` transition (D.2). Same components,
  correct ownership.

- **H.15 - the plan's `Backend` variants and the `connecting` label disagree.** D1 names three
  variants with `Offline { cache, since: DateTime<Utc> }`; D10 wants the worker to start with the
  label `connecting`. **Fix**: `since` is an `Option`, `None` reading `connecting` (C.16). Three
  variants, both labels, no fourth state.

- **H.16 - `ItemFilter::project_ids` narrows, it does not widen.** ANA-9 has no text for this; the
  conformance case `filter_status_project_tags_ready` shows `project_ids: Some([agy])` returning
  only `agy` items **within** the scope. The SQL is therefore
  `project_id = ANY($scope) AND ($filter IS NULL OR project_id = ANY($filter))` - two conjuncts,
  never a replacement (C.7).

- **H.17 - `ANY($1)` needs a `Vec<Uuid>`, not a `Vec<ProjectId>`.** Covered by the A.8 rule, but it
  is the mistake to expect: convert at the call site with
  `scope.project_ids.iter().map(ProjectId::as_uuid).collect::<Vec<_>>()`.

---

## Reviewer checklist (derived from this blueprint)

1. `migrations/0001_init.sql` creates all 32 tables of B.1 in that order, with the B.2 trigger loop
   over exactly twenty tables and the three B.3 `ALTER TABLE`s last.
2. Every `str_enum!` enum carries `#[sqlx(type_name = "text")]` and every variant its
   `#[sqlx(rename = ...)]`; every ID newtype carries `#[sqlx(transparent)]`.
3. No Postgres query binds an ID newtype or an enum as a parameter (A.8); no `as _` appears.
4. No mirror read decodes a `Uuid` or a `DateTime` directly (H.9, H.10).
5. `PgStore` has no `DELETE FROM item` and no `UPDATE` of `key_prefix` / `key_number` / `key`.
6. No write path sets `updated_at`; the trigger owns it.
7. `crates/htui-store/.sqlx/` is committed and `cargo sqlx prepare --check` is clean.
8. No view module and no `App` field holds a `PgStore`, a `CacheStore` or a `Backend`
   (`R-NF-3`); the only `Backend` lives in `store_worker`.
9. `event_loop::run` still has exactly three `select!` arms.
10. The refresher is the only writer of `cache.sqlite` (6.3), and no lock or pool connection is
    held across a UI await.
11. T2 and T3 changed no T1 file; both added only `.sqlx/query-*.json`.
12. `Harness` still reports `"memory"` and every MOD-1 snapshot is byte-identical.

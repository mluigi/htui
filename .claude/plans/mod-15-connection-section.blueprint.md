# Blueprint: MOD-15 milestone 6 — a box with no DSN can fix itself

**From** `.claude/plans/mod-15-connection-section.plan.md` (D1–D20, F-1..F-22, tasks T1–T4) and
`docs/ANA-10.md` §4.9 (seven sub-verdicts, which win over the plan on the DSN's handling; the
local-only half — `Backend::Local`, `LocalStore`, the first-run overlay, `local_setting`,
`AckLocalOnlyNotice` — is withdrawn by MOD-25 and is **not** built).
**Tree** `0928182 docs(mod-15): milestone 5 landed`, branch `main`, 2026-09-17. `graphify-out/`
(built 2026-09-15) was used to locate symbols only; every coordinate below was re-read on this tree.
Line numbers are **pre-edit**; each edit shifts the ones below it in the same file, so an implementer
anchors on the quoted text, not the number.
**Read notes.** sqlx-postgres 0.9.0 `src/options/parse.rs`, `ssl_mode.rs`, `mod.rs` and keyring 3.6.3
`src/mock.rs` were read from the registry because three plan facts depend on their exact behaviour
(flags H, J, L below).

## 0. Choices and flags

### 0.1 Amendments to the plan

Every flag is a place where the tree, sqlx or keyring contradicts the plan or where the plan has a
hole. None is silently applied; each is resolved below and the resolution is what the sections after
§0 build.

| Flag | Plan says | Tree / dependency says | Resolution |
|---|---|---|---|
| **A** | D20: `Started` gains `connect: Option<ConnectContext>`. | `Started`'s `Debug` is hand-written with `finish_non_exhaustive` (`connect.rs:121-129`), so a new field is not printed automatically; workspace lint `missing_debug_implementations = "warn"` under `-D warnings` means `ConnectContext` must have `Debug`. | `ConnectContext` derives `Debug, Clone` (its fields are `PathBuf`, `Duration`, `bool` — nothing secret). `Started`'s printer adds `.field("connect", &self.connect)`. |
| **B** | D1: `Dsn` has "no public text accessor". | `secret::set_dsn(dsn: &str)` (`secret.rs:38-40`) and `identity::db_fingerprint(dsn: &str)` (`identity.rs:132-144`) both take `&str`, and `apply_dsn` lives in `connect.rs`, another module of the same crate. | `Dsn::as_str(&self) -> &str` is **`pub(crate)`**, documented as the only reader, used by exactly four call sites inside `htui-store`: `apply_dsn` (keyring write), `reconnect_for` (per-dial copy), `Dsn::fingerprint`, `Dsn::summary`. Never `pub`; a test in `tests/dsn.rs` cannot call it, which is the point. |
| **C** | D20: `reconnect_for(dsn: &Dsn, root, timeout) -> Reconnect` "extracted from `start`'s closure". | `attempt(dsn: Option<String>, root: PathBuf, connect_timeout: Duration) -> ConnEvent` (`connect.rs:240-255`) takes a plain `Option<String>`, and `start`'s closure (`:198-208`) holds one permanently. A stored DSN that `--set-dsn` wrote raw may not pass `Dsn::parse` (an unknown query key connects today because sqlx only warns — flag H), so `start` cannot be made to go through `Dsn::parse` without changing M1 behaviour. | `attempt`'s signature is **unchanged**. One private `reconnect_over(dsn: Option<Zeroizing<String>>, root: PathBuf, connect_timeout: Duration) -> Reconnect` holds the text in a zeroizing buffer inside the `Arc` and copies it into a plain `String` **per dial** (which `try_connect` consumes and drops, as today's `dsn.clone()` at `:199` does). `pub fn reconnect_for(&Dsn, ..)` and `start` both call it; `start` wraps its keyring read in `Zeroizing::new` at `:179-182`. Net residue is strictly less than today. |
| **D** | D11 steps (7)/(8) assign `reconnect` and dial. | `spawn_with` destructures `reconnect` **immutably** (`store_worker.rs:1016-1023`, `reconnect,` at `:1022`); `settings: RefreshSettings` is passed by value at `:1069` and `:1137`, so it is `Copy`. | `let Started { .., mut reconnect, connect, .. } = started;` — `mut` on `reconnect` only. `connect` is moved in as loop state beside `refresher`/`health` (`:1027-1031`). `held` is also cleared by `SetDsn` (the held `PgStore` belongs to the old server). |
| **E** | D3: `TextField::masked()` reserves 256 and zeroizes on `clear()`/`take()`; the manifest change is "one line in `[workspace.dependencies]` + one in `crates/htui-store/Cargo.toml`". | `TextField` is in `crates/htui` (`ui/text_field.rs:31-39`, `text: String`) and `crates/htui/Cargo.toml` has no `zeroize`. `unsafe_code = "forbid"` (workspace lints) rules out wiping a `String` by hand. `TextField` derives `Clone, Default` (`:31`) and `masked()` uses `..Self::default()` struct update, so a hand-written `Drop` on `TextField` is E0509. | **Three** manifest lines: workspace, `htui-store`, **and `htui`**. `TextField.text` becomes `Zeroizing<String>` for every field (a slug zeroized on drop costs nothing); `take()` keeps returning `String` by `core::mem::take(&mut *self.text)` — the heap buffer **moves**, no copy — and the masked consumer wraps it in `Zeroizing::new` at once. |
| **F** | D7: `SettingsRegistry::focus(SectionId) -> bool`, `Tab::focus_section` default `false`. | Neither exists. `TabRegistry::focus(TabId) -> bool` (`registry.rs:133-138`, `position` + `select`) is the pattern; `SettingsRegistry::cycle_next/prev` (`settings/mod.rs:212-224`) shows the `active` index. `Tab` trait (`registry.rs:33-49`) has seven required methods, none defaulted. | Built as the plan says, placed as §5 pins. `SettingsTab::wants_requests` already collects **every** section's reads (`settings/mod.rs:281-289`), so switching sections needs no re-activation; `update_tab`'s existing `if self.tabs.active_id() != before { self.activate_tab(); }` (`update.rs:49-51`) covers the cross-tab case. |
| **G** | Files table: placements unstated. | Modules are alphabetical in both `lib.rs` files and in `settings/mod.rs:11-14`. | `crates/htui/src/lib.rs`: `pub mod connection;` between `pub mod cli;` (`:15`) and `pub mod event_loop;` (`:16`). `settings/mod.rs`: `pub mod connection;` between `pub mod agents;` (`:11`) and `pub mod hierarchy;` (`:12`); `pub use connection::ConnectionSection;` first in the `pub use` block (`:29-32`). `htui-store/src/lib.rs`: `pub mod dsn;` between `pub mod connect;` (`:15`) and `pub mod error;` (`:16`); `pub use dsn::{Dsn, DsnError};` after `:26`. `action.rs` and `registry.rs` each gain `use crate::ui::tabs::settings::SectionId;`. |
| **H** | D2: `Dsn::parse` yields `unrecognised parameter`; "nothing logs on a validation failure". | sqlx `PgConnectOptions::parse_from_url` does **not** error on an unknown query key: it runs `tracing::warn!(%key, %value, "ignoring unrecognized connect parameter")` (`parse.rs:107`) — the **value** goes to the log. `PgSslMode::from_str` quotes the offending value in its error (`ssl_mode.rs:48`). `port` and `statement-cache-capacity` errors carry `ParseIntError` text. `Url::parse` failure text names nothing but is sqlx's. | `Dsn::parse` runs a **string-level pre-scan first** (scheme, host, authority port, every query key against the allow-list of keys `parse.rs:51-103` handles, `sslmode` value against the six names, `port` value as `u16`) and only then calls `PgConnectOptions::from_str` as the authoritative gate, mapping any residual `Err` to `DsnError::NotAUrl` without reading it. The scan is what makes the fixed vocabulary true and what keeps sqlx's `warn!` from ever running on a value the user typed. No `url` crate is declared; the scan is a hand split on `://`, `@`, `/`, `?`, `&`, `=` (an IPv6 `[..]` host is handled; a percent-encoded key is refused as unrecognised, which is the safe direction). |
| **I** | D1/D3: the section constructs `Dsn` from the field. | `TextField::take() -> String` (`text_field.rs:161-164`). | `let raw = Zeroizing::new(editor.input.take());` then `Dsn::parse(&raw)`. `Dsn::parse(text: &str)` copies into its own `Zeroizing<String>`; `raw` zeroizes on scope exit. Two buffers, both wiped; the `String` move out of the field copies nothing. |
| **J** | D1/ANA-10 (3): `Dsn::parse` runs in the section (UI task). | `PgConnectOptions::from_str` → `parse_from_url` starts from `new_without_pgpass()` (env reads: `PGHOST`, `PGUSER`/`whoami`, `PGPASSWORD`, …) and ends with `apply_pgpass()` (`parse.rs:109`, `mod.rs:104-115`), which reads `~/.pgpass` when the DSN carries no password. `identity::db_fingerprint` already does the same in `connect::start` (off the UI task). | Accepted and priced: one small synchronous file read on the UI task, only when the typed DSN has no password, on `Enter` only. Moving the gate to the worker would lose the fixed-vocabulary refusal at the field, which ANA-10 (5) requires. Recorded in §10 and in the HANDOFF paragraph. |
| **K** | D9: `ConnectionInfo` is served in `try_serve`. | `try_serve(backend, request)` sees only the `Backend`; the last dial outcome and the `--offline` flag are loop state (`connect`, and the events arm `:1132-1155`). | `connection::snapshot(backend: &Backend, attempt: Option<&Attempt>, context: Option<&ConnectContext>)`. `try_serve`'s arm calls it with `(backend, None, None)` (that is what `testkit::Harness::settle` and `serve()` see); the loop `match` has its **own** `ConnectionInfo` arm that passes `last_attempt.as_ref()` and `connect.as_ref()`. Both are documented as the same read with and without the loop's memory. |
| **L** | D18/F-2: `htui_store::testkit::mock_keyring()` installs `keyring::mock` so the worker tests can store a DSN and read it back. | keyring's mock keeps the secret **in the entry** (`mock.rs`: `persistence()` is `EntryOnly`; `build` returns a fresh `MockCredential` per call), and `secret::get_dsn`/`set_dsn`/`clear_dsn` each open a **fresh** `Slot` (`secret.rs:29-49`, doc at `:51-56` says exactly this). Under the crate's mock, `set_dsn` then `get_dsn` returns `None`. The headline test cannot pass as planned. | `secret.rs` gains a `#[cfg(feature = "test-support")]` process-wide **fake slot** (`static FAKE: Mutex<Option<Option<String>>>`), consulted first by the three functions when installed; `testkit::mock_keyring().await` installs it empty, returns a guard that serialises keyring-touching tests through a `tokio::sync::Mutex` and uninstalls on drop. `keyring::mock` stays what `secret.rs`'s own unit tests use. Production binaries never see the fake unless built with `test-support` **and** a test installs it. Full shape in §1.5. |
| **M** | T2 tests: "`SetDsn` with `connect: None` → `Failed`". | `Started`'s fields are `pub` (`connect.rs:103-119`); `detached` will set `connect: None`. | Tests build `Started::detached(backend)` and set `started.connect = Some(ConnectContext { .. })` directly for the live cases; no builder method is added. |
| **N** | Plan test list has `rebuild` keeping `built_at`. | `CacheStore::rebuild` doc (`cache/mod.rs:166-171`) — the implementer re-reads it; `CacheMeta` fields are `pub` (`:88-98`) so the snapshot reads them without accessors. | The D14 confirm copy is written from the doc at `:166-171` and the test pins whatever `rebuild` actually preserves; if `built_at` moves, the copy and the test change together and the deviation is recorded in HANDOFF. |

### 0.2 Choices

- **B-1 `DsnError` is `thiserror`.** `htui-store` already depends on `thiserror`; each variant carries
  its sentence as `#[error("..")]`, so `Display` is the vocabulary and there is no second table to
  drift.
- **B-2 `Dsn::summary()` re-parses.** The newtype stays a single-field tuple (D1); `summary()` and
  `fingerprint()` call `PgConnectOptions::from_str` again on the stored text. Called once per
  `SetDsn` and once per `ConnectionInfo`; not cached, so there is no second copy of anything.
- **B-3 The summary's shape** is `postgres://{user}@{host}:{port}/{db} · sslmode={mode}`; `/{db}` is
  omitted when `get_database()` is `None`; a socket DSN (`get_socket()` is `Some`) renders the path
  instead of `host:port`. `PgSslMode` has no `Display` (only `Debug`, `ssl_mode.rs:7`), so the six
  names are spelled in one `match` in `dsn.rs`. The password is unreachable — `PgConnectOptions` has
  no getter for it — and a test pins its absence anyway.
- **B-4 `Backend::Memory` refuses the three writers** with the sentence
  `a demo session has no DSN to change`; `try_serve` refuses them with
  `no connection worker in this build` (D9). Two different sentences because they are two different
  situations and the tests tell them apart by text.
- **B-5 `Attempt` is `{ at: DateTime<Utc>, outcome: AttemptOutcome }`** with
  `enum AttemptOutcome { Online, MigrationsPending(usize), Failed(String) }`. `Failed`'s text is the
  `ConnEvent::Failed(why)` string the worker already logs at `:1153`; it is sqlx's connect error,
  never the DSN.
- **B-6 The connection section issues `ConnectionInfo` from `wants_requests`** like every other
  section, and the shell issues it once more under `Origin::App` from `App::start` for the redirect
  (D6). The two reads are addressed differently and neither is stale to the other (`latest` is keyed
  by origin).
- **B-7 The DSN row for a stored-but-unparseable DSN** (one `--set-dsn` wrote raw before this
  milestone) reads `stored — not readable by this build; e replaces it`. The snapshot carries
  `dsn_stored: Some(true), dsn_summary: None` for it. `connect::start` keeps connecting with it
  (flag C).
- **B-8 `Notice` gets the hand-written `Debug`** M5's review L-9 asked for: the connection notice only
  ever holds this module's constants and the seam's sentences, but the rule is that no section's
  `Debug` prints text that came from a field.
- **B-9 `e` opens the editor from any row**, not only the DSN row; `Enter` on the `Rebuild cache` row
  is the same as `R`. Fewer surprises than a row-dependent `e`.

### 0.3 Main-thread rulings on the flags (maintainer's session, 2026-09-17)

- Flags **A–N are adopted as written**. Flag H is a security finding, not a style choice: without the
  pre-scan a typed DSN's query-parameter **value** reaches `--log` through sqlx's own `warn!`.
- **O-1 is not deferred — it is fixed in this milestone** (see §10 H-6). The guard is a dial
  generation counter local to the worker: `let dials = Arc::new(AtomicU64::new(0));` beside the other
  loop state; every spawned dial (the ticker's at `:1162-1171` **and** `SetDsn`'s step (8)) captures
  `let generation = dials.load(Ordering::SeqCst);` and a clone of the `Arc`, and sends its
  `ConnEvent` only when `dials.load(Ordering::SeqCst) == generation`; `SetDsn` bumps with
  `dials.fetch_add(1, Ordering::SeqCst)` at step (6). No channel type changes, no `htui-store`
  change. A test drives it: a dial spawned before a `SetDsn` must not install its `PgStore` after the
  swap.
- Flag **L is adopted with its blast radius stated in review**: the fake slot is
  `#[cfg(feature = "test-support")]`, the binary never enables that feature, and step 13's
  `cargo check -p htui-store` (no features) is the proof it is compiled out. `rust-reviewer` is asked
  to look at it specifically.

## 1. T1 — `htui-store`: the newtype, the two entry points, the fake keyring

### 1.1 `crates/htui-store/src/dsn.rs` (new)

```rust
//! The DSN as a redacting newtype (MOD-15 milestone 6, D1–D3; `docs/ANA-10.md` §4.9).
//!
//! [`Dsn::parse`] is the only way to make one. It refuses with one of five fixed sentences
//! ([`DsnError`]) that never carry any of the text, and it refuses **before** sqlx sees the
//! string, because sqlx logs an unrecognised query parameter's value at `warn` rather than
//! rejecting it (sqlx-postgres 0.9.0 `options/parse.rs:107`).

use core::fmt;
use core::str::FromStr as _;

use sqlx::postgres::{PgConnectOptions, PgSslMode};
use zeroize::Zeroizing;

use crate::identity;

/// A connection string that passed [`Dsn::parse`].
///
/// Prints as `Dsn(<redacted>)`; the text is reachable only through [`Dsn::as_str`], which is
/// `pub(crate)`. Cloned into the store worker's request and moved into the keyring write.
#[derive(Clone)]
pub struct Dsn(Zeroizing<String>);

impl fmt::Debug for Dsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dsn(<redacted>)")
    }
}

/// Why a string is not a DSN this build stores (D2).
///
/// Five sentences and nothing else: no sqlx text, no fragment of what was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DsnError {
    /// Not `postgres://` / `postgresql://`, or a shape sqlx's parser still refuses after the scan.
    #[error("not a URL")]
    NotAUrl,
    /// The authority has no host.
    #[error("no host")]
    NoHost,
    /// `sslmode=` is not one of the six names.
    #[error("unsupported sslmode")]
    UnsupportedSslMode,
    /// A `:port` or `port=` that is not a `u16`.
    #[error("port out of range")]
    PortOutOfRange,
    /// A query key sqlx would log rather than use.
    #[error("unrecognised parameter")]
    UnrecognisedParameter,
}

/// The query keys sqlx-postgres 0.9.0 handles (`options/parse.rs:51-103`), plus the
/// `options[<name>]` family checked separately. Anything else is logged by sqlx with its value.
const KNOWN_PARAMETERS: [&str; 18] = [
    "sslmode", "ssl-mode", "sslrootcert", "ssl-root-cert", "ssl-ca", "sslcert", "ssl-cert",
    "sslkey", "ssl-key", "statement-cache-capacity", "host", "hostaddr", "port", "dbname",
    "user", "password", "application_name", "options",
];

/// `PgSslMode::from_str`'s six spellings (`ssl_mode.rs:38-45`), matched case-insensitively as it does.
const SSL_MODES: [&str; 6] = ["disable", "allow", "prefer", "require", "verify-ca", "verify-full"];

impl Dsn {
    /// Validates `text` and takes a zeroizing copy of it.
    ///
    /// # Errors
    ///
    /// One [`DsnError`]; see the type. Nothing is logged on any path.
    pub fn parse(text: &str) -> Result<Self, DsnError> {
        scan(text)?;
        PgConnectOptions::from_str(text).map_err(|_| DsnError::NotAUrl)?;
        Ok(Self(Zeroizing::new(text.to_owned())))
    }

    /// `postgres://user@host:port/db · sslmode=mode` — never the password (there is no getter
    /// for it on `PgConnectOptions`), never the query string.
    #[must_use]
    pub fn summary(&self) -> String { /* B-3; `"stored"` if from_str somehow fails */ }

    /// [`identity::db_fingerprint`] of the text: the mirror directory name.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        identity::db_fingerprint(&self.0)
    }

    /// The text, for this crate's keyring write, dial and fingerprint. Not `pub`.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pre-scan (flag H): everything sqlx would log, quote or default silently is caught here.
fn scan(text: &str) -> Result<(), DsnError> { .. }

fn ssl_mode_name(mode: PgSslMode) -> &'static str { .. }
```

`scan` in order, each step returning the variant named:

1. Any `char::is_control` → `NotAUrl` (the field already swallows them, `text_field.rs:108-113`; this
   is the belt).
2. `strip_prefix("postgres://")` or `"postgresql://"` else `NotAUrl`.
3. Split the rest at the first `?` into `(before, query)`; `before` at the first `/` into
   `(authority, _path)`; `authority` after the **last** `@` is `hostport`. `hostport` starting with
   `[` ends its host at `]`; otherwise `rsplit_once(':')` gives `(host, port)` when the tail is all
   digits, else the whole is the host. Empty host → `NoHost`. A present port not parsing as `u16` →
   `PortOutOfRange`.
4. For each `&`-separated pair in `query` (`split_once('=')`; a bare key has an empty value): key must
   be in `KNOWN_PARAMETERS` or be `options[..]` with a closing `]`, else `UnrecognisedParameter`;
   `sslmode`/`ssl-mode` value lower-cased must be in `SSL_MODES`, else `UnsupportedSslMode`; `port`
   value must be `u16`, else `PortOutOfRange`.

Sequence matters: an unrecognised key is reported before `from_str` runs, because `from_str` would log
it. The exhaustive `match` in `ssl_mode_name` is over the six variants; `PgSslMode` is not
`#[non_exhaustive]` on 0.9.0.

### 1.2 `crates/htui-store/src/connect.rs` (modify)

| Anchor | Edit |
|---|---|
| `:12-29` imports | add `use zeroize::Zeroizing;` and `use crate::dsn::Dsn;` (keep `use crate::secret;` at `:29`). |
| after `Reconnect` (`:70`) | new `ConnectContext` and `Applied` (below). |
| `Started` (`:103-119`) | new last field `pub connect: Option<ConnectContext>` with doc "What `apply_dsn` needs (D20). `None` from `detached` and therefore under `--demo`, which is what makes `SetDsn` refuse there." |
| `Started` Debug (`:121-129`) | add `.field("connect", &self.connect)` before `finish_non_exhaustive()`. |
| `detached` (`:138-149`) | literal gains `connect: None,`. |
| `start` `:179-182` | `let dsn: Option<Zeroizing<String>> = match opts.dsn { Some(dsn) => Some(Zeroizing::new(dsn)), None => secret::get_dsn()?.map(Zeroizing::new) };` |
| `start` `:184-186` | `dsn.as_deref().map_or_else(|| NO_DSN_FINGERPRINT.to_owned(), \|text\| identity::db_fingerprint(text))` — `as_deref` on `Option<Zeroizing<String>>` yields `Option<&String>`; coerce with `text.as_str()`. |
| `start` `:198-208` | replace the closure with `Some(reconnect_over(dsn.clone(), root.clone(), timeout))`. |
| `start` `:210-216` | first dial: `attempt(dsn.as_deref().map(String::clone), root, timeout)` — a plain copy that `try_connect` consumes, as today. |
| `start` literal `:224-231` | add `connect: Some(ConnectContext { config_root: root_for_context, connect_timeout: timeout, offline: opts.offline }),` — take `let root_for_context = root.clone();` before `root` moves into the spawn at `:210`. |
| after `attempt` (`:255`) | `reconnect_over`, `reconnect_for`, `apply_dsn`, `forget_dsn` (below). |

```rust
/// What [`apply_dsn`] needs from the session [`start`] set up (D20).
#[derive(Debug, Clone)]
pub struct ConnectContext {
    /// The directory `cache/<fingerprint>/` is opened under.
    pub config_root: PathBuf,
    /// Passed to every dial.
    pub connect_timeout: Duration,
    /// `--offline`: the DSN is stored and the mirror re-opened, but nothing dials (D12).
    pub offline: bool,
}

/// What a stored DSN produced, for the worker to install (D1, D11).
pub struct Applied {
    /// The mirror for the new server, opened and migrated.
    pub cache: CacheStore,
    /// One dial per call, over the new DSN.
    pub reconnect: Reconnect,
    /// `db_fingerprint` of the new DSN — the mirror's directory name.
    pub fingerprint: String,
    /// The redacted summary the section shows.
    pub summary: String,
}

impl fmt::Debug for Applied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Applied")
            .field("cache", &self.cache)
            .field("fingerprint", &self.fingerprint)
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

/// One dial per call over `dsn`, copied into a plain `String` for the length of the dial only.
fn reconnect_over(dsn: Option<Zeroizing<String>>, root: PathBuf, connect_timeout: Duration) -> Reconnect {
    Arc::new(move || {
        let dsn = dsn.as_deref().map(String::clone);
        let root = root.clone();
        Box::pin(attempt(dsn, root, connect_timeout)) as ConnFuture
    })
}

/// [`reconnect_over`] for a validated DSN (D20).
#[must_use]
pub fn reconnect_for(dsn: &Dsn, root: PathBuf, connect_timeout: Duration) -> Reconnect {
    reconnect_over(Some(Zeroizing::new(dsn.as_str().to_owned())), root, connect_timeout)
}

/// Stores `dsn` in the keyring and opens the mirror it names (D11 steps 2–3, 7).
///
/// The keyring write runs on the blocking pool with the `Dsn` moved into it (ANA-10 §4.9 (6),
/// (7)). Nothing here touches the running backend: the worker installs the result.
///
/// # Errors
///
/// The keyring's refusal, the blocking task's join failure, or `CacheStore::open`'s. On any of
/// them nothing was installed and the caller's backend is untouched; a keyring write that
/// succeeded before `open` failed **stays written** (see §10 H-3).
pub async fn apply_dsn(dsn: Dsn, ctx: &ConnectContext) -> Result<Applied> {
    let stored = dsn.clone();
    tokio::task::spawn_blocking(move || secret::set_dsn(stored.as_str()))
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))??;
    let fingerprint = dsn.fingerprint();
    let cache = CacheStore::open(&ctx.config_root, &fingerprint, PgStore::schema_version()).await?;
    let reconnect = reconnect_for(&dsn, ctx.config_root.clone(), ctx.connect_timeout);
    let summary = dsn.summary();
    Ok(Applied { cache, reconnect, fingerprint, summary })
}

/// Removes the keyring entry (D13). A missing entry is `Ok(())` (`secret.rs:47-49` via `Slot::clear`, `:88-95`).
///
/// # Errors
///
/// The keyring's refusal or the blocking task's join failure.
pub async fn forget_dsn() -> Result<()> {
    tokio::task::spawn_blocking(secret::clear_dsn)
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))?
}
```

The `JoinError` mapping copies `hierarchy.rs:415-429`'s (`StoreError::Backend(err.to_string())` there;
the `keyring task failed:` prefix is this milestone's, so a join failure reads differently from a
keyring refusal).

### 1.3 `crates/htui-store/src/lib.rs` (modify)

`pub mod dsn;` between `:15` and `:16`; `pub use dsn::{Dsn, DsnError};` after `:26`;
`pub use connect::{ConnEvent, StartOptions, Started};` at `:26` becomes
`pub use connect::{Applied, ConnectContext, ConnEvent, StartOptions, Started};`.

### 1.4 Manifests

- `/home/mluigi/projects/htui/Cargo.toml` `[workspace.dependencies]`: `zeroize = "1.9"` beside
  `keyring` (`:44-45`). `Cargo.lock` already holds `zeroize 1.9.0` through keyring, so the lock does
  not move.
- `/home/mluigi/projects/htui/crates/htui-store/Cargo.toml` `[dependencies]`:
  `zeroize = { workspace = true }` after `keyring`.
- `/home/mluigi/projects/htui/crates/htui/Cargo.toml` `[dependencies]`:
  `zeroize = { workspace = true }` after `tracing-subscriber` (flag E), with the comment
  `# MOD-15 M6 D3: the masked field's buffer and the DSN on its way to the newtype are wiped.`

### 1.5 `crates/htui-store/src/secret.rs` and `testkit.rs` (flag L)

`secret.rs`, after `USER` (`:18`):

```rust
/// A process-wide stand-in for the keyring, for tests that must round-trip a DSN (D18).
///
/// `keyring::mock` keeps the secret **in the entry** and this module opens a fresh [`Slot`] per
/// call (`:51-56`), so the crate's own mock cannot see a `set_dsn` from a later `get_dsn`. Outer
/// `None`: not installed, the real keyring answers. `Some(slot)`: every call reads and writes
/// `slot` instead. Installed only by `testkit::mock_keyring`.
#[cfg(feature = "test-support")]
pub(crate) static FAKE: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);

#[cfg(feature = "test-support")]
fn fake() -> std::sync::MutexGuard<'static, Option<Option<String>>> {
    FAKE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
```

Each of `get_dsn`, `set_dsn`, `clear_dsn` gains, as its first statement:

```rust
#[cfg(feature = "test-support")]
if let Some(slot) = &mut *fake() {
    // get: return Ok(slot.clone().filter(|s| !s.trim().is_empty()))
    // set: *slot = Some(dsn.to_owned()); return Ok(())
    // clear: *slot = None; return Ok(())
}
```

`testkit.rs`, after `parse_dsn` (`:269`):

```rust
/// Serialises the tests that touch the fake keyring: it is one process-wide slot.
static KEYRING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Holds the fake keyring installed and empty until dropped.
#[derive(Debug)]
pub struct KeyringGuard(tokio::sync::MutexGuard<'static, ()>);

impl Drop for KeyringGuard {
    fn drop(&mut self) {
        *crate::secret::fake() = None;
    }
}

/// Routes `secret::{get,set,clear}_dsn` to an empty in-process slot until the guard drops (D18).
///
/// Never the developer's keyring: a test that forgets the guard and calls `set_dsn` writes to
/// the OS store, which is why every test in `tests/connection.rs` takes one first.
pub async fn mock_keyring() -> KeyringGuard {
    let guard = KEYRING.lock().await;
    *crate::secret::fake() = Some(None);
    KeyringGuard(guard)
}

/// What the fake keyring holds, for assertions. Panics outside a [`mock_keyring`] guard.
#[must_use]
pub fn fake_dsn() -> Option<String> {
    crate::secret::fake().clone().expect("fake keyring not installed")
}
```

`fake()` becomes `pub(crate)`. `expect` is already the testkit's idiom (`:478`).
`tokio::sync::Mutex::const_new` is const on the workspace tokio.

### 1.6 `crates/htui-store/tests/dsn.rs` (new) — tests first

| Test | Asserts |
|---|---|
| `a_plain_string_is_not_a_url` | `Dsn::parse("nope")` → `Err(DsnError::NotAUrl)`; `err.to_string() == "not a URL"`. |
| `a_url_without_a_host_is_refused` | `postgres:///htui` and `postgres://user:pw@/htui` → `NoHost`. |
| `an_unknown_sslmode_is_refused` | `postgres://h/d?sslmode=maybe` → `UnsupportedSslMode`; `?sslmode=REQUIRE` is accepted (sqlx lower-cases). |
| `a_port_above_u16_is_refused` | `postgres://h:70000/d` and `postgres://h/d?port=70000` → `PortOutOfRange`. |
| `an_unknown_parameter_is_refused_before_sqlx_sees_it` | `postgres://h/d?foo=bar` → `UnrecognisedParameter`; every key in `KNOWN_PARAMETERS` plus `options[x]=y` parses. |
| `the_error_never_carries_the_text` | for each refusal above, `format!("{err}")` and `format!("{err:?}")` contain neither the host nor the password used. |
| `debug_is_redacted` | `format!("{:?}", Dsn::parse("postgres://u:s3cret@h:5432/d").unwrap()) == "Dsn(<redacted>)"`. |
| `the_summary_names_everything_but_the_password` | summary of `postgres://htui:s3cret@db.example:5432/htui?sslmode=require` is `postgres://htui@db.example:5432/htui · sslmode=require`; `!summary.contains("s3cret")`. |
| `the_summary_omits_a_missing_database` | `postgres://u@h:1/` → `postgres://u@h:1 · sslmode=prefer`. |
| `credentials_do_not_change_the_fingerprint` | `Dsn::parse("postgres://a:x@h:5432/d")` and `("postgres://b:y@h:5432/d")` have equal `fingerprint()`, equal to `identity::db_fingerprint("postgres://h:5432/d")`. |
| `apply_dsn_stores_then_opens_the_new_mirror` | `let _k = testkit::mock_keyring().await;` `tempdir`; `apply_dsn(dsn, &ctx)` → `Ok(applied)`; `applied.cache.dir()` ends in `dsn.fingerprint()`; `testkit::fake_dsn() == Some(text)`; `applied.summary == dsn.summary()`; `applied.cache.close().await`. |
| `apply_dsn_leaves_the_old_mirror_on_disk` | open a cache under one fingerprint, `apply_dsn` another DSN; both directories exist. |
| `forget_dsn_on_an_empty_keyring_is_ok` | `mock_keyring`; `forget_dsn().await` is `Ok(())`; after `apply_dsn`, `forget_dsn` leaves `fake_dsn() == None`. |
| `the_fake_keyring_round_trips` | `mock_keyring`; `secret::set_dsn("x")`, `secret::get_dsn() == Ok(Some("x"))`, `clear_dsn`, `get_dsn() == Ok(None)` — the property flag L exists for. |

None is Postgres-gated: `apply_dsn` never dials.
`cargo test -p htui-store --features test-support --test dsn` proves the file.

## 2. T2 — `htui::connection` and the worker

### 2.1 `crates/htui/src/connection.rs` (new)

```rust
//! The connection section's read and the three writes behind it (MOD-15 milestone 6, D4, D9–D14).
//!
//! `snapshot` is the read; `serve` is the `try_serve` face, which has no loop and therefore no
//! memory of the last dial and no `--offline` flag; the worker's own `ConnectionInfo` arm hands
//! both in (blueprint flag K). The three writers are **not** here: they rewire the worker's
//! loop state (`backend`, `refresher`, `health`, `held`, `reconnect`) and live beside it.

use chrono::{DateTime, Utc};
use htui_core::store::{Result, StoreError};
use htui_store::{Backend, ConnectContext, Dsn, secret};
use zeroize::Zeroizing;

use crate::store_worker::{StoreReply, StoreRequest};

/// The connection as the section shows it. Never the DSN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSnapshot {
    /// `Backend::label()` at the time of the read.
    pub label: String,
    /// `Some(true)`: a DSN is in the keyring; `Some(false)`: none; `None`: `Backend::Memory`, no
    /// keyring is consulted (D10).
    pub dsn_stored: Option<bool>,
    /// `Dsn::summary()` of the stored DSN; `None` when nothing is stored or the stored text does
    /// not pass `Dsn::parse` (B-7).
    pub dsn_summary: Option<String>,
    /// The mirror's `cache_meta`, `None` on `Memory`.
    pub mirror: Option<MirrorInfo>,
    /// The last dial this session, `None` before the first and from `try_serve`.
    pub last_attempt: Option<Attempt>,
    /// `--offline` (D12): a stored DSN takes effect on the next launch.
    pub offline: bool,
}

/// `CacheMeta` plus where the file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorInfo {
    pub db_fingerprint: String,
    pub schema_version: i64,
    pub built_at: DateTime<Utc>,
    pub last_full_refresh_at: Option<DateTime<Utc>>,
}

/// One dial and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub at: DateTime<Utc>,
    pub outcome: AttemptOutcome,
}

/// How a dial ended (`ConnEvent` without its `PgStore`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    Online,
    MigrationsPending(usize),
    Failed(String),
}

/// The read (D4). Blocking keyring access goes through `spawn_blocking` (ANA-10 §4.9 (7)).
pub async fn snapshot(
    backend: &Backend,
    attempt: Option<&Attempt>,
    context: Option<&ConnectContext>,
) -> Result<ConnectionSnapshot> { .. }

/// `try_serve`'s arm: the read without the loop's memory, and a refusal for each writer (D9).
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::ConnectionInfo => Ok(StoreReply::Connection(snapshot(backend, None, None).await?)),
        StoreRequest::SetDsn(_) | StoreRequest::ClearDsn | StoreRequest::RebuildCache => {
            Err(StoreError::Backend(NO_WORKER.to_owned()))
        }
        other => Err(StoreError::Backend(format!("not a connection request: {}", other.name()))),
    }
}

/// `try_serve` has no worker loop to rewire (D9).
pub const NO_WORKER: &str = "no connection worker in this build";
/// `Backend::Memory` has no keyring and no mirror (D10, B-4).
pub const DEMO_SESSION: &str = "a demo session has no DSN to change";

/// Every request this module answers, in `StoreRequest` order; the section refuses `r` while one
/// of these is in flight and filters `Failed` by them.
pub const REQUEST_NAMES: [&str; 4] = ["connection_info", "set_dsn", "clear_dsn", "rebuild_cache"];
/// `REQUEST_NAMES[0]`: the read.
pub const READ_NAME: &str = REQUEST_NAMES[0];
```

`snapshot` body, in order: `label = backend.label()`; on `Backend::Memory(_)` return
`{ label, dsn_stored: None, dsn_summary: None, mirror: None, last_attempt: attempt.cloned(), offline: false }`
without touching the keyring; otherwise
`let text = tokio::task::spawn_blocking(secret::get_dsn).await.map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))??.map(Zeroizing::new);`
then
`(dsn_stored, dsn_summary) = match text { None => (Some(false), None), Some(text) => (Some(true), Dsn::parse(&text).ok().map(|dsn| dsn.summary())) }`;
`mirror = match backend.cache() { Some(cache) => { let meta = cache.meta().await?; Some(MirrorInfo { db_fingerprint: meta.db_fingerprint, schema_version: meta.schema_version, built_at: meta.built_at, last_full_refresh_at: meta.last_full_refresh_at }) } None => None }`;
`offline = context.is_some_and(|c| c.offline)`. The `Dsn::parse` here is the one place a raw stored
text meets the newtype; it fails closed to `dsn_summary: None`.

Names: `name()` arms return `"connection_info"`, `"set_dsn"`, `"clear_dsn"`, `"rebuild_cache"` —
`REQUEST_NAMES` is what `connection_names_are_stable` (§3) pins them against.

### 2.2 `crates/htui/src/store_worker.rs` (modify)

| Anchor | Edit |
|---|---|
| `:30` `use htui_store::{Backend, ConnEvent, PgStore, Started, connect};` | becomes `use htui_store::{Backend, ConnEvent, ConnectContext, Dsn, PgStore, Started, connect};` |
| `:38` `use crate::prompt_settings::{self, SettingsSnapshot};` | add `use crate::connection::{self, Attempt, AttemptOutcome, ConnectionSnapshot};` beside it (alphabetical: before `prompt_settings`). |
| after `ClearSetting` (`:465-474`), before `}` (`:475`) | four variants (doc comments below). |
| `name()` after `:535` | `Self::ConnectionInfo => "connection_info", Self::SetDsn(_) => "set_dsn", Self::ClearDsn => "clear_dsn", Self::RebuildCache => "rebuild_cache",` |
| `StoreReply` after `PromptSettingsStale` (`:682`), before `Failed` (`:684`) | ``/// `ConnectionInfo`, and every connection writer's success (D4): the section re-renders from it.`` `Connection(ConnectionSnapshot),` |
| `try_serve` after the prompt arm (`:949-951`) | `StoreRequest::ConnectionInfo \| StoreRequest::SetDsn(_) \| StoreRequest::ClearDsn \| StoreRequest::RebuildCache => connection::serve(backend, request).await,` (or-ed, unguarded — a guarded arm is E0004 here). |
| `spawn_with` destructure `:1016-1023` | `reconnect,` → `mut reconnect,`; add `connect,`. |
| loop state `:1027-1031` | add `let mut last_attempt: Option<Attempt> = None;` and `let dials = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));` (ruling O-1). |
| loop `match` after `ApplyMigrations` (`:1059-1081`), before the runtime group (`:1090`) | five arms: `ConnectionInfo`, `SetDsn(dsn)`, `ClearDsn`, `RebuildCache`, each producing `reply` for the send at `:1126-1129`. |
| events arm `:1133-1141` `Online` | after `go_online(..)`: `last_attempt = Some(Attempt { at: Utc::now(), outcome: AttemptOutcome::Online });` |
| `:1142-1150` `MigrationsPending` | `last_attempt = Some(Attempt { at: Utc::now(), outcome: AttemptOutcome::MigrationsPending(n) });` |
| `:1151-1154` `Failed(why)` | `last_attempt = Some(Attempt { at: Utc::now(), outcome: AttemptOutcome::Failed(why.clone()) });` before the `warn!`. |
| ticker arm `:1162-1171` | the spawned dial captures `dials.clone()` and the current generation and sends only when the generation still matches (ruling O-1). |

`chrono::Utc` is already imported in this file (the existing tests use `Utc::now()`); confirm at
`:14-40`.

Variant doc comments (verbatim):

```rust
    /// The connection as the Settings > Connection section shows it (MOD-15 M6, D4): backend
    /// label, whether a DSN is stored (never the DSN), the mirror's `cache_meta`, the last dial.
    ConnectionInfo,
    /// Store `dsn` in the keyring, open its mirror and start dialling it, without a restart
    /// (D11). Carries the redacting newtype, never a `String` (M5 review L-9).
    SetDsn(Dsn),
    /// Remove the keyring entry and stop dialling. The live connection, if any, is kept until
    /// quit (D13).
    ClearDsn,
    /// `CacheStore::rebuild()` on the current mirror: the sixteen mirrored tables and the cursor
    /// go, the file and its `cache_meta` stay (D14).
    RebuildCache,
```

The `ConnectionInfo` loop arm:

```rust
StoreRequest::ConnectionInfo => {
    connection::snapshot(&backend, last_attempt.as_ref(), connect.as_ref())
        .await
        .map_or_else(|err| failed("connection_info", &err), StoreReply::Connection)
}
```

The `SetDsn` arm — ordered steps, each with its failure behaviour (D11, D12; flag D):

```rust
StoreRequest::SetDsn(dsn) => 'set: {
    // (0) Memory has no keyring and no mirror (D10).
    if matches!(backend, Backend::Memory(_)) {
        break 'set StoreReply::Failed { request: "set_dsn", message: connection::DEMO_SESSION.to_owned() };
    }
    // (0') No context means `detached` outside `--demo`: nothing to open a mirror under.
    let Some(ctx) = connect.as_ref() else {
        break 'set StoreReply::Failed { request: "set_dsn", message: connection::NO_WORKER.to_owned() };
    };
    // (1) The DSN was validated by the section; (2) keyring write, (3) new mirror — both
    // inside `apply_dsn`. Any failure here leaves `backend`, `refresher`, `health`, `held`,
    // `reconnect` exactly as they were and answers `Failed`.
    let applied = match connect::apply_dsn(dsn.clone(), ctx).await {
        Ok(applied) => applied,
        Err(err) => break 'set failed("set_dsn", &err),
    };
    // (4) The refresher mirrored the old server; the watch was armed for it.
    if let Some(previous) = refresher.take() { previous.abort(); }
    health = None;
    // (5) The old mirror's pool. `close()` is idempotent and awaits the writer.
    if let Some(old) = backend.cache() { old.close().await; }
    // (6) Nothing after this point can fail. The old `PgStore`, if any, drops with the old
    // backend (its pool closes on drop); the held one belongs to the old server too. The dial
    // generation is bumped here, so a dial spawned before this point is discarded (O-1).
    held = None;
    pending = None;
    dials.fetch_add(1, Ordering::SeqCst);
    backend = Backend::Offline { cache: applied.cache, since: if ctx.offline { Some(Utc::now()) } else { None } };
    // (7) Every later tick dials the new server.
    reconnect = if ctx.offline { None } else { Some(applied.reconnect) };
    // (8) One immediate dial, on the events channel the ticker uses (D11 step 8; D12 skips it).
    if let Some(dial) = reconnect.clone() {
        let sender = events_tx.clone();
        let dials = dials.clone();
        let generation = dials.load(Ordering::SeqCst);
        tokio::spawn(async move {
            let event = dial().await;
            if dials.load(Ordering::SeqCst) == generation {
                let _ = sender.send(event).await;
            }
        });
    }
    last_attempt = None;
    connection::snapshot(&backend, None, connect.as_ref())
        .await
        .map_or_else(|err| failed("set_dsn", &err), StoreReply::Connection)
}
```

The section decides the D12 notice from `snapshot.offline` (§4.6), not from a worker message, so the
reply is the same `Connection` shape on both paths. `events_tx` is the `Started` field the worker
already holds (`connect.rs:110-112`); it is in the destructure at `:1016-1023` (the ticker arm at
`:1162-1171` uses it as `sender`). `'set: { .. break 'set .. }` is a labelled block (edition 2024,
MSRV 1.98 — fine); an implementer who prefers a helper `async fn set_dsn(..)` taking every loop
variable by `&mut` may do that instead, as long as the order and the "nothing after (6) fails" line
survive.

`ClearDsn` (D13):

```rust
StoreRequest::ClearDsn => 'clear: {
    if matches!(backend, Backend::Memory(_)) {
        break 'clear StoreReply::Failed { request: "clear_dsn", message: connection::DEMO_SESSION.to_owned() };
    }
    if let Err(err) = connect::forget_dsn().await {
        break 'clear failed("clear_dsn", &err);
    }
    reconnect = None;  // the ticker's guard is `reconnect.is_some() && ..` (`:1162-1171`)
    connection::snapshot(&backend, last_attempt.as_ref(), connect.as_ref())
        .await
        .map_or_else(|err| failed("clear_dsn", &err), StoreReply::Connection)
}
```

`backend`, `refresher`, `health`, `held` untouched: an `Online` backend stays online until quit, which
is the sentence the section shows.

`RebuildCache` (D14):

```rust
StoreRequest::RebuildCache => 'rebuild: {
    let Some(cache) = backend.cache() else {
        break 'rebuild StoreReply::Failed { request: "rebuild_cache", message: connection::DEMO_SESSION.to_owned() };
    };
    if let Err(err) = cache.rebuild().await {
        break 'rebuild failed("rebuild_cache", &err);
    }
    connection::snapshot(&backend, last_attempt.as_ref(), connect.as_ref())
        .await
        .map_or_else(|err| failed("rebuild_cache", &err), StoreReply::Connection)
}
```

Same call the two production callers make (`hierarchy.rs:339`, `catalogue.rs:171-177`); the refresher,
if running, refills from cursor zero on its next pass exactly as after those.

## 3. T2 tests — `crates/htui/tests/connection.rs` (worker half)

Header `#![cfg(feature = "testkit")]`, imports as `tests/prompt_settings.rs:15-36`, plus
`htui::connection::{AttemptOutcome, ConnectionSnapshot, REQUEST_NAMES}`, `htui_store::testkit as common`,
`htui_store::{ConnectContext, Dsn}`. Every test that can reach the keyring begins with
`let _keyring = common::mock_keyring().await;`. The worker is driven as `tests/prompt_preview.rs:330-340`
drives it: `htui::store_worker::spawn(started, request_rx, reply_tx)` with
`RequestEnvelope { seq, origin: Origin::Tab(SettingsTab::ID), request }` and a `reply_rx.recv()` loop
that ignores replies whose `seq` is not the one sent. Helper `connection(reply) -> ConnectionSnapshot`
panics on anything but `StoreReply::Connection`; `refusal(reply) -> (&'static str, String)` as
`prompt_settings.rs:81`.

| Test | Asserts |
|---|---|
| `connection_names_are_stable` | `name()` of the four requests equals `REQUEST_NAMES` in order; `READ_NAME == "connection_info"`. |
| `try_serve_refuses_every_writer_by_name` | `store_worker::serve(&Backend::memory(MemStore::demo()), req)` for the three writers → `Failed { request, message }` with `request` in `REQUEST_NAMES[1..]` and `message == "no connection worker in this build"`. |
| `memory_answers_none_for_dsn_stored_and_never_opens_the_keyring` | **no** `mock_keyring` guard; `serve(memory, ConnectionInfo)` → `Connection { dsn_stored: None, mirror: None, label: "memory", .. }`. |
| `the_worker_refuses_the_writers_on_memory` | spawned worker over `Started::detached(memory)`: `SetDsn`, `ClearDsn`, `RebuildCache` → `Failed` with `"a demo session has no DSN to change"`. |
| `set_dsn_without_a_context_fails_and_changes_nothing` | `mock_keyring`; `Started::detached(Backend::Offline { cache, since: Some(..) })` with `connect: None`; `SetDsn(valid)` → `Failed { request: "set_dsn", message: "no connection worker in this build" }`; then `ConnectionInfo` → `dsn_stored: Some(false)`, `mirror.db_fingerprint` unchanged; `fake_dsn() == None`. |
| `an_offline_backend_reports_a_stored_dsn_by_summary_only` | `mock_keyring`; `secret::set_dsn(text)`; `ConnectionInfo` over `Offline` → `dsn_stored: Some(true)`, `dsn_summary == Some(Dsn::parse(text).unwrap().summary())`; `format!("{snapshot:?}")` does not contain the password. |
| `an_unparseable_stored_dsn_reports_stored_without_a_summary` | `secret::set_dsn("postgres://h/d?foo=bar")`; `ConnectionInfo` → `dsn_stored: Some(true), dsn_summary: None` (B-7). |
| `clear_dsn_empties_the_keyring_and_keeps_the_backend` | `mock_keyring`; store; `ClearDsn` → `Connection { dsn_stored: Some(false), .. }`; `fake_dsn() == None`; label unchanged. |
| `set_dsn_under_offline_stores_but_does_not_dial` | `connect: Some(ConnectContext { offline: true, .. })`; `SetDsn(live)` → `Connection { offline: true, dsn_stored: Some(true), label starts with "offline", .. }`; after 3 s the label still starts with `offline`. Postgres-gated. |
| `set_dsn_goes_online_without_a_restart` (**headline**) | `mock_keyring`; `fresh_db`; `Started::detached(Offline { cache under tempdir, since: Some })` + `connect: Some(ctx)`; `SetDsn(Dsn::parse(&db.dsn))` → `Connection { dsn_stored: Some(true), label: "connecting", mirror.db_fingerprint == fingerprint, .. }`; poll `ConnectionInfo` every 250 ms up to 30 s until `label == "online"`; `<tempdir>/cache/<fingerprint>/` exists. Postgres-gated. |
| `a_second_set_dsn_swaps_the_mirror_and_leaves_the_old_file` | two `fresh_db`s; after the second `SetDsn`, `mirror.db_fingerprint` is the second's; the first directory still exists with its `cache.sqlite`. Postgres-gated. |
| `a_dial_in_flight_across_a_set_dsn_is_discarded` (ruling O-1) | a slow `reconnect` spawned before `SetDsn`; after the swap its `ConnEvent::Online` never installs a backend — the label stays `connecting`/`offline` for the old server and the mirror is the new one. |
| `rebuild_cache_empties_the_mirrored_tables_and_keeps_the_meta` | `Offline` over a cache seeded via `common::seed_mirror`; the three `cache_meta` fields equal before/after, `last_full_refresh_at: None`, `SELECT count(*) FROM item` is 0. Not Postgres-gated. |
| `a_failed_dial_is_the_last_attempt` | `Started::detached` with a `reconnect` answering `ConnEvent::Failed("refused")`; after one tick, `ConnectionInfo.last_attempt == Some(Attempt { outcome: Failed("refused"), .. })`. |

`cargo test -p htui --all-features --test connection` runs the file; with `HTUI_TEST_DATABASE_URL` set
the gated cases run too.

## 4. T3 — `crates/htui/src/ui/tabs/settings/connection.rs` (new)

### 4.1 Imports

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::Scope;
use htui_store::Dsn;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use zeroize::Zeroizing;

use crate::app::{Ctx, Handled};
use crate::connection::{AttemptOutcome, ConnectionSnapshot, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::TextField;
use crate::ui::tabs::settings::{SectionId, SettingsSection, wrapped};
use crate::ui::FieldOutcome;
```

### 4.2 Constants (every user-visible string)

| Const | Text |
|---|---|
| `NOT_READ` | `not read yet` |
| `UNAVAILABLE` | `connection info is unavailable: {message}` (format, `message` = the seam's) |
| `HINT_BROWSE` | `e edit DSN · c clear DSN · R rebuild cache · r reload · j/k rows` |
| `HINT_EDITING` | `Enter store · Esc cancel · typed text is never shown` |
| `HINT_CONFIRM` | `y confirm · n / Esc cancel` |
| `STORED` | `stored` |
| `STORED_OFFLINE` | `stored; this session was started with --offline, so it takes effect on the next launch` |
| `CLEARED` | `the DSN is gone from the keyring — this session keeps its current connection until you quit` |
| `REBUILT` | `mirror rebuilt; the next refresh pass refills it` |
| `EMPTY_FIELD` | `nothing typed; the stored DSN is unchanged` |
| `NO_DSN_YET` | `no DSN is stored; type one and press Enter` |
| `DEMO_ROW` | `n/a in a demo session` |
| `CONFIRM_CLEAR` | `Remove the DSN from the keyring? This session keeps its current connection; the next launch starts offline. y / n` |
| `CONFIRM_REBUILD` | `Rebuild the mirror? Survives: the file, schema_version, db_fingerprint, built_at, the pending/ buffer. Goes: the 16 mirrored tables, cache_cursor, last_full_refresh_at. The next refresh pass refills it. y / n` |
| `NOT_STORED` | `not stored` |
| `STORED_UNREADABLE` | `stored — not readable by this build; e replaces it` |
| `NO_ATTEMPT` | `no dial yet this session` |
| `BUSY` | `{busy} in flight` (hint suffix) |

`rebuild_copy_names_both_lists` greps `CONFIRM_REBUILD` for `schema_version`, `db_fingerprint`,
`built_at`, `pending/`, `cache_cursor`, `last_full_refresh_at`, `16 mirrored tables`.

### 4.3 Types

```rust
/// The four rows, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row { Status, Dsn, Mirror, Rebuild }

impl Row {
    const ALL: [Self; 4] = [Self::Status, Self::Dsn, Self::Mirror, Self::Rebuild];
    fn label(self) -> &'static str { /* "Status", "DSN", "Mirror", "Rebuild cache" */ }
}

/// The DSN field. Its buffer is masked and zeroizing (D3); this printer shows only the length.
struct Editor { input: TextField }

impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor").field("len", &self.input.len()).finish()
    }
}

/// Where the section is. `captures_input` is `!Browse` (D17).
#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    Editing(Editor),
    ConfirmClear,
    ConfirmRebuild { stage: ConfirmStage },
}

impl core::fmt::Debug for Mode { /* Browse / Editing(editor) / ConfirmClear / ConfirmRebuild { stage } */ }

/// `kinds::DeleteStage` (`kinds.rs:307-314`), one section over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmStage { Asking, InFlight }

/// The last outcome (B-8: hand-written `Debug`, prints the kind and the length only).
#[derive(Clone, PartialEq, Eq)]
enum Notice { Info(String), Error(String) }

impl Notice { fn text(&self) -> &str; fn is_error(&self) -> bool; }

/// Settings > Connection.
#[derive(Debug)]
pub struct ConnectionSection {
    snapshot: Option<ConnectionSnapshot>,
    /// The seam's sentence when the read failed, shown as `UNAVAILABLE`.
    unavailable: Option<String>,
    selected: usize,
    mode: Mode,
    notice: Option<Notice>,
    /// The name of the request in flight, for the hint and for `Esc`/`e` blocking.
    busy: Option<&'static str>,
    /// D8: the editor opened itself once for `dsn_stored == Some(false)`; reset on scope change.
    opened_for_empty: bool,
}

impl ConnectionSection {
    pub const ID: SectionId = SectionId("connection");
    #[must_use] pub fn new() -> Self;
}
impl Default for ConnectionSection { fn default() -> Self { Self::new() } }
```

`ConfirmClear` has no `InFlight` stage on purpose: `y` sends and returns to `Browse` with
`busy = Some("clear_dsn")`, the same as `prompt`'s writes; `ConfirmRebuild` keeps the two-stage shape
because a rebuild can take seconds and a second `y` must be inert.

### 4.4 Rows and the pane

`row_text(&self, row) -> String` (the value column, wrapped by `wrapped` from `settings/mod.rs:73-94`):

- `Status`: `{label}` then ` · ` and `last dial {at:%H:%M:%S}: online` / `{n} migrations pending` /
  `failed: {why}` / `NO_ATTEMPT`.
- `Dsn`: `dsn_stored == None` → `DEMO_ROW`; `Some(false)` → `NOT_STORED`; `Some(true)` with summary →
  `stored — {summary}`; without → `STORED_UNREADABLE`.
- `Mirror`: `mirror == None` → `DEMO_ROW`; else
  `{fp[..12]} · built {built_at:%Y-%m-%d %H:%M} · last full refresh {ts|never} · schema {schema_version}`,
  with `fp.get(..12).unwrap_or(fp)`.
- `Rebuild`: `press Enter or R`.

Before the first reply every value is `NOT_READ`; while `unavailable` is `Some`, the pane's first line
is `UNAVAILABLE`.

`pane(&self, width) -> Vec<Line>`: `Mode::Editing` → label `DSN:` +
`editor.input.line(width - 5, true, theme)` (the `•` window, `text_field.rs:197-251`) +
`wrapped(NO_DSN_YET or the notice)`; `ConfirmClear` → `wrapped(CONFIRM_CLEAR)`;
`ConfirmRebuild { Asking }` → `wrapped(CONFIRM_REBUILD)`; `{ InFlight }` → `rebuilding…`; `Browse` →
the notice if any. `render` layout `[Min(3), Length(pane.len()), Length(1)]` as `prompt.rs:875-912`;
the selected row carries `theme.selected`; hint on the last line.

### 4.5 Keys

| Mode | Key | Effect |
|---|---|---|
| Browse | `j` / `Down`, `k` / `Up` | move `selected` within `Row::ALL`; consumed. |
| Browse | `e` | if `blocked()` → consumed with nothing; else `open_edit()`. |
| Browse | `c` | if `dsn_stored != Some(true)` → `refuse(NOT_STORED)`; if `blocked()` → nothing; else `mode = ConfirmClear`. |
| Browse | `R`, or `Enter` on `Rebuild` | if `mirror.is_none()` → `refuse(DEMO_ROW)`; if `blocked()` → nothing; else `mode = ConfirmRebuild { stage: Asking }`. |
| Browse | `r` | `send(ConnectionInfo)` — allowed while busy, as M3 chose (HANDOFF `:455-459`). |
| Browse | `Esc` | consumed only when `notice.is_some()` (clears it); otherwise `Pass`. |
| Browse | anything else | `Pass`. |
| Editing | `CONTROL` chords | `Pass` (`ctrl-c` still quits). |
| Editing | field keys | `editor.input.on_key(key)`: `Consumed` → consumed; `Submit` → `submit(ctx)`; `Cancel` → drop the editor, `mode = Browse`; `Pass` → `Handled::Consumed`. |
| ConfirmClear | `y` | `mode = Browse; send(ClearDsn)`. |
| ConfirmClear | `n` / `Esc` | `mode = Browse`. |
| ConfirmRebuild Asking | `y` | `stage = InFlight; send(RebuildCache)`. |
| ConfirmRebuild Asking | `n` / `Esc` | `mode = Browse`. |
| ConfirmRebuild InFlight | any (non-CONTROL) | swallowed. |
| any non-Browse | unlisted | `Consumed` (`kinds.rs:677-710`'s rule). |

`captures_input(&self) -> bool { !matches!(self.mode, Mode::Browse) }` (D17).
`blocked(&self) -> bool { self.busy.is_some() || self.unavailable.is_some() || self.snapshot.is_none() }`
(M5 review L-10, `48ef06d`).

### 4.6 Submit and replies

```rust
fn open_edit(&mut self) {
    self.notice = None;
    self.mode = Mode::Editing(Editor { input: TextField::masked() });
}

fn submit(&mut self, ctx: &mut Ctx<'_>) {
    let Mode::Editing(editor) = &mut self.mode else { return };
    if editor.input.is_empty() {
        self.mode = Mode::Browse;
        self.say(EMPTY_FIELD);
        return;
    }
    // Moves the buffer out (no copy) into a wiping wrapper; `Dsn::parse` takes its own.
    let raw = Zeroizing::new(editor.input.take());
    match Dsn::parse(&raw) {
        Ok(dsn) => {
            self.mode = Mode::Browse;
            self.send(StoreRequest::SetDsn(dsn), ctx);
        }
        Err(err) => {
            // The field is already empty; the user retypes. Nothing is emitted, nothing logged.
            self.refuse(err.to_string());
        }
    }
}
```

A refusal keeps `Mode::Editing` with an empty field and shows the sentence under it; `Esc` leaves. The
three disposal points ANA-10 (6) names: `take()` at submit, `Esc`, and `on_scope_change`.

`on_reply(&mut self, reply, _ctx)`:

- `StoreReply::Connection(snapshot)`: `unavailable = None; snapshot = Some(..)`, then by `busy.take()`:
  `Some("set_dsn")` → `say(if snapshot.offline { STORED_OFFLINE } else { STORED })`;
  `Some("clear_dsn")` → `say(CLEARED)`; `Some("rebuild_cache")` → `mode = Browse; say(REBUILT)`;
  `Some(READ_NAME) | None` → nothing said. Then D8:
  `if snapshot.dsn_stored == Some(false) && !self.opened_for_empty && matches!(self.mode, Mode::Browse) { self.opened_for_empty = true; self.open_edit(); self.say(NO_DSN_YET); }`.
- `Failed { request, message } if *request == READ_NAME` → `unavailable = Some(..); busy = None`.
- `Failed { request, message } if REQUEST_NAMES.contains(request)` → `busy = None;` leave
  `ConfirmRebuild` for `Browse`; `refuse(message.clone())`.
- anything else ignored.

`send(request, ctx) { self.busy = Some(request.name()); self.notice = None; ctx.request(request); }`
as `prompt.rs:455-458`.

`impl SettingsSection`: `id` → `Self::ID`; `title` → `"Connection"`; `wants_requests(_)` →
`vec![StoreRequest::ConnectionInfo]`; `on_scope_change` →
`mode = Browse; opened_for_empty = false; notice = None` (snapshot kept: it is not scoped);
`captures_input`; `on_key`; `on_reply`; `render`.

### 4.7 In-module tests

- `an_editor_never_prints_its_buffer`: `format!("{editor:?}") == "Editor { len: 19 }"`;
  `format!("{:?}", Mode::Editing(editor))` contains no password.
- `a_notice_never_prints_its_text`: `format!("{:?}", Notice::Error("postgres://x".into()))` contains
  no `postgres`.
- `the_request_names_are_the_module_constants`: the `busy` matches use `REQUEST_NAMES[1..]` by
  constant, not by literal.

## 5. Shell and widget edits

### 5.1 `crates/htui/src/ui/tabs/settings/mod.rs`

- `:11-14`: `pub mod connection;` after `pub mod agents;`.
- `:29-32`: `pub use connection::ConnectionSection;` first.
- `SettingsRegistry`, after `cycle_prev` (`:220-224`):

```rust
    /// Activates the section with this id (`TabAction::FocusSection`). `false` when it is not
    /// registered, and nothing moves.
    pub fn focus(&mut self, id: SectionId) -> bool {
        match self.sections.iter().position(|section| section.id() == id) {
            Some(idx) => { self.active = idx; true }
            None => false,
        }
    }
```

- `impl Tab for SettingsTab`, after `on_reply` (`:321-325`):

```rust
    fn focus_section(&mut self, section: SectionId) -> bool {
        self.sections.focus(section)
    }
```

`SectionId` needs `PartialEq`/`Copy` — check the derives at `:116-117` and add what is missing (a
one-word change with no other effect).

### 5.2 `crates/htui/src/ui/tabs/registry.rs`

After `render` (`:48`), inside the trait:

```rust
    /// Moves this tab's own focus to `section` (a Settings section today). `false` when the tab
    /// has no such section — every tab but Settings. A default, so no other tab changes.
    fn focus_section(&mut self, _section: SectionId) -> bool {
        false
    }
```

plus `use crate::ui::tabs::settings::SectionId;`.

### 5.3 `crates/htui/src/app/action.rs`

`use crate::ui::tabs::settings::SectionId;` after `:10`; in `TabAction` (`:50-60`) after `Focus(TabId)`:

```rust
    /// Focus `tab` and, within it, `section` (MOD-15 M6 D7): the shell's redirect to the
    /// connection section when no DSN is stored. Unknown ids are no-ops, as `Focus`'s is.
    FocusSection(TabId, SectionId),
```

### 5.4 `crates/htui/src/app/update.rs`

`update_tab` (`:37-52`), after the `Focus` arm (`:45-47`):

```rust
            TabAction::FocusSection(tab, section) => {
                if self.tabs.focus(tab)
                    && let Some(active) = self.tabs.by_id_mut(tab)
                    && !active.focus_section(section)
                {
                    tracing::debug!(%tab, %section, "no such section to focus");
                }
            }
```

The existing `if self.tabs.active_id() != before { self.activate_tab(); }` at `:49-51` then issues every
Settings section's reads when the tab changed (flag F).

`on_app_reply` (`:262-279`), a second `if let` after the workspaces one:

```rust
        if let StoreReply::Connection(snapshot) = reply
            && snapshot.dsn_stored == Some(false)
            && !self.connection_redirect_done
        {
            // Once per session (D6): a box with no DSN lands on the field that fixes it. `None`
            // is `--demo` and never redirects; a later `Connection` reply under `Origin::App`
            // cannot fire it twice.
            self.connection_redirect_done = true;
            self.update(Action::Tab(TabAction::FocusSection(
                SettingsTab::ID,
                ConnectionSection::ID,
            )));
        }
```

### 5.5 `crates/htui/src/app/state.rs`

After `migration_prompt_shown` (`:184`): `/// D6: the no-DSN redirect fired; it fires once per session.`
`pub(super) connection_redirect_done: bool,`. In `App::new`'s literal after `migration_prompt_shown: false,`
(`:218`): `connection_redirect_done: false,`. In `start` (`:228-232`):
`self.dispatch(Origin::App, StoreRequest::ConnectionInfo);` as the fourth line.

### 5.6 `crates/htui/src/ui/text_field.rs` (flag E)

- `:10-14` imports: `use zeroize::{Zeroize as _, Zeroizing};`.
- `:34` `text: String,` → `text: Zeroizing<String>,` with the doc extended: "Wiped on drop (D3) — for a
  slug that costs nothing, for a DSN it is the point."
- `masked()` (`:70-75`): `Self { masked: true, text: Zeroizing::new(String::with_capacity(256)), ..Self::default() }`
  — the doc gains "Reserves 256 bytes so a DSN of ordinary length never reallocates, which would leave
  an unwiped copy behind."
- `with_text` (`:79-85`): `text: Zeroizing::new(text.to_owned())`.
- `text()` (`:156-158`): `Some(self.text.as_str())`.
- `take()` (`:161-164`): `core::mem::take(&mut *self.text)` — doc: "Moves the buffer out; a masked
  caller wraps it in `Zeroizing` at once."
- `clear()` (`:167-170`): `self.text.zeroize(); self.cursor = 0;`.
- Every other `self.text.` use compiles unchanged through `DerefMut`.
- Module doc `:7-8` ("`Zeroizing` … deliberately not built — milestone 6 owns") → "`Zeroizing` arrived
  with milestone 6."
- Tests: `debug_never_prints_the_text` (`:498-514`) unchanged; add `a_masked_field_reserves_its_buffer`
  and `clear_wipes_the_buffer_in_place`.

Callers of `take()`/`text()` elsewhere need no change: signatures are identical.

## 6. Registration and the strip pin

- `crates/htui/src/app/mod.rs:14`:
  `use crate::ui::tabs::settings::{AgentsSection, ConnectionSection, HierarchySection, KindsSection, PromptSection};`;
  `:47-52` vector gains `Box::new(ConnectionSection::new()),` after `PromptSection` (D19: last).
- `crates/htui/tests/settings.rs:11-14` import gains `ConnectionSection`;
  `the_section_strip_fits_the_frame` (`:942-958`) vector (`:944-949`) gains
  `Box::new(ConnectionSection::new())`. Five titles at 46 of 100 columns; the assertion is the existing
  one. If it no longer fits, the fix is the strip's, recorded in HANDOFF, not a shorter title.

## 7. T3 tests — `crates/htui/tests/connection.rs` (section half) and snapshots

Helpers mirror `tests/prompt_settings.rs:559-611`: `connection_over(store) -> Harness` (five sections,
four `l` to reach the fifth), `bench_with(snapshot) -> (SectionBench, ConnectionSection)`,
`stored_snapshot()`, `empty_snapshot()`, `demo_snapshot()`, `feed(bench, section, chars)` typing a
string as a burst of `KeyCode::Char` (ANA-10 (4): paste is a burst).

| Test | Asserts |
|---|---|
| `the_field_renders_dots_and_a_count_never_the_text` | `e`, feed `postgres://u:pw@h/d`; render contains `•••` and `(19)`, not `pw`; `format!("{section:?}")` not `pw`. |
| `a_refused_dsn_emits_no_store_request` | feed `nope`, `Enter`; `drained()` has no `Action::Store`; render contains `not a URL`; still editing. |
| `each_refusal_is_one_of_five_sentences` | five inputs → the five `DsnError` sentences, no other text from the input. |
| `a_valid_dsn_emits_set_dsn_and_marks_busy` | exactly one `Action::Store(StoreRequest::SetDsn(_))`; hint ends `set_dsn in flight`; `e` inert while busy. |
| `l_h_and_brackets_are_characters_while_editing` | count rises by four; `captures_input()` is `true`. |
| `esc_disposes_the_field` | `captures_input()` false; `e` reopens with `(0)`. |
| `a_scope_change_disposes_the_field` | `captures_input()` false after `on_scope_change`. |
| `c_needs_a_confirmation` | `c` → no action + `CONFIRM_CLEAR`; `n` → back; `c`,`y` → one `ClearDsn`. |
| `c_refuses_when_nothing_is_stored` | error text `not stored`, no action. |
| `rebuild_needs_a_confirmation_and_names_both_lists` | the seven greps; `y` → one `RebuildCache`; a second `y` → nothing. |
| `enter_on_the_rebuild_row_is_r` | `j` ×3, `Enter` → the confirm. |
| `the_offline_notice_after_set_dsn` | `offline: true` → `STORED_OFFLINE`; `false` → `STORED`. |
| `clear_reply_says_the_session_keeps_its_connection` | `CLEARED` text. |
| `an_empty_snapshot_opens_the_editor_once` | opens; `Esc`; the same reply again → stays browse. |
| `a_demo_snapshot_never_opens_the_editor` | browse, DSN row `n/a in a demo session`. |
| `a_failed_read_blocks_e` | `UNAVAILABLE` on screen; `e` inert. |
| `a_failed_write_shows_the_seams_sentence` | the seam's message on screen, busy cleared. |
| `focus_section_selects_from_any_tab` (Harness) | `FocusSection(SettingsTab::ID, ConnectionSection::ID)` from Backlog selects it; an unknown `SectionId` is a no-op. |
| `the_redirect_fires_once` (Harness) | empty keyring → Settings/Connection with the editor open; after `Esc` and `1`, a second App-origin `Connection` reply leaves it on Backlog. |
| `a_demo_session_never_redirects` (Harness) | `Harness::over(MemStore::demo())` stays on Backlog. |

Snapshots (`crates/htui/tests/snapshots/connection__*.snap`, width 100): `connection__empty`,
`connection__stored`, `connection__editor`, `connection__confirm`.

## 8. Data flow

`App::start` → `ConnectionInfo` under `Origin::App` → worker loop arm → `connection::snapshot`
(keyring via `spawn_blocking`, `cache.meta()`) → `StoreReply::Connection` → `on_app_reply` → (once,
`dsn_stored == Some(false)`) `TabAction::FocusSection` → `TabRegistry::focus` +
`SettingsTab::focus_section` → `activate_tab` → the section's own `ConnectionInfo` under
`Origin::Tab(settings)` → `on_reply` → `open_edit` (D8). The user types → `TextField` (masked,
zeroizing) → `Enter` → `Dsn::parse` (scan, then sqlx gate) → `Ctx::request(SetDsn(Dsn))` → worker
`SetDsn` arm → `connect::apply_dsn` (keyring write on the blocking pool, `CacheStore::open`) →
refresher aborted, old cache closed, backend swapped, `reconnect` replaced, generation bumped, one
dial spawned → `Connection` reply → `STORED` → the dial's `ConnEvent::Online` → `go_online` →
`last_attempt` → the top bar reads `online` on its next `StoreState`. `ClearDsn` and `RebuildCache`
are the two short arcs of the same shape.

## 8b. T4 — docs

- `README.md:57-71` "Storing the DSN": one paragraph after the `htui --set-dsn` example — the in-app
  path (`Settings > Connection`, `e`, the masked field, the five refusals, `c`, `R`), and that a box
  with no DSN lands there on first launch.
- `HANDOFF.md`: the MOD-15 line (`:284`) gets a "**Milestone 6 landed (`<first>`..`<last>`,
  2026-09-17): a box with no DSN can fix itself.**" paragraph after M5's (`:460-520`), naming:
  `Dsn`/`DsnError`, `apply_dsn`/`forget_dsn`/`ConnectContext`, `StoreRequest` 50 → **54**,
  `StoreReply` 28 → **29**, the eight-step swap and its generation guard, flags H/J/L as live
  coordinates (sqlx logs unknown parameters; `from_str` reads `.pgpass`; keyring's mock is
  entry-local so the fake slot exists), the test counts, the reviewer's verdict. `:288-289` is
  corrected in place to "`rebuild` has two production callers (`hierarchy.rs:339`,
  `catalogue.rs:172`) and, from this milestone, the `RebuildCache` request." The `MaskedField` note
  (`:317-319`) gets "landed as `TextField::masked()` with a zeroizing buffer".
- `.claude/prds/mod-15-hierarchy-management.prd.md` milestone row 6:
  `complete | [plan](../plans/mod-15-connection-section.plan.md)`.
- Close-out (this is the item's last milestone): the whole HANDOFF entry moves to
  `docs/decisions/mod/mod-15.md`; `DECISIONS.md` gains its one index line; the checklist line is
  deleted; the summary table and status line updated;
  `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` proves it.

## 9. Build order and commit plan

Tests first in every step; the "red" command fails to compile or fails, the "green" one passes.

| # | Commit | Files | Red → green |
|---|---|---|---|
| 1 | `build(htui-store): promote zeroize` | 3 manifests | `cargo tree -p htui-store -i zeroize` shows a direct edge; `cargo check -p htui-store` green. |
| 2 | `test(htui-store): the DSN newtype's contract` | `crates/htui-store/tests/dsn.rs` | `cargo test -p htui-store --features test-support --test dsn` — E0433. |
| 3 | `feat(htui-store): Dsn, DsnError and the pre-scan (D1, D2)` | `src/dsn.rs`, `src/lib.rs` | the parse/summary/fingerprint tests green; the rest still red. |
| 4 | `feat(htui-store): a fake keyring for the tests (flag L)` | `src/secret.rs`, `src/testkit.rs` | `the_fake_keyring_round_trips` green; `cargo test -p htui-store --lib secret` still green. |
| 5 | `feat(htui-store): apply_dsn, forget_dsn, ConnectContext (D11, D13, D20)` | `src/connect.rs`, `src/lib.rs` | whole `dsn.rs` green; `cargo test -p htui-store --all-features` green. |
| 6 | `test(htui): connection names and refusals` | `tests/connection.rs` (worker half) | `cargo test -p htui --all-features --test connection` — E0433. |
| 7 | `feat(htui): connection snapshot and the four requests (D4, D9, D10)` | `src/connection.rs`, `src/lib.rs`, `store_worker.rs`, `app/state.rs` (start dispatch) | `name_arms_are_stable` updated for 54; the read tests green; writer tests red via `try_serve`'s refusal. |
| 8 | `feat(htui): SetDsn, ClearDsn, RebuildCache in the worker loop (D11–D14, O-1)` | `store_worker.rs` loop arms, `last_attempt`, `dials` | every non-gated worker test green; then the Postgres-live run for the headline and its siblings. |
| 9 | `feat(htui): TextField zeroizes (D3)` | `ui/text_field.rs` | `cargo test -p htui --lib text_field` green incl. the two new tests. |
| 10 | `test(htui): the connection section` | `tests/connection.rs` (section half), `tests/settings.rs` | E0433 on `ConnectionSection`. |
| 11 | `feat(htui): ConnectionSection (D8, D15–D17, D19)` | `settings/connection.rs`, `settings/mod.rs`, `app/mod.rs` | section tests green except focus/redirect; four snapshots accepted and read by eye. |
| 12 | `feat(htui): FocusSection and the no-DSN redirect (D6, D7)` | `settings/mod.rs`, `registry.rs`, `app/action.rs`, `app/update.rs`, `app/state.rs` | the three Harness tests green. |
| 13 | gates | — | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-features --all-targets -- -D warnings`; `cargo test --workspace --all-features`; the same with `HTUI_TEST_DATABASE_URL=…`; `cargo check -p htui-store` (no features, proves the fake is compiled out). |
| 14 | `docs(mod-15): milestone 6 landed` | README, HANDOFF, PRD | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`. |

Each commit is made as it goes — uncommitted subagent work dies with the session. No `.sqlx` file, no
migration, no `query!` and no `WriteStore` change at any step;
`git diff --stat -- '.sqlx' crates/htui-store/migrations crates/htui-store/cache_migrations` is empty
at step 13.

## 10. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | sqlx logs an unrecognised query parameter with its **value** (`parse.rs:107`). | The pre-scan refuses before `from_str` (flag H); `an_unknown_parameter_is_refused_before_sqlx_sees_it`. |
| H-2 | `PgConnectOptions::from_str` reads env and `~/.pgpass` on the UI task (flag J). | Priced: one small read on `Enter`, only without a password. Recorded in HANDOFF. |
| H-3 | `apply_dsn`: the keyring write succeeds, `CacheStore::open` fails. | The new DSN is stored, the backend is untouched, the reply is `Failed`; the next launch uses the new DSN. Documented on `apply_dsn`; `r` then shows `dsn_stored: Some(true)` with the new summary, so the screen is honest. |
| H-4 | Old `PgStore` dropped rather than closed at step (6): `PgStore` has no `close`. | The pool closes on drop; the refresher that used it was aborted at (4). Open item O-2. |
| H-5 | The old cache's `close().await` at (5) while an aborted refresher's last write is in flight. | `close` awaits the writer (`cache/mod.rs:215-226`); the refresher is aborted first. |
| H-6 | A `ConnEvent` from a dial spawned **before** `SetDsn` installs the old server's `PgStore` over the new mirror. | **Fixed here** (ruling O-1): the `dials` generation counter; the dial drops its event when the generation moved. Test: `a_dial_in_flight_across_a_set_dsn_is_discarded`. |
| H-7 | keyring's mock is entry-local (flag L). | The fake slot; `the_fake_keyring_round_trips`. |
| H-8 | Two tests touching the fake slot in parallel. | `mock_keyring()` holds a process-wide `tokio::sync::Mutex` until the guard drops. |
| H-9 | A test without the guard calling `set_dsn` writes the developer's keyring. | The doc on `mock_keyring`; `memory_answers_none…` proves `Memory` never reads it; every keyring test takes the guard first. |
| H-10 | `TextField::take()` on a masked field returns a `String` a careless caller could print. | One masked caller, wrapping in `Zeroizing` on the same line; the reviewer enforces it. |
| H-11 | `Notice::Error(err.to_string())`. | `DsnError`'s `Display` is the fixed sentence; `the_error_never_carries_the_text`. |
| H-12 | The section strip at 46 columns with five titles. | `the_section_strip_fits_the_frame`. |
| H-13 | `StoreRequest` derives `Clone`; the shell keeps only the discriminant (`state.rs:271-272`). | Nothing to add. |
| H-14 | `RequestEnvelope` derives `Debug`. | `Dsn`'s `Debug` is `Dsn(<redacted>)`; `debug_is_redacted`. |
| H-15 | `--offline` + `SetDsn` shows `offline · 0s`, which reads as failure. | `STORED_OFFLINE` says exactly what happened. |

## 11. What must not change

`WriteStore`/`ReadStore` (no new method), `.sqlx/`, `migrations/`, `cache_migrations/`,
`CacheStore::rebuild`'s body, `secret.rs`'s production path (the fake is
`cfg(feature = "test-support")` and installs only when a test asks), `attempt`/`try_connect`
signatures, `Backend`'s variants (no `Local`), `TextField`'s public signatures, every existing
snapshot, `name_arms_are_stable`'s existing arms (four appended), workspace lints, MSRV 1.98.
`--set-dsn`/`--clear-dsn` keep working unvalidated (flag C) — the CLI path is M1's and is not touched.

## 12. Open items

- **O-1 — closed by the maintainer's ruling (§0.3)**: the dial generation counter is built here, not
  deferred. H-6 carries the test.
- **O-2** `PgStore` has no `close()`; the old pool closes on drop (H-4).
- **O-3** `--set-dsn` still stores without `Dsn::parse` (flag C). Making the CLI validate is a one-line
  change in `lib.rs:120-133` that would also make an old raw DSN refusable at startup; left for the
  maintainer to decide.
- **O-4** `try_serve`'s `ConnectionInfo` answers without `last_attempt`/`offline` (flag K).
  `Harness::settle` therefore never sees a dial outcome; the Harness redirect tests do not need one.
- **O-5** The `StoreReply::Connection` for a writer carries the snapshot taken **before** the immediate
  dial resolves; the section says `stored`, and the top bar flips a moment later. A `Connection` push
  on `ConnEvent` would close it and is a worker→UI unsolicited-reply pattern this tree does not have.

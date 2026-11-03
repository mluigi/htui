# Blueprint: MOD-158 — Qdrant URL as a database-wide setting, API key sources (T1–T6)

> Produced by `code-architect` 2026-10-27 from the confirmed plan
> `.claude/plans/mod-158-qdrant-url-setting-and-key.plan.md` (D1–D12, Q1–Q3 answered as
> recommended). Tree read at `ee4b80c8` (branch `hr/MOD-158`). The plan stays the spec. §0 lists where
> the tree disagrees with a plan claim or a plan file set; every row is **adopted** and none reopens
> a decision. Line numbers are `HEAD` at `ee4b80c8`.

## 0. Plan corrections

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | **D12's place is wrong.** The run's test variables are not exported by `scripts/hr`: `HTUI_TEST_QDRANT_URL` is in the `dev` service's `environment:` block of `docker/hr/compose.hr.yaml:66`. `scripts/hr:636` and `:872` export only the host-side compose interpolation variables (`HR_TMP`, `HR_QDRANT`). | `docker/hr/compose.hr.yaml:64-66`; `scripts/hr:636, 872` | **adopted**: T6 adds `HTUI_QDRANT_URL: http://localhost:6334` beside `HTUI_TEST_QDRANT_URL` in `compose.hr.yaml`, pins it in `tests/hr/hr.bats` (beside test `3h`, which already pins that block with `jq`), and edits `docs/hr-sandbox.md:184-191, 479`. `scripts/hr` is **not** changed. |
| P-2 | **`go_online` is not the only backend swap.** `SetDsn` reassigns `backend = Backend::Offline { .. }` without a deref (`store_worker.rs:3070`), which the plan's `*backend = ` search missed, and `go_offline` (`:3527`) turns `Online` into `Offline` in place through `Backend::went_offline` (`htui-store/src/backend.rs:210`). Publishing only at start and in `go_online` would leave a dead old-server `PgStore` in the slot after a `SetDsn` and after a lost server. | `store_worker.rs:3070, 3486-3507, 3527-3545` | **adopted**: the loop publishes at the **top of every iteration**, next to `host_the_backend` (`store_worker.rs:2927`, MOD-11 B-2's rule for the same problem): `concepts.publish(&backend)` (§T3.4). It is a pointer swap under a `std::sync::RwLock`, with no `.await` (`R-NF-3`). |
| P-3 | D1's `StoredQdrantUrl { url: QdrantUrl, .. }` makes a **malformed row impossible to fix** through D1: its read is an error, so no token can be read, and a compare-and-set miss's re-read errors too. | D1; `pg/write.rs:111` `cas_miss` re-reads | **adopted**: `StoredQdrantUrl { url: Result<QdrantUrl, QdrantUrlError>, token }`, plus `StoredQdrantUrl::usable()`, which returns D1's error naming the key. **Every resolver (D3) goes through `usable()`**, so a malformed row is never "no URL". Settings and the CLI can still overwrite it (an UPDATE by its token) or clear it. |
| P-4 | The CAS result for a row that is gone. The queue code answers `NotFound` (`mem.rs:4866-4872`); for one fixed key this is the same "changed elsewhere". | `mem.rs:4834-4888` | **adopted**: `set_qdrant_url`/`clear_qdrant_url` return `CasOutcome<Option<StoredQdrantUrl>>`. `Stale(None)` means the row expected is gone; `Stale(Some(_))` means it moved, or exists when none was expected. They never return `NotFound`. |
| P-5 | `QdrantSettings::new(QdrantUrl, ..)` (D2) breaks callers outside T1's planned set: `htui-store/src/vector.rs:915` (`Qdrant::from_url(&settings.url)`) and the test at `:1668`, `htui-store/tests/qdrant_live.rs:50, 530, 637`, `htui/src/concepts.rs:111, 369`, `htui/src/concepts_worker.rs:885`, `htui/tests/qdrant_worker.rs:27`, `htui/tests/state_cli_pg.rs:536`. | `search_text QdrantSettings` | **adopted**: T1 updates them **mechanically** (parse, then pass). T3 later rewrites the `htui` ones. T1's set grows, and is still disjoint from T2's (§W1). |
| P-6 | Plan row `crates/htui/src/app.rs` does not exist: the shell is `app/{mod,state,update}.rs`. A `QdrantInfo` added to `App::start` (`app/state.rs:856-861`) would also run in **every** `Harness` (`testkit.rs:134` calls `app.start()`). Over an `Offline` backend that reads the developer's **real** keyring (`qdrant-url`, `qdrant-key`), and a host keyring holding an old URL would put the notice into shell frames. | `app/state.rs:856`; `testkit.rs:122-134`; `lib.rs:250-251` (`register_all` then `start`) | **adopted**: `register_all` (`app/mod.rs:67`) sets `app.qdrant_offer_check = true`, and `App::start` sends `QdrantInfo` under `Origin::App` only when it is set. The binary runs `register_all` **before** `start`. The harness runs `start` in its constructor, before any test can call `register_all`, so no harness asks unless a test calls `start` again. T4 owns `app/{mod,state,update}.rs`. |
| P-7 | `tests/secrets_settings.rs` builds `QdrantSnapshot` literals (`:196 qdrant_stored`, `:621 a_typed_qdrant_key_is_sent_whole_and_redacted`). | grep | **adopted**: in T4's set. |
| P-8 | D5's test needs `crates/htui-agent/tests/launch.rs`, where the `spawn_sh` fixture lives (`:735-750`), and that file is not in the plan. | `tests/launch.rs:735` | **adopted**: in T2's set. |
| P-9 | **Other spawns inherit the environment.** (a) `htui-orch/src/isolate/git.rs:316` `Cli::command` runs git verbs (`merge --no-ff`, `commit`), and those run the **managed repository's hooks**, which are repository code. (b) `htui-agent/src/auth/browser.rs:146-178` spawns the platform opener during an agent login. Neither is an agent or step command, but both hand an env-borne key to code htui does not own. Not covered and not changed: `htui/src/editor.rs:143-154` (`$EDITOR`, the user's own tool) and `provision/remote.rs:109` (`ssh`, which forwards only what `SendEnv` names). | `git.rs:96-108 SCRUBBED_ENV`, `:316-322`; `browser.rs:146-178` | **adopted** (a) and (b) in T2: `HTUI_OWN_ENV` is removed in `Cli::command` and in the opener builders. The editor and ssh cases are reported in §9 R-7 for the reviewer (both stripped since R1 H1). |
| P-10 | D5 says the strip "removes a project secret of that name too". The resolver already refuses any project secret whose name starts `HTUI_` (`check_env`, `htui-core/src/secret.rs:453, 515`), so none can reach `launch.env`. | `secret.rs:449-453, 515` | **adopted**: `env_remove` goes after `.envs(&launch.env)` as D5 says. That placement is a second guard, not the only one. |
| P-11 | T5 needs no `main.rs` change. `state_cmd::StateExit` (`state_cmd.rs:52-72`) already maps Offline 3, Refused 2 and Failed 1, and already keeps 2 and 3 out of Sentry (`main.rs:148-163`). | `main.rs:105-163` | **adopted**: `qdrant_cmd` answers `StateExit`. |
| P-12 | D11 touches more than `provision/{plan,script}.rs`. `provision/mod.rs` builds INSTALL's stdin (`install_stdin`, `:849`), its arguments (`:471-482`) and `LocalFacts` (`:348`), and `run` (`:216-247`) is what reads the keyring. INSTALL_ROOT would hold the key in a shell variable and `printf` it, so a host whose `sh` lacks a `printf` builtin must be refused, as the password case already is (`plan.rs:118-125`). The unit is pinned in a third place, the guide (`docs/htui-worker.md:674-735`, `GUIDE_UNIT` in `script.rs`). | `provision/mod.rs:216-247, 348, 455-482, 849-858`; `plan.rs:118` | **adopted**: T6's set is `provision/{mod,plan,script}.rs`, `tests/provision.rs` and `docs/htui-worker.md` (the provisioning section). |
| P-13 | **D12 leaks into tests.** With `HTUI_QDRANT_URL` in the run's compose environment, every test process in a sandbox inherits it. A test that reads the process environment then behaves differently in a sandbox than on the host. | P-1 | **adopted constraint**: no test reads `HTUI_QDRANT_*` from the process. `QdrantEnv::from_process()` is called only by production entry points (`QdrantIndex::new()`/`default()`, `concepts::search_items`, `concepts::index_items`, `worker_cmd::serve`, `qdrant_settings_info::serve`, `qdrant_cmd`). `QdrantIndex::with_loader` defaults to `QdrantEnv::default()`. Tests inject a `QdrantEnv` literal. |
| P-14 | D8's new order means `SearchContext` (`concepts.rs:474-486`) can no longer carry resolved `QdrantSettings`. `state_cli_pg.rs:552 a_search_without_a_qdrant_url_is_refused` ("Needs no database: never skips") cannot keep that property. | `concepts.rs:474-528` | **adopted**: `SearchContext` takes `env: &QdrantEnv` and a `keyring_key` seam (§T3.3). The case is rewritten over `demo_db`, and a no-database case pins "unreachable before no URL" instead. |
| P-15 | `docs/htui-worker.md` describes the keyring-read-once index job (`:407-421`) and the `search_concepts` gate (`:240`). | doc | **adopted**: T3 edits those sections and adds "Where the Qdrant API key comes from" after `:380-398`. T6 edits only the provisioning sections. The two run in different waves. |
| P-16 | No task extends `FakeSlots`. It already has `qdrant_url` and `qdrant_key` (`htui-store/src/secret.rs:62-75`), and the fake honours both (`:303-379`). | secret.rs | **adopted**: T2, T4 and T5 change neither `FakeSlots` nor `testkit.rs`. `testkit::fake_qdrant_dsn()` (`testkit.rs:373`) reads the `qdrant_url` slot, so T4 uses it to assert that the old entry is gone. |
| P-17 | The offer's "use it" action needs a new act (every key goes through `keys.toml`). That moves the catalogue count pin `CATALOGUE.len() == 212` (`keys/catalogue.rs:2896`), and `tests/keys_file.rs:728` pins the printed defaults in `docs/htui-keys.md`. | catalogue.rs, keys_file.rs | **adopted**: T4 adds `Act::QdrantAdopt` (§T4.5), moves the pin to 213 and regenerates the doc's toml block. |
| P-18 | `tests/settings.rs:6145 tab_in_the_qdrant_editor_keeps_the_settings_tab` relies on an `Offline` backend answering URL `NotStored`, which opens the URL editor by itself. Under D9 the offline URL row "cannot be read", so nothing opens. | settings.rs:6142-6190 | **adopted**: T4 rewrites the case to open the **key** editor (`j`, `e`), which works offline. |
| P-19 | D2 names four refusals. A URL is drawn on a terminal and handed to the gRPC client, so whitespace and control characters, and a port that is not 1..=65535, are refused too. | — | **adopted**: `QdrantUrlError::{Space, Port}`, plus `NotAString` for a stored value that is not a JSON string. No variant carries the input text (a refused URL may hold a password). |
| P-20 | **As shipped by T1 (`623693ff`, `994ff42b`), D2 is stricter than §T1.1.** An `@` anywhere after the scheme is `Credentials`, not only in the authority: otherwise a password holding `/` would move the userinfo into the path and pass. Any character outside printable ASCII is `Space`, which covers Unicode format characters (bidi overrides, zero-width spaces) and IDN hosts. A bracketed literal must be a real IPv6 address. `Space`'s sentence reads "a Qdrant URL may only contain printable ASCII characters (no spaces or control characters)". | `crates/htui-core/src/qdrant.rs:26-60, 141-144`; T1 verify round 3 | **adopted**: T4 and T5 show the shipped sentences from `QdrantUrlError`'s `Display`, never a copy of the §T1.1 text. |

---

## 1. Shared decisions

- **Names.** `app_setting` key `qdrant_url` (`htui_core::qdrant::QDRANT_URL_KEY`). The env vars are
  `HTUI_QDRANT_URL` and `HTUI_QDRANT_API_KEY` (`htui_core::secret::{QDRANT_URL_ENV,
  QDRANT_KEY_ENV}`). `HTUI_OWN_ENV = [QDRANT_KEY_ENV]`. The credential is `htui-qdrant-key`
  (`htui_store::secret::QDRANT_KEY_CREDENTIAL`).
- **Resolution (D3, D4) lives in one function**, `concepts_worker::resolve_settings`. The TUI index,
  the worker job, `--search-items` and `--index-items` all call it, and `Unresolved` is its one error
  type (§T3.1).
- **Sentences** (constants, `pub(crate)` unless noted):
  - `concepts::NO_QDRANT_URL` = "no Qdrant URL is set; set one in Settings > Qdrant or with `htui --set-qdrant-url`". It replaces `concepts.rs:97`.
  - `concepts_worker::URL_OFFLINE` (pub) = "the Qdrant URL cannot be read: Postgres is not connected".
  - `htui_core::qdrant::stored_url_unusable(why)` = "the Qdrant URL setting (`app_setting` `qdrant_url`) is not usable: {why}; set it again in Settings > Qdrant or with `htui --set-qdrant-url`".
  - `concepts_worker::env_url_unusable(why)` = "HTUI_QDRANT_URL is not a usable Qdrant URL: {why}".
- **No test reads `HTUI_QDRANT_*` from the process** (P-13).
- **`R-NF-3`.** No Settings section holds a store. The URL slot is published with a non-`async`
  pointer swap. Settings resolution runs on the concepts task, not the loop. `QdrantInfo` and the
  six Qdrant writes are served on the loop with `.await`s, as the four Qdrant requests and the five
  Secrets keyring requests are today (`store_worker.rs:2310-2330`, `spawn_blocking` for every
  keyring call).
- **No migration and no `.sqlx` entry** (§T1.3).

---

## T1 — `QdrantUrl` and the `qdrant_url` row (D1, D2) · W1

### T1.1 `crates/htui-core/src/qdrant.rs` (CREATE) and `lib.rs` (UPDATE: `pub mod qdrant;` after `pub mod prompt;`, `lib.rs:13`)

```rust
//! The Qdrant URL every box uses (MOD-158 D1, D2; `R-STO-1`): one `app_setting` row, validated by
//! [`QdrantUrl::parse`] wherever a URL enters htui (setter, env override, CLI, TUI field).

/// `app_setting.key` of the URL (D1). The value is a JSON string.
pub const QDRANT_URL_KEY: &str = "qdrant_url";

/// A Qdrant base URL that passed D2: trimmed; `http://` or `https://` (ASCII case-insensitive,
/// stored lowercase); a non-empty host (a bracketed IPv6 literal allowed); an optional port
/// 1..=65535; an optional path; no userinfo, query, fragment, whitespace or control character.
/// Not secret: `Debug`/`Display` print it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QdrantUrl(String);

impl QdrantUrl {
    /// # Errors
    /// [`QdrantUrlError`]; never echoes `text`.
    pub fn parse(text: &str) -> Result<Self, QdrantUrlError>;
    /// D3's override: `None` or a blank value is unset (`Ok(None)`); anything else must parse.
    pub fn from_env(value: Option<&str>) -> Result<Option<Self>, QdrantUrlError>;
    #[must_use] pub fn as_str(&self) -> &str;
    /// The `app_setting.value` this URL is stored as.
    #[must_use] pub fn to_value(&self) -> serde_json::Value;   // Value::String
}
impl core::fmt::Display for QdrantUrl { /* the URL */ }

/// Why a text is not a Qdrant URL. No variant carries the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QdrantUrlError {
    #[error("the Qdrant URL is empty")] Empty,
    #[error("a Qdrant URL may not contain spaces or control characters")] Space,
    #[error("a Qdrant URL starts with http:// or https://")] Scheme,
    #[error("a Qdrant URL needs a host")] NoHost,
    #[error("the port of a Qdrant URL is a number from 1 to 65535")] Port,
    #[error("a Qdrant URL may not carry a user name or password; store the API key in Settings > Qdrant or with `htui --set-qdrant-key`")] Credentials,
    #[error("a Qdrant URL may not carry a query (`?...`)")] Query,
    #[error("a Qdrant URL may not carry a fragment (`#...`)")] Fragment,
    #[error("the stored value is not text")] NotAString,
}

/// The `qdrant_url` row: its value as D2 reads it, and its compare-and-set token
/// (`app_setting.updated_at`). A malformed value is kept with its token (blueprint P-3), so it can
/// be overwritten or cleared; [`Self::usable`] is how every resolver reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredQdrantUrl {
    pub url: Result<QdrantUrl, QdrantUrlError>,
    pub token: chrono::DateTime<chrono::Utc>,
}

impl StoredQdrantUrl {
    /// From a stored `(value, updated_at)`: a JSON string that parses, else `Err(why)`.
    #[must_use] pub fn from_row(value: &serde_json::Value, token: DateTime<Utc>) -> Self;
    /// D1: the URL, or an error naming the key, never "no URL".
    /// # Errors
    /// `StoreError::Backend(stored_url_unusable(why))`.
    pub fn usable(&self) -> crate::store::Result<&QdrantUrl>;
}

/// D1's error sentence for a malformed row (§1).
#[must_use] pub fn stored_url_unusable(why: QdrantUrlError) -> String;
```

The check order is fixed so that the error is deterministic: trim → `Empty` → `Space` (any
`char::is_whitespace` or `is_control` left inside) → `Scheme` → split the authority at the first
`/` → `Query` (a `?` anywhere) → `Fragment` (a `#` anywhere) → `Credentials` (an `@` in the
authority) → host/port (`[..]` for IPv6, else up to the last `:`) → `NoHost` → `Port`. Hand-written,
with no `url` dependency (D2).

### T1.2 `MemStore` inherent methods (`crates/htui-core/src/store/mem.rs`, beside `app_settings` `:668` and `set_app_setting` `:684`)

```rust
/// D1: the `qdrant_url` row, malformed or not (P-3); `None` when there is none.
/// # Errors
/// Never; the signature matches `PgStore`'s.
pub async fn qdrant_url(&self) -> Result<Option<StoredQdrantUrl>>;
/// D1: compare-and-set. `expected: None` means "I expect no row". `Applied(Some(row))` or
/// `Stale(current)`, where `current` is `None` when the row is gone (P-4).
pub async fn set_qdrant_url(&self, url: &QdrantUrl, expected: Option<DateTime<Utc>>)
    -> Result<CasOutcome<Option<StoredQdrantUrl>>>;
/// D1: removes the row whose token is `expected`: `Applied(None)`, or `Stale(current)`.
pub async fn clear_qdrant_url(&self, expected: DateTime<Utc>)
    -> Result<CasOutcome<Option<StoredQdrantUrl>>>;
```

These mirror `State::write_queue_setting`'s App arm (`mem.rs:4855-4888`): `let now = self.now();`
before `self.write(..)`, and `state.app_settings` holds `(Value, now)`. They are inherent, not trait
verbs (D1), so `writer.rs`, the conformance spies and `SpyStore` do not change.

### T1.3 `PgStore` (`crates/htui-store/src/pg/read.rs` beside `app_settings` `:1840`; `pg/write.rs` in a new `impl PgStore` block after `stored_queue_setting`, `:9588-9633`)

The four statements are reused **byte for byte**. The offline cache keys an entry by the SHA-256 of
the literal text, newlines and leading spaces included. Each literal below is the exact `"query"`
string of its existing `.sqlx` file:

| Use | Existing site | `.sqlx` file (`crates/htui-store/.sqlx/`) | Exact text |
|---|---|---|---|
| read | `pg/write.rs:9597` (`stored_queue_setting`, App arm) | `query-e798257bef18e35b4d41997f0e874ac80b52257a479efe02b9d56932779039f2.json` | `"SELECT value, updated_at FROM app_setting WHERE key = $1"` (a plain one-line literal, `sqlx::query!`) |
| insert, expecting no row | `pg/write.rs:5526-5531` | `query-39fbb9fbf98bc9526966356c707e87315754342db995988a07e9318d498096d7.json` | `"\n                        INSERT INTO app_setting (key, value) VALUES ($1, $2)\n                        ON CONFLICT (key) DO NOTHING\n                        RETURNING updated_at AS \"updated_at!\"\n                        "` (`sqlx::query_scalar!`) |
| update by token | `pg/write.rs:5538-5543` | `query-174cf062636909d34dd8140792cf4f81ca3656aa57ea4ddb365a8e7bf31286f0.json` | `"\n                        UPDATE app_setting SET value = $2\n                         WHERE key = $1 AND updated_at = $3\n                        RETURNING updated_at AS \"updated_at!\"\n                        "` (`query_scalar!`) |
| delete by token | `pg/write.rs:5648-5652` | `query-04f5b72f9711b093631910f3c09b503a406e3ff7ffb9701aae6287d05aa84c61.json` | `"\n                    DELETE FROM app_setting WHERE key = $1 AND updated_at = $2\n                    RETURNING updated_at AS \"updated_at!\"\n                    "` (`query_scalar!`) |

In the source, the INSERT and UPDATE raw strings have every inner line, and the closing `"#`, at
**24 spaces**; the UPDATE's `WHERE` line is at **25**. The DELETE's are at **20**, and its closing
`"#` at **20**. Where the `r#"` token itself sits does not matter. These lines will look
mis-indented at the new site; say so in a comment, as `pg/write.rs:5503-5506` does. The bind types
are the existing ones: `&str` (`QDRANT_URL_KEY`), `&serde_json::Value` (`url.to_value()`) and
`DateTime<Utc>`. **No `cargo sqlx prepare` is needed.**

If `SQLX_OFFLINE=true cargo check -p htui-store --all-targets --all-features` fails with "no cached
data for this query", a byte drifted. Diff the literal against the table above; do not run
`prepare`. Prepare is the last resort, and is unexpected here. Recipe (`docs/hr-sandbox.md:206-218`):
`psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"`, then
`cd crates/htui-store`, `export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx`,
`cargo sqlx migrate run --source migrations` and
`cargo sqlx prepare -- --all-targets --all-features`. Commit only the new `.sqlx` file.

Signatures are as in §T1.2, with `# Errors` = `map_sqlx`'s. A miss re-reads with `self.qdrant_url()`
and answers `Stale(current)` (P-4). `cas_miss` (`pg/write.rs:111`) is **not** used.

### T1.4 `crates/htui-store/src/qdrant_settings.rs` (UPDATE)

```rust
pub struct QdrantSettings { pub url: QdrantUrl, pub api_key: Option<Zeroizing<String>> }
/// D2: only a checked URL reaches the client. A blank key is no key, as before.
#[must_use] pub fn new(url: QdrantUrl, api_key: Option<String>) -> Self;
```

`vector.rs:915` becomes `Qdrant::from_url(settings.url.as_str())`. `Connections::get`
(`concepts_worker.rs:114`) still compiles (`QdrantUrl: PartialEq`). Callers (P-5) become
`QdrantSettings::new(QdrantUrl::parse(&url)?, key)`: in `concepts.rs:111` the `?` turns the error
into `anyhow`; at `:369` it is `.map_err(|e| format!("the stored Qdrant URL is not usable: {e}"))`.
Tests use `.expect("a valid URL")`.

### T1.5 Tests first

`crates/htui-core/src/qdrant.rs` `#[cfg(test)] mod tests`:
`a_plain_http_url_with_a_port_parses` (`http://host:6334`) ·
`https_a_path_and_surrounding_space_parse_trimmed` ·
`the_scheme_is_lowercased_and_only_http_or_https` (`HTTP://h`, `localhost:6334`, `grpc://h`) ·
`userinfo_is_refused_and_never_echoed` (`http://u@h`, `http://u:SENTINEL@h:6334`; `Display` lacks
`SENTINEL`) · `a_query_is_refused` (`http://h:6334/?api-key=x`) · `a_fragment_is_refused` ·
`an_empty_host_is_refused` (`http://`, `http://:6334`, `https:///p`) ·
`a_bad_port_is_refused` (`h:`, `h:abc`, `h:0`, `h:70000`) ·
`whitespace_or_control_inside_is_refused` (`http://h 1`, `http://h\u{1b}[2J`) ·
`an_ipv6_literal_parses` (`http://[::1]:6334`) ·
`from_env_blank_is_unset_and_invalid_is_an_error` ·
`a_stored_value_that_is_not_a_string_is_unusable_with_its_token` ·
`usable_names_the_key` (the error contains `qdrant_url`).

`crates/htui-store/tests/qdrant_url_pg.rs` (CREATE, no feature gate: the crate's self
dev-dependency turns `test-support` on). A local `trait UrlStore` wraps the three inherent methods
for `MemStore` and `PgStore`, and one `async fn cases(store)` runs every case on both:
`mem_store_keeps_the_qdrant_url_row` (always runs) and `pg_store_keeps_the_qdrant_url_row` (prints
`common::SKIP` without `HTUI_TEST_DATABASE_URL`, `testkit::demo_db()`).
The cases, in order:
1. no row → `qdrant_url() == None`;
2. insert expecting none → `Applied(Some)`, and `app_settings()["qdrant_url"]` is a JSON string;
3. a second insert expecting none → `Stale(Some(first))`, value unchanged;
4. update by token → `Applied`, new token ≠ old;
5. update with the spent token → `Stale(Some(current))`, nothing written;
6. clear by token → `Applied(None)` and no row; clear again with that token → `Stale(None)`;
7. a malformed row (Mem: `set_app_setting("qdrant_url", json!(42))`; Pg: an **unchecked**
   `sqlx::query("INSERT INTO app_setting (key, value) VALUES ('qdrant_url', '42'::jsonb)")`, which
   adds no `.sqlx` entry) reads as `url: Err(NotAString)` with a token; `usable()` errs naming the
   key; an update by its token fixes it.

### T1.6 Commit groups

1. `feat(MOD-158): QdrantUrl and StoredQdrantUrl` (qdrant.rs, lib.rs).
2. `feat(MOD-158): the qdrant_url row on MemStore and PgStore, reusing the app_setting statements` (mem.rs, pg/read.rs, pg/write.rs, tests/qdrant_url_pg.rs).
3. `refactor(MOD-158): QdrantSettings takes a QdrantUrl` (qdrant_settings.rs, vector.rs, qdrant_live.rs, the four `htui` call sites).

---

## T2 — Key sources and the spawn strip (D4 store half, D5) · W1

### T2.1 `crates/htui-core/src/secret.rs` (UPDATE, after `RESERVED_PREFIX` `:453`)

```rust
/// D3 (`R-STO-1`): the development/CI override of the Qdrant URL. Not secret: agents see it.
pub const QDRANT_URL_ENV: &str = "HTUI_QDRANT_URL";
/// D4: the development/CI Qdrant API key. Secret: in [`HTUI_OWN_ENV`].
pub const QDRANT_KEY_ENV: &str = "HTUI_QDRANT_API_KEY";
/// D5 (`R-SEC-2`): htui's own variables that carry a secret. Every agent, step-shell, git and
/// opener spawn removes them, **after** the launch's own env, so nothing can re-add them.
pub const HTUI_OWN_ENV: [&str; 1] = [QDRANT_KEY_ENV];
```

Test: `htui_own_env_holds_the_key_and_not_the_url`.

### T2.2 Spawn sites (each adds `for key in HTUI_OWN_ENV { command.env_remove(key); }`)

| Site | Where |
|---|---|
| `htui-agent/src/launch.rs:1187-1201` `spawn_supervised` | inside `build`, after `.envs(&launch.env)` (`:1195`), before `CommandWrap::from` |
| `htui-orch/src/verify.rs:375-387` `ShellVerifier::spawn_and_wait` | inside `build`. Also replace the comment at `:371-375` ("handed to the child unchanged") with one that names D5 |
| `htui-orch/src/verify.rs:550-562` `run_shell` | inside `build`. Also update `run_shell`'s doc (`:536`: "with the process environment unchanged") |
| `htui-orch/src/isolate/git.rs:316-322` `Cli::command` (P-9a) | after the `SCRUBBED_ENV` loop. `probe_within` (`:217`) runs only `--version`, so it is left alone |
| `htui-agent/src/auth/browser.rs:146, 157, 166, 178` (P-9b) | each builder, before it returns |

**No other spawn exists** in `htui-agent`, `htui-orch`, `htui-worker` or `htui-mcp`
(`search_text Command::new|process::Command|CommandWrap`). `contained.rs:109-124` spawns tasks,
not processes. Every agent launch goes through `launch::spawn` (`acp/mod.rs:384`, `cli/mod.rs:687`,
`auth/run.rs:86`, `probe.rs:1052, 1330`). The worker reaches shells only through
`ShellVerifier`/`run_shell` (`htui-worker/src/runtime.rs:534`, `htui-mcp/src/tools/command.rs:260`).
`htui mcp` inherits from the agent (D5). `to_acp_config` (`launch.rs:521`) builds an SDK config
that no production path spawns.

**Tests without `set_var`** (`unsafe_code = "forbid"`, `Cargo.toml:194`). Explicit env goes
through `launch.env`. Inherited env uses the self re-exec pattern of
`htui/src/editor.rs:1080-1108`: the test re-runs its own binary with
`--exact <name> --test-threads=1` and `HTUI_QDRANT_API_KEY=SENTINEL-QK` in the child's
environment; the child does the spawn and asserts; the parent asserts `"1 passed"`.

- `htui-agent/tests/launch.rs`:
  - `the_launch_env_cannot_hand_an_agent_htui_qdrant_api_key` (unix): `spawn_sh` with
    `launch.env = {HTUI_QDRANT_API_KEY: SENTINEL, PLAIN: kept}`, script
    `printenv HTUI_QDRANT_API_KEY; echo "rc=$?"; printenv PLAIN`. stdout has `rc=1` and `kept`, and
    not the sentinel.
  - `an_inherited_htui_qdrant_api_key_never_reaches_an_agent` (unix, re-exec).
- `htui-orch/src/verify.rs` `shell_tests`:
  - `run_shell_hides_an_inherited_htui_qdrant_api_key` (re-exec; command `printenv
    HTUI_QDRANT_API_KEY || echo absent`; output has `absent` and no sentinel).
  - `a_verify_command_never_sees_htui_qdrant_api_key` (re-exec, through `verifier()` `:998`).
- `htui-orch/src/isolate/git.rs` tests: `the_git_command_removes_htui_own_env`. It reads
  `cli.command(dir).as_std().get_envs()` and finds `(HTUI_QDRANT_API_KEY, None)`. No process is
  needed.
- `htui-agent/src/auth/browser.rs` tests: `every_opener_removes_htui_own_env` (same `get_envs()`
  check over `platform_command`/`opener_command`).

### T2.3 `crates/htui-store/src/secret.rs` (UPDATE, after `headless_dsn` `:262-301`)

```rust
/// D4, D11: the systemd credential name of the Qdrant API key (`LoadCredentialEncrypted=htui-qdrant-key:...`).
pub const QDRANT_KEY_CREDENTIAL: &str = "htui-qdrant-key";

/// D3, D4: the two development/CI variables as this process read them; blank reads as unset.
/// `Debug` never prints the key. Tests build it as a literal (blueprint P-13).
#[derive(Clone, Default)]
pub struct QdrantEnv { pub url: Option<String>, pub api_key: Option<Zeroizing<String>> }
impl QdrantEnv {
    /// Reads `HTUI_QDRANT_URL` and `HTUI_QDRANT_API_KEY` (`std::env::var`, no `unsafe`). Production
    /// entry points only.
    #[must_use] pub fn from_process() -> Self;
}

/// D4: which process asks, which decides the sources after the env var.
#[derive(Clone, Copy)]
pub enum KeyHost<'a> {
    /// The TUI and the CLI verbs: this box's keyring, per request; a keyring failure is an error.
    Interactive,
    /// `htui worker`: its `--qdrant-key-stdin` line and nothing else when given; else the keyring
    /// (a failure is "no keyring"); else, on Linux, `credentials_dir/htui-qdrant-key`.
    Headless { stdin: Option<&'a str>, credentials_dir: Option<&'a std::path::Path> },
}

/// Why no key could be read. No variant carries the key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QdrantKeyError {
    #[error("the Qdrant API key could not be read from the OS keyring: {0}")] Keyring(String),
    #[error("the systemd credential {path} could not be read: {why}")] Credential { path: String, why: String },
}

/// D4: `env` (non-blank) first, then `host`'s sources. `keyring` is
/// [`get_qdrant_api_key`] in production; a test passes a closure, so no case reads a keyring.
/// No key anywhere is `Ok(None)`: a Qdrant without a key is allowed (ANA-37 §8 Q2).
/// # Errors
/// [`QdrantKeyError`].
pub fn qdrant_api_key(
    env: Option<&str>,
    host: KeyHost<'_>,
    keyring: &dyn Fn() -> Result<Option<String>>,
) -> core::result::Result<Option<Zeroizing<String>>, QdrantKeyError>;
```

The stdin line is read once, at worker start, with the existing `read_dsn_line` (`:248`), which is
generic over a line. A blank credential file is `Ok(None)`, as for the DSN (`:283-290`).

Tests (`#[cfg(test)] mod qdrant_key_tests`, all with injected closures and no fake keyring):
`the_env_key_wins_and_the_keyring_is_not_read` (the closure panics) · `a_blank_env_key_is_unset` ·
`interactive_reads_the_keyring_and_a_failure_is_an_error` ·
`a_headless_stdin_key_wins_and_nothing_else_is_read` ·
`headless_reads_the_keyring_before_the_credential` (linux) ·
`a_headless_keyring_failure_falls_back_to_the_credential` (linux) ·
`no_key_anywhere_is_none_not_an_error` ·
`an_unreadable_credential_names_its_path_and_never_the_key` (linux: `htui-qdrant-key` is a
directory → `EISDIR`) · `a_blank_credential_is_no_key` ·
`the_key_credential_is_named_htui_qdrant_key` · `qdrant_env_debug_never_prints_the_key`.

### T2.4 Commit groups

1. `feat(MOD-158): HTUI_OWN_ENV, stripped at every agent, shell, git and opener spawn` (htui-core secret.rs, launch.rs, tests/launch.rs, verify.rs, git.rs, browser.rs).
2. `feat(MOD-158): Qdrant API key sources and QdrantEnv` (htui-store secret.rs).

---

## T3 — Resolution, live pickup, worker and CLI-verb reads (D3, D6, D7, D8 reads) · W2

### T3.1 The settings source (`crates/htui/src/concepts_worker.rs`)

```rust
/// D6: what the `qdrant_url` row is read through: the store this process is connected to.
pub trait UrlRow: Send + Sync {
    fn qdrant_url(&self) -> BoxFuture<'_, Result<Option<StoredQdrantUrl>, StoreError>>;
}
impl UrlRow for PgStore { /* PgStore::qdrant_url */ }
impl UrlRow for MemStore { /* MemStore::qdrant_url: unconditional, the tests' store */ }

/// D4 for this process, owned (the borrowed form is `htui_store::secret::KeyHost`).
#[derive(Clone)]
pub enum KeySource {
    Interactive,
    Headless { stdin: Option<Zeroizing<String>>, credentials_dir: Option<PathBuf> },
}

/// Why no settings resolved (D3, D4). `sentence()` is what a search, an index run or a log line says.
#[derive(Debug, Clone)]
pub enum Unresolved {
    /// No override and no row: [`concepts::NO_QDRANT_URL`].
    NoUrl,
    /// No store published and no override: [`URL_OFFLINE`].
    Offline,
    /// The override or the row is not a usable URL (its sentence).
    Unusable(String),
    /// The row could not be read.
    Read(StoreError),
    /// The key could not be read.
    Key(QdrantKeyError),
}

/// D3, D4: the override; else the row (through `usable()`); else `NoUrl`. Then the key. The keyring
/// and the credential file are read under `spawn_blocking`.
pub(crate) async fn resolve_settings(
    env: &QdrantEnv,
    row: Option<&dyn UrlRow>,
    key: &KeySource,
    keyring: fn() -> htui_core::store::Result<Option<String>>,
) -> Result<QdrantSettings, Unresolved>;

/// D6: one process's source. The slot holds the current store (`None` offline, under `--demo`, or
/// before a connect); every read takes an `Arc` clone and drops the lock before any `.await`.
pub struct QdrantSource {
    env: QdrantEnv,
    store: std::sync::RwLock<Option<Arc<dyn UrlRow>>>,
    key: std::sync::RwLock<KeySource>,
    keyring: fn() -> htui_core::store::Result<Option<String>>,
}
impl QdrantSource {
    /// Logs once, at `info`, an override in force (D3).
    #[must_use] pub fn new(env: QdrantEnv, key: KeySource) -> Self;          // keyring = secret::get_qdrant_api_key
    #[must_use] pub fn with_keyring(self, keyring: fn() -> ..) -> Self;    // tests
    #[must_use] pub fn from_process() -> Self;                             // QdrantEnv::from_process(), Interactive
    pub fn publish(&self, store: Option<Arc<dyn UrlRow>>);
    pub fn set_key_source(&self, key: KeySource);
    /// D3 alone: the worker's search gate (D7, Q3).
    pub async fn url(&self) -> Result<QdrantUrl, Unresolved>;
    pub async fn settings(&self) -> Result<QdrantSettings, Unresolved>;
}
```

`QdrantIndex` (`:79-89`) gains `source: Arc<QdrantSource>`. `new()`/`default()` (`:146-157`) use
`QdrantSource::from_process()`. `with_loader` (`:159-169`) uses
`QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)` (P-13), and
`with_source(self, Arc<QdrantSource>) -> Self` and `source(&self) -> &Arc<QdrantSource>` are added.
`search`/`sync` (`:229-250`) call `self.source.settings()` in place of the deleted
`qdrant_settings()` (`:216-227`), map `Unresolved` with `.sentence()`, and keep the
settings-before-model order and the `Connections` cache. A changed URL or key therefore misses the
cache and reconnects (`Connections::get`, `:110-119`).

`ConceptIndex` (`:52-57`) gains `fn publish(&self, _store: Option<Arc<dyn UrlRow>>) {}`. It defaults
to a no-op, so `MemIndex` stays as it is, and `QdrantIndex` forwards to its source.
`ConceptsRuntime::publish(&self, backend: &Backend)` calls
`self.index.publish(backend.writable().map(|pg| Arc::new(pg.clone()) as Arc<dyn UrlRow>))`.

### T3.2 The worker index job (D7, `crates/htui/src/concepts.rs:297-447`)

`spawn_index_job` and `spawn_index_job_over` become infallible and **always spawn** (return
`JoinHandle<()>`):

```rust
/// D7: per cycle, resolve the settings; no URL is one `info` line and a sleep; else connect through
/// `open` (cached by URL and key in production, so the model loads only once a URL resolves), then
/// `index_cycle`. A failed open is `open_failure_level`'s line and a sleep.
pub async fn job_cycle<S, V, F, Fut>(source: &S, settings: &QdrantSource, open: &F, mismatch: &mut bool) -> Duration
where S: IndexSource, V: VectorStore, F: Fn(QdrantSettings) -> Fut, Fut: Future<Output = Result<Arc<V>, String>>;
pub async fn index_job<..>(source: &S, settings: &QdrantSource, open: F) -> !;
```

In production, `open` is `|s| models.connect(s)`. `QdrantIndex::connect` (`:196-213`) becomes
`pub(crate)` and the `mismatch` flag comes from the error text through `is_embedder_mismatch`.
Delete `keyring_settings` (`:359-370`) and `open_index` (`:383-407`), and keep
`open_failure_level`. `settings_from` (`:104-112`) and `qdrant_settings` (`:524-533`) are replaced
by `resolve_settings` (§T3.1).

### T3.3 The CLI verbs (D8, `concepts.rs`)

- `--index-items` (`open`, `:119-133`): DSN (`NO_DSN`), headless connect, then
  `resolve_settings(&QdrantEnv::from_process(), Some(&pg), &KeySource::Interactive,
  secret::get_qdrant_api_key)`. `NoUrl` bails with `NO_QDRANT_URL`. Then the model, then the
  connect.
- `--search-items`:
  - `search_inputs` (`:503-516`) reads only the DSN and `box.toml`.
  - `SearchContext` (`:474-486`) drops `settings` and gains `pub env: &'a QdrantEnv` and
    `pub keyring_key: fn() -> htui_core::store::Result<Option<String>>`. Its `Debug` prints
    `env.url` and never `api_key`.
  - `search_online` (`:562-588`): read-only connect, then `resolve_settings(ctx.env, Some(&pg),
    &KeySource::Interactive, ctx.keyring_key)`, then the scope, then `search_index`. The `Failure`s:
    `NoUrl` → `ErrorCode::NoQdrantUrl` with `NO_QDRANT_URL`; `Unusable` → `QdrantUrlUnusable`;
    `Read(e)` → `Failure::from_read(&e)`; `Key` → `KeyringUnreadable`. `Offline` cannot happen
    here.
  - The new order is no DSN → the connect → no URL → the key → the scope → the index. The error
    codes are unchanged; `docs/htui-cli.md` is T5's.

### T3.4 Publishing (D6)

- **The store worker** (`store_worker.rs:2924-2928`): `concepts.publish(&backend);` at the top of
  every iteration, beside `host_the_backend` (P-2).
- **`htui worker`** (`worker_cmd.rs:68-118`), in order:
  1. `read_dsn_apart(args.dsn_stdin, args.qdrant_key_stdin)` reads the key line **on the DSN thread**
     (`:143-170`) after the DSN. A blank key line is
     `WorkerExit::Refused("no Qdrant API key on stdin; nothing was read")`.
  2. After the connect: `let qdrant = concepts_worker::shared_qdrant();
     qdrant.source().set_key_source(KeySource::Headless { stdin, credentials_dir });
     qdrant.source().publish(Some(Arc::new(pg.clone())));`
  3. D10: the keyring `qdrant-url`, read through `concepts::apart` and raced against shutdown. When
     present, log once at `info`: "this box's keyring holds an old Qdrant URL ({url}) that htui no
     longer reads; use it for every box in Settings > Qdrant or with `htui --set-qdrant-url`".
  4. The gate (Q3): `let offered = select!{ shutdown, qdrant.source().url() }.is_ok();`.
  5. `spawn_index_job(pg.clone())` always.
  6. `worker_search(offered)` (`:124-131`).
- **`crates/htui/src/cli.rs` `WorkerArgs` (`:163-171`)**: `#[arg(long)] pub qdrant_key_stdin: bool`,
  doc "Read the Qdrant API key from one line of stdin, after the DSN line with `--dsn-stdin`,
  instead of the keyring or the credential `htui-qdrant-key`". It reads no environment, so
  `no_worker_argument_reads_the_environment` holds.

### T3.5 Tests first

- `concepts_worker.rs` tests, each with an injected `QdrantEnv` and `with_keyring(|| Ok(None))`:
  - `no_published_store_reads_as_postgres_not_connected`
  - `the_env_url_wins_and_needs_no_store`
  - `an_invalid_env_url_is_an_error_never_a_fallback`
  - `a_row_set_between_two_reads_is_read_by_the_second` (MemStore published)
  - `a_malformed_row_is_an_error_naming_the_key_not_no_url`
  - `the_env_key_wins_over_the_keyring`
  - `the_runtime_publishes_the_writable_store_only` (Memory → `None`; `Online` over `PgStore::lazy`
    and a temp `CacheStore` → `Some`)
  - **Rewrite** `qdrant_index_without_a_stored_url_names_where_to_set_it` (`:839`). It needs no
    keyring guard now; the no-URL text names `Settings > Qdrant` **and** `htui --set-qdrant-url`.
- `concepts.rs` `index_job`, under paused time, over `MemStore` + `Counting` and an `open` closure
  that records URLs:
  - `no_url_is_one_info_line_then_the_interval` (`open` never called)
  - `a_url_set_after_the_job_starts_is_used_by_the_next_cycle`
  - `a_changed_url_is_opened_on_the_next_cycle`
  - `a_failed_open_is_retried_next_cycle`
  - `the_job_starts_whatever_the_setting_says`
  - Delete `no_qdrant_url_means_no_job`, `an_unreadable_keyring_means_no_job` and
    `a_stored_qdrant_url_starts_the_job` (`:1186-1219`). Rewrite
    `the_index_job_and_the_search_share_one_model_load` (`:1222`) to use
    `with_loader(..).with_source(env url http://127.0.0.1:1)`.
  - Delete `a_missing_url_names_where_to_set_it` and `a_stored_url_and_key_become_settings`
    (`:720-726`) with `settings_from`.
- `worker_cmd.rs` tests:
  - rename `the_worker_searches_only_where_it_indexes` → `the_worker_offers_search_only_where_a_url_resolves_at_start`
  - `the_key_line_is_read_after_the_dsn_line`
  - `a_blank_key_line_is_a_refusal`
  - `the_key_line_alone_leaves_the_dsn_to_the_keyring` (these three go over a `read_startup_secrets_from(&mut Cursor, ..)` seam; the keyring cases take `mock_keyring`)
- `cli.rs`: update `worker_parses_its_two_flags` (`:283-305`) to three flags and the
  `WorkerArgs` literal.
- `tests/qdrant_worker.rs`: add `the_worker_job_follows_the_url_row`. It sets the row on `demo_db`
  to `HTUI_TEST_QDRANT_URL`, publishes `db.store`, and runs `job_cycle` with an `open` closure that
  calls `QdrantStore::connect_to(&s, HashEmbedder, &throwaway)`.
- `tests/state_cli_pg.rs`:
  - `searched()` (`:506-533`) takes a `QdrantEnv` and passes `keyring_key: || Ok(None)`; `qdrant()`
    (`:535`) goes.
  - Rewrite `a_search_without_a_qdrant_url_is_refused` over `demo_db`: `no_qdrant_url`, exit 2.
  - Add `postgres_unreachable_comes_before_no_url` (DSN `127.0.0.1:1`, no env URL: `unreachable`,
    exit 3; no database needed).
  - Add `a_search_reads_the_url_row` (the row set to `QDRANT_DOWN` → `qdrant_unreachable`).
  - Add `the_env_url_wins_over_the_row`.
  - Add `a_malformed_row_is_qdrant_url_unusable`.
  - The other cases change only through `searched()`.

### T3.6 Commit groups

1. `feat(MOD-158): QdrantSource, resolve_settings and the index reading through them` (concepts_worker.rs).
2. `feat(MOD-158): the store worker and htui worker publish their store; --qdrant-key-stdin` (store_worker.rs, worker_cmd.rs, cli.rs).
3. `feat(MOD-158): the worker job resolves per cycle; search and index read the row` (concepts.rs, tests/qdrant_worker.rs, tests/state_cli_pg.rs, docs/htui-worker.md).

---

## T4 — Settings > Qdrant and the offer (D9, D10) · W3

### T4.1 Snapshot (`crates/htui/src/qdrant_settings_info.rs`)

```rust
pub struct QdrantSnapshot {
    /// The database-wide `qdrant_url` row (D9).
    pub url: UrlRowState,
    /// `HTUI_QDRANT_URL` in force in this process (D3), parsed. `None` under `--demo`.
    pub url_override: Option<Result<QdrantUrl, QdrantUrlError>>,
    /// This box's keyring `qdrant-key` (QdrantState as today).
    pub key_state: QdrantState,
    /// `HTUI_QDRANT_API_KEY` is set in this process. `false` under `--demo`.
    pub key_override: bool,
    /// D10: this box's keyring `qdrant-url`, retired. `None` when absent, unreadable, or under `--demo`.
    pub old_url: Option<OldUrl>,
}
pub enum UrlRowState {
    Set { url: QdrantUrl, token: DateTime<Utc> },
    Unusable { why: QdrantUrlError, token: DateTime<Utc> },   // P-3: e overwrites, c clears
    NotSet, Offline, Unreadable(String), NotApplicable,
}
pub enum OldUrl { Usable(QdrantUrl), Unusable(QdrantUrlError) }
```

`QdrantState` (`:24-35`) stays for the key. `serve` (`:84-138`) becomes
`serve_with(backend, request, env: &QdrantEnv)`, and `serve` calls it with
`&QdrantEnv::from_process()` (P-13). `Memory` answers every field n/a. `Online` reads
`pg.qdrant_url()`: `Unreachable` propagates with `?` so the loop drops offline, and any other error
becomes `Unreadable(text)`. `Offline` answers the URL row `Offline`.

### T4.2 Requests and replies (`crates/htui/src/store_worker.rs`)

| `StoreRequest` (replaces `:917-925`) | `name()` (`:1315-1318`) | Payload | Replies |
|---|---|---|---|
| `QdrantInfo` | `qdrant_info` | — | `Qdrant` |
| `SetQdrantUrl { url: QdrantUrl, expected: Option<DateTime<Utc>> }` | `set_qdrant_url` | | `Qdrant`, `QdrantStale` |
| `ClearQdrantUrl { expected: DateTime<Utc> }` (new) | `clear_qdrant_url` | | `Qdrant`, `QdrantStale` |
| `SetQdrantApiKey(Redacted)` | `set_qdrant_api_key` | | `Qdrant` |
| `ClearQdrantApiKey` (replaces `ClearQdrantSettings`) | `clear_qdrant_api_key` | | `Qdrant` |
| `AdoptOldQdrantUrl` (new) | `adopt_old_qdrant_url` | — (the worker re-reads the keyring) | `Qdrant`, `QdrantStale`, `Failed` |
| `DiscardOldQdrantUrl` (new) | `discard_old_qdrant_url` | — | `Qdrant` |

- `StoreReply::QdrantStale(Box<QdrantSnapshot>)` goes next to `Qdrant` (`:1587-1588`), in the
  `*Stale` pattern (`:1487-1771`).
- `qdrant_settings_info::REQUEST_NAMES: [&str; 7]` gets a test, `request_names_match_the_name_arms`,
  in the shape of `connection.rs:259`.
- `try_serve`'s arm (`:2310-2318`) or-s the seven.
- Under `--demo` every write is refused with `DEMO_SESSION`.
- Offline, the URL writes and the adopt answer `Failed` with "the Qdrant URL cannot be changed
  while Postgres is offline"; the key writes and the discard work.
- **Adopt:**
  1. read the keyring entry: none → `Failed` "this box's keyring holds no old Qdrant URL"; one that
     does not parse → `Failed` "the old Qdrant URL is not usable: {why}; discard it";
  2. `set_qdrant_url(&url, None)`: `Stale` → `QdrantStale`, and the keyring entry is **kept**;
  3. `Applied` → `clear_qdrant_url` from the keyring. A failure there is `Failed` "the URL is set for
     every box, but the old keyring entry could not be removed: {err}".

### T4.3 Rows and modes (`crates/htui/src/ui/tabs/settings/qdrant.rs`)

The rows are dynamic: `[Url, Offer?, Key]`. The cursor is clamped on every snapshot. `LABEL_WIDTH`
stays 8, and the offer's label is `old URL`.

| Row | Value | Help line under it |
|---|---|---|
| URL | `Set`: "{url} · for every box". `Unusable`: "not usable: {why} · for every box". `NotSet`: "not set · for every box". `Offline`: "cannot be read while Postgres is offline". `NotApplicable`: `DEMO_ROW`. `Unreadable(w)`: "could not be read: {w}" | override: "HTUI_QDRANT_URL={url} is in force in this process, which does not use this setting", or "HTUI_QDRANT_URL is not usable ({why}); this process cannot search" |
| old URL (D10) | usable, no row: "this box's keyring holds {url} from before: {adopt} uses it for every box, {clear} discards it". Usable, row set: "this box's keyring holds {url} from before, no longer used: {clear} discards it". Unusable: "this box's keyring holds an old Qdrant URL that is not usable ({why}): {clear} discards it". The keys are labels in force | — |
| API key | as today (`key_value`, `:485-496`) | override: "HTUI_QDRANT_API_KEY is in force in this process" |

- **`e`** on URL: opens the URL editor. On submit, `QdrantUrl::parse`; an `Err` stays in the field
  as its error. `Ok` sends `SetQdrantUrl { url, expected }`, where `expected` is the `Set`/`Unusable`
  token and `None` for `NotSet`. It is refused `Offline` and under demo. **`e`** on Key: as today.
  **`e`** on old URL: refused, "{adopt} uses the old URL, {clear} discards it".
- **`c`** clears the focused row (Q2). URL `Set`/`Unusable` asks "Remove the Qdrant URL for every
  box?". Key `Stored` asks "Remove this box's Qdrant API key from the keyring?". Old URL asks
  "Discard the old Qdrant URL from this box's keyring?". Otherwise `c` is refused with that row's
  "nothing to clear" sentence. The unreadable refusal is as today (`unreadable_refusal`,
  `:97-103`).
- **`u`** (`Act::QdrantAdopt`), only on the old URL row when it is usable and no row is set, asks
  "Use {url} for every box?". On an old URL row that cannot be adopted it is refused with the
  reason. On any other row it is `continue`, so it falls through to `Pass`.
- `Mode::ConfirmClear` becomes `Mode::Confirm(Question)`, where `Question` is `ClearUrl`,
  `ClearKey`, `Discard` or `Adopt`, and it still runs through `QDRANT_CONFIRM`.
- **Notices** after a write's reply:
  - `set_qdrant_url`: "the Qdrant URL is set for every box"
  - `clear_qdrant_url`: "the Qdrant URL is cleared for every box"
  - `set_qdrant_api_key`: `STORED`
  - `clear_qdrant_api_key`: "this box's API key is gone from the keyring"
  - `adopt_old_qdrant_url`: "the old URL is now the Qdrant URL for every box; this box's keyring entry is gone"
  - `discard_old_qdrant_url`: "the old Qdrant URL is gone from this box's keyring"
  - `QdrantStale`: `CHANGED_ELSEWHERE_CLOSED` (`settings/mod.rs:113`, drawn red by `is_error`)
  - `doing()` (`:640-649`) gains one phrase per name.
- **The auto-opened URL editor** (`:623-632`) opens only on `NotSet` with no override.
- **Hint specs**:
  - `HINT_BROWSE`: Edit "edit", Clear "clear" (was "clear all"), Reload, rows.
  - `HINT_BROWSE_OFFER`: QdrantAdopt "use for every box", Clear "discard", Reload, rows.
  - `HINT_BROWSE_DISCARD`: Clear "discard", Reload, rows.

### T4.4 The status-line notice (P-6)

- `app/mod.rs:67` `register_all`: `app.qdrant_offer_check = true`.
- `app/state.rs`: the field, next to `below_target_shown` (`:481`, `:583`). `start` (`:856-861`)
  sends `QdrantInfo` under `Origin::App` when the field is set.
- `app/update.rs` `on_app_reply` (`:548-575`): on `StoreReply::Qdrant(s)` with `s.old_url.is_some()`
  and `!self.qdrant_offer_noted`, set the status to "this box's keyring holds an old Qdrant URL;
  Settings > Qdrant offers to use it for every box or discard it", once per launch.

### T4.5 Keys

- `keys/catalogue.rs`:
  - Add `Act::QdrantAdopt`, doc `settings.qdrant.adopt`: "use the old keyring Qdrant URL for every
    box".
  - Append it **last** in `ALL` (after `RequirementsPriorityLater`, `position` 212), so no existing
    position moves.
  - Add `row(Act::QdrantAdopt, SettingsQdrant, "adopt", &["u"], "use old URL")` after the
    `[settings.connection]` rows (`:1335-1349`) under `// [settings.qdrant]`.
  - Move `CATALOGUE.len()` from 212 to **213** (`:2896`).
- `keys/views.rs:366-373`: `QDRANT_BROWSE`'s view layer becomes
  `Layer::view(Context::SettingsQdrant, &[Act::QdrantAdopt])`. `QDRANT_CONFIRM` is unchanged.
- `u` collides with nothing in that stack (`[global]`, `[list]`, `[settings]`, `[history]`, and
  `[common]`'s e/n/d/c/r).
- `docs/htui-keys.md`: regenerate the toml block with
  `cargo run -q -p htui -- --default-keys --print-keys`. A `[settings.qdrant]` table appears.

### T4.6 Tests and snapshots

- **`tests/qdrant_settings.rs`** (literal snapshots, no keyring). The fixtures `stored()`, `empty()`
  and `unreadable()` (`:36-61`) are rebuilt with the new fields.
  - Changed cases (all 19, `:89-489`): every literal moves, and these assertions change:
    - `c_asks_before_it_clears`: "Remove the Qdrant URL for every box? y / n".
    - `the_clear_question_sits_under_its_row`: the same text.
    - `a_write_in_flight_reads_in_words`: "clearing the Qdrant URL for every box".
    - `an_empty_keyring_opens_the_url_editor` is renamed `no_url_row_opens_the_url_editor`.
    - `an_unreadable_keyring_says_so_and_opens_nothing` now covers the key row only, with an
      unreadable URL row as its own case.
  - New cases:
    - `the_url_row_says_it_applies_to_every_box`
    - `an_override_is_shown_under_its_row`
    - `offline_the_url_row_cannot_be_read_and_e_is_refused`
    - `c_on_the_key_row_clears_only_this_box_s_key`
    - `the_offer_row_sits_under_the_url_row` (snapshot `offer`)
    - `u_asks_then_adopts`
    - `an_unusable_old_url_can_only_be_discarded` (snapshot `offer_discard_only`)
    - `a_stale_reply_says_changed_elsewhere`
    - `a_malformed_row_can_be_overwritten_and_cleared`
    - `the_field_refuses_a_url_with_credentials_before_sending`
- **`tests/settings.rs`** (`:5731-6260`):
  - `qdrant_stored` gets the new fields.
  - `a_qdrant_write_says_what_it_did_or_why_it_failed`, `a_qdrant_write_in_flight_refuses_e_and_c_but_not_r`,
    `a_qdrant_reload_just_before_a_write_is_not_taken_as_its_answer` and
    `the_qdrant_clear_question_answers_y_and_n_and_passes_ctrl` use `ClearQdrantUrl { expected }`
    and the new text.
  - `qdrant_demo_reads_no_keyring` asserts the new fields.
  - `qdrant_demo_refuses_every_keyring_write` is renamed `qdrant_demo_refuses_every_write` and runs
    all six writes.
  - `tab_in_the_qdrant_editor_keeps_the_settings_tab` opens the key editor (P-18).
  - `qdrant_r_re_reads_and_recovers_the_unavailable_state`, `an_empty_qdrant_key_submit_sends_nothing`,
    `qdrant_demo_section_offers_no_edit`, `the_qdrant_editors_keep_tab_and_pass_only_ctrl_alt_and_function_keys`
    and `ctrl_e_and_ctrl_r_in_qdrant_browse_do_nothing` change their literals only.
- **`tests/secrets_settings.rs`**: `:196` and `:621`, literals only.
- **`tests/qdrant_settings_pg.rs`** (CREATE, `#![cfg(feature = "testkit")]`). Each case runs
  `serve_with` over `Online { pg: demo_db store, cache: temp CacheStore }` with
  `QdrantEnv::default()` and a `mock_keyring()` guard:
  - `set_then_clear_the_url_row`
  - `a_spent_token_answers_qdrant_stale`
  - `adopt_moves_the_keyring_url_to_the_row_and_clears_the_entry`
  - `adopt_over_a_set_row_is_stale_and_keeps_the_entry`
  - `discard_removes_only_the_entry`
  - `a_malformed_row_reads_unusable_with_its_token`
  - `offline_refuses_url_writes_but_stores_the_key` (over `Backend::Offline`)
- **`app/update.rs` unit**: `the_qdrant_offer_is_said_once_per_launch` and
  `start_asks_for_the_offer_only_after_register_all`.
- **`ui/tabs/settings/qdrant.rs` unit**: `the_qdrant_sentences_without_their_keys` (`:806`) lists
  all seven names.
- **Expected to move** (review each in a full insta run):
  - all seven of `crates/htui/tests/snapshots/qdrant_settings__{confirm,demo_80,editor,editor_key,empty,stored,unreadable}.snap`.
    `demo_80`'s hint line changes from "c clear all" to "c clear".
  - new: `qdrant_settings__{offer,offer_discard_only,offline,override}.snap`.
  - Nothing outside `qdrant_settings__*` is expected to move. `connection__demo_80`,
    `secrets_settings__demo*` and `tests/keys.rs:140` (demo shell, "r reload · j/k rows") are
    expected unchanged and must be confirmed by the full run.

### T4.7 Commit groups

1. `feat(MOD-158): Qdrant snapshot, requests and stale reply over the qdrant_url row` (qdrant_settings_info.rs, store_worker.rs, tests/qdrant_settings_pg.rs).
2. `feat(MOD-158): Settings > Qdrant rows, per-row clear and the old-URL offer` (ui/tabs/settings/qdrant.rs, keys/catalogue.rs, keys/views.rs, docs/htui-keys.md, tests/qdrant_settings.rs, tests/settings.rs, tests/secrets_settings.rs, snapshots).
3. `feat(MOD-158): the offer said once on the status line` (app/{mod,state,update}.rs).

---

## T5 — CLI flags (D8) · W3

### T5.1 `crates/htui/src/cli.rs` `Args` (after `clear_dsn`, `:31-34`)

```rust
/// Set the Qdrant URL every box uses (one setting in Postgres) and exit. A URL with a user name,
/// password, query or fragment is refused (`R-STO-1`).
#[arg(long, value_name = "URL", conflicts_with_all = [
    "clear_qdrant_url", "set_qdrant_key", "clear_qdrant_key", "set_dsn", "clear_dsn", "dsn_stdin",
    "demo", "offline", "index_items", "search_items", "keys", "default_keys", "print_keys"])]
pub set_qdrant_url: Option<String>,
/// Remove the Qdrant URL every box uses and exit.
#[arg(long, conflicts_with_all = ["set_qdrant_url", "set_qdrant_key", "clear_qdrant_key", /* same tail */])]
pub clear_qdrant_url: bool,
/// Read a Qdrant API key from stdin, store it in this box's OS keyring and exit.
#[arg(long, conflicts_with_all = ["set_qdrant_url", "clear_qdrant_url", "clear_qdrant_key", /* same tail */])]
pub set_qdrant_key: bool,
/// Remove this box's Qdrant API key from the OS keyring and exit.
#[arg(long, conflicts_with_all = ["set_qdrant_url", "clear_qdrant_url", "set_qdrant_key", /* same tail */])]
pub clear_qdrant_key: bool,
```

The tail is `"set_dsn", "clear_dsn", "dsn_stdin", "demo", "offline", "index_items", "search_items",
"keys", "default_keys", "print_keys"`. clap checks conflicts both ways, so the existing flags'
lists need no edit. `args_conflicts_with_subcommands` (`:12`) already refuses a subcommand beside
them. The URL is an `Option<String>` checked by `qdrant_cmd`, so a refusal is htui's sentence, not
a clap usage error.

### T5.2 `crates/htui/src/qdrant_cmd.rs` (CREATE) and `lib.rs` routing (after `clear_dsn`, `lib.rs:141-145`, plus `pub mod qdrant_cmd;`)

```rust
/// The four Qdrant flags (MOD-158 D8). Answers `state_cmd::StateExit`: Refused 2, Offline 3,
/// Failed 1 (P-11). Success lines go to stderr, as `--set-dsn`'s do.
pub async fn set_url(text: &str) -> Result<(), StateExit>;
pub async fn clear_url() -> Result<(), StateExit>;
pub fn set_key(reader: &mut dyn std::io::BufRead) -> Result<String, StateExit>;  // the line printed
pub fn clear_key() -> Result<String, StateExit>;
/// The testable cores over a connected store.
pub async fn set_url_on(pg: &PgStore, text: &str, env_url: Option<&str>) -> Result<String, StateExit>;
pub async fn clear_url_on(pg: &PgStore) -> Result<String, StateExit>;
```

The URL flags connect headless with the keyring DSN, as `concepts::open` does (`concepts.rs:119-128`):
`NO_DSN`, `identity::load_or_mint`, `connect_headless`, with refusals through
`concepts::headless_refusal`. `StoreError::Unreachable` is Offline. Then read-then-write: the token
from `qdrant_url()`, then `set_qdrant_url(&url, token)`.

| Ending | Sentence | Exit |
|---|---|---|
| set | "Qdrant URL set for every box: {url}." When `HTUI_QDRANT_URL` is set in this environment, add: "HTUI_QDRANT_URL overrides it in processes that have it." | 0 |
| clear | "Qdrant URL cleared for every box." (no row: "no Qdrant URL is set; nothing was cleared") | 0 |
| key set or cleared | "Qdrant API key stored in this box's OS keyring." / "Qdrant API key removed from this box's OS keyring." | 0 |
| an invalid URL | "not setting the Qdrant URL: {why}" | 2 |
| stale | "the Qdrant URL changed while it was being set; nothing was written — run the command again" | 2 |
| no DSN, keyring, schema | `NO_DSN`, "the OS keyring could not be read: {err}", the headless sentences | 2 |
| a blank key line | "no Qdrant API key on stdin; nothing was stored" | 2 |
| Postgres unreachable | "cannot reach Postgres: {err}" | 3 |
| anything else | the error | 1 |

On a terminal, the key prompt goes to stderr: "paste the Qdrant API key and press Enter (it will be
visible):". The line is read with `secret::read_dsn_line`.

### T5.3 Tests first

- `cli.rs`:
  - `the_qdrant_flags_parse_alone` (with `--log x` too)
  - `the_qdrant_flags_exclude_each_other_the_dsn_and_key_flags_and_tui_flags` (every pair in the
    tail, every pair of the four, and `worker` beside them)
  - `no_qdrant_flag_reads_the_environment` (the existing `no_tui_argument_but_log_reads_the_environment`
    already covers it; assert by name)
- `tests/qdrant_cli_pg.rs` (CREATE, `#![cfg(feature = "testkit")]`, `--test-threads=1`):
  - over `demo_db`:
    - `set_stores_the_row_as_a_json_string`
    - `a_url_with_credentials_is_refused_and_writes_nothing` (no row; stderr lacks the password)
    - `set_over_a_set_row_updates_it_by_its_token`
    - `set_over_a_malformed_row_fixes_it`
    - `clear_removes_the_row_and_twice_is_nothing_to_clear`
    - `an_env_override_is_named_on_set`
  - over `mock_keyring()`:
    - `set_key_stores_one_stdin_line`
    - `a_blank_key_line_is_refused_and_stores_nothing`
    - `clear_key_removes_it`
    - `a_broken_keyring_is_a_refusal_naming_it` (`mock_keyring_broken`)
  - The stale path goes through `set_url_on` after a concurrent write between the read and the
    write. Expose `set_url_expecting(pg, url, Some(spent_token))` (`pub(crate)`, used by the
    core), and pin it.
- **`docs/htui-cli.md`**:
  - add a section "Setting the Qdrant URL and API key" with the four flags, their exits and "the
    URL is one setting for every box; the key is this box's";
  - `:306` drop "no Qdrant URL" from "before Postgres answered";
  - `:344-345` new `no_qdrant_url` / `qdrant_url_unusable` texts naming Settings › Qdrant,
    `htui --set-qdrant-url` and `HTUI_QDRANT_URL`;
  - `:354-358` the new order (DSN, `box.toml`, the connect, the Qdrant URL and key, the scope...);
  - `:431-433` the URL from Postgres or `HTUI_QDRANT_URL`, the key from the keyring or
    `HTUI_QDRANT_API_KEY`.

### T5.4 Commit groups

1. `feat(MOD-158): --set/--clear-qdrant-url and --set/--clear-qdrant-key` (cli.rs, lib.rs, qdrant_cmd.rs, tests/qdrant_cli_pg.rs).
2. `docs(MOD-158): the Qdrant flags and the search refusal order` (docs/htui-cli.md).

---

## T6 — Provision the key credential; the sandbox override (D11, D12) · W3

### T6.1 `provision/plan.rs`

- `LocalFacts` (`:17-22`) gains `pub qdrant_key: bool`.
- `decide` (`:118-125`): refuse when `!facts.printf_builtin && (sudo == SudoMode::Password ||
  local.qdrant_key)`. The key case says: "the remote sh has no printf builtin (as in posh or mksh),
  so sending the Qdrant API key would put it in a process's arguments; remove this box's key
  (`htui --clear-qdrant-key`) or encrypt it on the host by hand". The password sentence is
  unchanged.

### T6.2 `provision/script.rs`

- **INSTALL** (`:127-146`): `$7` is `qkey` (`0`|`1`), passed on as INSTALL_ROOT's `$5`. The doc
  states the stdin order: the password line, then the key line when `qkey=1`, then the DSN line.
- **INSTALL_ROOT** (`:151-203`):
  - `qkey=$5`, `qcred="$creds/htui-qdrant-key"`, a literal-newline `nl`;
  - `cleanup` also removes `"$qcred.new"`;
  - after `install -d`, `cl="LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn"`;
  - when `qkey=1`: `IFS= read -r qk || exit 1`; when the credential is absent or `replace=1`,
    `printf "%s%s" "$qk" "$nl" | systemd-creds encrypt --name=htui-qdrant-key - "$qcred.new"`
    then `mv`; `unset qk`;
    `cl="$cl${nl}LoadCredentialEncrypted=htui-qdrant-key:/etc/credstore.encrypted/htui-qdrant-key"`;
  - the heredoc's credential line becomes `$cl`. **Without a key the unit is byte-identical.**
  - There is still no backslash and no single quote anywhere.
- **Tests**:
  - `install_root_writes_the_credential_then_the_unit_then_starts`: its needle becomes
    `systemd-creds encrypt --name=htui-dsn`, because `line_of` requires a single hit.
  - `the_unit_is_the_guides_sample` substitutes `$cl` with the DSN line.
  - Add `the_key_is_read_before_the_dsn_and_only_with_qkey`.
  - Add `the_unit_with_a_key_names_both_credentials`.

### T6.3 `provision/mod.rs`

- `Ctx` (`:162-185`) gains `pub qdrant_key: Option<Zeroizing<String>>`, and its `Debug` redacts it.
- `run` (`:216-247`) reads `secret::get_qdrant_api_key()`. An `Err` is a progress warning ("the OS
  keyring could not be read for a Qdrant API key (…); provisioning without one"), and provisioning
  goes on without the key.
- `LocalFacts { qdrant_key: ctx.qdrant_key.is_some(), .. }` (`:348`).
- `install_stdin(password, qdrant_key, dsn)` (`:849-858`).
- INSTALL gets the extra argument (`:471-482`).
- On `AlreadyProvisioned` with a key, add a note: "the Qdrant API key was not sent; pass
  --replace-credential to add it".
- Unit tests that change: `ctx_debug_redacts_the_dsn` (and the key), `install_stdin_and_box_text`,
  `a_fresh_password_host_runs_four_sessions_with_their_stdin`, `the_sentinels_reach_only_stdin`,
  `ssh_argv_of_every_session_carries_no_sentinel` (add `SENTINEL-QDRANT-KEY`), and
  `preflight_refusals_are_refusals` (the key printf refusal).

### T6.4 `crates/htui/tests/provision.rs`

- `STUB_SYSTEMD_CREDS` (`:150-158`) accepts `--name=htui-dsn` or `--name=htui-qdrant-key`.
- `World` gains `qdrant_credential()`, and `expected_unit(key: bool)` (`:296-306`).
- `Setup` (`:510-523`) gains `qdrant_key: Option<&str>`, threaded into `Ctx` (`:547-559`).
- New cases: `a_box_with_a_qdrant_key_gets_both_credentials_and_both_unit_lines`,
  `without_a_key_the_unit_is_unchanged` (`assert_fresh_install` as today),
  `replace_credential_re_encrypts_the_key_too` and
  `a_key_on_a_host_without_a_printf_builtin_is_refused_before_any_write` (`world_without_printf_builtin`,
  `:852`). `sweep` (`:618`) also looks for the key sentinel.

### T6.5 D12 (P-1)

- `docker/hr/compose.hr.yaml:66`: add `HTUI_QDRANT_URL: http://localhost:6334` after
  `HTUI_TEST_QDRANT_URL`.
- `tests/hr/hr.bats`: a new test `3i. compose: a run's htui uses the run's own Qdrant`, with
  `[[ "$(jq -r '.services.dev.environment.HTUI_QDRANT_URL' <<<"$cfg")" == http://localhost:6334 ]]`.
- `docs/hr-sandbox.md:184-191` (the block and a sentence: a sandbox TUI or worker never reads the
  database's `qdrant_url` row, so it cannot point at another Qdrant; `HTUI_QDRANT_API_KEY` is not
  set because the run's Qdrant has no key), and `:479` (the list of set variables).
- **`docs/htui-worker.md`** (provisioning only): the credential table (`:549`) gets a
  `/etc/credstore.encrypted/htui-qdrant-key` row ("when this box's keyring holds a Qdrant key");
  the systemd section (`:674-735`) gets the optional second `LoadCredentialEncrypted=` line and its
  `systemd-creds encrypt --name=htui-qdrant-key` command.

### T6.6 Commit groups

1. `feat(MOD-158): htui provision encrypts the Qdrant key beside the DSN` (provision/*, tests/provision.rs, docs/htui-worker.md).
2. `chore(MOD-158): hr runs set HTUI_QDRANT_URL to their own Qdrant` (compose.hr.yaml, hr.bats, docs/hr-sandbox.md).

---

## 7. File sets and wave intersections

The plan's split and wave order stand: **W1** T1 ∥ T2 → **W2** T3 → **W3** T4 ∥ T5 ∥ T6. T3 must
follow T1 and T2 (it uses `QdrantUrl`, `StoredQdrantUrl`, `QdrantEnv`, `qdrant_api_key`). T4 and T5
must follow T3 (`store_worker.rs`, `cli.rs`). T6 needs only T2 and stays in W3.

| Task | Files (C = create) |
|---|---|
| T1 | `crates/htui-core/src/qdrant.rs` (C), `crates/htui-core/src/lib.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/qdrant_settings.rs`, `crates/htui-store/src/vector.rs`, `crates/htui-store/tests/qdrant_live.rs`, `crates/htui-store/tests/qdrant_url_pg.rs` (C), `crates/htui/src/concepts.rs`, `crates/htui/src/concepts_worker.rs`, `crates/htui/tests/qdrant_worker.rs`, `crates/htui/tests/state_cli_pg.rs` |
| T2 | `crates/htui-core/src/secret.rs`, `crates/htui-store/src/secret.rs`, `crates/htui-agent/src/launch.rs`, `crates/htui-agent/tests/launch.rs`, `crates/htui-agent/src/auth/browser.rs`, `crates/htui-orch/src/verify.rs`, `crates/htui-orch/src/isolate/git.rs` |
| T3 | `crates/htui/src/concepts_worker.rs`, `crates/htui/src/concepts.rs`, `crates/htui/src/worker_cmd.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/cli.rs`, `crates/htui/tests/qdrant_worker.rs`, `crates/htui/tests/state_cli_pg.rs`, `docs/htui-worker.md` |
| T4 | `crates/htui/src/qdrant_settings_info.rs`, `crates/htui/src/ui/tabs/settings/qdrant.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs`, `docs/htui-keys.md`, `crates/htui/src/app/mod.rs`, `crates/htui/src/app/state.rs`, `crates/htui/src/app/update.rs`, `crates/htui/tests/qdrant_settings.rs`, `crates/htui/tests/qdrant_settings_pg.rs` (C), `crates/htui/tests/settings.rs`, `crates/htui/tests/secrets_settings.rs`, `crates/htui/tests/snapshots/qdrant_settings__*.snap` |
| T5 | `crates/htui/src/cli.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/qdrant_cmd.rs` (C), `crates/htui/tests/qdrant_cli_pg.rs` (C), `docs/htui-cli.md` |
| T6 | `crates/htui/src/provision/mod.rs`, `crates/htui/src/provision/plan.rs`, `crates/htui/src/provision/script.rs`, `crates/htui/tests/provision.rs`, `docs/htui-worker.md`, `docker/hr/compose.hr.yaml`, `tests/hr/hr.bats`, `docs/hr-sandbox.md` |

**W1: T1 ∩ T2 = ∅.** T1 touches no `secret.rs`. T2 touches no `htui` crate file, no `pg/*`, no
`mem.rs` and no `lib.rs`.

Hidden couplings, checked:
- `htui-store/src/lib.rs` and `htui-core/src/lib.rs` (only T1 adds a module);
- `.sqlx/` (nobody);
- `FakeSlots`/`testkit.rs` (nobody, P-16);
- `Cargo.toml`/`Cargo.lock` (no new dependency);
- snapshots (nobody).

**W2:** T3 alone.

**W3, pairwise:**
- T4 ∩ T5 = ∅. `lib.rs` is T5's only: the notice goes through `app/`, P-6. `cli.rs` is T5's only.
- T4 ∩ T6 = ∅.
- T5 ∩ T6 = ∅ (`docs/htui-cli.md` ≠ `docs/htui-worker.md`).

Hidden couplings, checked:
- `StoreRequest::name()` and its pinned lists (T4 only: `qdrant_settings_info::REQUEST_NAMES`, and
  the `doing()` test in `qdrant.rs`);
- `keys.toml` defaults and `docs/htui-keys.md` (T4 only);
- insta snapshots (T4 only: `qdrant_settings__*`; T5 and T6 render no frame);
- `docs/htui-cli.md` (T5 only);
- fixtures (`tests/provision.rs` stubs are T6's; `tests/fixtures/keys/*.toml` set no
  `settings.qdrant`, so they are unaffected);
- `main.rs` (nobody, P-11);
- `testkit.rs` (nobody).

---

## 8. Verification per task

The sandbox environment is `HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres`
and `HTUI_TEST_QDRANT_URL=http://localhost:6334`. `htui` integration tests need
`--features testkit` or `--all-features`, or they run 0 tests and report `ok`. Run touched suites
with `--test-threads=1`.

| Task | Commands |
|---|---|
| T1 | `cargo test -p htui-core --lib qdrant` · `cargo test -p htui-core --features test-support --lib store::mem` · `SQLX_OFFLINE=true cargo check -p htui-store --all-targets --all-features` and `git status --porcelain crates/htui-store/.sqlx` (empty) · `cargo test -p htui-store --all-features --test qdrant_url_pg -- --test-threads=1` · `cargo check -p htui --all-targets --all-features` |
| T2 | `cargo test -p htui-core --lib secret` · `cargo test -p htui-store --all-features --lib secret -- --test-threads=1` · `cargo test -p htui-agent --all-features --test launch -- --test-threads=1` · `cargo test -p htui-agent --all-features --lib auth::browser` · `cargo test -p htui-orch --all-features --lib verify -- --test-threads=1` · `cargo test -p htui-orch --all-features --lib isolate::git::tests::the_git_command_removes_htui_own_env` |
| T3 | `cargo test -p htui --all-features --lib concepts -- --test-threads=1` (covers `concepts_worker`) · `cargo test -p htui --all-features --lib worker_cmd cli store_worker -- --test-threads=1` · `cargo test -p htui --all-features --test qdrant_worker --test state_cli_pg -- --test-threads=1` |
| T4 | `cargo test -p htui --all-features --test qdrant_settings --test qdrant_settings_pg --test settings --test secrets_settings --test keys --test keys_file -- --test-threads=1` · `cargo test -p htui --all-features --lib keys app qdrant -- --test-threads=1` · full insta: `INSTA_FORCE_PASS=0 cargo insta test -p htui --all-features -- --test-threads=1`, then review every `.snap.new` (memory: grepping `.snap` undercounts) |
| T5 | `cargo test -p htui --all-features --lib cli -- --test-threads=1` · `cargo test -p htui --all-features --test qdrant_cli_pg -- --test-threads=1` · `cargo test -p htui --all-features --bin htui` (exit mapping unchanged) |
| T6 | `cargo test -p htui --all-features --lib provision -- --test-threads=1` · `cargo test -p htui --all-features --test provision -- --test-threads=1` · `bash -n scripts/hr` · `bats tests/hr/hr.bats -f 3i` (needs `docker compose config`: on the host if the sandbox has no docker) |
| Close | `bash scripts/gate.sh` · `cargo clippy --workspace -- -D warnings` (featureless) · `cargo test -p htui --all-features -- --test-threads=1` |

---

## 9. Risks and gotchas

| # | Risk | Mitigation |
|---|---|---|
| R-1 | A reused SQL literal drifts by one space and needs an `.sqlx` entry. | §T1.3's exact strings. `SQLX_OFFLINE=true cargo check` is the first thing T1 runs after writing them. `git status crates/htui-store/.sqlx` must stay clean. |
| R-2 | `HTUI_QDRANT_URL` in a sandbox's environment changes test outcomes there and not on the host (P-13). | `QdrantEnv` is injected everywhere a test reaches. The reviewer greps `from_process` under `tests/` and `#[cfg(test)]`, and it must not appear. |
| R-3 | A Settings write sent with a token from an old snapshot. | The CAS answers `QdrantStale(snapshot)`, and the section takes the new token from it. |
| R-4 | Two processes adopt the same old URL. | The insert expecting no row lets one win; the other gets `QdrantStale` and keeps its keyring entry, which the offer row then shows as discard-only. |
| R-5 | The store loop awaits a Postgres read for `QdrantInfo`. | This is the same as every Settings read today, and the keyring calls stay in `spawn_blocking`. Searches resolve on the concepts task (`R-NF-3`). |
| R-6 | The fake keyring is process-wide. | `--test-threads=1` for `settings`, `qdrant_settings_pg`, `qdrant_cli_pg`, `worker_cmd`. T2's key-source tests use closures and no fake. |
| R-7 | Not stripped, by D5's scope: `$EDITOR` (`editor.rs:143-154`), and `ssh` in provision (`remote.rs:109`; only `SendEnv` names travel). | Reported for the reviewer. Adding `HTUI_OWN_ENV` there is a one-line follow-up if the maintainer wants it. **Superseded by R1 H1 (maintainer, 2026-10-28):** both now remove `HTUI_OWN_ENV`. |
| R-8 | `docs/htui-keys.md` goes stale. | `tests/keys_file.rs:728` fails until the block is regenerated (T4.5). |
| R-9 | `catalogue.rs`'s `position` numbers. | Appending `QdrantAdopt` last keeps every existing position (`every_act_is_listed_and_has_a_row`, `:2869`). |
| R-10 | A TUI session with no keyring (`htui --dsn-stdin` over `ssh -t`): the interactive key read fails, so search fails even against a Qdrant that needs no key, as it does today. | Unchanged by D4. `HTUI_QDRANT_API_KEY` is the development way around it. The wording belongs to MOD-160. |
| R-11 | A provisioned host keeps an orphan `htui-qdrant-key` file after the key is removed and the host is re-provisioned. | The unit no longer names it. Say so in `docs/htui-worker.md`. |
| R-12 | The forbidden literal `zeta` (memory) and featureless dead code. | Avoid it in fixtures. Run featureless clippy: `QdrantSource::with_keyring` and `UrlRow for MemStore` must be used outside tests, or `#[cfg]`-gated. |
| R-13 | The worker job's cached connection after a dropped Qdrant: today the job keeps one store forever, and the cache keeps it the same way. | No regression. `Connections::forget` is used by search and sync failures only. |

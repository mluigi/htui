# Blueprint: MOD-10 milestone 2 — Infisical provider

**Plan**: `.claude/plans/mod-10-m2-infisical-provider.plan.md`. D1–D10, T1–T4 and the plan's
"Verified claims" table are binding. They were re-checked against the tree at `f751aa68`
(`hr/MOD-10`, plan confirmed) on 2026-10-04.
**PRD**: `.claude/prds/mod-10-secret-provider.prd.md`. **ANA**: `docs/ANA-7.md` §3.2–§3.3, §4.
**Rule for the tree vs. the plan**: where they disagree, the tree wins. Each such point is
recorded under **Amendments** (A-n) with its evidence. Where the plan leaves a detail open, this
blueprint fixes it and says so. Anything not proven in source is marked **VERIFY — implementer
must check**.

Conventions inherited unchanged:
- toolchain and lint headers:
  - MSRV 1.98 (`Cargo.toml:8`), edition 2024 (`Cargo.toml:7`);
  - `#![warn(missing_docs)]` in `htui-core` (`lib.rs:9`), `htui-store` (`lib.rs:11`) and
    `htui-mcp` (`lib.rs:17`), so the new crate carries it too;
  - workspace lints (`Cargo.toml:181-195`): `unsafe_code = forbid`,
    `missing_debug_implementations = warn`, `unused_qualifications = warn`, `clippy::all = warn`
    (not pedantic), and the rustdoc lints `broken_intra_doc_links`, `private_intra_doc_links`,
    `redundant_explicit_links` at deny. The gate runs at `-D warnings`;
- error enums are `#[derive(Debug, …, thiserror::Error)]` with a doc per variant
  (`crates/htui-orch/src/isolate.rs:36-54`, `crates/htui-store/src/secret.rs:186-213`);
- dyn-compatible async seams use a boxed future alias (`isolate.rs:28-34`); `async_trait` is not
  used anywhere;
- a struct holding a secret gets a hand-written `Debug` (`secret.rs:175-183` `DsnSources`,
  `crates/htui-agent/src/driver.rs:294` `SessionSpec`);
- no store-trait change, no migration, no `.sqlx` change, no snapshot change.

---

## Amendments (plan ≠ tree, or plan underspecified)

| # | Plan says | Tree / analysis says | Resolution |
|---|---|---|---|
| A-1 | D7: "`set_machine_identity` writes both. If the second write fails, the first is cleared so no half remains." | When an identity is **already stored** and is being replaced, clearing only the client ID after the secret write fails leaves the **old client secret** alone. `get_machine_identity` then answers "half stored" (D7's own rule), which is exactly the half D7 wants to rule out | On a failed second write, `set_machine_identity` removes **both** entries (best effort, both attempted) and returns the write error. D7's intent ("no half remains") is kept; only the mechanism is widened. Test `a_failed_secret_write_over_an_old_identity_leaves_nothing` pins it. **Flag for the maintainer** |
| A-2 | D1: htui-secrets depends on `thiserror` | `SecretError` lives in `htui-core` (D4), and htui-secrets defines no error type of its own, so `thiserror` has no user there. `unused_crate_dependencies` is allow-by-default, so no gate fails | Declared as D1 lists it (literal plan). The reviewer may drop it; nothing else depends on it. **Flag** |
| A-3 | T3: "If `tests/support/mod.rs` is shared with `infisical_live.rs`, guard it against dead-code warnings" | The live test talks to a real server and needs no stub. No crate in the workspace has a `tests/support/` directory yet (`ls crates/*/tests`) | **Not shared.** `infisical_live.rs` has no `mod support;`. Every helper in `support/mod.rs` is used by `tests/infisical.rs` (§D.3 maps each one), so no `allow(dead_code)` is needed. If a helper ends up unused, delete it (H-9) |
| A-4 | D4: server `message` quoted "truncated to 200 characters, with `\r` and `\n` removed" | A message can also carry other control characters (ESC sequences would reach a terminal through the TUI) | Every `char::is_control` is removed, which includes `\r` and `\n`. Conforming strengthening |
| A-5 | D4: `Protocol { endpoint, detail }` for "a non-JSON or ill-shaped body" | `serde_json::Error`'s `Display` quotes a mistyped **string value**: `invalid type: string "…", expected u64`. A login answer whose `expiresIn` held the token, or a list answer whose `secretValueHidden` held a value, would put the secret in `detail`. `reqwest`'s `Response::json` wraps the same error in its `source()` chain | Bodies are read with `Response::bytes()` and parsed with `serde_json::from_slice`; on failure `detail` is **only** the category and position: `"the body is not the expected JSON ({category:?} error at line {line}, column {column})"` from `err.classify()`, `err.line()`, `err.column()`. Never `err.to_string()`. Tests `a_malformed_login_answer_never_quotes_the_token` and `a_malformed_list_answer_never_quotes_a_value` pin it (H-4) |
| A-6 | D2: `list_keys` returns "key names … never values"; D6: the list request "always sends … `viewSecretValue=true`" | Together they mean `list_keys` fetches values (then drops them zeroized) and needs the identity's read-value permission, exactly like `resolve` | `list_keys` = the same fetch, merge and validation as `resolve`, returning `ResolvedSecrets::keys()`. So `list_keys` and `resolve().keys()` agree whenever either succeeds (T4's live assertion), and fail identically. **Flag for M4**, whose settings view may want a value-free listing later |
| A-7 | D4: `UnsupportedServer` for the v4 route's Fastify 404 | A server without Universal Auth answers the **login** route with the same Fastify 404. D4's login rule ("map status plus a known message, never quote") allows it: the Fastify sentence is a known message and is not quoted | Login's Fastify 404 → `UnsupportedServer { endpoint: "/api/v1/auth/universal-auth/login" }`, never latched. Conforming extension |
| A-8 | Validate `cargo test -p htui-store --features test-support secret` | `htui-store`'s `[dev-dependencies]` already enable `htui-store/test-support` on itself (`crates/htui-store/Cargo.toml` dev-deps), so `cargo test -p htui-store` turns it on anyway. The `headless_dsn_tests` module (`secret.rs:525`) depends on this | No change. The flag is kept in the gate for clarity. The **featureless** `cargo clippy -p htui-store -- -D warnings` (no `--all-targets`) is the build in which the fake is absent (H-14) |

Every other plan claim re-checked holds (§G).

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-core/Cargo.toml` | UPDATE | T1 | `zeroize = { workspace = true }` after `regex` (`:21-22`), with a reason comment (§B.5) |
| 2 | `Cargo.lock` | UPDATE | T1 | Only `htui-core`'s `dependencies` list (`:3101-3116`) gains `"zeroize"` after `"uuid"`. **Never `cargo update`** |
| 3 | `crates/htui-core/src/lib.rs` | UPDATE | T1 | `pub mod secret;` between `pub mod scrub;` (`:15`) and `pub mod seed;` (`:16`). Crate doc unchanged |
| 4 | `crates/htui-core/src/secret.rs` | CREATE | T1 | §B.1: `SecretFuture`, `SecretProvider`, `SecretScope`, `ResolvedSecrets`, `MachineIdentity`, `ProviderHealth`, `SecretError`, plus `mod tests` (§D.1) |
| 5 | `crates/htui-store/src/secret.rs` | UPDATE | T2 | §B.2: three `*_USER` constants after `:36`; three `FakeSlots` fields plus `refuse_store` (`:46-50`); private slot helpers and the six public functions inserted after `clear_qdrant_api_key` (ends `:358`), before `Slot` (`:360-367`); module doc paragraph (`:1-20`); new `#[cfg(test)] mod machine_identity_tests` after `:697`. **Not**: `Slot`, `backend`, any DSN or Qdrant function, `Fake`, `FAKE`, `FAKE_DSN_READS` |
| 6 | `crates/htui-store/src/testkit.rs` | UPDATE | T2 | `fake_machine_identity()` and `refuse_fake_store()` after `fake_dsn` (~`:386-396`). **Not**: `mock_keyring`, `mock_keyring_broken`, `KeyringGuard` |
| 7 | `Cargo.toml` | UPDATE | T3 | `members` (`:2-3`) gains `"crates/htui-secrets"`; `[workspace.dependencies]` gains `htui-secrets = { path = "crates/htui-secrets" }` after `htui-mcp` (`:54`) |
| 8 | `Cargo.lock` | UPDATE | T3 | A new `[[package]] name = "htui-secrets"` block. No version moves |
| 9 | `crates/htui-secrets/Cargo.toml` | CREATE | T3 | §B.5 |
| 10 | `crates/htui-secrets/src/lib.rs` | CREATE | T3 | Crate doc, `#![warn(missing_docs)]`, `mod infisical; mod wire;`, re-exports (§B.3) |
| 11 | `crates/htui-secrets/src/infisical.rs` | CREATE | T3 | `InfisicalConfig`, `InfisicalProvider`, `normalise_base_url`, `TokenState`, request/mapping logic, `merge`, `validate`, `mod tests` |
| 12 | `crates/htui-secrets/src/wire.rs` | CREATE | T3 | serde shapes (§B.3.4), `mod tests` |
| 13 | `crates/htui-secrets/tests/support/mod.rs` | CREATE | T3 | Loopback stub (§B.6) |
| 14 | `crates/htui-secrets/tests/infisical.rs` | CREATE | T3 | Stub-backed tests (§D.3) |
| 15 | `crates/htui-secrets/tests/infisical_live.rs` | CREATE | T4 | Env-gated live test (§D.4) |
| 16 | `README.md` | UPDATE | T4 | Development → Tests, after the Qdrant paragraph (`:481-482`); Further reading list (`:539-542`) |
| 17 | `docs/htui-secrets.md` | CREATE | T4 | Operator page (§B.7) |

No migration, no SQL, no `.sqlx` change, no `insta` snapshot. Nothing outside these files.

---

## B. Interfaces, exactly

### B.1 T1 — `htui-core::secret` (D2, D3, D4)

```rust
//! The secret-provider seam (MOD-10 D1–D4): the trait a provider implements, the scope it reads,
//! the map it returns and the one error it fails with.
//!
//! Lives in `htui-core` so `htui-worker`, `htui-orch` and `htui` can hold an
//! `Arc<dyn SecretProvider>` without depending on HTTP. The Infisical implementation is the
//! `htui-secrets` crate (plain backticks: no intra-doc link, H-12). The machine identity's keyring
//! entries are `htui-store::secret`'s.
//!
//! **No type here prints a secret.** `ResolvedSecrets` and `MachineIdentity` have hand-written
//! `Debug`s, and no `SecretError` variant carries a value, a client secret or a token.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};
```

#### B.1.1 Future alias and trait (D2)

```rust
/// The boxed future every [`SecretProvider`] method returns: the shape of `IsolatorFuture`
/// (`htui-orch`) and `DriverFuture` (`htui-agent`), for the same reason. A provider is held as
/// `Arc<dyn SecretProvider>`, and a plain `async fn` in a trait is not dyn-compatible.
pub type SecretFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, SecretError>> + Send + 'a>>;

/// A secret provider (MOD-10 D2). One instance per process and identity (D5: the login latch is
/// per instance, so M3 must share it).
pub trait SecretProvider: Send + Sync + core::fmt::Debug {
    /// Stable provider name, the value `project.secret_provider` holds (`"infisical"`).
    fn kind(&self) -> &'static str;
    /// Reachability plus a fresh login with the configured identity.
    fn health(&self) -> SecretFuture<'_, ProviderHealth>;
    /// Key names visible in `scope`, sorted; never values.
    fn list_keys<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>>;
    /// Every key → value in `scope`, imports merged, validated for an environment block.
    fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets>;
}
```

#### B.1.2 `SecretScope` (D3)

Fields are **private**, so a `SecretScope` that exists has passed validation. The serde shape is
a private wire struct, so `Deserialize` cannot bypass `parse`.

```rust
/// Which secrets a project reads: an Infisical project, environment slug and folder path (D3).
/// Stored in `project.secret_scope` as compact JSON (`to_column`); M4 writes it, M3 reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretScope {
    project_id: String,
    environment: String,
    path: String,
}

/// The column's JSON, field for field. Private: only `parse`/`to_column` touch it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeColumn {
    project_id: String,
    environment: String,
    #[serde(default = "root_path")]
    path: String,
}

fn root_path() -> String { "/".to_owned() }

impl SecretScope {
    /// A validated scope.
    ///
    /// # Errors
    ///
    /// [`SecretError::Config`] when `project_id` or `environment` is empty or whitespace, or
    /// `path` does not start with `/`.
    pub fn new(
        project_id: impl Into<String>,
        environment: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, SecretError>;

    /// Parses `project.secret_scope`. Strict: unknown fields refused; `path` defaults to `/`.
    ///
    /// # Errors
    ///
    /// [`SecretError::Config`]: not JSON of this shape, or a [`Self::new`] refusal.
    pub fn parse(column: &str) -> Result<Self, SecretError>;

    /// The column text: `{"project_id":"…","environment":"…","path":"…"}`, compact, in that key
    /// order, `path` always written.
    #[must_use]
    pub fn to_column(&self) -> String;

    /// The Infisical project ID.
    #[must_use]
    pub fn project_id(&self) -> &str;
    /// The environment slug, e.g. `dev`.
    #[must_use]
    pub fn environment(&self) -> &str;
    /// The folder path, starting with `/`.
    #[must_use]
    pub fn path(&self) -> &str;
}
```

- `to_column` serialises `ScopeColumn` with `serde_json::to_string(..).expect("a struct of three strings always serialises")`. A derived struct keeps field order whatever `serde_json/preserve_order` unification does; `json!` would not (H-13).
- Values are not trimmed or rewritten; whitespace-only is refused, anything else is kept as is.
- `Config` sentences (exact):
  - `"the secret scope is not valid: {serde_json error}"` (the column holds no secret, so serde's message may be quoted here; contrast A-5);
  - `"the secret scope has an empty project_id"`;
  - `"the secret scope has an empty environment"`;
  - `"the secret scope path must start with `/`"`.

#### B.1.3 `ResolvedSecrets` (D3)

```rust
/// A resolved `key → value` map (D3). Lends `&BTreeMap<String, String>` to
/// `MinimalScrubber::from_resolved` and `SessionSpec.env` (M3). Its `Debug` prints key names only,
/// and dropping it zeroizes every value. Not `Clone`, `Serialize` or `Display`.
///
/// M3 note: `from_resolved` and `SessionSpec.env` take unzeroized copies; the zeroize covers this
/// wrapper only.
pub struct ResolvedSecrets {
    map: BTreeMap<String, String>,
}

impl ResolvedSecrets {
    /// Wraps an already validated map (providers call this after their own validation).
    #[must_use]
    pub fn new(map: BTreeMap<String, String>) -> Self;
    /// The map, for `from_resolved` and `SessionSpec.env`.
    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<String, String>;
    /// The key names, sorted (map order).
    #[must_use]
    pub fn keys(&self) -> Vec<String>;
    /// How many keys.
    #[must_use]
    pub fn len(&self) -> usize;
    /// Whether there are none (`clippy::len_without_is_empty`).
    #[must_use]
    pub fn is_empty(&self) -> bool;
}

impl core::fmt::Debug for ResolvedSecrets {
    /// `ResolvedSecrets { keys: ["A", "B"] }`: names only.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResolvedSecrets").field("keys", &self.map.keys().collect::<Vec<_>>()).finish()
    }
}

impl Drop for ResolvedSecrets {
    fn drop(&mut self) {
        for value in self.map.values_mut() {
            value.zeroize();
        }
    }
}
```

`Drop` means the map cannot be moved out (E0509); there is deliberately no `into_map`.

#### B.1.4 `MachineIdentity` (D3)

```rust
/// An Infisical Universal Auth machine identity (D3). Both halves are held wiped-on-drop.
/// `Debug` is hand-written and never prints the secret; there is no `Display`, `Serialize`,
/// `Clone` or `PartialEq`.
pub struct MachineIdentity {
    client_id: Zeroizing<String>,
    client_secret: Zeroizing<String>,
}

impl MachineIdentity {
    /// Wraps both halves at once. No validation here: blank halves are refused by the provider's
    /// constructor (`Config`) and read as absent by the keyring (`Slot::get`, `secret.rs:383-390`).
    #[must_use]
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self;
    /// The client ID. Not secret (it is shown in `Debug`).
    #[must_use]
    pub fn client_id(&self) -> &str;
    /// The client secret. **Secret**: for the login body and the keyring write only; never log,
    /// format or compare it in an assert message.
    #[must_use]
    pub fn client_secret(&self) -> &str;
}

impl core::fmt::Debug for MachineIdentity {
    /// `MachineIdentity { client_id: "…", client_secret: "<redacted>" }`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MachineIdentity")
            .field("client_id", &self.client_id.as_str())
            .field("client_secret", &"<redacted>")
            .finish()
    }
}
```

`new` wraps with `Zeroizing::new(client_id.into())`, which moves a `String` without copying.

#### B.1.5 `ProviderHealth` (D3)

```rust
/// What [`SecretProvider::health`] reports. No token and no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealth {
    /// The normalised base URL the provider talks to.
    pub base_url: String,
    /// Whether the server's unauthenticated status endpoint answered 2xx.
    pub server_ok: bool,
}
```

#### B.1.6 `SecretError` (D4) — every variant, fields and exact `Display`

**Not `#[non_exhaustive]`** (H-3): the leak tests in both crates match it exhaustively.

```rust
/// Why a secret-provider call failed (D4). **No variant ever carries a secret value, the client
/// secret or the access token.** `endpoint` is a path (`/api/v4/secrets`), never a URL with query
/// values. Login failures never quote the server's body; other endpoints may quote the server's
/// `message`, cleaned and cut to 200 characters. Key names are not secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The keyring holds no machine identity. Built by the caller that reads the keyring (M3).
    #[error("no Infisical machine identity is stored in the OS keyring")]
    NoIdentity,
    /// A bad base URL, a blank identity half, a client that would not build, or a scope that
    /// does not parse.
    #[error("secret provider configuration: {0}")]
    Config(String),
    /// The request did not complete: DNS, connect, TLS, timeout, or a body cut short. `cause` is
    /// the `source()` chain of `reqwest::Error::without_url()`, joined with `": "`.
    #[error("cannot reach Infisical at {endpoint}: {cause}")]
    Unreachable {
        /// The endpoint path.
        endpoint: &'static str,
        /// The error chain, no URL.
        cause: String,
    },
    /// Login 401 without the lockout text: wrong, expired or exhausted credentials. Latches (D5).
    #[error(
        "Infisical refused the machine identity's login: the client ID or client secret is \
         wrong, expired or used up"
    )]
    BadCredentials,
    /// Login 401 with the lockout text. Latches (D5).
    #[error(
        "Infisical has temporarily locked the machine identity after repeated failed logins; \
         wait for the lockout to end before trying again"
    )]
    IdentityLocked,
    /// An earlier login by this provider was refused; no request was made (D5).
    #[error(
        "an earlier login with this machine identity was refused; no new login is tried until \
         the identity is entered again"
    )]
    LoginRefusedEarlier,
    /// The list endpoint answered 404 `NotFound`.
    #[error("Infisical has no project with the configured project ID")]
    ProjectNotFound,
    /// The list endpoint answered 404 `SecretPathNotFound`. Both fields come from the scope.
    #[error("Infisical has no environment `{environment}` or folder `{path}` in the project")]
    PathNotFound {
        /// The scope's environment slug.
        environment: String,
        /// The scope's folder path.
        path: String,
    },
    /// 403 `PermissionDenied`, `ProjectMembershipNotFound` or any other non-token 403, or a
    /// merged entry with `secretValueHidden: true`.
    #[error("the machine identity may not read these secrets: {detail}")]
    PermissionDenied {
        /// The cleaned server message, or which key is hidden.
        detail: String,
    },
    /// 429 from either endpoint (Infisical Cloud only). Never retried here; M3 decides.
    #[error("Infisical rate-limited the request{}", retry_hint(.retry_after_secs))]
    RateLimited {
        /// `Retry-After` in whole seconds, when the server sent a number.
        retry_after_secs: Option<u64>,
    },
    /// Fastify's "Route … not found" 404: the server predates the endpoint.
    #[error("this Infisical predates v0.150 ({endpoint} does not exist); upgrade it")]
    UnsupportedServer {
        /// The missing endpoint path.
        endpoint: &'static str,
    },
    /// A merged key that is not `^[A-Za-z_][A-Za-z0-9_]*$`. Printed with `{:?}` because the
    /// name may hold control characters.
    #[error("the secret name {key:?} is not a valid environment variable name; rename it in Infisical")]
    InvalidKey {
        /// The key as received.
        key: String,
    },
    /// A merged value holding NUL. The key is already a valid name (validation order, §B.4.6).
    #[error(
        "the secret {key} holds a NUL byte, which an environment variable cannot carry; fix its \
         value in Infisical"
    )]
    InvalidValue {
        /// The key.
        key: String,
    },
    /// Anything else: a non-JSON or ill-shaped body, an unexpected status, a redirect, or a
    /// token refused right after a fresh login.
    #[error("unexpected answer from Infisical at {endpoint}: {detail}")]
    Protocol {
        /// The endpoint path.
        endpoint: &'static str,
        /// What was wrong; never a body quote on login, never serde's message (A-5).
        detail: String,
    },
}

/// `"; retry after {n} s"`, or nothing.
fn retry_hint(secs: &Option<u64>) -> String {
    secs.map(|s| format!("; retry after {s} s")).unwrap_or_default()
}
```

`thiserror` passes `.field` arguments by reference, hence `&Option<u64>` (H-11).

Byte-exact rows the T1 table test pins (§D.1):

| Value | `to_string()` |
|---|---|
| `NoIdentity` | `no Infisical machine identity is stored in the OS keyring` |
| `Config("the Infisical base URL is empty".into())` | `secret provider configuration: the Infisical base URL is empty` |
| `Unreachable { endpoint: "/api/status", cause: "error sending request: connection refused".into() }` | `cannot reach Infisical at /api/status: error sending request: connection refused` |
| `BadCredentials` | `Infisical refused the machine identity's login: the client ID or client secret is wrong, expired or used up` |
| `IdentityLocked` | `Infisical has temporarily locked the machine identity after repeated failed logins; wait for the lockout to end before trying again` |
| `LoginRefusedEarlier` | `an earlier login with this machine identity was refused; no new login is tried until the identity is entered again` |
| `ProjectNotFound` | `Infisical has no project with the configured project ID` |
| `PathNotFound { environment: "dev".into(), path: "/app".into() }` | ``Infisical has no environment `dev` or folder `/app` in the project`` |
| `PermissionDenied { detail: "no read on dev".into() }` | `the machine identity may not read these secrets: no read on dev` |
| `RateLimited { retry_after_secs: Some(30) }` | `Infisical rate-limited the request; retry after 30 s` |
| `RateLimited { retry_after_secs: None }` | `Infisical rate-limited the request` |
| `UnsupportedServer { endpoint: "/api/v4/secrets" }` | `this Infisical predates v0.150 (/api/v4/secrets does not exist); upgrade it` |
| `InvalidKey { key: "1BAD".into() }` | `the secret name "1BAD" is not a valid environment variable name; rename it in Infisical` |
| `InvalidValue { key: "TOKEN".into() }` | `the secret TOKEN holds a NUL byte, which an environment variable cannot carry; fix its value in Infisical` |
| `Protocol { endpoint: "/api/v4/secrets", detail: "status 500".into() }` | `unexpected answer from Infisical at /api/v4/secrets: status 500` |

### B.2 T2 — `htui-store::secret` keyring slots (D7)

Constants, after `QDRANT_KEY_USER` (`:36`):

```rust
/// Keyring user name of the Infisical base URL (MOD-10 D7).
pub const INFISICAL_URL_USER: &str = "infisical-url";
/// Keyring user name of the Infisical machine identity's client ID (MOD-10 D7, ANA-7 §3.3).
pub const INFISICAL_CLIENT_ID_USER: &str = "infisical-client-id";
/// Keyring user name of the Infisical machine identity's client secret (MOD-10 D7, ANA-7 §3.3).
pub const INFISICAL_CLIENT_SECRET_USER: &str = "infisical-client-secret";
```

`FakeSlots` (`:44-50`) gains four fields. It stays `Debug, Clone, Default`, and only `default()`
builds it (`testkit.rs:337`):

```rust
    pub infisical_url: Option<String>,
    pub infisical_client_id: Option<String>,
    pub infisical_client_secret: Option<String>,
    /// MOD-10 T2: the user name whose **store** fails, as `Fake::Broken` would fail it. Every
    /// other call still answers. Honoured by the Infisical slots only. `Fake::Broken` cannot test
    /// "the second write fails" because it fails the first one too.
    pub refuse_store: Option<&'static str>,
```

Private helpers (new, after `:358`). They follow the Qdrant functions' shape but are keyed by
user name, so the six public functions stay one line each:

```rust
/// The fake field for one of the Infisical users.
#[cfg(feature = "test-support")]
fn infisical_field<'a>(slots: &'a mut FakeSlots, user: &str) -> &'a mut Option<String> {
    match user {
        INFISICAL_URL_USER => &mut slots.infisical_url,
        INFISICAL_CLIENT_ID_USER => &mut slots.infisical_client_id,
        INFISICAL_CLIENT_SECRET_USER => &mut slots.infisical_client_secret,
        other => unreachable!("not an Infisical keyring user: {other}"),
    }
}

/// Reads one Infisical entry; blank reads as `None` (as `Slot::get`, `secret.rs:383-390`).
fn read_slot(user: &'static str) -> Result<Option<String>>;
/// Writes one Infisical entry. Under the fake, `refuse_store == Some(user)` fails with
/// `fake_failure("store", user, "refused by the test")`.
fn write_slot(user: &'static str, value: &str) -> Result<()>;
/// Removes one Infisical entry; a missing entry is `Ok(())`.
fn remove_slot(user: &'static str) -> Result<()>;
```

Each helper takes `fake()` in its own statement and releases it before returning (H-15). The fake
branches return `fake_failure("read" | "store" | "remove", user, why)` for `Fake::Broken`, exactly
as the Qdrant branches do (`:281-283`).

Public functions:

```rust
/// The stored Infisical base URL, if any (D7; the Qdrant URL's shape).
///
/// # Errors
/// [`StoreError::Backend`] for any keyring failure other than a missing entry.
pub fn get_infisical_url() -> Result<Option<String>>;
/// Stores the Infisical base URL. Not validated here: `htui-secrets` normalises it, and M4 calls
/// that before storing.
///
/// # Errors
/// [`StoreError::Backend`] when the keyring refuses the write.
pub fn set_infisical_url(url: &str) -> Result<()>;
/// Removes the Infisical base URL. A missing entry is `Ok(())`.
///
/// # Errors
/// [`StoreError::Backend`] when the keyring refuses the delete.
pub fn clear_infisical_url() -> Result<()>;

/// The stored machine identity (D7): both halves → `Some`, neither → `None`.
///
/// # Errors
/// [`StoreError::Backend`] for a keyring failure, **and** for a half-stored identity, naming the
/// missing half. A half identity is not "no identity". The message never carries a value.
pub fn get_machine_identity() -> Result<Option<MachineIdentity>>;
/// Writes both halves: client ID first, then client secret. If the secret write fails, both
/// entries are removed (A-1), and the write error is returned.
///
/// # Errors
/// [`StoreError::Backend`] when either write fails.
pub fn set_machine_identity(identity: &MachineIdentity) -> Result<()>;
/// Removes both halves. Both removals are attempted; the first error is returned.
///
/// # Errors
/// [`StoreError::Backend`] when either removal fails.
pub fn clear_machine_identity() -> Result<()>;
```

`get_machine_identity` algorithm:
1. `let id = read_slot(INFISICAL_CLIENT_ID_USER)?;`
2. `let secret = read_slot(INFISICAL_CLIENT_SECRET_USER)?.map(Zeroizing::new);` — wrapped at once.
3. Then:
   - `(Some(id), Some(secret))` → `Ok(Some(MachineIdentity::new(id, std::mem::take(&mut *secret))))`. `take` moves the allocation, with no copy; the emptied `Zeroizing` drops harmlessly.
   - `(None, None)` → `Ok(None)`.
   - `(Some(_), None)` → `Err(StoreError::Backend(format!("the Infisical machine identity is half stored: {SERVICE}/{INFISICAL_CLIENT_SECRET_USER} is missing; enter the identity again")))`.
   - `(None, Some(_))` → the same sentence naming `{SERVICE}/{INFISICAL_CLIENT_ID_USER}`.

`set_machine_identity` algorithm:
1. `write_slot(INFISICAL_CLIENT_ID_USER, identity.client_id())?;`
2. `if let Err(err) = write_slot(INFISICAL_CLIENT_SECRET_USER, identity.client_secret()) { let _ = remove_slot(INFISICAL_CLIENT_ID_USER); let _ = remove_slot(INFISICAL_CLIENT_SECRET_USER); return Err(err); }`
3. `Ok(())`.

Module doc addition (after `:20`): one paragraph saying the module also holds the Infisical base URL
and machine identity (`infisical-url`, `infisical-client-id`, `infisical-client-secret` under
`SERVICE`, MOD-10 D7). A half-stored identity is an error, not `None`. The fake honours
`refuse_store` for these slots only.

`crates/htui-store/src/testkit.rs`, after `fake_dsn`:

```rust
/// What the fake keyring holds for the Infisical identity, `(client_id, client_secret)`, for
/// assertions.
///
/// # Panics
///
/// Outside a [`mock_keyring`] guard, and under a [`mock_keyring_broken`] one (as
/// [`fake_qdrant_dsn`]).
pub fn fake_machine_identity() -> (Option<String>, Option<String>);

/// Makes the installed fake refuse every **store** to `user` (one of the `INFISICAL_*_USER`
/// constants) until the guard drops. Used to prove `set_machine_identity` leaves no half.
///
/// # Panics
///
/// Outside a [`mock_keyring`] guard, and under a [`mock_keyring_broken`] one.
pub fn refuse_fake_store(user: &'static str);
```

### B.3 T3 — `htui-secrets` crate API (D1, D5, D6, D8, D9)

#### B.3.1 `src/lib.rs`

```rust
//! `htui-secrets`: htui's secret providers (MOD-10 M2). Today one: [`InfisicalProvider`], an
//! in-process Infisical client behind `htui_core::secret::SecretProvider`.
//!
//! It never reads the keyring. The caller reads the `MachineIdentity` and base URL from
//! `htui-store::secret` and hands them over (M3 composes the two in the process that runs the run).
//! Share **one** provider per process and identity: the login latch (D5) is per instance.
#![warn(missing_docs)]

mod infisical;
mod wire;

pub use infisical::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_TIMEOUT, InfisicalConfig, InfisicalProvider,
    normalise_base_url,
};
```

#### B.3.2 Public items in `src/infisical.rs`

```rust
use std::collections::BTreeMap;
use std::sync::Once;
use std::time::{Duration, Instant};

use htui_core::secret::{
    MachineIdentity, ProviderHealth, ResolvedSecrets, SecretError, SecretFuture, SecretProvider,
    SecretScope,
};
use tokio::sync::Mutex;
use url::{Host, Position, Url};
use zeroize::Zeroizing;

/// D9: the connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// D9: the whole-request timeout (`install/http.rs`'s `short` client shape).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// How to reach an Infisical (D8, D9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfisicalConfig {
    /// The base URL as entered; [`InfisicalProvider::new`] normalises it.
    pub base_url: String,
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl InfisicalConfig {
    /// `base_url` with the D9 timeouts.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self;
}

/// The Infisical client (D5, D6, D9). `Debug` is hand-written: base URL and client ID only.
pub struct InfisicalProvider {
    /// Normalised: origin plus optional prefix, no trailing `/`.
    base: String,
    identity: MachineIdentity,
    client: reqwest::Client,
    /// D5: held across the login request (single flight).
    state: Mutex<TokenState>,
}

impl InfisicalProvider {
    /// `project.secret_provider`'s value for this provider.
    pub const KIND: &'static str = "infisical";

    /// Builds the provider. Opens no socket.
    ///
    /// # Errors
    ///
    /// [`SecretError::Config`] when the base URL is refused (§B.4.1), a half of the identity is
    /// blank (`"the machine identity's client ID is empty"` /
    /// `"the machine identity's client secret is empty"`, checked with `trim().is_empty()`), or
    /// the HTTP client does not build (`"cannot build the HTTP client: {cause}"`, cause chain as
    /// in `Unreachable`).
    pub fn new(config: InfisicalConfig, identity: MachineIdentity) -> Result<Self, SecretError>;

    /// The normalised base URL.
    #[must_use]
    pub fn base_url(&self) -> &str;
}

impl core::fmt::Debug for InfisicalProvider {
    /// `InfisicalProvider { base_url: "…", client_id: "…", .. }`. Never the secret, never the
    /// token, and no lock taken.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InfisicalProvider")
            .field("base_url", &self.base)
            .field("client_id", &self.identity.client_id())
            .finish_non_exhaustive()
    }
}

impl SecretProvider for InfisicalProvider {
    fn kind(&self) -> &'static str { Self::KIND }
    fn health(&self) -> SecretFuture<'_, ProviderHealth> { Box::pin(self.health_inner()) }
    fn list_keys<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>> {
        Box::pin(async move { Ok(self.resolve_inner(scope).await?.keys()) })
    }
    fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets> {
        Box::pin(self.resolve_inner(scope))
    }
}

/// Normalises an Infisical base URL (D8): origin plus optional path prefix, no trailing `/`, a
/// trailing `/api` dropped. M4 calls it before storing a URL.
///
/// # Errors
///
/// [`SecretError::Config`], naming the reason (§B.4.1). Never echoes the raw input.
pub fn normalise_base_url(raw: &str) -> Result<String, SecretError>;
```

Private items in `infisical.rs`:

```rust
const USER_AGENT: &str = concat!("htui/", env!("CARGO_PKG_VERSION"));
const LOGIN_PATH: &str = "/api/v1/auth/universal-auth/login";
const SECRETS_PATH: &str = "/api/v4/secrets";
const STATUS_PATH: &str = "/api/status";
static PROVIDER: Once = Once::new();

/// `ring`, once; "already installed" ignored (`install/http.rs:51-55`, `model.rs:624-628`).
fn install_crypto_provider();

/// `(normalised, is_loopback)`; `normalise_base_url` returns `.0`.
fn parse_base(raw: &str) -> Result<(String, bool), SecretError>;
fn is_loopback(host: &Host<&str>) -> bool;           // Domain("localhost") | Ipv4 127/8 | Ipv6 ::1

/// D5 token state. **No `Debug`** (it holds the token; private, so the lint does not ask).
enum TokenState {
    /// No token yet, or the last one was dropped.
    Empty,
    /// A token, reused while `Instant::now() < reuse_until`.
    Valid { token: Zeroizing<String>, reuse_until: Instant },
    /// A login was refused: every later call is `LoginRefusedEarlier`, with no request.
    Refused,
}

/// `expires_in - max(60, expires_in / 10)`, saturating at zero.
fn reuse_window(expires_in_secs: u64) -> Duration;
/// Lock held across the login (single flight). §B.4.3.
async fn token(&self) -> Result<Zeroizing<String>, SecretError>;
/// After a data 401 / 403 `TokenError` with `used`: one more login, unless another caller
/// already replaced the token. §B.4.3.
async fn refresh(&self, used: &str) -> Result<Zeroizing<String>, SecretError>;
/// Health always logs in afresh (D2). §B.4.3.
async fn fresh_login(&self) -> Result<Zeroizing<String>, SecretError>;
/// The POST itself plus §B.4.2's login mapping; the caller holds the lock and updates the state.
async fn login(&self) -> Result<(Zeroizing<String>, Instant), LoginFailure>;
/// `LoginFailure::Refused(SecretError)` latches; `LoginFailure::Other(SecretError)` does not.
enum LoginFailure { Refused(SecretError), Other(SecretError) }

async fn health_inner(&self) -> Result<ProviderHealth, SecretError>;
async fn resolve_inner(&self, scope: &SecretScope) -> Result<ResolvedSecrets, SecretError>;
/// One GET of the list endpoint with `token`; `Ok(None)` means "token refused" (401, or 403
/// `TokenError`).
async fn list(&self, scope: &SecretScope, token: &str) -> Result<Option<ListResponse>, SecretError>;
fn list_url(&self, scope: &SecretScope) -> Result<Url, SecretError>;   // §B.4.4
fn endpoint_url(&self, path: &'static str) -> Result<Url, SecretError>; // Url::parse(base + path); Err → Config

/// The cause chain of `err.without_url()`: top-level `Display` then every `source()`, joined
/// with `": "`.
fn cause_chain(err: reqwest::Error) -> String;
fn unreachable(endpoint: &'static str, err: reqwest::Error) -> SecretError;
/// The body as `T`; on failure `Protocol` with A-5's category-and-position detail only.
fn decode<T: serde::de::DeserializeOwned>(endpoint: &'static str, body: &[u8]) -> Result<T, SecretError>;
/// The server `message`: `None`/non-string → `""`; control characters removed; first 200
/// `char`s (never a byte slice).
fn clean_message(body: &ErrorBody) -> String;
fn is_fastify_not_found(body: &ErrorBody) -> bool;   // message starts "Route " and ends " not found"
fn is_lockout(body: &ErrorBody) -> bool;             // message, lowercased, contains "temporarily locked"
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64>; // RETRY_AFTER, trimmed, parse::<u64>().ok()
fn is_env_name(key: &str) -> bool;                   // bytes: first [A-Za-z_], rest [A-Za-z0-9_], non-empty

/// D6 merge (§B.4.5). Values are moved into `Zeroizing` as they are read, so shadowed entries
/// drop wiped.
pub(crate) fn merge(list: ListResponse) -> BTreeMap<String, Merged>;
pub(crate) struct Merged { pub(crate) value: Zeroizing<String>, pub(crate) hidden: bool }
/// D6 validation (§B.4.6): three passes, then the values are moved out with `std::mem::take`.
pub(crate) fn validate(merged: BTreeMap<String, Merged>) -> Result<ResolvedSecrets, SecretError>;
```

The client (D9), in `new`:

```rust
install_crypto_provider();
let mut builder = reqwest::Client::builder()
    .user_agent(USER_AGENT)
    .connect_timeout(config.connect_timeout)
    .timeout(config.timeout)
    .redirect(reqwest::redirect::Policy::none());
if loopback {
    builder = builder.no_proxy();          // D9: system-proxy would route 127.0.0.1 via HTTP_PROXY
}
let client = builder.build().map_err(|e| SecretError::Config(format!("cannot build the HTTP client: {}", cause_chain(e))))?;
```

#### B.3.3 Request shapes

- **Login**: `POST {base}/api/v1/auth/universal-auth/login`, `.json(&LoginRequest { client_id, client_secret })`.
  `.json` sets `Content-Type: application/json` and needs reqwest's `json` feature (§B.5).
- **List**: `GET {list_url}` with `.bearer_auth(token)`.
- **Status**: `GET {base}/api/status`, no auth, body ignored.
- Bodies are always read with `.bytes().await` (`Err` → `Unreachable`, except that a 401 is decided on its status first; see the login and data tables), then `decode`/`ErrorBody::from_bytes`. `Response::json` is **never** used (A-5).

#### B.3.4 `src/wire.rs` — serde shapes (all `pub(crate)`, **no `Debug` on any of them**, H-2)

```rust
use serde::{Deserialize, Serialize};

/// `POST /api/v1/auth/universal-auth/login` body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginRequest<'a> {
    pub(crate) client_id: &'a str,        // "clientId"
    pub(crate) client_secret: &'a str,    // "clientSecret"
}

/// Its 200 answer. Only the read fields are declared (H-10): serde ignores `accessTokenMaxTTL`,
/// `tokenType` and anything new. `zeroize` 1.9 has no `serde` feature, so the token is a `String`
/// moved into `Zeroizing` right after decoding.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginResponse {
    pub(crate) access_token: String,      // "accessToken"
    pub(crate) expires_in: u64,           // "expiresIn", seconds
}

/// `GET /api/v4/secrets` 200 answer.
#[derive(Deserialize)]
pub(crate) struct ListResponse {
    pub(crate) secrets: Vec<WireSecret>,
    #[serde(default)]
    pub(crate) imports: Option<Vec<WireImport>>,  // absent or null → no imports
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSecret {
    pub(crate) secret_key: String,                        // "secretKey"
    #[serde(default)]
    pub(crate) secret_value: String,                      // "secretValue"
    #[serde(default)]
    pub(crate) secret_value_hidden: bool,                 // "secretValueHidden"
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,                      // "shared" | "personal"; absent → shared
}

/// One import. `secretPath` and `environment` are not read, so not declared (H-10).
#[derive(Deserialize)]
pub(crate) struct WireImport {
    pub(crate) secrets: Vec<WireSecret>,
}

/// `{reqId, statusCode, message, error}`. Only `message` and `error` are read. `message` is a
/// `Value` because validation errors send an array.
#[derive(Deserialize, Default)]
pub(crate) struct ErrorBody {
    #[serde(default)]
    pub(crate) message: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

impl ErrorBody {
    /// Parsed leniently: a body that is not this JSON is `ErrorBody::default()`.
    pub(crate) fn from_bytes(body: &[u8]) -> Self;
    /// `message` when it is a JSON string.
    pub(crate) fn message(&self) -> &str;
    /// `error`, or `""`.
    pub(crate) fn error(&self) -> &str;
}
```

### B.4 T3 — behaviour, as tables

#### B.4.1 Base URL policy (D8) — `normalise_base_url` / `parse_base`

Algorithm:
1. `let raw = raw.trim();` If empty → `Config("the Infisical base URL is empty")`.
2. `Url::parse(raw)`; on `Err(e)` → `Config(format!("the Infisical base URL is not an absolute URL ({e})"))`. `url::ParseError`'s `Display` does not echo the input.
3. Scheme must be `https` or `http`, else `Config(format!("the Infisical base URL must use https (got {scheme})"))`.
4. Non-empty `username()` or any `password()` → `Config("the Infisical base URL must not carry a user name or password")`.
5. `query()` or `fragment()` present → `Config("the Infisical base URL must not carry a query or a fragment")`.
6. `loopback = url.host().is_some_and(|h| is_loopback(&h))`. If the scheme is `http` and not loopback → `Config(format!("the Infisical base URL must use https unless its host is loopback: the client secret would cross the network in plain text to {host}"))`, where `host = url.host_str()`.
7. Prefix: `url.path()`, `trim_end_matches('/')`, then `strip_suffix("/api")` if present, then `trim_end_matches('/')` again.
8. Result: `format!("{}{}", &url[..Position::BeforePath], prefix)`, with `BeforePath` = scheme, host and port (userinfo is already refused).

| Input | Result |
|---|---|
| `https://app.infisical.com` | `https://app.infisical.com` |
| `https://app.infisical.com/` | `https://app.infisical.com` |
| `https://app.infisical.com/api` | `https://app.infisical.com` |
| `https://app.infisical.com/api/` | `https://app.infisical.com` |
| `  https://eu.infisical.com  ` | `https://eu.infisical.com` |
| `HTTPS://Infisical.Example.COM` | `https://infisical.example.com` (the parser lowercases) |
| `https://example.com:8443` | `https://example.com:8443` |
| `https://example.com/infisical/api/` | `https://example.com/infisical` |
| `https://example.com/api/v4` | `https://example.com/api/v4` (a prefix per D8; the server then 404s → `UnsupportedServer`; the docs page warns: give the root) |
| `http://localhost:8080` | `http://localhost:8080` (loopback) |
| `http://127.0.0.1:1234/api` | `http://127.0.0.1:1234` (loopback) |
| `http://127.8.9.10` | `http://127.8.9.10` (127/8 is loopback) |
| `http://[::1]:8080` | `http://[::1]:8080` (loopback) |
| `""`, `"   "` | `Config` empty |
| `app.infisical.com` | `Config` not an absolute URL |
| `ftp://example.com` | `Config` must use https (got ftp) |
| `https://user:pw@example.com` | `Config` user name or password |
| `https://example.com/?x=1`, `https://example.com/#f` | `Config` query or fragment |
| `http://192.168.1.10` | `Config` plain text to `192.168.1.10` |
| `http://infisical.lan` | `Config` plain text to `infisical.lan` |
| `http://localhost.` / `http://0.0.0.0` | `Config` (neither is loopback by this rule) |

#### B.4.2 HTTP status/body → `SecretError`

**Login** (`POST /api/v1/auth/universal-auth/login`). The body is **never** quoted.

| Answer | Result | Latches? |
|---|---|---|
| network error from `send()`, or from `bytes()` on a non-401 status | `Unreachable { endpoint: LOGIN_PATH, cause }` | no |
| 401 whose body cannot be read (cut short, timed out) — decided on status first (D5; amended after T3's verify, `d8f4a7ca`) | `BadCredentials` (`IdentityLocked` only from a fully read body carrying the lockout text) | **yes** |
| 2xx, `LoginResponse` decodes, `access_token` non-empty | token; `reuse_until = now + reuse_window(expires_in)` | — |
| 2xx, decode fails | `Protocol { LOGIN_PATH, A-5 detail }` | no |
| 2xx, empty `accessToken` | `Protocol { LOGIN_PATH, "the login answer carried no access token" }` | no |
| 3xx (any) | `Protocol { LOGIN_PATH, format!("the server answered a redirect ({code}); redirects are not followed") }` | no |
| 401, `is_lockout` | `IdentityLocked` | **yes** |
| 401, otherwise (incl. non-JSON body) | `BadCredentials` | **yes** |
| 404, `is_fastify_not_found` | `UnsupportedServer { endpoint: LOGIN_PATH }` (A-7) | no |
| 429 | `RateLimited { retry_after_secs: retry_after(headers) }` | no |
| any other status | `Protocol { LOGIN_PATH, format!("status {code}") }` | no |

**List** (`GET /api/v4/secrets`). The cleaned `message` (`m`) may be quoted.

| Answer | Result |
|---|---|
| network error from `send()`, or from `bytes()` on a non-401 status | `Unreachable { endpoint: SECRETS_PATH, cause }` |
| 401, decided before the body is read (amended after T3's verify, `d8f4a7ca`) | token refused → the one refresh (the 401 row above), even when the body cannot be read |
| 2xx, decodes | merge (§B.4.5) → validate (§B.4.6) |
| 2xx, decode fails | `Protocol { SECRETS_PATH, A-5 detail }` |
| 3xx | `Protocol { SECRETS_PATH, "the server answered a redirect ({code}); redirects are not followed" }` |
| 401 | token refused → `refresh` once (§B.4.3) |
| 403, `error == "TokenError"` | token refused → `refresh` once |
| 403, `error ∈ {"PermissionDenied", "ProjectMembershipNotFound"}` or any other 403 | `PermissionDenied { detail: m }`, or `format!("status 403 {error}")` when `m` is empty |
| token refused again after the one refresh | `Protocol { SECRETS_PATH, "the access token was refused right after a fresh login" }` |
| 404, `is_fastify_not_found` (checked **first**) | `UnsupportedServer { endpoint: SECRETS_PATH }` |
| 404, `error == "NotFound"` | `ProjectNotFound` |
| 404, `error == "SecretPathNotFound"` | `PathNotFound { environment: scope.environment(), path: scope.path() }` |
| 404, other | `Protocol { SECRETS_PATH, format!("status 404: {m}") }` |
| 429 | `RateLimited { retry_after_secs }` |
| any other status | `Protocol { SECRETS_PATH, format!("status {code}: {m}") }` (`format!("status {code}")` when `m` is empty) |

**Status** (`GET /api/status`, health only).

| Answer | Result |
|---|---|
| network error | `Unreachable { endpoint: STATUS_PATH, cause }` |
| 2xx | `server_ok = true` |
| 3xx | `Protocol { STATUS_PATH, redirect sentence }` |
| other | `server_ok = false`, and the fresh login still runs |

Fastify's default 404 body is `{"message":"Route GET:/api/v4/secrets?… not found","error":"Not Found","statusCode":404}`.
Detection uses only `message.starts_with("Route ") && message.ends_with(" not found")`. Infisical's
own `NotFound` has a different `message` and `error == "NotFound"`, which is why the Fastify check
runs first.

#### B.4.3 Token reuse and the single-flight latch (D5)

- **Margin:** `reuse_window(e) = e.saturating_sub(max(60, e / 10))` seconds. Examples:
  2 592 000 → 2 332 800; 600 → 540; 300 → 240; 60 → 0; 30 → 0. A zero window means the token
  serves the call that fetched it and is never reused.
- `Instant` is `std::time::Instant`. Expiry tests use a short `expiresIn` (30), never a sleep.
- The `tokio::sync::Mutex<TokenState>` is held **across the login request** and released before
  any data request. A `std::sync::Mutex` guard is not `Send` across the `.await` and would not
  compile inside `SecretFuture` (H-6).

| Entry | State on lock | Action | New state | Returns |
|---|---|---|---|---|
| `token()` | `Refused` | none (no request) | `Refused` | `Err(LoginRefusedEarlier)` |
| `token()` | `CoolingDown{until}`, `now < until` | none (no request) | same | `Err(LoginCoolingDown { retry_after_secs })`, the seconds left rounded up |
| `token()` | `Valid`, `now < reuse_until` | none | same | a `Zeroizing` clone of the token |
| `token()` | `Empty`, `Valid` expired, or `CoolingDown` over | `login()` on a spawned task that owns the lock guard (R1 #1) | ok → `Valid{…}` | the token |
| | | | `LoginFailure::Refused(e)` → `Refused` | `Err(e)` (`BadCredentials`/`IdentityLocked`) |
| | | | `LoginFailure::Other(e)` → `Empty` (nothing sent: connect error; or a non-refusal answer) | `Err(e)` |
| | | | `LoginFailure::Unanswered(e)` → `CoolingDown { until: now + login_cool_down }` (30 s default; any send error but a connect error) | `Err(e)` (`Unreachable`) |
| `refresh(used)` | `Refused` / `CoolingDown` running | none | same | as `token()` |
| `refresh(used)` | `Valid{token}`, `token != used`, not expired | none (another caller refreshed) | same | that token |
| `refresh(used)` | otherwise | `login()` as `token()` | as `token()` | as `token()` |
| `fresh_login()` (health) | `Refused` / `CoolingDown` running | none | same | as `token()` |
| `fresh_login()` | anything else | `login()` as `token()` | as `token()` | as `token()` |

*(R1 #1, amended.)* The login task records its outcome before it releases the guard, so a caller
dropped mid-login (a timeout, a `select!`) loses nothing and the callers queued on the lock see
that outcome: still one login. `health_inner` peeks `Refused` and a running `CoolingDown` before
the status request.

`resolve_inner(scope)`:
1. `let token = self.token().await?;`
2. `match self.list(scope, &token).await? { Some(list) => list, None => { let token = self.refresh(&token).await?; self.list(scope, &token).await?.ok_or(Protocol{SECRETS_PATH, "the access token was refused right after a fresh login"})? } }`
3. `validate(merge(list))`.

`health_inner()`:
1. If the state is `Refused` → `Err(LoginRefusedEarlier)` **before** the status request (lock, peek, unlock).
2. GET status (§B.4.2) → `server_ok`.
3. `self.fresh_login().await?` (D2: health performs the login; the new token replaces any cached one).
4. `Ok(ProviderHealth { base_url: self.base.clone(), server_ok })`.

There is no other retry: 429 and network errors return at once (D5).

#### B.4.4 List query (D6)

Built on `endpoint_url(SECRETS_PATH)` with `url.query_pairs_mut()` (the workspace reqwest has no
`query` feature), in this order:

| # | Key | Value |
|---|---|---|
| 1 | `projectId` | `scope.project_id()` |
| 2 | `environment` | `scope.environment()` |
| 3 | `secretPath` | `scope.path()` |
| 4 | `viewSecretValue` | `true` |
| 5 | `expandSecretReferences` | `true` |
| 6 | `includeImports` | `true` (v0.159+) |
| 7 | `include_imports` | `true` (v0.150–v0.158) |
| 8 | `recursive` | `false` |
| 9 | `includePersonalOverrides` | `false` |

`query_pairs_mut` percent-encodes `/` in `secretPath` as `%2F`. The stub decodes it, so tests
compare decoded values.

#### B.4.5 Merge (D6)

```text
out = BTreeMap::new()
for s in list.secrets            where kind is None or "shared":  out.entry(s.key).or_insert(Merged{value: Zeroizing::new(s.value), hidden})
for imp in list.imports.rev()    (last import first):
    for s in imp.secrets         where kind is None or "shared":  out.entry(s.key).or_insert(...)
```

A key already present is kept. The folder therefore beats every import, and among imports the
bottom-most wins (Infisical CLI `InjectRawImportedSecret`). A duplicate key within one list keeps
its first occurrence. `personal` entries are skipped whole. Each value is wrapped in `Zeroizing`
**before** the `entry` call, so a shadowed value drops wiped.

Worked example (the `merge_worked_example` unit test, and stub test inputs):

| Source | Entries |
|---|---|
| folder `secrets` | `A=f1`, `B=f2`, `P=p1 (personal)` |
| `imports[0]` | `B=i0b`, `C=i0c`, `D=i0d` |
| `imports[1]` | `C=i1c`, `E=e1 (personal)`, `D=i1d` |

| Key | Value | Why |
|---|---|---|
| `A` | `f1` | folder only |
| `B` | `f2` | folder beats `imports[0]` |
| `C` | `i1c` | `imports[1]` (last) beats `imports[0]` |
| `D` | `i1d` | last import wins |
| `E` | — | personal only, ignored |
| `P` | — | personal, ignored |

#### B.4.6 Validation order (D6) — three passes over the merged map, in key order

1. **Hidden**: the first key with `hidden == true` → `PermissionDenied { detail: format!("the value of {key:?} is hidden from this identity") }`.
2. **Name**: the first key failing `is_env_name` → `InvalidKey { key }`.
3. **NUL**: the first value containing `'\0'` → `InvalidValue { key }`.

So a map with a hidden `B` and a bad `1A` reports `PermissionDenied` (pass 1), whatever the key
order. Then `ResolvedSecrets::new(merged.into_iter().map(|(k, mut m)| (k, std::mem::take(&mut *m.value))).collect())`.
Values pass through byte for byte: no trim and no newline normalisation (M3's job).

### B.5 Cargo manifests (T1, T3)

`crates/htui-core/Cargo.toml`, after `regex`:

```toml
# MOD-10 D3: `MachineIdentity` holds both halves in `Zeroizing`, and `ResolvedSecrets` wipes its
# values on drop. Already locked (1.9.0, workspace `Cargo.toml:78`); adds no compiled crate.
zeroize    = { workspace = true }
```

`Cargo.toml` (workspace):

```toml
members  = ["crates/htui-core", "crates/htui", "crates/htui-store", "crates/htui-agent",
            "crates/htui-orch", "crates/htui-worker", "crates/htui-mcp", "crates/htui-secrets"]
...
htui-mcp           = { path = "crates/htui-mcp" }
htui-secrets       = { path = "crates/htui-secrets" }
```

`crates/htui-secrets/Cargo.toml` (`htui-mcp/Cargo.toml` shape):

```toml
[package]
name        = "htui-secrets"
version     = "0.1.0"
description = "htui's secret providers: the Infisical client behind htui-core's SecretProvider (MOD-10)"

edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
publish.workspace      = true

[dependencies]
# MOD-10 D1: the seam (`SecretProvider`, `SecretScope`, `ResolvedSecrets`, `MachineIdentity`,
# `SecretError`) is htui-core's. Never htui-store: the caller reads the keyring and hands the
# provider a `MachineIdentity` (M3).
htui-core  = { workspace = true }
# D9: the workspace reqwest has no `json` feature. Workspace-wide builds get it only through
# `sentry`, so `-p htui-secrets` must name it. `Cargo.lock` does not move.
reqwest    = { workspace = true, features = ["json"] }
# D9: named only so the `ring` provider can be installed; with `rustls-no-provider` and no
# provider, `Client::build` panics (as `htui-agent`'s `install/http.rs` explains).
rustls     = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }
thiserror  = { workspace = true }
zeroize    = { workspace = true }
# D9: the base URL's host and the list query (`query_pairs_mut`); the workspace reqwest has no
# `query` feature. 2.5.8, the same crate `reqwest::Url` is.
url        = { workspace = true }
# D5: `tokio::sync::Mutex`, held across the login. `sync` is in the workspace set; named for intent.
tokio      = { workspace = true, features = ["sync"] }

[dev-dependencies]
# `#[tokio::test]`: `macros` and `rt-multi-thread` come from the workspace set. Nothing else:
# a dev-dependency that pulls `sentry` (aws-lc) would hide a missing `ring` install (H-1).
tokio      = { workspace = true }

[lints]
workspace = true
```

Facts: `url` is a workspace dependency (`Cargo.toml:172`, `"2.5"`, locked 2.5.8 at
`Cargo.lock:7116`, one version only), and `htui-agent` declares it as `url = { workspace = true }`
(`crates/htui-agent/Cargo.toml:38`). The workspace `tokio` already has `sync`, `macros` and
`rt-multi-thread` (`Cargo.toml:60-61`).

### B.6 T3 — the loopback stub (`crates/htui-secrets/tests/support/mod.rs`)

It extends `model.rs`'s `Stub` (`crates/htui-store/src/model.rs:655-738`: std thread, `TcpListener`
on `127.0.0.1:0`, `Connection: close`, requests recorded). It adds methods, query, headers, body and
per-route sequences, with no new dev-dependency (`url` and `serde_json` are regular dependencies,
so integration tests can use them).

```rust
//! A loopback stand-in for Infisical (MOD-10 M2): HTTP/1.1 on a std thread, scripted answers per
//! `(method, path)`, every request recorded **before** it is answered.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// One scripted answer.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    /// `status` with a JSON body and `Content-Type: application/json`.
    pub fn json(status: u16, body: &serde_json::Value) -> Self;
    /// `status` with a raw text body (a non-JSON answer).
    pub fn text(status: u16, body: &str) -> Self;
    /// `status` with `Location: location` and no body.
    pub fn redirect(status: u16, location: &str) -> Self;
    /// One extra header (e.g. `Retry-After`).
    #[must_use]
    pub fn header(self, name: &str, value: &str) -> Self;
}

/// One recorded request.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// The path without the query.
    pub path: String,
    /// Decoded query pairs, in order.
    pub query: Vec<(String, String)>,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// A header by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str>;
    /// Every value of a query key.
    pub fn query(&self, key: &str) -> Vec<&str>;
    /// The body as JSON. Panics (naming the path, never the body) if it is not JSON.
    pub fn json(&self) -> serde_json::Value;
}

/// The stub. Each test owns one.
#[derive(Debug)]
pub struct Stub {
    addr: SocketAddr,
    script: Arc<Mutex<HashMap<(String, String), VecDeque<Reply>>>>,
    seen: Arc<Mutex<Vec<Request>>>,
}

impl Stub {
    /// Binds `127.0.0.1:0` (never `localhost`, which may resolve to `::1` first) and serves
    /// connections one at a time on a std thread.
    pub fn start() -> Self;
    /// Appends `reply` to the route's queue. Each request pops the front; the **last** reply of a
    /// queue is sticky (served for every later request).
    pub fn on(&self, method: &str, path: &str, reply: Reply) -> &Self;
    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> String;
    /// Every request so far, in arrival order.
    pub fn requests(&self) -> Vec<Request>;
    /// How many requests hit `(method, path)`.
    pub fn count(&self, method: &str, path: &str) -> usize;
}

/// A loopback base URL nothing listens on (bind port 0, read it, drop the listener), for the
/// `Unreachable` cases.
pub fn closed_port_base() -> String;
```

`serve(stream)` algorithm, one request per connection:
1. `let mut reader = BufReader::new(&stream);` Read the request line `METHOD TARGET HTTP/1.1`.
2. Read header lines until the empty line. Split each at the first `:`, lowercase the name, trim the value. Remember `content-length`.
3. `reader.read_exact(&mut body)` for exactly `content-length` bytes, from the **same** `BufReader`, which may already hold body bytes (H-7).
4. Split `TARGET` at the first `?` into the path and `url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()`.
5. **Push the `Request` to `seen`**, then pick the reply: `queue.len() > 1` → `pop_front()`; one left → `front().clone()`; no route → `Reply::json(418, &json!({"message": "stub: unscripted route"}))`. Status 418 is not an Infisical status, so it surfaces as `Protocol "status 418: …"`.
6. Write `HTTP/1.1 {status} X\r\nContent-Length: {n}\r\nConnection: close\r\n{headers}\r\n{body}`, then flush.

**Asserting "no request was made"**: `let before = stub.requests().len(); let err = provider.resolve(&scope).await.unwrap_err(); assert_eq!(stub.requests().len(), before, "a latched provider made a request");`.
This is race-free. A request the client did send would have been recorded before its reply
(step 5), and the awaited call cannot return before reading that reply. `stub.count("POST",
"/api/v1/auth/universal-auth/login") == 1` states "exactly one login".

Sequences make the refresh case scriptable:
```rust
stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on("POST", LOGIN, login_ok(TOKEN_2, 2_592_000));
stub.on("GET", SECRETS, error(403, "TokenError", "Token expired")).on("GET", SECRETS, list_ok(...));
```

Test-file helpers (in `tests/infisical.rs`, not in `support`): `login_ok(token, expires_in) -> Reply`,
`list_ok(secrets: Value, imports: Value) -> Reply`, `error(status, name, message) -> Reply`,
`fastify_404(method, path) -> Reply`, `secret(key, value) -> Value`,
`hidden(key) -> Value`, `personal(key, value) -> Value`, `provider(stub: &Stub) -> InfisicalProvider`,
`scope() -> SecretScope`, and the sentinels `CLIENT_ID`, `CLIENT_SECRET`, `TOKEN`, `TOKEN_2`, `VALUE`.

### B.7 T4 — documents

`docs/htui-secrets.md` (operator page; style of `docs/htui-mcp.md`: a plain lead paragraph, a
contents list, then sections):
1. **What it is**: htui reads one project scope's secrets from Infisical with a machine identity in the OS keyring. M2 ships the client; injection into sessions (M3) and the Settings section (M4) are noted as not yet available.
2. **Create a machine identity**: Universal Auth identity in the organisation; create a client secret; add the identity to the project with a role that can **read secret values** in the environment(s); note that `list_keys` needs value access too (A-6).
3. **Keyring entries**: `htui/infisical-url`, `htui/infisical-client-id`, `htui/infisical-client-secret`. A half-stored identity is an error.
4. **Base URL**: the §B.4.1 rules: https unless loopback, trailing `/` or `/api` dropped, give the server root and not an API path.
5. **Scope**: `project_id`, `environment` slug, `path` (default `/`); the column JSON.
6. **Imports and precedence**: the folder beats imports; the last import beats earlier ones; personal overrides are ignored; servers v0.150–v0.158 are handled.
7. **What htui refuses**: hidden values, names that are not environment variable names, NUL in a value. Fix in Infisical.
8. **Lockout safety**: one refused login per provider; re-enter the identity to try again.
9. **Errors and what to do**: one line per `SecretError` variant (14 lines).
10. **Testing against a real server**: the five env vars and the command.

`README.md`:
- After `:481-482`, add: "The Infisical live test reads `HTUI_TEST_INFISICAL_URL`, `HTUI_TEST_INFISICAL_CLIENT_ID`, `HTUI_TEST_INFISICAL_CLIENT_SECRET`, `HTUI_TEST_INFISICAL_PROJECT_ID` and `HTUI_TEST_INFISICAL_ENVIRONMENT` (`cargo test -p htui-secrets --test infisical_live`); without the URL it prints `skipped: HTUI_TEST_INFISICAL_URL not set` and passes. Use a throwaway identity with read access to one test environment."
- In Further reading (`:539-542`), add: ``- [`docs/htui-secrets.md`](docs/htui-secrets.md): the Infisical secret provider — machine identity, base URL, errors.``

---

## C. Data flow

1. **Keyring → identity (T2)**: `htui_store::secret::get_machine_identity()` gives `Ok(Some(MachineIdentity))`, `Ok(None)` (M3 maps it to `SecretError::NoIdentity`) or `Err` (keyring failure or half identity). `get_infisical_url()` gives the base URL.
2. **Build (T3)**: `InfisicalProvider::new(InfisicalConfig::new(url), identity)` normalises the URL (D8), checks both halves are non-blank, installs `ring`, and builds one client (timeouts, no redirects, no proxy on loopback). No socket is opened.
3. **Resolve**: `token()` (single flight; reuse until `reuse_until`; a refusal latches) → `GET /api/v4/secrets?…` with the bearer → `401`/`403 TokenError` → one `refresh` → `merge` (folder first, imports last-to-first, shared only) → `validate` (hidden → name → NUL) → `ResolvedSecrets`.
4. **M3 (future)**: `resolved.as_map()` feeds `MinimalScrubber::from_resolved` (`crates/htui-core/src/scrub.rs:211`) and a copy goes to `SessionSpec.env` (`crates/htui-agent/src/driver.rs:270`). One provider is shared per process and identity.
5. **Health (M4)**: latch peek → `GET /api/status` → fresh login → `ProviderHealth`.
6. **Errors**: every path ends in a `SecretError` built from a status, a known message, a cleaned non-login server message, a scope field, a key name or a cause chain. Never from a value, the client secret or the token.

---

## D. Tests, per task (written first; each must fail before its implementation)

### D.1 T1 (`crates/htui-core/src/secret.rs` `mod tests`)

- `scope_parses_the_column_shape`: `{"project_id":"p1","environment":"dev","path":"/app"}` gives the three accessors.
- `scope_path_defaults_to_root`: no `path` → `"/"`.
- `scope_round_trips_through_the_column`: `parse(to_column(s)) == s`.
- `scope_column_is_compact_and_ordered`: `SecretScope::new("p1","dev","/").to_column() == r#"{"project_id":"p1","environment":"dev","path":"/"}"#`.
- `scope_refuses_unknown_fields`: `{"project_id":"p","environment":"dev","extra":1}` → `Config`.
- `scope_refuses_an_empty_project_or_environment`: `""` and `"  "` for each → `Config` with the exact sentence.
- `scope_refuses_a_path_without_a_leading_slash`: `"app"` and `""` → `Config`.
- `scope_refuses_text_that_is_not_json`: `"p1/dev"` → `Config`.
- `resolved_secrets_debug_prints_keys_only`: `{"A":"s3cret-a","B":"s3cret-b"}` gives `Debug == r#"ResolvedSecrets { keys: ["A", "B"] }"#`, without the values.
- `resolved_secrets_lends_the_map_and_sorted_keys`: `as_map()` equals the input; `keys() == ["A","B"]` for input inserted `B`, `A`; `len`, `is_empty`.
- `machine_identity_debug_redacts_the_secret`: `MachineIdentity::new("cid-1","csecret-xyz")`: `Debug` contains `cid-1` and `<redacted>`, not `csecret-xyz`; the accessors return both.
- `secret_error_display_is_exact`: the §B.1.6 table, one `assert_eq!` per row.
- `every_secret_error_variant_is_covered`:
  - `fn variant(e: &SecretError) -> &'static str` is a `match` with **no wildcard**, so a new variant fails to compile here;
  - `fn every_variant() -> Vec<SecretError>` builds one of each, with fields set to the sentinels `VALUE="leak-value-1"`, `SECRET="leak-secret-1"` and `TOKEN="leak-token-1"` **only where a field is legitimately free text** (`Config`, `cause`, `detail`). Those fields are allowed to carry text, so this test asserts the **structure** only;
  - `assert_eq!(names.len(), 14)` with `names: BTreeSet<_>` of `variant(..)`;
  - the key-carrying variants print only their key (`InvalidKey`, `InvalidValue`);
  - the no-field variants' `Display` and `Debug` contain no sentinel.
- `a_provider_is_usable_as_arc_dyn` (`#[tokio::test]`; dev `tokio` has `macros`, `rt`, `crates/htui-core/Cargo.toml` dev-deps): a local `#[derive(Debug)] struct Fixed;` implementing `SecretProvider` with `Box::pin(async { … })`; `let p: Arc<dyn SecretProvider> = Arc::new(Fixed);` then `kind()`, `resolve`, `list_keys`, `health` through the `dyn`.

### D.2 T2 (`crates/htui-store/src/secret.rs`, new `#[cfg(test)] mod machine_identity_tests`)

Every case takes `mock_keyring()` or `mock_keyring_broken()` as its **first statement** (H-15).
Imports: `use super::{…}; use crate::testkit::{fake_machine_identity, mock_keyring, mock_keyring_broken, refuse_fake_store};`.

- `the_infisical_entries_are_named_exactly_as_the_plan_says`: the three constants equal `"infisical-url"`, `"infisical-client-id"`, `"infisical-client-secret"`, and `SERVICE == "htui"`.
- `a_machine_identity_round_trips`: set `("cid-1","csecret-1")`; get → `Some` with equal `client_id()` and `client_secret()`; `fake_machine_identity() == (Some("cid-1"), Some("csecret-1"))`.
- `no_identity_reads_as_none`: an empty fake → `Ok(None)`.
- `a_lone_client_id_is_an_error_naming_the_secret`: write only the client ID with the module-private `super::write_slot(INFISICAL_CLIENT_ID_USER, "cid-1")` (not `refuse_fake_store`, which under A-1 leaves nothing) → `Err` whose text contains `infisical-client-secret`.
- `a_lone_client_secret_is_an_error_naming_the_client_id_and_never_the_secret`: `super::write_slot(INFISICAL_CLIENT_SECRET_USER, "csecret-1")` only; the text contains `infisical-client-id` and not `csecret-1`.
- `a_blank_half_reads_as_absent`: fake secret `"   "` with an id → `Err` naming the secret half (blank = absent).
- `a_failed_secret_write_leaves_nothing`: empty fake, `refuse_fake_store(INFISICAL_CLIENT_SECRET_USER)`, set → `Err`; `fake_machine_identity() == (None, None)`.
- `a_failed_secret_write_over_an_old_identity_leaves_nothing` (A-1): set `("old-id","old-secret")`, then refuse the secret store, then set `("new-id","new-secret")` → `Err`; `(None, None)`; get → `Ok(None)`.
- `clear_removes_both_halves`: set, clear → `(None, None)`; clearing twice is `Ok`.
- `a_broken_keyring_is_an_error_never_none`: under `mock_keyring_broken()`, `get_machine_identity`, `set_machine_identity`, `clear_machine_identity`, `get_infisical_url`, `set_infisical_url` and `clear_infisical_url` all `Err`, each text containing `BROKEN_KEYRING`'s sentence.
- `the_infisical_url_round_trips_and_blank_reads_as_none`: set/get/clear; `"  "` → `None`.
- `refuse_store_touches_only_the_named_user`: refuse the secret user; `set_infisical_url` still `Ok`.

### D.3 T3

**`src/infisical.rs` `mod tests` (unit):**
- `base_urls_are_normalised`: every accepted row of §B.4.1 (input → output, loopback flag via `parse_base`).
- `base_urls_are_refused_with_a_reason`: every refused row; asserts `Config` and a reason substring (`"empty"`, `"absolute"`, `"https"`, `"user name"`, `"query"`, `"plain text"`); `user:pw` → the message contains no `pw`.
- `loopback_is_localhost_127_slash_8_and_ipv6_one_only`.
- `the_reuse_margin_is_the_larger_of_a_minute_and_a_tenth`: the five §B.4.3 examples.
- `env_names_match_the_posix_shape`: valid `A`, `_A1`, `a_b`; invalid `""`, `1A`, `A-B`, `A.B`, `É`, `A B`, `A\n`.
- `server_messages_lose_control_characters_and_stop_at_200_characters`: a 300-`é` message with `\r\n\x1b` → 200 `char`s, no control characters, no panic at the boundary.
- `the_fastify_not_found_body_is_recognised`: the Fastify body yes; Infisical `NotFound` no.
- `the_lockout_message_is_recognised_case_insensitively`.
- `retry_after_reads_whole_seconds_only`: `"30"` → 30; `" 7 "` → 7; an HTTP-date → `None`; absent → `None`.
- `validation_reports_hidden_then_name_then_nul`: through `validate` directly (merged maps built by hand).

**`src/wire.rs` `mod tests` (unit):**
- `merge_worked_example`: the §B.4.5 tables, exactly.
- `merge_keeps_the_first_duplicate_within_one_list`.
- `the_login_request_serialises_camel_case`: `{"clientId":"…","clientSecret":"…"}`.
- `a_list_body_with_unknown_fields_and_null_imports_parses`.
- `an_error_body_that_is_not_json_is_empty`: `from_bytes(b"<html>")` → empty `message()`/`error()`; an array `message` → `""`.

**`tests/infisical.rs` (stub-backed; `mod support;`; each `#[tokio::test]`; one fresh stub and provider per test):**

*Build and config*
1. `a_provider_builds_with_only_this_crate_installing_ring` (sync `#[test]`): `InfisicalProvider::new(InfisicalConfig::new("https://app.infisical.com"), identity())` `.expect(…)` twice. The doc comment says it is meaningful only under `cargo test -p htui-secrets`, whose graph has no `sentry`/aws-lc (H-1).
2. `an_http_base_url_on_a_lan_host_is_refused_before_any_request`: `http://192.168.1.10` → `Config` containing `https`.
3. `a_blank_identity_half_is_refused_at_build`: both halves, `"  "` → `Config` with the exact sentence.
4. `the_base_url_is_normalised_before_use`: base `{stub}/api/` → the stub sees `/api/v1/auth/universal-auth/login`, not `/api/api/…`.
5. `kind_is_infisical`.

*Login + list*

6. `resolve_logs_in_then_lists_the_scope`:
   - requests are exactly `[POST login, GET secrets]`;
   - the login body JSON is `{clientId: CLIENT_ID, clientSecret: CLIENT_SECRET}`, with `content-type` containing `application/json`;
   - the GET carries `authorization == "Bearer " + TOKEN`;
   - `user-agent` starts with `htui/`;
   - the map is as scripted.
7. `the_list_query_carries_every_flag_and_both_imports_spellings`: each §B.4.4 pair, `query(key) == [value]`, including `secretPath == "/app"` decoded.
8. `list_keys_returns_sorted_names_matching_resolve`.

*Merge*

9. `the_folder_beats_every_import`.
10. `the_last_import_beats_earlier_ones`.
11. `personal_entries_are_ignored`.
12. `values_pass_through_byte_for_byte`: `"a b\n"`, `"  x"`, `"é✓"`, `""` are unchanged.
13. `an_absent_or_null_imports_field_is_no_imports`.

*Validation*

14. `a_hidden_value_is_refused_naming_the_key`: `PermissionDenied`, `detail` contains `"HIDDEN_KEY"`, not `VALUE`.
15. `a_hidden_import_value_shadowed_by_the_folder_is_not_a_refusal`.
16. `a_bad_key_name_is_refused_naming_it`: `"my-key"` → `InvalidKey { key: "my-key" }`.
17. `a_nul_in_a_value_is_refused_naming_the_key`: `"a\u0000b"` → `InvalidValue { key }`; the text does not contain `VALUE`.
18. `hidden_is_reported_before_a_bad_name_before_a_nul`.

*Login refusal and lockout (D5, D10 "lockout-safe")*

19. `a_refused_login_is_bad_credentials_and_never_retried`: 401 `"Invalid credentials"` → `BadCredentials`. The second `resolve` → `LoginRefusedEarlier`, with `requests().len()` unchanged and `count(POST login) == 1`; `list_keys` likewise.
20. `the_lockout_text_is_identity_locked_and_latches`: 401 `"Identity is temporarily locked …"` → `IdentityLocked`; then `LoginRefusedEarlier`, with no request.
21. `health_after_a_refused_login_makes_no_request`: not even `/api/status`.
22. `concurrent_first_calls_share_one_login`: `tokio::join!(p.resolve(&s), p.resolve(&s))` with a 401 login → exactly one login; the results are `{BadCredentials, LoginRefusedEarlier}` in either order.
23. `concurrent_first_calls_with_a_good_login_share_one_login`: a 200 login → `count(POST login) == 1`, two GETs.
24. `a_login_429_is_rate_limited_and_does_not_latch`: login `[429 + Retry-After: 12, 200]` → `RateLimited{Some(12)}`, then `Ok`.
25. `a_login_network_failure_is_unreachable_and_does_not_latch`: `closed_port_base()` → `Unreachable{endpoint:"/api/v1/auth/universal-auth/login"}` twice (the second is not `LoginRefusedEarlier`). The `cause` is non-empty and contains no `127.0.0.1:` URL with a path.
26. `a_login_route_404_is_unsupported_server` (A-7).
27. `a_login_500_is_protocol_and_never_quotes_the_body`: the body message holds `CLIENT_SECRET` and `TOKEN`; the error contains neither.

*Data errors*

28. `a_fastify_route_404_is_unsupported_server`: `UnsupportedServer{"/api/v4/secrets"}`; the text contains `v0.150`.
29. `a_not_found_404_is_project_not_found`.
30. `a_secret_path_404_is_path_not_found_naming_the_scope`: `PathNotFound{environment:"dev", path:"/app"}`.
31. `a_permission_denied_403_carries_the_cleaned_message`.
32. `a_membership_403_is_permission_denied`.
33. `a_429_is_rate_limited_with_or_without_retry_after`.
34. `a_long_server_message_is_cleaned_and_cut`: a 300-char message with `\r\n` → `detail` ≤ 200 chars, no control characters.
35. `an_unexpected_status_is_protocol_naming_the_endpoint`: 500 → `Protocol{"/api/v4/secrets", "status 500: …"}`.
36. `a_non_json_list_body_is_protocol`: `Reply::text(200, "<html>")`.
37. `a_data_redirect_is_protocol_and_never_followed`: GET secrets `302 Location: {base}/elsewhere`; `/elsewhere` scripted 200; → `Protocol`; `count("GET","/elsewhere") == 0`.
38. `a_login_redirect_is_protocol_and_the_body_never_follows`: login `307 Location: {base}/steal`; `count("POST","/steal") == 0`; not latched.

*Token (D5)*

39. `a_live_token_is_reused_across_calls`: `expiresIn 2592000`; resolve, list_keys, resolve → one login, three GETs.
40. `a_token_inside_the_margin_is_replaced`: `expiresIn 30`; two resolves → two logins.
41. `a_token_error_403_logs_in_once_more_then_succeeds`: logins `[TOKEN, TOKEN_2]`, GETs `[403 TokenError, 200]` → `Ok`; two logins; the second GET's bearer is `TOKEN_2`.
42. `a_data_401_logs_in_once_more_then_succeeds`.
43. `a_token_refused_twice_is_protocol_after_one_relogin`: GET always 403 `TokenError` → `Protocol`; exactly two logins and two GETs.
44. `a_refused_relogin_latches`: logins `[200, 401]`, GET `[403 TokenError]` → `BadCredentials`; the next call → `LoginRefusedEarlier` with no request.

*Health*

45. `health_reads_the_status_then_logs_in`: requests `[GET /api/status, POST login]`; `ProviderHealth{base_url: stub.base(), server_ok: true}`.
46. `health_reports_a_failing_status_and_still_logs_in`: status 503 → `server_ok: false`, `Ok`.
47. `health_always_logs_in_afresh`: resolve, then health → two logins (D2).
48. `health_on_an_unreachable_server_names_the_status_endpoint`: `closed_port_base()` → `Unreachable{"/api/status"}`.

*Leak (D10 "no value leaks via an error, log or Debug")*

49. `no_error_or_debug_carries_a_value_the_client_secret_or_the_token`:
   - The test owns `fn variant(e: &SecretError) -> &'static str`, an exhaustive `match` with no wildcard. A new variant fails to compile here and needs a case.
   - A scenario table drives the stub to produce **every variant the provider can build**: `Config` (LAN http), `Unreachable` (closed port), `BadCredentials`, `IdentityLocked`, `LoginRefusedEarlier`, `ProjectNotFound`, `PathNotFound`, `PermissionDenied` (hidden key), `RateLimited`, `UnsupportedServer`, `InvalidKey`, `InvalidValue`, `Protocol` (malformed list).
   - `NoIdentity` is pushed by hand: the caller builds it, not the provider.
   - Asserted: the set of `variant(..)` names equals all 14.
   - For each error, neither `to_string()` nor `format!("{e:?}")` contains `VALUE`, `CLIENT_SECRET`, `TOKEN` or `TOKEN_2`. The assert message names the variant only (H-8).
   - **Sentinel placement rule:**
     - `CLIENT_SECRET` is the identity's secret;
     - `TOKEN` is in every 200 login body;
     - `VALUE` is every `secretValue`;
     - login error bodies' `message` carries `CLIENT_SECRET` and `TOKEN` (never quoted, D4);
     - malformed bodies carry `TOKEN`/`VALUE` in mistyped positions (A-5);
     - non-login error `message`s carry **no** sentinel, because D4 allows quoting them.
50. `the_provider_debug_shows_neither_the_secret_nor_the_token`: after a successful resolve, `format!("{provider:?}")` contains `CLIENT_ID` and the base URL, and neither `CLIENT_SECRET` nor `TOKEN`.
51. `a_malformed_login_answer_never_quotes_the_token`: `{"accessToken": TOKEN, "expiresIn": TOKEN}` → `Protocol` without `TOKEN`.
52. `a_malformed_list_answer_never_quotes_a_value`: `{"secrets":[{"secretKey":"K","secretValue":"x","secretValueHidden": VALUE}]}` → `Protocol` without `VALUE`.

Every `support` item is used:
- `Reply::json`: everywhere;
- `Reply::text`: 36;
- `Reply::redirect`: 37, 38;
- `Reply::header`: 24, 33;
- `Request::header`: 6;
- `Request::query`: 7;
- `Request::json`: 6;
- `Stub::count`: 19+;
- `Stub::requests`: 6, 19+;
- `closed_port_base`: 25, 48.

### D.4 T4 (`crates/htui-secrets/tests/infisical_live.rs`, no `mod support;`)

```rust
//! The Infisical provider against a real server (MOD-10 D10). Gated on `HTUI_TEST_INFISICAL_URL`;
//! unset, it prints the skip line and passes (as `htui-store/tests/qdrant_live.rs:31-40`). With
//! the URL set, the other four variables are required, and a missing one panics by name.

const ENV_URL: &str = "HTUI_TEST_INFISICAL_URL";
const ENV_CLIENT_ID: &str = "HTUI_TEST_INFISICAL_CLIENT_ID";
const ENV_CLIENT_SECRET: &str = "HTUI_TEST_INFISICAL_CLIENT_SECRET";
const ENV_PROJECT_ID: &str = "HTUI_TEST_INFISICAL_PROJECT_ID";
const ENV_ENVIRONMENT: &str = "HTUI_TEST_INFISICAL_ENVIRONMENT";

fn infisical_url() -> Option<String> {
    let url = std::env::var(ENV_URL).ok();
    if url.is_none() {
        eprintln!("skipped: {ENV_URL} not set");
    }
    url
}
```

- `resolves_the_configured_scope` (`#[tokio::test]`):
  - setup: `health()` is `Ok` with `server_ok`; scope `SecretScope::new(project, env, "/")`;
  - `resolve` is non-empty;
  - `list_keys() == resolved.keys()`;
  - `eprintln!("resolved {} keys", resolved.len())`: the count only, never a name or value.
- No assert message interpolates the client secret, the token or a value (H-8).

### D.5 T4 documents
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` is green.

---

## E. Hazards

- **H-1 `ring` install hidden by `sentry`.** With `rustls-no-provider` and no installed provider, `Client::build` panics. Workspace builds hide it, because `sentry` turns on reqwest's `rustls` (aws-lc).
  - `cargo test -p htui-secrets` is the only build that proves `install_crypto_provider` runs before `build()`.
  - Never add a dev-dependency that pulls `sentry`, `htui` or `htui-store/local-embed` into htui-secrets.
  - The gate checks the graph: `cargo tree -p htui-secrets -e normal,dev | grep -c aws-lc` must print `0`.
- **H-2 `Debug` on secret holders.** `Zeroizing<String>`'s derived `Debug` prints the value (plan grounding, probed).
  - `MachineIdentity`, `ResolvedSecrets` and `InfisicalProvider` have hand-written `Debug`s.
  - `TokenState`, `LoginRequest`, `LoginResponse`, `WireSecret` and `Merged` have **no** `Debug` at all. They are private or `pub(crate)`, so `missing_debug_implementations` does not ask for one.
  - Never add `#[derive(Debug)]` "to debug", and never `dbg!` a response.
  - `FakeSlots` derives `Debug` and is test-only (existing).
- **H-3 `#[non_exhaustive]`** on `SecretError` would force a wildcard arm in htui-secrets' leak test and silently defeat it. Do not add it.
- **H-4 `serde_json` and reqwest messages quote data.** Never format a `serde_json::Error` or a decode `reqwest::Error` into a variant built from a server body (A-5).
  - Use `cause_chain(err)` only for transport errors, after `without_url()`. reqwest's `Display` appends ` for url (…)`, with the query (`reqwest error.rs` `Display`).
- **H-5 `.no_proxy()` on loopback only.** With `HTTP_PROXY` set, `system-proxy` routes `127.0.0.1` to the proxy. Tests cannot unset it: `set_var` is `unsafe` in edition 2024 and `unsafe_code = forbid`.
  - The T3 gate runs the suite once with `HTTP_PROXY=http://127.0.0.1:9` on the command line; every stub test must still pass.
  - Non-loopback hosts keep the system proxy (D9).
- **H-6 Mutex kind and scope.** Use `tokio::sync::Mutex`. A `std::sync::MutexGuard` held across `.await` makes the future `!Send`, so `SecretFuture` does not compile.
  - Hold the lock across the **login** only, never across the data GET. That would serialise every resolve, and a nested `refresh` would deadlock on itself.
- **H-7 Stub body read.** Read `Content-Length` bytes from the **same** `BufReader` that read the headers, which may already hold body bytes; reading the raw `TcpStream` loses them.
  - Header names arrive lowercase from hyper; compare case-insensitively anyway.
  - Bind `127.0.0.1`, not `localhost`.
  - Record before replying (§B.6).
- **H-8 Secrets in assert messages.** `assert!(!text.contains(TOKEN), "{variant} carries the token")`: name the variant, never `{text}` or `{err}`, because if the assertion fires the message would print the leak.
  - Same in the live test: never print a key, value, client secret or token.
  - `assert_eq!` on two secret-bearing strings prints both on failure: prefer `assert!(a == b, "…")`.
- **H-9 Dead code in `tests/support/mod.rs`.** It is a private module of the integration-test crate, so an unused `pub fn` warns and fails `-D warnings`. Every helper in §B.6 has a user (§D.3 list).
  - Use `tests/support/mod.rs`, never `tests/support.rs`: Cargo would build the latter as its own empty test binary.
- **H-10 Unread `Deserialize` fields** (`tokenType`, `accessTokenMaxTTL`, `reqId`, `statusCode`, an import's `secretPath`/`environment`) trigger `dead_code` "field is never read". Declare only what is read; serde ignores unknown fields by default. **No** `deny_unknown_fields` on Infisical shapes: a server upgrade adds fields.
- **H-11 thiserror argument form.** `#[error("…{}", retry_hint(.retry_after_secs))]` passes `&Option<u64>`. `clippy::ref_option` is pedantic, so it is not in the gate.
- **H-12 Rustdoc lints at deny.** `htui-core` docs must not intra-link `htui_secrets` or `htui_store` items: htui-core depends on neither, so the link breaks under `cargo doc`. Use plain backticks. Private items linked from public docs trip `private_intra_doc_links`.
- **H-13 `preserve_order` unification.** `SecretScope::to_column` serialises the derived `ScopeColumn` struct (fixed field order), never a `json!`/`Map` (insertion order differs between `-p` and workspace builds; M1 H-11).
- **H-14 Clippy gates.**
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`, **and** the featureless `cargo clippy --workspace -- -D warnings`. In the latter `test-support` is off, so `infisical_field`, `refuse_store` and the fake branches vanish. The non-fake path of `read_slot`/`write_slot`/`remove_slot` must still be used (it is, by the public functions).
  - New `pub` items need docs (`missing_docs` in htui-core, htui-store and htui-secrets), including every enum-variant field.
  - `unused_qualifications`: once `use` brings a name in, do not write its full path.
  - `clippy::len_without_is_empty`: `ResolvedSecrets` has both.
  - `clippy::new_without_default` does not apply (`new` takes arguments, `Stub::start`).
- **H-15 Keyring fake locking.** `fake()` is a non-reentrant `std::sync::Mutex`.
  - Never hold its guard while calling a public slot function, in implementation or in a test: that deadlocks.
  - Every T2 test takes `mock_keyring()` first. The `KEYRING` lock is process-wide (`testkit.rs:307`).
  - A test that forgets it writes the developer's real keyring.
- **H-16 `Cargo.lock`.** T1 adds one line (`"zeroize"` in htui-core). T3 adds the `htui-secrets` block. T2 must not touch it: htui-store already depends on `zeroize` and `htui-core`.
  - Build `--offline`; never `cargo update`.
  - T2 and T3 run in parallel, so T2's diff must show no `Cargo.lock` change.
- **H-17 Byte-safe truncation.** `clean_message` counts `char`s (`.chars().filter(|c| !c.is_control()).take(200).collect()`). A byte slice at 200 panics on multibyte text.
- **H-18 Token comparison in `refresh`** compares the token used with the cached one to avoid a double re-login. It is not a security comparison, and its result is never logged.
- **H-19 `query_pairs_mut` borrow.** Drop the serializer (end the statement or scope) before using the `Url`. The borrow checker enforces it, but chaining `.append_pair` and then `url` in one expression does not compile.
- **H-20 Unzeroized copies are out of scope.** Each of these keeps an unwiped copy, documented as a limit (plan D3's M3 note):
  - reqwest's request body (the serialized login JSON);
  - the `Authorization` header value;
  - the response `Bytes`;
  - serde's intermediate buffers.

  Do not try to wipe these.
- **H-21 Featureless consumers.** htui-secrets is in `members` but no crate depends on it until M3. `cargo build --workspace` still compiles it, so it must build and lint clean alone.
- **H-22 Suite scheduling.** Verify the merged tree with `--test-threads=1` for the `htui` crate (keyring fake process-wide; memory note). htui-secrets' stubs are per test and port-0, so they are safe in parallel.

**22 hazards.**

---

## F. Build sequence, lanes, commits and gates

| Lane | Order | Task | Files |
|---|---|---|---|
| **A** | first, serial | T1 | `crates/htui-core/{Cargo.toml, src/lib.rs, src/secret.rs}`, `Cargo.lock` |
| **B** | after T1, ∥ C | T2 | `crates/htui-store/src/secret.rs`, `crates/htui-store/src/testkit.rs` |
| **C** | after T1, ∥ B | T3 | `Cargo.toml`, `Cargo.lock`, `crates/htui-secrets/{Cargo.toml, src/**, tests/support/mod.rs, tests/infisical.rs}` |
| **D** | after C | T4 | `crates/htui-secrets/tests/infisical_live.rs`, `README.md`, `docs/htui-secrets.md` |

B ∩ C = ∅ (files). The hidden coupling is `Cargo.lock`, which T1 and T3 touch in sequence and T2
never touches (H-16). There are no `.sqlx` files and no snapshots.

Commit with `git add <explicit paths>`: never `-A`, never stash, never amend. Each commit ends with
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Every commit below is green on its own
gate.

**T1** (TDD: write §D.1's tests for the step first, see them fail, then implement):
1. `feat(mod-10): htui-core secret module - SecretScope` — `Cargo.toml` `zeroize`, `Cargo.lock` (one line), `lib.rs`, `secret.rs` with `SecretError` (all variants, so `Config` exists) and `SecretScope` + their tests.
2. `feat(mod-10): htui-core secret value types` — `ResolvedSecrets`, `MachineIdentity`, `ProviderHealth` + tests.
3. `feat(mod-10): htui-core SecretProvider seam` — `SecretFuture`, `SecretProvider`, the Display table and coverage tests, `a_provider_is_usable_as_arc_dyn`.

Gate T1:
```bash
cargo test -p htui-core --offline --lib secret
cargo clippy -p htui-core --offline --all-targets -- -D warnings
cargo clippy -p htui-core --offline -- -D warnings
cargo doc -p htui-core --offline --no-deps
git diff HEAD~3 -- Cargo.lock | grep -c '^+ "zeroize"'   # 1, and no other +/- line in Cargo.lock
```

**T2:**
1. `feat(mod-10): Infisical URL keyring slot` — constants, `FakeSlots` fields (all four), helpers, URL functions, the URL and naming tests.
2. `feat(mod-10): machine identity keyring slots` — the identity functions, `refuse_store`, testkit `fake_machine_identity`/`refuse_fake_store`, the remaining §D.2 tests, the module doc.

Gate T2:
```bash
cargo test -p htui-store --offline --features test-support --lib secret
cargo clippy -p htui-store --offline --all-targets --features test-support -- -D warnings
cargo clippy -p htui-store --offline -- -D warnings          # featureless: the fake is absent
git diff --stat HEAD~2 -- Cargo.lock                          # empty
```

**T3:**
1. `feat(mod-10): htui-secrets crate - client, base URL policy` — workspace `Cargo.toml`, `Cargo.lock`, the crate manifest, `lib.rs`, `wire.rs`, `infisical.rs` with `InfisicalConfig`, `new`, `normalise_base_url`, `install_crypto_provider`, `Debug`; unit tests for the base URL, `retry_after`, `is_env_name`, `clean_message`; integration tests 1–5.
2. `feat(mod-10): Infisical login, token state and resolve` — `support/mod.rs`, `TokenState`, `token`/`refresh`/`fresh_login`, `list`, `merge`, `validate`, the trait impl; unit tests for the margin, merge and validation; integration tests 6–23, 39–44.
3. `feat(mod-10): Infisical error mapping, health and leak tests` — the §B.4.2 tables in full, `health_inner`; integration tests 24–38, 45–52.

Gate T3:
```bash
cargo test -p htui-secrets --offline
HTTP_PROXY=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 cargo test -p htui-secrets --offline   # H-5
cargo clippy -p htui-secrets --offline --all-targets -- -D warnings
cargo clippy -p htui-secrets --offline -- -D warnings
cargo doc -p htui-secrets --offline --no-deps
cargo tree -p htui-secrets --offline -e normal,dev | grep -c aws-lc    # 0 (H-1)
```

**T4:**
1. `test(mod-10): Infisical live test, env-gated` — `infisical_live.rs`.
2. `docs(mod-10): htui-secrets operator page and README` — `docs/htui-secrets.md`, `README.md`.

Gate T4:
```bash
cargo test -p htui-secrets --offline --test infisical_live -- --nocapture   # prints "skipped: HTUI_TEST_INFISICAL_URL not set"
cargo clippy -p htui-secrets --offline --all-targets -- -D warnings
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
The maintainer then runs the live test once against the self-hosted instance with the five
variables set and records the result in the phase note (plan Acceptance).

**Merged tree** (plan Validation):
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui-core secret
cargo test -p htui-store --features test-support secret
cargo test -p htui-secrets
cargo test --workspace --all-features --no-fail-fast        # htui crate with -- --test-threads=1 if flaky (H-22)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## G. Tree facts relied on (re-verified at `f751aa68`)

| Fact | Evidence |
|---|---|
| Keyring constants | `crates/htui-store/src/secret.rs:27` `SERVICE`, `:30` `USER`, `:34` `QDRANT_URL_USER`, `:36` `QDRANT_KEY_USER` |
| `FakeSlots` (3 fields, `Debug, Clone, Default`, `pub(crate)`), built only by `default()` | `secret.rs:44-50`; `testkit.rs:337` (sole construction, symbol search) |
| `Fake`, `FAKE`, `fake()`, `fake_failure` | `secret.rs:54-60`, `:71`, `:86-90`, `:94-96` |
| Qdrant slot functions to mirror; end of that block | `secret.rs:278-358` |
| `Slot` blank → `None`; `backend()` | `secret.rs:383-390`; `:413` |
| `headless_dsn_tests` uses `crate::testkit` under `cfg(test)` | `secret.rs:525-530`; works because dev-deps self-enable `test-support` (`crates/htui-store/Cargo.toml` `[dev-dependencies]`) |
| `KEYRING`, `mock_keyring`, `mock_keyring_broken`, `BROKEN_KEYRING`, `fake_qdrant_dsn`, `fake_dsn` | `testkit.rs:307`, `:334`, `:360`, `:347-348`, `:373`, `:386` |
| `testkit` behind `test-support`; `htui-store` has `missing_docs` | `crates/htui-store/src/lib.rs:28-29`, `:11` |
| `StoreError::Backend(String)` | `crates/htui-core/src/store/error.rs:47` |
| htui-core modules and `missing_docs` | `crates/htui-core/src/lib.rs:9`, `:11-17`; no `secret` module, no seam-type name collision (symbol search) |
| htui-core deps / dev-deps (`tokio` macros+rt) | `crates/htui-core/Cargo.toml` |
| `from_resolved(&BTreeMap<String,String>) -> (Self, Vec<String>)` | `crates/htui-core/src/scrub.rs:211` |
| `SessionSpec.env: BTreeMap<String,String>`, hand-written redacting `Debug` | `crates/htui-agent/src/driver.rs:270`, `:294` |
| Boxed future alias and error-enum pattern | `crates/htui-orch/src/isolate.rs:28-34`, `:36-54` |
| `http_client`, `install_crypto_provider` | `crates/htui-store/src/model.rs:606-628` |
| Loopback stub to extend (GET-only, path-only, no body) | `model.rs:655-738` |
| `USER_AGENT`, `PROVIDER`, `install_crypto_provider`, `short` client (`timeout` + `connect_timeout`) | `crates/htui-agent/src/install/http.rs:22`, `:33`, `:51-55`, `:75-90` |
| Ring-install test precedent | `crates/htui-agent/tests/install.rs:1008` |
| Live gate pattern | `crates/htui-store/tests/qdrant_live.rs:9` (`cfg(feature)`), `:31-40` |
| Workspace members, deps, lints | `Cargo.toml:2-3`, `:49-54`, `:60-61` tokio, `:78` zeroize, `:118-121` reqwest/rustls, `:172` url, `:181-195` |
| htui-mcp manifest shape | `crates/htui-mcp/Cargo.toml` |
| `url` declared in htui-agent | `crates/htui-agent/Cargo.toml:38`; reqwest/rustls `:50-51` |
| Lock versions | `Cargo.lock:7116` url 2.5.8 (single), `:7892` zeroize 1.9.0, `:5224` reqwest 0.13.5, `:5423` rustls 0.23.43; htui-core deps `:3101-3116` |
| zeroize features: `alloc` default, **no `serde`** | `zeroize-1.9.0/Cargo.toml` `[features]` |
| reqwest: `json` feature; `Error::without_url`; `Display` appends the URL; `redirect`, `no_proxy`, `timeout`, `connect_timeout` | `reqwest-0.13.5/Cargo.toml:143-146`; `src/error.rs:91`, `:257-300`; `src/async_impl/client.rs:1392`, `:1436`, `:1450`, `:1475` |
| `Project.secret_provider` / `secret_scope` | `crates/htui-core/src/model/hierarchy.rs:66`, `:68` |
| No `tests/support/` anywhere yet | `ls crates/*/tests` |
| README Tests and Further reading | `README.md:464-489` (Qdrant line `:481`), `:537-542` |
| Validator script | `.claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |

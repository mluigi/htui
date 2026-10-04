# Plan: MOD-10 M2 — Infisical provider

**Source PRD**: `.claude/prds/mod-10-secret-provider.prd.md`
**Selected Milestone**: 2 — Infisical provider
**Complexity**: Medium
**Status**: DRAFT — awaiting fact-check and CONFIRM

## Summary

Give `htui` an in-process client for Infisical that a box with a machine identity in its keyring can use
to log in, list and resolve one project scope's secrets, and report health. Every failure is a typed
error that names its cause and never carries a secret value, the client secret or the access token.
There are three pieces:
- a `SecretProvider` seam and its value types in `htui-core`;
- three keyring slots in `htui-store::secret`, next to the DSN and Qdrant slots;
- a new `htui-secrets` crate holding the Infisical implementation, tested against a loopback stub
  server.

Nothing here injects a secret into a session. M3 does that by building the run's
`SessionSpec.env` and its scrubber from the map this milestone returns, and M4 adds the Settings
section.

Routed as PRD by `/handoff-run MOD-10` (accepted 2026-10-04, sandbox run `hr/MOD-10`, M2 only, with
ultracode for the implement phase).

## Grounding (read on `hr/MOD-10`, base `815528d1`)

- **Keyring slots** live in `crates/htui-store/src/secret.rs`:
  - the constants are `SERVICE = "htui"` (`:27`), `USER = "postgres-dsn"` (`:30`),
    `QDRANT_URL_USER` (`:34`) and `QDRANT_KEY_USER` (`:36`);
  - each slot has its own `get_*`/`set_*`/`clear_*` free functions (`:278-357`), and each function
    consults the `test-support` `FAKE` first (`:46-91`);
  - `Slot` reads a blank entry as `None` (`:383`), and `backend()` (`:413`) is the module's one
    error shape;
  - `FakeSlots` (`:46`) has one field per slot.
  - `crates/htui-store/src/testkit.rs` provides `mock_keyring()` (`:334`) and
    `mock_keyring_broken()` (`:360`), which share one process-wide `KEYRING` lock.
- **The scrubber's constructor**, `MinimalScrubber::from_resolved(&BTreeMap<String, String>) ->
  (Self, Vec<String>)` (`crates/htui-core/src/scrub.rs:211`), is what M3 feeds. The provider's
  output therefore has to lend out a `&BTreeMap<String, String>`.
- **`SessionSpec.env`** is a `BTreeMap<String, String>` (`crates/htui-agent/src/driver.rs:270`),
  so it has the same shape as the map above.
- **Dyn-compatible async traits** are written with a boxed future alias, for example
  `IsolatorFuture<'a, T>` (`crates/htui-orch/src/isolate.rs:33-34`) and
  `htui_agent::driver::DriverFuture`. `async_trait` is not used in the workspace.
- **HTTP clients** follow `crates/htui-store/src/model.rs:605-628`:
  - `http_client()` builds one `reqwest::Client` with a user agent, a connect timeout and a read
    timeout;
  - `install_crypto_provider()` installs `ring` once and ignores "already installed";
  - `crates/htui-agent/src/install/http.rs:40-53` gives the same reason in full.
  - The workspace `reqwest` is `default-features = false, features = ["rustls-no-provider",
    "stream", "system-proxy"]` (`Cargo.toml`).
- **HTTP tests** use a loopback stub on a std thread with scripted routes, no new dev-dependency
  (`crates/htui-store/src/model.rs:657-720`, `Stub::start`).
- **Live-service tests** are gated on an env var that prints a skip line and passes when unset
  (`crates/htui-store/tests/qdrant_live.rs:1-35`, `HTUI_TEST_QDRANT_URL`).
- **Project columns** `project.secret_provider TEXT` and `project.secret_scope TEXT` already exist
  (`crates/htui-store/migrations/0001_init.sql:148-149`) and surface as `Option<String>` on
  `Project` (`crates/htui-core/src/model/hierarchy.rs:66-68`). No writer exists yet (M4).
- **New crate precedent:** `htui-mcp` (MOD-11), whose `Cargo.toml` shape (workspace package keys,
  commented dependencies, `[lints] workspace = true`) is what we mirror.
  - Workspace lints are `unsafe_code = "forbid"`, `missing_debug_implementations = "warn"`,
    `unused_qualifications = "warn"` and `clippy::all = warn`.
- **Infisical API facts** come from a survey run 2026-10-04 against `Infisical/infisical` main and
  `Infisical/cli`:
  - **Login:** `POST /api/v1/auth/universal-auth/login` with `{clientId, clientSecret}` returns
    `{accessToken, expiresIn, accessTokenMaxTTL, tokenType}`. Both a bad id and a bad secret get
    401 `"Invalid credentials"`.
  - **Lockout:** on by default, 3 failures lock the identity for 300 s. A locked identity gets
    401 with "...temporarily locked...".
  - **Renewal:** `POST /api/v1/auth/token/renew` exists, but the default TTL equals the default max
    TTL (30 d), so renewal gains nothing.
  - **Listing:** `GET /api/v4/secrets` takes `projectId`, `environment`, `secretPath` and string
    booleans `viewSecretValue`, `expandSecretReferences`, `includeImports`, `recursive` and
    `includePersonalOverrides`.
    - Servers **v0.150–v0.158** spell the imports flag `include_imports` with default `false`.
      v0.159+ use `includeImports` with default `true`. Unknown query keys are dropped silently.
    - The response is `{secrets: [...], imports?: [{secretPath, environment, secrets: [...]}]}`.
      Each secret carries `secretKey`, `secretValue`, `secretValueHidden` and `type`.
    - The server does not merge. Infisical's CLI and docs agree: a folder's own secrets win, and
      among imports the **bottom-most** one wins.
    - With `viewSecretValue=true`, an identity lacking ReadValue on any secret gets 403 for the
      whole request.
  - **Version detection:** a server older than v0.150 has no `/api/v4` routes and answers with
    Fastify's default 404 ("Route GET:... not found"). `GET /api/status` needs no auth and has no
    version field.
  - **Errors** come as `{reqId, statusCode, message, error}`:
    - unknown project: 404 `NotFound`;
    - unknown environment or path: 404 `SecretPathNotFound`;
    - not a member: 403 `ProjectMembershipNotFound`;
    - permission: 403 `PermissionDenied`;
    - expired token: 403 `TokenError`;
    - rate limit (cloud only): 429 `RateLimitExceeded`.
  - **Base URLs:** cloud is `https://app.infisical.com` or `https://eu.infisical.com`; self-hosted
    uses the same `/api/...` prefixes.
  - **Keys** forbid only `:` and `/`, so names invalid in an environment are possible. A NUL in a
    value is not ruled out server-side.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-store/src/secret.rs:27-36` | `*_USER` constants under `SERVICE`; `get_/set_/clear_` free functions per slot |
| Errors | `crates/htui-orch/src/isolate.rs:44-58` | `#[derive(Debug, thiserror::Error)]` enum whose doc says what each variant means and who builds it |
| Async seam | `crates/htui-orch/src/isolate.rs:33-34` | `pub type XFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>` |
| HTTP client | `crates/htui-store/src/model.rs:605-628` | one `reqwest::Client`, explicit timeouts, `ring` provider installed once |
| Keyring fake | `crates/htui-store/src/secret.rs:46-91`, `testkit.rs:334-364` | `FakeSlots` field per slot, `mock_keyring()` guard |
| Tests (HTTP) | `crates/htui-store/src/model.rs:657-720` | std-thread loopback stub, scripted routes, requests recorded |
| Tests (live) | `crates/htui-store/tests/qdrant_live.rs:1-35` | env-gated; prints the skip line and passes when unset |
| Crate shape | `crates/htui-mcp/Cargo.toml` | workspace package keys, a reason comment per dependency, `[lints] workspace = true` |

## Decisions (proposed; CONFIRM accepts or overrides)

**D1 — Where the code lives.**
- The seam (trait, value types, error) goes in a new `htui-core::secret` module. `htui-worker`,
  `htui-orch` and `htui` can then hold an `Arc<dyn SecretProvider>` without depending on HTTP.
- The keyring slots go in `htui-store::secret`, beside the DSN and Qdrant slots, as ANA-7 §4
  asks.
- The Infisical client goes in a **new crate `crates/htui-secrets`**.
  - **Rejected: `htui-store`**, whose `reqwest` is optional behind `local-embed`. That would be a
    second feature gate on a crate whose subject is the store.
  - **Rejected: `htui-agent`**, which has `reqwest` for the installer but is about driving agents.
  - Like `htui-mcp`, the new crate depends on `htui-core` only. It never reads the keyring: the
    caller hands it a `MachineIdentity`, and M3 composes keyring → provider in the process that runs
    the run.

**D2 — The seam.** In `htui-core::secret`:

```rust
pub type SecretFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, SecretError>> + Send + 'a>>;

pub trait SecretProvider: Send + Sync + core::fmt::Debug {
    /// Stable provider name, the value `project.secret_provider` holds ("infisical").
    fn kind(&self) -> &'static str;
    /// Reachability plus a successful login with the configured identity.
    fn health(&self) -> SecretFuture<'_, ProviderHealth>;
    /// Key names visible in `scope`, sorted; never values.
    fn list_keys<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>>;
    /// Every key → value in `scope`, imports merged, validated for an environment block.
    fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets>;
}
```

The PRD's "bootstrap a machine identity" is login: `health` performs it, and `list_keys`/`resolve`
perform it when no usable token is cached. Writing an identity into the keyring belongs to
`htui-store` (D7).

**D3 — Value types** (`htui-core::secret`).
- **`SecretScope { project_id, environment, path }`** is stored in `project.secret_scope` as compact
  JSON (`{"project_id":"…","environment":"dev","path":"/"}`).
  - `SecretScope::parse(&str)` is strict: unknown fields and empty `project_id`/`environment` are
    refused. `path` defaults to `/` and must start with `/`.
  - `SecretScope::to_column()` writes it. M4 is the writer; M3 is the reader.
- **`ResolvedSecrets`** wraps `BTreeMap<String, String>`:
  - `as_map()` returns `&BTreeMap<String, String>` for `from_resolved` and `SessionSpec.env`;
  - `keys()` returns the key names;
  - its `Debug` prints key names only, and `Drop` zeroizes the values.
  - It is not `Clone`, `Serialize` or `Display`.
- **`MachineIdentity { client_id, client_secret }`** holds both as `Zeroizing<String>`. Its `Debug`
  prints `client_id` and `<redacted>`. It has no `Display` and no `Serialize`.
- **`ProviderHealth { base_url, server_ok: bool }`** contains no token and no secret.
- `htui-core` gains `zeroize` (already in the lock; no new compiled crate).

**D4 — A typed error that never carries a value.** `SecretError` variants:
- `NoIdentity`: the keyring holds none.
- `Config(String)`: a bad base URL or a scope parse failure.
- `Unreachable { endpoint, cause }`
- `BadCredentials`: login 401 "Invalid credentials", expired or exhausted secret.
- `IdentityLocked`: login 401 with the lockout text.
- `LoginRefusedEarlier`: see D5.
- `ProjectNotFound`
- `PathNotFound { environment, path }`
- `PermissionDenied { detail }`: 403 `PermissionDenied` or `ProjectMembershipNotFound`, and any
  `secretValueHidden: true`, which names the key.
- `RateLimited { retry_after_secs: Option<u64> }`
- `UnsupportedServer { endpoint }`: the v4 route returns Fastify's "Route … not found" 404. The
  message says "this Infisical predates v0.150; upgrade it".
- `InvalidKey { key }`: the name does not match `^[A-Za-z_][A-Za-z0-9_]*$`.
- `InvalidValue { key }`: NUL in the value.
- `Protocol { endpoint, detail }`: a non-JSON or ill-shaped body, an unexpected status, or a 3xx.

Rules for every variant:
- `endpoint` is a path such as `/api/v4/secrets`, never a URL with query values.
- No variant ever contains a value, the client secret or the token.
- Login errors never quote the server's body; they map status plus a known message.
- Other endpoints may quote the server's `message`, truncated to 200 characters, with `\r` and
  `\n` removed. Those messages name projects, folders and permissions, never values.
- `cause` is `reqwest::Error`'s text with the URL stripped (`without_url()`).
- Key names are not secret (they are what the scrubber's `[REDACTED: KEY]` prints, per ANA-7 §3.4).

**D5 — Login and lockout safety.**
- The access token is held in memory as `Mutex<TokenState>`, with the token in a
  `Zeroizing<String>` and its expiry. It is reused until `expires_at - margin`, where
  `margin = max(60 s, expiresIn / 10)`, and then a fresh login replaces it. Renewal is not used
  (equal TTLs by default; survey §2).
- **A rejected login is never retried.** A 401 from the login endpoint latches the provider into
  `TokenState::Refused`. Every later call returns `LoginRefusedEarlier` **without a request** until a
  new provider is built with a new identity, which M4's identity entry does. One provider therefore
  costs at most one failed attempt, far below the default threshold of 3.
- If a data request answers 401, or 403 `TokenError`, the cached token is dropped and the provider
  logs in **once** more. That is a token refresh, not a retried rejected login: if the second
  login is refused, it latches as above.
- No other automatic retry exists. A 429 or a network error returns its typed error, and M3
  decides.

**D6 — Request shape and merge.**
- The list request always sends `projectId`, `environment`, `secretPath`, `viewSecretValue=true`,
  `expandSecretReferences=true`, `includeImports=true`, **`include_imports=true`** (for v0.150–v0.158),
  `recursive=false` and `includePersonalOverrides=false`.
- **Merge:** the folder's own secrets first. Then the imports are walked **from last to first**,
  and a key already present is kept. The bottom-most import thus wins among imports, and the folder
  beats all imports, matching Infisical's CLI `InjectRawImportedSecret`.
- Only `type: "shared"` entries are used; a `personal` entry is ignored.
- **Fail-closed validation of the merged map:** a hidden value refuses with `PermissionDenied`
  naming the key, a bad name with `InvalidKey`, and a NUL with `InvalidValue`.
- Values are passed through byte for byte. The trailing-newline normalisation is M3's, per the PRD's
  "Carried into M3" list.

**D7 — Keyring slots.**
- Three entries under `SERVICE = "htui"`: `infisical-url`, `infisical-client-id` and
  `infisical-client-secret`.
- `get_machine_identity() -> Result<Option<MachineIdentity>>`:
  - both present → `Some`;
  - both absent → `None`;
  - exactly one present → `Err` naming the missing half (a half-written identity is not "no
    identity").
- `set_machine_identity(&MachineIdentity)` writes both. If the second write fails, the first is
  cleared so no half remains.
- `clear_machine_identity()` removes both.
- `get/set/clear_infisical_url` follow the Qdrant URL's shape.
- `FakeSlots` gains three fields, so `mock_keyring()` covers them unchanged.

**D8 — Base URL policy.**
- Accepted with or without a trailing `/` or `/api`, and normalised to the origin plus an optional
  path prefix.
- **`https` is required except for a loopback host** (`localhost`, `127.0.0.0/8`, `::1`). The
  client secret is sent in the login body, and plaintext to a LAN host would expose it. A self-hosted
  instance behind plain `http` on another host is refused with `Config`, which names the reason.
  *(Override at CONFIRM if the maintainer's instance is http-only on the LAN.)*

**D9 — Client settings.**
- One `reqwest::Client` per provider, with the user agent `htui/<version>`, a 5 s connect timeout
  and a 20 s overall timeout.
- **Redirects off** (`redirect::Policy::none()`): a 3xx becomes `Protocol`, so neither the login
  body nor the bearer token can follow a redirect.
- `ring` is installed once, mirroring `model.rs`. The system proxy is left as the workspace feature
  sets it.

**D10 — Tests owe the PRD's metrics for this milestone.**
- **"Lockout-safe"**: a refused login, then a second call, gives a stub that saw exactly one
  login.
- **"Clear refusal"**: one test per refusal cause against the stub.
- **"No value leaks via an error, log or `Debug`"**: every error variant's `Display` and `Debug`,
  and the provider's and identity's `Debug`, are checked for the stub's secret value, the client
  secret and the token.
- **Live:** a test against a real Infisical, gated on `HTUI_TEST_INFISICAL_URL`,
  `_CLIENT_ID`, `_CLIENT_SECRET`, `_PROJECT_ID` and `_ENVIRONMENT`, which prints a skip line when
  unset. The maintainer runs it once against the self-hosted instance and records the result in
  the phase note.

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/secret.rs` | CREATE | D2–D4: seam, value types, error |
| `crates/htui-core/src/lib.rs` | UPDATE | `pub mod secret;` |
| `crates/htui-core/Cargo.toml` | UPDATE | `zeroize` (workspace) |
| `crates/htui-store/src/secret.rs` | UPDATE | D7: three slots, identity get/set/clear, `FakeSlots` fields |
| `crates/htui-store/src/testkit.rs` | UPDATE | a `fake_machine_identity()` reader for assertions, beside `fake_qdrant_dsn` |
| `Cargo.toml` | UPDATE | workspace `members` + `htui-secrets` workspace dependency |
| `Cargo.lock` | UPDATE | new crate; `zeroize` edge for `htui-core` |
| `crates/htui-secrets/Cargo.toml` | CREATE | D1, D9 |
| `crates/htui-secrets/src/lib.rs` | CREATE | crate doc, `InfisicalProvider`, `InfisicalConfig` re-exports |
| `crates/htui-secrets/src/infisical.rs` | CREATE | D5, D6, D8, D9: client, token state, merge, validation |
| `crates/htui-secrets/src/wire.rs` | CREATE | serde request and response shapes, error body |
| `crates/htui-secrets/tests/support/mod.rs` | CREATE | loopback stub (method, path+query, headers, body recorded; scripted answers) |
| `crates/htui-secrets/tests/infisical.rs` | CREATE | D10 stub-backed behaviour tests |
| `crates/htui-secrets/tests/infisical_live.rs` | CREATE | D10 env-gated live test |
| `README.md` | UPDATE | crate list; the live test's env vars beside `HTUI_TEST_QDRANT_URL` |
| `docs/htui-secrets.md` | CREATE | operator page: identity setup in Infisical, base URL policy, errors and what to do |

## Tasks

TDD per repo convention: each task writes its failing tests first.

### T1: `htui-core::secret` — seam and value types (foundation; serial, first)
- **Files**: `crates/htui-core/src/secret.rs`, `crates/htui-core/src/lib.rs`,
  `crates/htui-core/Cargo.toml`, `Cargo.lock`.
- **Action**: D2–D4. Unit tests:
  - `SecretScope` parse/round-trip/refusals;
  - `ResolvedSecrets` and `MachineIdentity` `Debug` print no value;
  - `SecretError` `Display` for every variant contains no value (compile-time exhaustive
    `match`, so a new variant must add a case).
- **Mirror**: `isolate.rs` future alias and error enum.
- **Validate**: `cargo test -p htui-core secret`; `cargo clippy -p htui-core --all-targets -- -D warnings`.

### T2: Keyring slots (after T1; parallel with T3)
- **Files**: `crates/htui-store/src/secret.rs`, `crates/htui-store/src/testkit.rs`.
- **Action**: D7. Tests under `mock_keyring()`:
  - round trip;
  - both absent → `None`;
  - one half → `Err` naming it;
  - `set_machine_identity` with a broken second write leaves nothing (needs a `Fake` hook or a
    broken-keyring case);
  - `clear` removes both;
  - `mock_keyring_broken()` → `Err`, never `None`;
  - the URL slot round trip.
- **Mirror**: the Qdrant slot functions and their fake branches.
- **Validate**: `cargo test -p htui-store --features test-support secret`.

### T3: `htui-secrets` crate — Infisical client (after T1; parallel with T2)
- **Files**: `Cargo.toml`, `Cargo.lock`, `crates/htui-secrets/**` (all CREATE).
- **Action**: D1, D5, D6, D8, D9. Stub-backed tests first, covering:
  - login + list → map;
  - both imports spellings and every boolean present in the query;
  - the precedence cases (folder beats import, last import beats earlier);
  - a hidden value, a bad key and a NUL each refused, naming the key;
  - 401 login → `BadCredentials` and a second call makes **no** request;
  - the locked text → `IdentityLocked`;
  - Fastify "Route … not found" 404 → `UnsupportedServer`;
  - 404 `NotFound` → `ProjectNotFound`;
  - 404 `SecretPathNotFound` → `PathNotFound`;
  - 403 → `PermissionDenied`;
  - 429 → `RateLimited`;
  - token reuse (two resolves, one login);
  - an expired token → re-login;
  - 403 `TokenError` on data → one re-login, then success;
  - 3xx → `Protocol`, with the redirect target never requested;
  - an `http` non-loopback base URL → `Config`;
  - `health` hits `/api/status` and then logs in;
  - every error's and the provider's `Debug`/`Display` free of the stub's value, the client secret
    and the token.
- **Mirror**: `model.rs` client and stub; `htui-mcp/Cargo.toml` shape.
- **Validate**: `cargo test -p htui-secrets`; `cargo clippy -p htui-secrets --all-targets -- -D warnings`.

### T4: Live test and documents (after T3)
- **Files**: `crates/htui-secrets/tests/infisical_live.rs`, `README.md`, `docs/htui-secrets.md`.
- **Action**: the D10 live test, which resolves the configured scope and asserts a non-empty map
  plus a `list_keys` equal to `resolve().keys()`. The README crate list and test env vars. The
  operator page covers:
  - creating a Universal Auth identity;
  - granting it read on the project and environment;
  - the base URL policy;
  - one line per `SecretError` with what to do.
- **Validate**: `cargo test -p htui-secrets --test infisical_live` (prints the skip line here);
  `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

**Independence.** The T2 and T3 file sets are disjoint: T2 touches only `crates/htui-store/src/{secret,testkit}.rs`,
while T3 touches `Cargo.toml`, `Cargo.lock` and `crates/htui-secrets/**`. T1 precedes both and is the
only other writer of `Cargo.lock`. T4 follows T3 and touches T3's crate.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate (test-support hides dead code)
cargo test -p htui-core secret
cargo test -p htui-store --features test-support secret
cargo test -p htui-secrets
cargo test --workspace --all-features --no-fail-fast        # full gate; htui crate with --test-threads=1 if flaky
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Imports silently lost on v0.150–v0.158 servers | Medium | D6 sends both spellings; a stub test asserts both are in the query |
| Identity lockout from repeated failed logins | Low | D5 latch: one failed login per provider, then no requests; test |
| Secret, client secret or token in an error, log or `Debug` | Low | D3/D4 redacted `Debug`, no value-carrying variant, login body never quoted; leak test over every variant |
| Token or body follows a redirect to another host | Low | D9 redirects off; test |
| Pre-v0.150 self-hosted server | Low | `UnsupportedServer` names the upgrade; test on Fastify's 404 body |
| Plain-`http` self-hosted instance refused by D8 | Medium | Override at CONFIRM; loopback stays allowed for tests |
| A secret key that is not a valid env name blocks every run of the project | Medium | Refusal names the key, so the fix is a rename in Infisical; documented in `docs/htui-secrets.md` |
| `htui-core` gaining `zeroize` changes its compile set | Low | Already in the lock as a transitive dependency; verified by the fact-check |

## Acceptance

- [ ] All tasks complete; every new test written before its implementation
- [ ] Validation passes, featureless clippy included
- [ ] Patterns mirrored, not reinvented
- [ ] No secret value, client secret or token in any `Debug`, `Display` or error (leak test green)
- [ ] Live test run once by the maintainer against the self-hosted Infisical, result in the phase note

## Verified claims

*(filled by the fact-check, before CONFIRM)*

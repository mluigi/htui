# Blueprint: MOD-10 milestone 4 — Settings > Secrets section

**Plan**: `.claude/plans/mod-10-m4-settings-section.plan.md`. D1–D10, OQ-1..4 (all decided as
recommended), T1–T4 and the plan's "Verified claims" table are binding. They were re-checked
against the tree at `7e17fdad` (`hr/MOD-10`, plan confirmed) on 2026-10-06.
**PRD**: `.claude/prds/mod-10-secret-provider.prd.md`. **Earlier blueprints**:
`mod-10-m1-scrubber-hardening.blueprint.md`, `mod-10-m2-infisical-provider.blueprint.md`,
`mod-10-m3-run-start-injection.blueprint.md`.
**Rule for the tree vs. the plan**: where they disagree, the tree wins. Each such point is an
**Amendment** (A-n) with its evidence. Where the plan leaves a detail open, this blueprint fixes
it and says so. Anything not proven in source is marked **VERIFY — implementer must check**.

Conventions inherited unchanged:
- toolchain and lints: MSRV 1.98, edition 2024; workspace lints `unsafe_code = forbid`,
  `missing_debug_implementations`, `unused_qualifications`, `clippy::all` at `-D warnings`;
  `#![warn(missing_docs)]` in `htui` (`lib.rs:10`) and `htui-core`, so every new `pub` item has a
  doc comment;
- a type that holds a secret has a hand-written `Debug` that prints lengths or `<redacted>`; no
  section's `Debug` prints text that came out of a field (`settings/connection.rs` B-8);
- one test-helper set per file (the repo duplicates fixtures per file);
- `--demo` (`Backend::Memory`) never touches the keyring (D10);
- no migration. One query changes (`PgStore::update_project`), so `.sqlx` changes by exactly one
  file out and one file in.

---

## Amendments (plan ≠ tree, or plan underspecified)

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| A-1 | D6: "The reply is the hierarchy tree, so both sections refresh, and a CAS miss uses the shared `CHANGED_ELSEWHERE*` sentences" (i.e. `StoreReply::Hierarchy` / `HierarchyStale` via `hierarchy::cas`) | Every reply reaches every Settings section (`settings/mod.rs` `SettingsTab::on_reply`), and replies carry no correlation. `HierarchySection::on_stale` (`settings/hierarchy.rs:1112`) sets `busy = None` and, in `Mode::Browse`, `notice = RELOADED` = `"reloaded; press p again"` (`:73`); `on_tree` (`:1023`) does `self.busy.take()` and closes an open editor as if its own write had landed. A Secrets-section CAS miss would therefore put "reloaded; press p again" on the Hierarchy section for a key nobody pressed there, and a Hierarchy write in flight could be closed by the Secrets write's tree. The reverse also holds: the Secrets section cannot tell its own scope write's tree from any other `Hierarchy` reply | **Self-naming reply** (the MOD-59 pattern): `StoreReply::SecretScopeWritten { project, tree, outcome: ScopeWrite }` (§B.5). The Secrets section lands its write on this variant alone. The Hierarchy section gets **one passive arm** that adopts `tree` when it is the scope's workspace and touches nothing else (no `busy`, no `mode`, no `notice`, no `SetScope`), so D6's "both sections refresh" still holds. The shared `CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED` and `DELETED_ELSEWHERE` sentences (`settings/mod.rs`) are the Secrets section's CAS wording, as D6 says. `settings/hierarchy.rs` is in T1 already (the literal); T3 adds the arm |
| A-2 | Files table: T2a/T2b touch `store_worker.rs` and `agent_worker.rs` for the runtime routing | `Harness::drive` keeps its own hand-written list of runtime-served requests (`testkit.rs:278-297`, documented as "written out by hand … every request the runtime owns has to be added to it deliberately"). A check missing from it falls through to `store_worker::serve` and answers "no agent runtime in this build" from a harness that has one | **T2a and T2b also edit `crates/htui/src/testkit.rs`**: `CheckSecretProvider` (T2a) and `CheckSecretScope { .. }` (T2b) join that or-pattern |
| A-3 | T1: "A conformance case covers set, clear and a stale token" | `conformance::CASES` has two count pins outside the plan's file set: `crates/htui-core/tests/mem_store.rs:36` (`148`, with a sentence listing every milestone) and `crates/htui-store/tests/pg_conformance.rs:34` (`EXPECTED_CASES: usize = 148`) | New case `project_secret_columns_set_clear_cas`, appended to `CASES` and `run_case`. **T1's file set gains both pin files**: `148 → 149`, and the sentence gains ", and MOD-10 M4's one for the secret columns (plan D7)" |
| A-4 | D5: "storing a new identity clears [the latch], because the next `provider()` rebuilds"; `docs/htui-secrets.md` (Logins section): "To try again, enter the identity again" | `KeyringInfisical::current` (`crates/htui/src/secrets.rs:131-158`) reuses the cached provider whenever `Cached::built_from(url, client_id, secret_digest)` holds. Re-entering the **same** identity (the `IdentityLocked` case, where the credentials were right; or a server-side fix with an unchanged secret) changes none of the three, so the latched provider is kept and every walk, chat and check answers `LoginRefusedEarlier` until the process restarts | **A keyring-write generation** in `crates/htui/src/secrets.rs` (§B.3): every successful Settings keyring write (URL or identity, set or clear) calls `secrets::note_keyring_write()`, and `built_from` also compares the generation read at the start of `current`. Any Settings write therefore rebuilds on the next `provider()`, which is what D5 and the docs promise. **T2a's file set gains `crates/htui/src/secrets.rs`** |
| A-5 | D10: "`--demo` never touches the keyring" | `lib.rs::run` builds `KeyringInfisical` and passes `Some(secrets)` to `spawn_hosted` **under `--demo` too** (`lib.rs:170-180`, `store_worker.rs:2192-2216` hands it to both runtimes). Until M4 that was unreachable (no demo project could have a provider). M4 makes `SetProjectSecretScope` work on `MemStore` (it goes through `hierarchy::serve`, which runs on a memory writer), so a demo chat, walk or `t` on a scoped project would read the developer's real keyring | **`lib.rs` passes no source under `--demo`**: a private `fn secret_source(demo: bool) -> Option<Arc<dyn SecretSource>>` (§B.9), unit-tested. A demo walk or chat on a scoped project is then refused with M3's `NO_SECRET_SOURCE` sentence, and a check with `NO_SOURCE_TO_CHECK` (§B.4). The section also refuses `t` in demo on its own (§B.7), so no request is sent at all |
| A-6 | D9: "one construction site in `qdrant.rs`" switches to the redacting newtype | The construction reads `editor.input.text().unwrap_or("")` (`settings/qdrant.rs:172`). `TextField::text` is `None` for a masked field (`text_field.rs:240`, pinned by `text_is_none_when_masked`, `:695`), so the key path **always sends `""`**, and the worker's arm treats an empty key as **clear** (`store_worker.rs:2637-2650`). Typing a Qdrant API key in Settings today deletes the stored one | The construction site uses `TextField::take` into a `Zeroizing`, then `Redacted::new(raw.trim().to_owned())` (§B.2). **Behaviour change**: a typed key is stored for the first time. An empty submit still clears (unchanged worker rule). Pinned by `a_typed_qdrant_key_is_sent_whole_and_redacted` (§D.2), red today |
| A-7 | D4: Identity row shows `half stored: …` distinct from `unreadable: …` | Both are `Err(StoreError::Backend(_))` at the seam: `half_identity` (`htui-store/src/secret.rs:469-474`) and `backend("read", …)` (`:~575`). Only the message differs | **One prefix constant at the seam**: `pub const HALF_STORED_IDENTITY: &str = "the Infisical machine identity is half stored"` in `htui-store/src/secret.rs`, used by `half_identity`'s `format!` (sentence byte-identical) and by the snapshot's classifier (`starts_with`). No value is in either message (slot names only), so the row shows the seam's sentence verbatim. **T2a's file set gains `crates/htui-store/src/secret.rs`** (lane A touches `htui-store/src/pg/write.rs` only: disjoint) |
| A-8 | D2: "`Backend::Memory` … refuses the writes with Connection's demo sentence (D10)" | `connection::DEMO_SESSION` is `"a demo session has no DSN to change"` (`connection.rs` ~`:251`): the wrong noun for an Infisical write | `secrets_settings::DEMO_SESSION = "a demo session has no keyring to change"`, same shape and role |
| A-9 | Files table: `Redacted` / `IdentityEntry` in `store_worker.rs` | `store_worker.rs` is 5 620 lines; the request field types each live in their own module (`htui_store::Dsn`, `htui_agent::auth::loopback::RedirectUrl`, `hand_written::HandText`) | Both live in `crate::secrets_settings` (T2a, new file), beside their zeroize and `Debug` tests; `store_worker.rs` and `qdrant.rs` import them |
| A-10 | D5/D8: `SecretCheck::Provider { at, outcome }`, `SecretCheck::Scope { project, at, outcome: Result<usize, String> }`; row times `14:02:11` and `14:03` | `SecretError` derives `Clone, PartialEq, Eq` and carries no value (`htui-core/src/secret.rs` ~`:281`) | Outcomes are typed: `Result<ProviderHealth, SecretError>` and `Result<usize, SecretError>`; the section renders `Display`. Both rows format `%H:%M:%S` (UTC, as Connection's Status row) |
| A-11 | D6: "`crate::hierarchy::serve` handles it like `UpdateProject`" | `hierarchy::REQUEST_NAMES` is the Hierarchy section's `Failed` matcher (`settings/hierarchy.rs` ~`:1398`, `REQUEST_NAMES.contains(request)`) | `set_project_secret_scope` is **not** added to `REQUEST_NAMES` (that would hand its refusals to the Hierarchy section). The name is `secrets_settings::SET_PROJECT_SECRET_SCOPE`; `try_serve` routes the variant through the hierarchy or-ed arm (comment: "thirteen … plus MOD-10 M4's scope write") |

**11 amendments.** A-1, A-4, A-5 and A-6 change behaviour relative to the plan's wording; A-2,
A-3, A-7 and A-9 only move file sets; A-8, A-10 and A-11 fix details.

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-core/src/secret.rs` | UPDATE | T1 | `SecretScope` gains `Serialize, Deserialize` with `#[serde(try_from = "ScopeColumn", into = "ScopeColumn")]`; `impl TryFrom<ScopeColumn> for SecretScope` (`Error = SecretError`, via `Self::new`); `impl From<SecretScope> for ScopeColumn`; `to_column` re-expressed over the `From` (bytes unchanged). Tests. **Not**: `ScopeColumn`'s fields or attributes, `parse`'s sentence, any other item |
| 2 | `crates/htui-core/src/model/hierarchy.rs` | UPDATE | T1 | `ProjectPatch.secret: Option<Option<SecretScope>>` with doc; `use crate::secret::SecretScope;`. **Not**: `NewProject`, `RepoPatch`, derives |
| 3 | `crates/htui-core/src/store/mem.rs` | UPDATE | T1 | `State::update_project` writes both columns from `patch.secret`; `set_project_secret_columns`'s doc: "the validated writer is `update_project`'s `secret` (M4); this stays the tests' unvalidated planter" |
| 4 | `crates/htui-core/src/store/conformance.rs` | UPDATE | T1 | `CASES` + `run_case` gain `project_secret_columns_set_clear_cas`; the case. `project_create_update_cas` unchanged |
| 5 | `crates/htui-core/tests/mem_store.rs` | UPDATE | T1 (A-3) | `148 → 149` and the sentence |
| 6 | `crates/htui-store/src/pg/write.rs` | UPDATE | T1 | `update_project`'s SQL and binds (§B.1) |
| 7 | `crates/htui-store/tests/pg_conformance.rs` | UPDATE | T1 (A-3) | `EXPECTED_CASES = 149`, message "(149 since MOD-10 M4's secret-column case)" |
| 8 | `crates/htui-store/.sqlx/` | UPDATE | T1 | `query-11715e1015dcc19e7368790cae536658cd5ef884810c316cde12037bf80e3dbe.json` deleted, one new `query-<hash>.json`. Nothing else moves |
| 9 | `crates/htui/src/ui/tabs/settings/hierarchy.rs` | UPDATE | T1, T3 (A-1) | T1: the `ProjectPatch` literal (`:978`) gains `secret: None`. T3: one passive `StoreReply::SecretScopeWritten` arm in `on_reply` (§B.8) |
| 10 | `crates/htui-store/src/secret.rs` | UPDATE | T2a (A-7) | `pub const HALF_STORED_IDENTITY`; `half_identity` formats with it. Sentence bytes unchanged |
| 11 | `crates/htui/src/secrets_settings.rs` | CREATE | T2a, T2b | `Redacted`, `IdentityEntry`, `SecretsSnapshot`, `UrlState`, `IdentityState`, `SecretCheck`, `snapshot`, `serve`, name constants and sentences (§B.2–B.4) |
| 12 | `crates/htui/src/lib.rs` | UPDATE | T2a | `pub mod secrets_settings;`; `secret_source(demo)` and its use in `run` (A-5) + unit test |
| 13 | `crates/htui/src/secrets.rs` | UPDATE | T2a (A-4) | `KEYRING_WRITES`, `note_keyring_write`, `Cached.generation`, `built_from` compares it; one test |
| 14 | `crates/htui/src/store_worker.rs` | UPDATE | T2a, T2b | Request and reply variants, `name()` arms, `try_serve` arms, the loop's runtime arm, `SetQdrantApiKey(Redacted)` and its loop arm (§B.5); unit tests (loop routing, names) |
| 15 | `crates/htui/src/agent_worker.rs` | UPDATE | T2a, T2b | `serve` arms `CheckSecretProvider` (T2a), `CheckSecretScope` (T2b); `check_provider`, `check_scope`; two last-word fns (§B.6) |
| 16 | `crates/htui/src/testkit.rs` | UPDATE | T2a, T2b (A-2) | `drive`'s runtime list gains the two checks |
| 17 | `crates/htui/src/ui/tabs/settings/qdrant.rs` | UPDATE | T2a (A-6) | Key submit builds `Redacted` from `take()` |
| 18 | `crates/htui/src/hierarchy.rs` | UPDATE | T2b | `ScopeWrite`; `SetProjectSecretScope` arm; private `fresh_tree` helper (§B.5). **Not**: `reread`, `cas`, `REQUEST_NAMES` |
| 19 | `crates/htui/tests/secrets_settings.rs` | CREATE | T2a → T2b → T3 | Worker half (T2a, T2b) then section half (T3) |
| 20 | `crates/htui/src/ui/tabs/settings/secrets.rs` | CREATE | T3 | `SecretsSection` (§B.7) and its private unit tests |
| 21 | `crates/htui/src/ui/tabs/settings/mod.rs` | UPDATE | T3 | `pub mod secrets;` `pub use secrets::SecretsSection;` |
| 22 | `crates/htui/src/app/mod.rs` | UPDATE | T3 | `Box::new(SecretsSection::new())` appended after `PersonasSection` with the "Last … appending moves no existing section's line" comment |
| 23 | `crates/htui/tests/settings.rs` | UPDATE | T3 | `the_section_strip_fits_the_frame` lists nine sections (`assert_eq!(sections.len(), 9)`), doc updated |
| 24 | `crates/htui/tests/snapshots/secrets_settings__*.snap` | CREATE | T3 | Six frames (§D.4) |
| 25 | `docs/htui-secrets.md` | UPDATE | T4 | `## Settings` section; the two "not yet" passages (`:11-13`, `:70-71`) replaced; latch note (§F, T4 content) |

No migration. No change to `htui-secrets`, `htui-orch`, `htui-worker`, `htui-agent`.

---

## B. Interfaces, exactly

### B.1 T1 — the secret-column writer (D7)

**`SecretScope` serde** (`htui-core/src/secret.rs`):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ScopeColumn", into = "ScopeColumn")]
pub struct SecretScope { /* unchanged */ }

impl TryFrom<ScopeColumn> for SecretScope {
    type Error = SecretError;
    /// [`SecretScope::new`]'s checks: deserialising validates (MOD-10 M4 D7).
    fn try_from(column: ScopeColumn) -> Result<Self, SecretError> {
        Self::new(column.project_id, column.environment, column.path)
    }
}

impl From<SecretScope> for ScopeColumn {
    fn from(scope: SecretScope) -> Self {
        Self { project_id: scope.project_id, environment: scope.environment, path: scope.path }
    }
}
```

- `parse` keeps its own error mapping (its sentence escapes control characters; it is pinned).
  It may call `serde_json::from_str::<ScopeColumn>` then `Self::try_from` exactly as today.
- `to_column` becomes `serde_json::to_string(&ScopeColumn::from(self.clone()))` with the same
  `expect` and the same bytes (field order is the struct's).
- **VERIFY — implementer must check**: `impl TryFrom<ScopeColumn> for SecretScope` with a private
  `ScopeColumn` raises no `private_interfaces` / `private_bounds` warning under `-D warnings` (a
  trait impl's visibility is the minimum of its header's types, so it should not). If it does,
  make `ScopeColumn` `pub(crate)`, never `pub`.

**`ProjectPatch`** (`htui-core/src/model/hierarchy.rs:102`):

```rust
    /// `project.secret_provider` and `project.secret_scope`, written together (MOD-10 M4 D7):
    /// `None` keeps both, `Some(None)` clears both, `Some(Some(scope))` writes provider
    /// [`INFISICAL`](crate::secret::INFISICAL) and `scope.to_column()`. A provider without a scope
    /// cannot be stored through this field.
    pub secret: Option<Option<SecretScope>>,
```

Derives stay `Debug, Clone, Default, PartialEq, Serialize, Deserialize`; they compile because
`SecretScope` now has all of them. (Serde's default `Option<Option<T>>` reads `null` as `None`,
so a JSON round trip of `Some(None)` is lossy, exactly as `RepoPatch.remote_url` is; nothing
serialises a `ProjectPatch` today — no use in `htui-mcp`. Accepted, H-12.)

**MemStore** (`State::update_project`, `mem.rs:2547`), after the description write and before
`row.updated_at = now`:

```rust
        if let Some(secret) = patch.secret {
            (row.secret_provider, row.secret_scope) = match secret {
                Some(scope) => (Some(crate::secret::INFISICAL.to_owned()), Some(scope.to_column())),
                None => (None, None),
            };
        }
```

**Postgres** (`PgStore::update_project`, `pg/write.rs:2344`):

```rust
        let (secret_set, secret_provider, secret_scope) = match &patch.secret {
            None => (false, None, None),
            Some(None) => (true, None, None),
            Some(Some(scope)) => (true, Some(htui_core::secret::INFISICAL), Some(scope.to_column())),
        };
        let updated = sqlx::query_as!(
            Project,
            r#"
            UPDATE project SET
                slug            = COALESCE($3, slug),
                name            = COALESCE($4, name),
                description     = COALESCE($5, description),
                secret_provider = CASE WHEN $6 THEN $7 ELSE secret_provider END,
                secret_scope    = CASE WHEN $6 THEN $8 ELSE secret_scope END
             WHERE id = $1 AND updated_at = $2
            RETURNING id              AS "id: ProjectId",
                      slug,
                      name,
                      description,
                      secret_provider,
                      secret_scope,
                      settings,
                      created_by      AS "created_by: htui_core::model::UserId",
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.slug,
            patch.name,
            patch.description,
            secret_set,
            secret_provider,
            secret_scope,
        )
```

- `$6` is used twice (one `bool`), so a provider and a scope always move together. `$7`
  (`Option<&str>`) and `$8` (`Option<String>`) take `text` from the `ELSE` column, the
  `remote_url` precedent (`pg/write.rs:2458-2521`).
- `updated_at` is stamped by `trg_project_updated_at` (`0001_init.sql:564-579`), as today.
- The doc comment of `update_project` gains: "`secret` writes both secret columns in this
  statement (MOD-10 M4 D7)".

**Conformance case** `project_secret_columns_set_clear_cas` (generic over `WriteStore`, after
`project_create_update_cas` in the file and in `CASES`): §D.1.

### B.2 T2a — the redacting newtypes (D4, D9; A-6, A-9)

In `crates/htui/src/secrets_settings.rs`:

```rust
/// A secret on its way through [`StoreRequest`](crate::store_worker::StoreRequest), which derives
/// `Debug` and `Clone` (MOD-10 M4 D9): cloned zeroizing, wiped on drop, printed as
/// `Redacted(<redacted>)`. No `Display`, `Serialize` or `PartialEq`.
#[derive(Clone)]
pub struct Redacted(Zeroizing<String>);

impl Redacted {
    /// Wraps `text`, taking its allocation (no copy).
    #[must_use]
    pub fn new(text: String) -> Self { Self(Zeroizing::new(text)) }
    /// The secret. For the keyring write only; never format it.
    #[must_use]
    pub fn expose(&self) -> &str { &self.0 }
    /// Whether it is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

impl core::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Redacted(<redacted>)")
    }
}

/// A machine identity as typed in `Settings > Secrets` (D4). `Clone`, unlike
/// [`MachineIdentity`], so it can ride a `StoreRequest`. `Debug` is `IdentityEntry(<redacted>)`:
/// the client ID is not shown either (OQ-4).
#[derive(Clone)]
pub struct IdentityEntry {
    client_id: Zeroizing<String>,
    client_secret: Redacted,
}

impl IdentityEntry {
    /// Both halves; the section trims them and refuses a blank one before building this.
    #[must_use]
    pub fn new(client_id: String, client_secret: Redacted) -> Self;
    /// The client ID.
    #[must_use]
    pub fn client_id(&self) -> &str;
    /// The client secret. For the keyring write only.
    #[must_use]
    pub fn expose_client_secret(&self) -> &str;
    /// The worker's copy for `htui_store::secret::set_machine_identity`.
    pub(crate) fn to_identity(&self) -> MachineIdentity;
}
// impl Debug -> "IdentityEntry(<redacted>)"
```

`zeroize::Zeroizing<String>: Clone` (zeroize 1.x implements `Clone` for `Zeroizing<Z: Zeroize +
Clone>`); a clone is a fresh zeroizing allocation.

**Qdrant fold-in** (D9, A-6):

- `store_worker.rs:823`: `SetQdrantApiKey(crate::secrets_settings::Redacted)` with doc "The API
  key, redacted in `Debug` (MOD-10 M4 D9); empty clears the stored key."
- `settings/qdrant.rs` `on_editor_key`, `Mode::EditingKey` on `Submit`:
  ```rust
  let raw = zeroize::Zeroizing::new(editor.input.take());
  key_text = Some(Redacted::new(raw.trim().to_owned()));
  ```
  and the send becomes `StoreRequest::SetQdrantApiKey(key_text)`. The URL branch is untouched.
- The loop arm (`store_worker.rs:2637`): `let key = key.clone();` moved into the
  `spawn_blocking` closure, `key.expose().is_empty()` → clear, else `set_qdrant_api_key(key.expose())`.
  The `as_str().to_string()` unzeroized copy goes. The arm's `.unwrap()` stays (not in scope,
  plan D2 names only that it is not copied).

### B.3 T2a — `crates/htui/src/secrets.rs` (A-4)

```rust
/// MOD-10 M4 (blueprint A-4): how many keyring writes `Settings > Secrets` has made in this
/// process. A provider is reused only while this is unchanged, so entering the identity again —
/// even the same one — gives the next walk, chat or check a fresh provider without the old
/// one's login latch (M2 D5).
static KEYRING_WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Called by `secrets_settings` after every successful URL or identity write or clear.
pub(crate) fn note_keyring_write() { KEYRING_WRITES.fetch_add(1, Ordering::SeqCst); }
```

- `Cached` gains `generation: u64`; `built_from(&self, generation, url, client_id, digest)`
  compares it first.
- `current` loads `KEYRING_WRITES` **before** the keyring read (a write that lands during the read
  then forces one more rebuild next time, never one too few) and stores it in the new `Cached`.
- `KeyringInfisical`'s `Debug` is unchanged.

### B.4 T2a — `crates/htui/src/secrets_settings.rs` (D2, D3, D10; A-7, A-8)

**Names and sentences** (`pub const`, each with a doc line):

```rust
pub const REQUEST_NAMES: [&str; 5] =
    ["secrets_info", "set_infisical_url", "clear_infisical_url", "set_machine_identity", "clear_machine_identity"];
pub const READ_NAME: &str = REQUEST_NAMES[0];
pub const CHECK_SECRET_PROVIDER: &str = "check_secret_provider";
pub const CHECK_SECRET_SCOPE: &str = "check_secret_scope";            // T2b uses it
pub const SET_PROJECT_SECRET_SCOPE: &str = "set_project_secret_scope"; // T2b uses it (A-11)
pub const DEMO_SESSION: &str = "a demo session has no keyring to change";                 // A-8
pub const IDENTITY_INCOMPLETE: &str = "the machine identity needs both a client ID and a client secret";
pub const NO_SOURCE_TO_CHECK: &str = "this session has no secret provider to check";      // D5, A-5
pub const NO_PROVIDER_TO_CHECK: &str = "the project has no secret provider";              // D8 (T2b)
```

**Snapshot**:

```rust
/// What `Settings > Secrets` shows of the keyring (D2). Never a value: the URL is its normalised
/// form (no user info, query or fragment), the identity a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsSnapshot { pub url: UrlState, pub identity: IdentityState }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlState {
    /// `Backend::Memory`: no keyring consulted (D10).
    NotApplicable,
    NotStored,
    /// The stored URL through `htui_secrets::normalise_base_url`.
    Stored(String),
    /// Stored, but normalisation refuses it: its `SecretError` sentence, never the stored text.
    Unusable(String),
    /// The seam's sentence, without `StoreError`'s prefix (`connection::seam_sentence`'s rule).
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityState {
    NotApplicable,
    NotStored,
    Stored,
    /// One half present: the seam's sentence, which names the missing slot (A-7).
    HalfStored(String),
    /// The keyring could not be read: the seam's sentence. Never shown as `NotStored`.
    Unreadable(String),
}

/// One check's answer (D5, D8; A-10). Counts and sentences only: never a key name or value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretCheck {
    Provider { at: DateTime<Utc>, outcome: Result<ProviderHealth, SecretError> },
    Scope { project: ProjectId, at: DateTime<Utc>, outcome: Result<usize, SecretError> },
}
```

**`pub async fn snapshot(backend: &Backend) -> Result<SecretsSnapshot>`**:
`Backend::Memory` → both `NotApplicable`, nothing read. Otherwise **one** `spawn_blocking` that
calls `get_infisical_url()` then `get_machine_identity()` and classifies inside the closure (the
`MachineIdentity` is dropped there, wiped):

- URL: `Ok(None)` → `NotStored`; `Ok(Some(raw))` → `normalise_base_url(&raw)` → `Stored(n)` or
  `Unusable(err.to_string())` (`raw` wrapped in `Zeroizing` and dropped; never cloned out);
  `Err(e)` → `Unreadable(seam_sentence(&e))`.
- Identity: `Ok(Some(_))` → `Stored`; `Ok(None)` → `NotStored`;
  `Err(StoreError::Backend(m)) if m.starts_with(HALF_STORED_IDENTITY)` → `HalfStored(m)`;
  `Err(e)` → `Unreadable(seam_sentence(&e))`.
- A failed join → `Err(StoreError::Backend(format!("keyring task failed: {err}")))`, never a
  panic. `seam_sentence` is a private copy of `connection.rs`'s (it is private there; one more
  copy beats widening that module's surface).

**`pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>`**, routed
from `try_serve` for exactly the five `REQUEST_NAMES` variants:

| Request | Order |
|---|---|
| `SecretsInfo` | `Ok(StoreReply::Secrets(snapshot(backend).await?))` |
| `SetInfisicalUrl(url)` | Memory → `Failed { DEMO_SESSION }`; `normalise_base_url(url)` → `Err(e)` → `Failed { e.to_string() }` (defensive: the section already normalised); `spawn_blocking(set_infisical_url(&normalised))` (join error → `Err(Backend)`; keyring error → `Err(e)`); `note_keyring_write()`; fresh snapshot |
| `ClearInfisicalUrl` | Memory → `DEMO_SESSION`; `clear_infisical_url`; note; snapshot |
| `SetMachineIdentity(entry)` | Memory → `DEMO_SESSION`; a blank (after `trim`) half → `Failed { IDENTITY_INCOMPLETE }` (a blank half reads as absent and would store a half identity); `spawn_blocking(set_machine_identity(&entry.to_identity()))`; note; snapshot |
| `ClearMachineIdentity` | Memory → `DEMO_SESSION`; `clear_machine_identity`; note; snapshot |
| anything else | `Err(StoreError::Backend(format!("not a secrets request: {}", other.name())))` |

`Failed` replies are `Ok(StoreReply::Failed { request: request.name(), message })`. A keyring
error propagates as `Err` and `try_serve`'s caller renders it (`store backend error: cannot store
the keyring entry (htui/…): …`), as Connection's `SetDsn` does. The entry is moved into the
closure by `clone()`; nothing unzeroized is made.

**Routing** (`store_worker.rs`): one new or-ed arm in `try_serve`
`SecretsInfo | SetInfisicalUrl(_) | ClearInfisicalUrl | SetMachineIdentity(_) | ClearMachineIdentity
=> secrets_settings::serve(backend, request).await?`. The **loop needs no arm**: these need no loop
state, so its `other => try_serve(..)` serves them (inline, `spawn_blocking` keyring I/O, the
Connection read's accepted cost, H-7), and the harness and `--demo` get the same answer.

### B.5 Request and reply variants (`store_worker.rs`)

Appended to `StoreRequest` after `ClearQdrantSettings`, under one comment
`// MOD-10 milestone 4: Settings > Secrets.`:

```rust
    /// The keyring rows of `Settings > Secrets` (D2). Answered with [`StoreReply::Secrets`].
    SecretsInfo,
    /// Store the Infisical base URL, already normalised on the UI task (D3). Not a secret: a
    /// normalised URL has no user info, query or fragment.
    SetInfisicalUrl(String),
    /// Remove the stored URL.
    ClearInfisicalUrl,
    /// Store both halves of the machine identity (D4), redacted in `Debug`.
    SetMachineIdentity(crate::secrets_settings::IdentityEntry),
    /// Remove both halves.
    ClearMachineIdentity,
    /// Reachability plus a fresh login through the process's secret source (D5). Served by the
    /// agent runtime's own task (`R-NF-3`); answered once with [`StoreReply::SecretCheck`] or
    /// [`StoreReply::Failed`].
    CheckSecretProvider,
    /// How many keys `project`'s scope shows (D8). Served like [`Self::CheckSecretProvider`].
    CheckSecretScope { project: ProjectId },                                       // T2b
    /// Set or clear a project's secret scope, CAS on `project.updated_at` (D6). Answered with
    /// [`StoreReply::SecretScopeWritten`] (A-1).
    SetProjectSecretScope { id: ProjectId, expected: DateTime<Utc>, scope: Option<SecretScope> }, // T2b
```

`name()` arms (string literals, the fn stays `const`): `"secrets_info"`, `"set_infisical_url"`,
`"clear_infisical_url"`, `"set_machine_identity"`, `"clear_machine_identity"`,
`"check_secret_provider"`, `"check_secret_scope"`, `"set_project_secret_scope"`, under a comment
"The five of `secrets_settings::REQUEST_NAMES`, in that order, then the checks and the scope write
(MOD-10 M4)".

Appended to `StoreReply` (before `Failed`):

```rust
    /// `SecretsInfo` and every keyring write's success (D2): the section re-renders from it.
    Secrets(crate::secrets_settings::SecretsSnapshot),
    /// One check's answer, from the agent runtime's task (D5, D8).
    SecretCheck(crate::secrets_settings::SecretCheck),
    /// Answer to [`StoreRequest::SetProjectSecretScope`] (D6, A-1): the workspace re-read after
    /// the write and whether it applied. Self-naming: the Secrets section lands its write on
    /// this alone; the Hierarchy section only adopts `tree`.
    SecretScopeWritten {
        /// The project written.
        project: ProjectId,
        /// The tree of the workspace `hierarchy::workspace_of` found the project in.
        tree: Box<HierarchySnapshot>,
        /// Applied, or the token was spent.
        outcome: crate::hierarchy::ScopeWrite,
    },
```

**VERIFY — implementer must check** `clippy::large_enum_variant` on both enums after the change;
box the offending payload (`SecretCheck` first) if it fires.

**`try_serve` arms**: the five keyring requests (B.4); `CheckSecretProvider` (T2a) and
`CheckSecretScope { .. }` (T2b) join the "no agent runtime in this build" or-pattern
(`store_worker.rs:1828-1849`, its comment's count "eighteen" → "twenty"); `SetProjectSecretScope
{ .. }` joins the hierarchy or-pattern (A-11).

**Loop** (`spawn_with_concepts`, runtime arm `:2677-2703`): the two checks join the or-pattern that
calls `runtime.serve`; its comment names them.

**`hierarchy.rs`** (T2b):

```rust
/// What a scope write did (MOD-10 M4 D6, blueprint A-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeWrite {
    /// The columns were written.
    Applied,
    /// The token was spent; nothing was written.
    Stale,
}

// in `serve`:
StoreRequest::SetProjectSecretScope { id, expected, scope } => {
    // Before the write, for `UpdateProject`'s reason (`:313`).
    let ws = workspace_of(backend, *id).await?;
    let patch = ProjectPatch { secret: Some(scope.clone()), ..ProjectPatch::default() };
    let outcome = match writer.update_project(*id, *expected, patch).await? {
        CasOutcome::Applied(_) => ScopeWrite::Applied,
        CasOutcome::Stale(_) => ScopeWrite::Stale,
    };
    Ok(StoreReply::SecretScopeWritten {
        project: *id,
        tree: Box::new(fresh_tree(&writer, ws, this_box).await?),
        outcome,
    })
}

/// The tree for a write, or `NotFound` when the workspace vanished under it.
async fn fresh_tree(writer: &Writer, ws: WorkspaceId, this_box: Option<BoxId>) -> Result<HierarchySnapshot>;
```

Offline (`writer()` is `None`) it answers `Unreachable(DATABASE_UNREACHABLE)`, as every hierarchy
write does. `reread` and `cas` are not touched.

### B.6 Agent runtime (`agent_worker.rs`; D5, D8)

`serve` gains, before `other =>`:

```rust
StoreRequest::CheckSecretProvider => self.check_provider(replies, addr),
StoreRequest::CheckSecretScope { project } => {                                   // T2b
    match self.check_scope(backend, replies, addr, *project).await {
        Ok(served) => served,
        Err(err) => Served::Reply(failed(CHECK_SECRET_SCOPE, &err)),
    }
}
```

**`fn check_provider(&mut self, replies, addr) -> Served`** (the `preview` shape, `:1725-1764`):

1. `let Some(source) = self.secrets.clone() else { return Served::Reply(StoreReply::Failed {
   request: CHECK_SECRET_PROVIDER, message: NO_SOURCE_TO_CHECK.to_owned() }) };`
2. `let answer = Answer::at(replies.clone(), addr.clone(), provider_check_failed);`
3. `let task = tokio::spawn(answering("secret provider check", async move { let outcome = match
   source.provider().await { Ok(provider) => provider.health().await, Err(e) => Err(e) };
   let _ = replies.send(ReplyEnvelope { seq, origin, reply: StoreReply::SecretCheck(
   SecretCheck::Provider { at: Utc::now(), outcome }) }); }, Some(answer)));`
4. `self.background.push(Background::reading(task)); Served::Deferred`.

`Background::reading`: the task writes no `agent_box` row, so it holds no install claim. It is
swept, awaited by `finish_background`, aborted by `shutdown`, like the preview.

**`async fn check_scope(&mut self, backend, replies, addr, project) -> Result<Served, StoreError>`**:

1. No source → `Ok(Served::Reply(Failed { CHECK_SECRET_SCOPE, NO_SOURCE_TO_CHECK }))`.
2. `backend.project(project)` (the `chat_secrets` read, `:1199-1216`) or `NotFound`.
3. `project_scope(&row)`: `Ok(None)` → `Ok(Served::Reply(StoreReply::SecretCheck(Scope { project,
   at: Utc::now(), outcome: Err(SecretError::Config(NO_PROVIDER_TO_CHECK.to_owned())) })))`;
   `Err(e)` → the same with `Err(e)`. The source is **not** asked in either case.
4. Spawn (as above, last word `scope_check_failed`): `source.provider().await` →
   `provider.list_keys(&scope).await.map(|keys| keys.len())` → reply. The key list is dropped in
   the task; only its length leaves.

No `kind()` comparison: `project_scope` already refuses any provider column but `INFISICAL`, and
`htui-core`'s `kind_mismatch` is private. The demo rule is A-5's (no source), not a backend check
here.

Two last words, beside `preview_failed`:

```rust
fn provider_check_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Failed { request: CHECK_SECRET_PROVIDER, message }]
}
fn scope_check_failed(message: String) -> Vec<StoreReply> { /* CHECK_SECRET_SCOPE */ }
```

**What each caller answers for a check:**

| Caller | Answer |
|---|---|
| Store loop (production) | `runtime.serve` → `Deferred`, then the task's `SecretCheck`; no source (demo, A-5) → `Failed { NO_SOURCE_TO_CHECK }` |
| `Harness::drive` with a runtime | the same, through the runtime (A-2); the task is spawned, so a case waits on it with `drive_to_end()` or a reply-channel `timeout` |
| `Harness::drive` without a runtime | `Failed { "no agent runtime in this harness" }` |
| `Harness::settle`, `store_worker::serve` | `Failed { "no agent runtime in this build" }` (`try_serve`) |

### B.7 T3 — `SecretsSection` (`ui/tabs/settings/secrets.rs`; D1, D3–D6, D8)

**Identity**: `pub const ID: SectionId = SectionId("secrets");` `title()` = `"Secrets"` (strip
`71 → 80` of 100 columns, verified). Appended last in `register_all`.

**Reads**: `wants_requests(scope) = vec![StoreRequest::SecretsInfo,
StoreRequest::SecretsTree(scope.workspace_id)]`. (R1 L-3: the tree is read under this section's own
name, served like `Hierarchy` and answered `SecretsTree(tree)`, so a Secrets read is never taken by
the Hierarchy section for its own write's answer; that section adopts it passively. This replaced
the shared `Hierarchy(ws)` slot of H-9.)

**State** (hand-written `Debug` on every type holding a `TextField`; derived elsewhere):

```rust
pub struct SecretsSection {
    snapshot: Option<SecretsSnapshot>,
    unavailable: Option<String>,                 // Failed { READ_NAME }
    tree: Option<HierarchySnapshot>,             // project rows; scope's workspace only
    cursor: usize,                               // index into rows()
    mode: Mode,
    busy: Option<Write>,                         // one write in flight
    checking: Option<Checking>,                  // one check in flight
    provider_check: Option<(DateTime<Utc>, Result<ProviderHealth, SecretError>)>, // session only
    scope_checks: BTreeMap<ProjectId, (DateTime<Utc>, Result<usize, SecretError>)>,
    notice: Option<Notice>,                      // Info/Error, Debug = kind + len (Connection's)
}
enum Row { Provider, Url, Identity, Health, Project(usize) }
enum Write { Url, ClearUrl, Identity, ClearIdentity, Scope { project: ProjectId, clear: bool } } // fn name() -> &'static str
enum Checking { Provider, Scope(ProjectId) }
enum Mode {
    Browse,
    EditingUrl(TextField),                                        // plain; prefilled with the stored normalised URL
    EditingIdentity { client_id: TextField, client_secret: TextField, focus: usize }, // new() / masked()
    EditingScope { project: ProjectId, expected: DateTime<Utc>, fields: [TextField; 3], focus: usize },
    ConfirmClearUrl,
    ConfirmClearIdentity,
    ConfirmClearScope { project: ProjectId, slug: String, expected: DateTime<Utc> },
}
```

**Rows** (cursor order; no wrap; `j`/`Down`, `k`/`Up`):

| Row | Value column |
|---|---|
| `Provider` | `infisical` |
| `URL` | `NotApplicable` → `n/a in a demo session`; `NotStored` → `not stored`; `Stored(u)` → `stored · {u}`; `Unusable(s)` → `stored — not usable: {s}`; `Unreadable(s)` → `the keyring could not be read: {s}` |
| `Identity` | `n/a in a demo session` / `not stored` / `stored` / `{HalfStored sentence}` / `the keyring could not be read: {s}` |
| `Health` | demo → `n/a in a demo session`; `checking…` while `Checking::Provider`; else `not checked this session`, or `last check {at:%H:%M:%S}: server ok · login ok` / `last check …: server status not ok · login ok` (`server_ok == false`) / `last check …: {SecretError}` |
| then a dim `Projects` line, then one row per `tree.projects` entry, label = project slug | `no secret provider` (`project_scope` → `Ok(None)`); `infisical · {project_id} · {environment} · {path}` (`Ok(Some)`); the `project_scope` refusal sentence (`Err`); suffix ` · checking…` or ` · checked {at:%H:%M:%S}: {n} keys visible` (`1 key visible`) or ` · checked …: {SecretError}` |

`tree == None` → the `Projects` line reads `no workspace: project scopes need one`. Labels for the
four fixed rows pad to 8; project slugs pad to the longest slug. Values wrap with
`super::wrapped` under their column (Connection's `lines`). Under the rows, a dim guide line:
`LATCHED` when the last provider check's error is `BadCredentials`, `IdentityLocked` or
`LoginRefusedEarlier`; otherwise `HEALTH_GUIDE` while the cursor is on `Health`.

**Keys (Browse)** — `t` is unbound globally and in Settings (plan verified):

| Key | Row | Effect |
|---|---|---|
| `e` | URL | `EditingUrl(TextField::with_text(stored) or new())`; demo → refuse `DEMO_KEYRING` |
| `e` | Identity | `EditingIdentity` (both empty; `focus = 0`); demo → `DEMO_KEYRING` |
| `e` | project | `EditingScope` prefilled from the parseable column, else `["", "", "/"]`; `expected = project.updated_at` |
| `e` | Provider / Health | refuse `PROVIDER_FIXED` / `NOTHING_TO_EDIT` |
| `c` | URL / Identity | stored → `ConfirmClearUrl` / `ConfirmClearIdentity`; else refuse with the row's own text (Connection's `dsn_row_refusal` rule; over `Unreadable`/`HalfStored` the clear **is** offered for Identity, since clearing a half is the fix; over `Unreadable` URL it is refused) |
| `c` | project with a provider column | `ConfirmClearScope`; else refuse `NO_SCOPE_TO_CLEAR` |
| `t` | Provider/URL/Identity/Health | demo → refuse `DEMO_CHECK`; a check in flight → refuse `CHECK_IN_FLIGHT`; else `checking = Provider`, send `CheckSecretProvider` |
| `t` | project | `project_scope` not `Ok(Some)` → refuse `NO_SCOPE_TO_CHECK`, **no request**; demo → `DEMO_CHECK`; in flight → `CHECK_IN_FLIGHT`; else `CheckSecretScope { project }` |
| `r` | any | `SecretsInfo` + `Hierarchy(ctx.scope.workspace_id)`; **never refused** |
| `Esc` | any | clears the notice, only when there is one |

`e`/`c` are refused while `busy` (`` `{name}` is still in flight ``) or with no snapshot (the
keyring rows) / no tree (project rows). A check does not block writes and a write does not block a
check (different request kinds, separate slots).

**Forms**: the focused field answers first (`TextField::on_key`); `Tab`/`Down` and
`BackTab`/`Up` cycle focus; other passed keys are swallowed except `CONTROL` chords; `Esc` drops
the form (buffers zeroize). `on_paste` goes to the focused field; a masked paste that does not fit
emits `Action::Error(PASTE_DOES_NOT_FIT)` (Connection's `on_paste`). `captures_input` is `true` in
every mode but `Browse`.

- URL `Enter`: `text().trim()`; empty → Browse + `EMPTY_URL`; `normalise_base_url` →
  `Err(e)` → `Notice::Error(e.to_string())`, field kept, **nothing emitted** (D3); `Ok(n)` →
  Browse, `busy = Url`, send `SetInfisicalUrl(n)`.
- Identity `Enter` (either field): `client_id = text().trim()`;
  `raw = Zeroizing::new(client_secret.take())`; either half blank → `IDENTITY_BLANK`, a fresh
  `TextField::masked()` replaces the secret field, nothing emitted; else Browse, `busy =
  Identity`, send `SetMachineIdentity(IdentityEntry::new(client_id.to_owned(),
  Redacted::new(raw.trim().to_owned())))`.
- Scope `Enter`: `SecretScope::new(f0.trim(), f1.trim(), f2.trim())` → `Err(e)` → notice
  `e.to_string()` minus nothing (it names the field, never the value), form kept, nothing
  emitted; `Ok(s)` → `busy = Scope { project, clear: false }`, send
  `SetProjectSecretScope { id, expected, scope: Some(s) }`, **form stays open until the reply**
  (Hierarchy's rule; a second `Enter` while busy is refused).
- Confirm modes: `y` sends (`ClearInfisicalUrl` / `ClearMachineIdentity` /
  `SetProjectSecretScope { scope: None }`) and returns to Browse with `busy`; `n`/`Esc` back out;
  other keys swallowed, `CONTROL` passed.

**Replies** (`on_reply`):

| Reply | Effect |
|---|---|
| `Secrets(s)` | `unavailable = None`; replace snapshot. A read's answer only: never touches `busy` (R1 M-1) |
| `SecretsWritten { request, generation, snapshot }` | a keyring write's own answer (R1 M-1): `unavailable = None`; replace snapshot; `written_generation = max(written_generation, generation)` (R1 L-1); only if `busy` is the keyring write named `request`, take it and say `URL_STORED` / `URL_CLEARED` / `IDENTITY_STORED` / `IDENTITY_CLEARED` |
| `Hierarchy(Some(t))`, `SecretsTree(Some(t))`, `HierarchyStale(t)`, `RepoPathsInferred { tree: t, .. }` | adopt `t` **only if** `t.workspace.id == ctx.scope.workspace_id`; clamp the cursor; never touches `busy`, `mode`, `notice` (passive) |
| `Hierarchy(None)`, `SecretsTree(None)` | `tree = None`, clamp; an open scope form or question on a project no longer in the tree closes with `DELETED_ELSEWHERE` (R1 L-4) |
| `SecretScopeWritten { project, tree, outcome }` | `busy` taken only if it is `Write::Scope { project: p, .. }` with `p == project`; adopt `tree` (same-workspace rule; otherwise also `ctx.request(SecretsTree(scope ws))`, R1 L-3). `Applied` → Browse, `SCOPE_SAVED` or `SCOPE_CLEARED`, drop `scope_checks[project]`. `Stale` with `EditingScope` open → the project's row in `tree` (any workspace: the token is the row's) → `expected = updated_at`, `CHANGED_ELSEWHERE`; row absent → Browse, `DELETED_ELSEWHERE`; `Stale` with no form → `CHANGED_ELSEWHERE_CLOSED` |
| `SecretCheck(Provider { at, generation, outcome })` | `checking = None` if `Provider`; `provider_check = Some((at, outcome))`; `check_generation = generation`. The latch line shows only while `check_generation >= written_generation`: a provider built before a landed write is rebuilt on the next `provider()` (A-4), whichever reply came first (R1 L-1) |
| `SecretCheck(Scope { project, at, outcome })` | clear `checking` if it is this project; store in `scope_checks` only if the project is in `tree` |
| `Failed { READ_NAME }` | `unavailable = Some(message)`; `busy` untouched: a write's read-back failure answers under the write's name (R1 M-1) |
| `Failed { r }` with `r ∈ REQUEST_NAMES[1..]` or `SET_PROJECT_SECRET_SCOPE` | `busy = None`, `Notice::Error(message)`; confirm modes back to Browse; an open scope form stays; a refused scope write also sends `SecretsTree(scope ws)`, whose tree closes the form if the project is gone (R1 L-4) |
| `Failed { CHECK_SECRET_PROVIDER / CHECK_SECRET_SCOPE }` | clear `checking`, `Notice::Error(message)`; not stored as a check result |
| anything else | ignored |

**`on_scope_change`**: `tree = None`, `scope_checks.clear()`, `mode = Browse` (forms dropped,
zeroized: the D3 disposal points are `Esc`, `take`, scope change), `busy = None` if it was
`Write::Scope`, `checking = None` if it was a scope check, `cursor = 0`, `notice = None`.
`snapshot` and `provider_check` survive (not scoped).

**Sentences** (private `const`s in `secrets.rs` unless noted; `·` is `\u{b7}`, `—` `\u{2014}`,
`…` `\u{2026}`):

```text
NOT_READ            "not read yet"
DEMO_ROW            "n/a in a demo session"
NOT_STORED          "not stored"
UNREADABLE          "the keyring could not be read"
NOT_CHECKED         "not checked this session"
CHECKING            "checking…"
NO_PROVIDER         "no secret provider"
NO_WORKSPACE        "no workspace: project scopes need one"
UNAVAILABLE         "secret settings are unavailable"
URL_STORED          "stored; the next walk, chat or check uses it"
URL_CLEARED         "the Infisical URL is gone from the keyring"
IDENTITY_STORED     "identity stored; the next walk, chat or check logs in with it"
IDENTITY_CLEARED    "the machine identity is gone from the keyring"
SCOPE_SAVED         "scope saved; the next walk or chat on this project uses it"
SCOPE_CLEARED       "scope cleared; walks and chats on this project get no secrets"
EMPTY_URL           "nothing typed; the stored URL is unchanged"
IDENTITY_BLANK      "both the client ID and the client secret are required"
PROVIDER_FIXED      "infisical is the only provider this build knows"
NOTHING_TO_EDIT     "nothing to edit on this row; t checks the provider"
NO_SCOPE_TO_CHECK   "this project has no secret scope to check"
NO_SCOPE_TO_CLEAR   "this project has no secret scope to clear"
CHECK_IN_FLIGHT     "a check is still running"
DEMO_KEYRING        "a demo session never reads or writes the keyring"
DEMO_CHECK          "a demo session has no secret provider to check"
HEALTH_GUIDE        "t logs in afresh: a refused login stops walks, chats and checks from logging in again until the identity is entered again"
LATCHED             "the last login was refused: walks, chats and checks are refused until the identity is entered again (e on Identity)"
CONFIRM_CLEAR_URL       "Remove the Infisical URL from the keyring? Walks and chats on provider projects are refused until one is stored. y / n"
CONFIRM_CLEAR_IDENTITY  "Remove the machine identity (both halves) from the keyring? Walks and chats on provider projects are refused until one is stored. y / n"
confirm_clear_scope(slug) "Remove `{slug}`'s secret scope? Its walks and chats then get no secrets. y / n"
URL field guide     "the base URL, e.g. https://infisical.example.com; it is checked before it is stored"
identity guide      "the client secret is never shown; storing replaces both halves"
scope guide         "Infisical project ID, environment slug and folder path (starts with /)"
form labels         "URL: ", "client ID: ", "client secret: ", "project ID: ", "environment: ", "path: "
```

**Hints** (all ≤ 98 cells, the bordered section width at 100 columns):

```text
HINT_BROWSE        "e edit · c clear · t check · r reload · j/k rows"
HINT_NO_SNAPSHOT   "r reload"
HINT_URL           "Enter store · Esc cancel"
HINT_IDENTITY      "Tab next field · Enter store · Esc cancel · the secret is never shown"
HINT_SCOPE         "Tab next field · Enter save · Esc cancel"
HINT_CONFIRM       "y confirm · n / Esc cancel"
```

In Browse with a write in flight and no notice: `"{keys} · {name} in flight"`. The notice shares
the hint line in Browse and wins it when both do not fit (Connection's `hint`, MOD-60 cells).

### B.8 T3 — the Hierarchy section's passive arm (A-1)

In `HierarchySection::on_reply`, before the `Failed` arms:

```rust
// MOD-10 M4 (blueprint A-1): the Secrets section's scope write. The tree is adopted so the
// tokens stay current; nothing else here is this section's — no `busy`, no editor, no notice,
// no scope follow.
StoreReply::SecretScopeWritten { tree, .. } => {
    if tree.workspace.id == ctx.scope.workspace_id {
        self.snapshot = Some((**tree).clone());
        self.clamp_cursor();
    }
}
```

### B.9 T2a — `lib.rs` (A-5)

```rust
/// MOD-10 D15, M4 blueprint A-5: the process's one secret source, or none under `--demo`, which
/// never touches the keyring (D10). A demo walk, chat or check on a scoped project is refused.
fn secret_source(demo: bool) -> Option<Arc<dyn htui_core::secret::SecretSource>> {
    (!demo).then(|| Arc::new(secrets::KeyringInfisical::new()) as Arc<dyn htui_core::secret::SecretSource>)
}
```

`run` passes `secret_source(args.demo)` to `spawn_hosted` in place of `Some(secrets)`. Unit test
in `lib.rs`'s `mod tests` (or a new one).

---

## C. Data flow

1. **Activation** of the Settings tab: `SettingsTab::wants_requests` collects every section's
   reads → `SecretsInfo`, `SecretsTree(ws)` and Hierarchy's own `Hierarchy(ws)` (R1 L-3) → loop
   `other => try_serve` → `secrets_settings::snapshot` (one `spawn_blocking`, two keyring reads;
   Memory: none) and `hierarchy::serve`. Every reply reaches every section; Secrets keeps the
   snapshot and the tree.
2. **URL**: `e` → field → `Enter` → `normalise_base_url` on the UI task → `SetInfisicalUrl(n)` →
   `serve` (re-normalise, keyring write, `note_keyring_write`) → `SecretsWritten { fresh }` (R1
   M-1) → `URL_STORED`.
3. **Identity**: `e` → two fields → `Enter` → `take()` into `Zeroizing` → `IdentityEntry` →
   `SetMachineIdentity` (Debug `IdentityEntry(<redacted>)`) → `set_machine_identity` (pairwise,
   removes both on a failed second write) → `note_keyring_write` → `SecretsWritten { fresh }`.
4. **Next walk / chat / check**: `KeyringInfisical::provider` reads the keyring, sees a new
   generation (A-4) or changed inputs, builds a fresh `InfisicalProvider`: no latch carried over.
5. **Health**: `t` → `CheckSecretProvider` → loop runtime arm → `AgentRuntime::check_provider` →
   spawned `Background::reading` task → `source.provider()` → `health()` (latch peek, status GET,
   fresh login; a refusal latches the shared provider) → `SecretCheck::Provider` at the request's
   address → Health row.
6. **Scope write**: `e` on a project → three fields → `SecretScope::new` on the UI task →
   `SetProjectSecretScope { id, expected, Some(scope) }` → `hierarchy::serve`: `workspace_of` →
   `update_project(ProjectPatch { secret })` (one statement, both columns, CAS) → `fresh_tree` →
   `SecretScopeWritten` → Secrets closes its form; Hierarchy adopts the tree (A-1).
7. **M3 pickup**: the next walk's `RunSecrets::prepare` / the next chat's `chat_secrets` reads the
   project row → `project_scope` → the new scope. No restart, no cache (Postgres row; MemStore row).
8. **Scope check**: `t` on a scoped project → `CheckSecretScope` → `check_scope`: project row and
   `project_scope` inline (no I/O beyond the row) → task: `provider()` → `list_keys(scope)` →
   `len()` → `SecretCheck::Scope { outcome: Ok(n) }`.
9. **Demo**: rows `NotApplicable`, keyring writes refused at the section and at `serve`; checks
   refused at the section, and the runtime has no source (A-5).

---

## D. Tests, per task (written first; each must fail — red or not compiling — before its implementation)

Fixtures: secrets are long and not pattern-shaped (`zq7-client-secret-0123456789`); a client ID
`cid-typed-1`; a URL with a password `https://user:hunter2@x.example`. Every `Debug` assertion
checks **all** typed values are absent. The `htui` integration file starts
`#![cfg(feature = "testkit")]` and every case that can reach a keyring takes
`htui_store::testkit::mock_keyring()` / `mock_keyring_broken()` as its first statement, except the
`Backend::Memory` cases that prove no keyring is read (a guard would hide the bug).

### D.1 T1

`crates/htui-core/src/secret.rs` `mod tests`:
- `a_scope_serialises_as_its_column`: `serde_json::to_string(&scope)? == scope.to_column()`, and
  `from_str(&scope.to_column())? == scope`.
- `deserialising_a_scope_validates_it`: `{"project_id":"","environment":"dev"}` is `Err` whose text
  contains `empty project_id`; `{"project_id":"p","environment":"dev","path":"x"}` refused (`must
  start with`); an unknown field refused; a missing `path` is `/`.
- `to_column_is_unchanged`: the literal `{"project_id":"p1","environment":"dev","path":"/"}` for
  `SecretScope::new("p1","dev","/")` (pins the bytes M3 reads).

`crates/htui-core/src/store/conformance.rs` — `project_secret_columns_set_clear_cas`:
1. create `new_project("vault-ops")`; `Some(Some(scope))` at its token → `Applied`;
   `(secret_provider, secret_scope) == (Some("infisical"), Some(scope.to_column()))`;
   `project_scope(&row) == Ok(Some(scope))`; slug, name, description, settings unchanged;
   `updated_at` advanced.
2. a `name`-only patch at the new token → the two columns unchanged (`secret: None` keeps).
3. `Some(Some(other))` at the **old** token → `Stale(current)` with the step-2 row; read-back
   columns still `scope`'s.
4. `Some(None)` at the current token → both `None`.
5. `ProjectPatch::default().secret == None`.
Pins: `mem_store.rs` `149` + sentence; `pg_conformance.rs` `EXPECTED_CASES = 149`.

Run: `cargo test -p htui-core --all-features` (MemStore through `tests/mem_store.rs`);
`cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1` (PgStore, with
`HTUI_TEST_DATABASE_URL`).

### D.2 T2a (`crates/htui/tests/secrets_settings.rs` worker half; unit tests where private)

Worker half, through `htui::store_worker::serve` over `Backend::Offline { cache, since }` from a
local `mirror(fingerprint)` helper (copy of `tests/connection.rs:248`) unless named demo:
1. `demo_rows_are_not_applicable_and_read_no_keyring` (no guard): `SecretsInfo` on
   `Backend::memory(MemStore::demo())` → both rows `NotApplicable`.
2. `demo_refuses_every_keyring_write` (no guard): the four writes → `Failed { request: name(),
   message: DEMO_SESSION }`.
3. `an_empty_keyring_is_not_stored_on_both_rows`.
4. `a_stored_url_is_shown_normalised`: raw `https://Infisical.Example.com/api/` →
   `Stored("https://infisical.example.com")`.
5. `a_stored_url_that_does_not_normalise_is_unusable_and_never_echoed`: raw
   `https://user:hunter2@x.example` → `Unusable(_)`; `format!("{reply:?}")` lacks `hunter2`.
6. `a_half_stored_identity_is_half_stored_not_unreadable`:
   `set_machine_identity(&MachineIdentity::new("   ", SECRET))` → `HalfStored(m)` with `m`
   containing `infisical-client-id`; reply `Debug` lacks `SECRET`.
7. `a_broken_keyring_is_unreadable_never_not_stored` (`mock_keyring_broken`): both `Unreadable`
   containing `BROKEN_KEYRING`.
8. `set_infisical_url_stores_the_normalised_form_and_answers_a_fresh_snapshot`.
9. `set_infisical_url_refuses_what_does_not_normalise_and_stores_nothing`: `Failed`; the slot is
   still empty.
10. `set_machine_identity_stores_both_halves_and_answers_a_fresh_snapshot`:
    `fake_machine_identity() == (Some(id), Some(secret))`, row `Stored`.
11. `set_machine_identity_refuses_a_blank_half_and_stores_nothing`: `IDENTITY_INCOMPLETE`.
12. `a_failed_secret_write_is_failed_and_leaves_no_half`: `refuse_fake_store(INFISICAL_CLIENT_SECRET_USER)`
    → `Failed` naming `infisical-client-secret`; both halves `None`.
13. `clear_machine_identity_removes_both_halves`; `clear_infisical_url_removes_the_url`.
14. `no_secret_request_prints_what_was_typed`: `Debug` of `SetMachineIdentity(IdentityEntry::new(
    "cid-typed-1", Redacted::new(SECRET)))`, of `SetQdrantApiKey(Redacted::new("qk-typed-123"))`,
    and of a `RequestEnvelope` around each, contain none of the values and do contain
    `<redacted>`.
15. `a_typed_qdrant_key_is_sent_whole_and_redacted` (A-6, red today): `SectionBench` +
    `QdrantSection`, a local `qdrant_stored()` snapshot reply, `j`, `e`, type `qk-typed-123`,
    `Enter` → exactly one `Action::Store(SetQdrantApiKey(k))` with `k.expose() == "qk-typed-123"`
    and its `Debug` lacking it.
16. `check_secret_provider_answers_server_ok`: `AgentRuntime::new(DriverFactory::new())
    .with_secret_source(Arc::new(FakeSecretSource::new(Arc::new(FakeSecretProvider::resolving(&[])))))`,
    `runtime.serve(&demo backend, &tx, &envelope)` → `Deferred`; `timeout(5s, rx.recv())` →
    `SecretCheck(Provider { outcome: Ok(ProviderHealth { server_ok: true, .. }), .. })` at the
    envelope's `seq`.
17. `check_secret_provider_passes_a_refused_login_through`: a local `RefusingHealth` provider
    (implements `SecretProvider`; `health` → `Err(BadCredentials)`) → `Err(BadCredentials)`.
18. `check_secret_provider_passes_a_source_refusal_through`: `FakeSecretSource::failing(NoIdentity)`
    → `Err(NoIdentity)`.
19. `check_secret_provider_without_a_source_is_failed_with_one_sentence`: `Served::Reply(Failed {
    CHECK_SECRET_PROVIDER, NO_SOURCE_TO_CHECK })`.
20. `a_harness_without_an_agent_runtime_refuses_the_provider_check_by_name`: `Harness::demo()`,
    `Action::Store(CheckSecretProvider)`, `drive()` → status
    `"check_secret_provider: no agent runtime in this harness"` (proves A-2's list).

Unit tests:
- `secrets_settings.rs`: `redacted_debug_is_fixed_and_clone_keeps_the_text`,
  `identity_entry_debug_hides_both_halves`, `the_request_names_are_the_module_constants`
  (`name()` of each of the five equals `REQUEST_NAMES[i]`, of the checks and the scope write equals
  the consts).
- `store_worker.rs` `mod tests`: `the_loop_hands_the_provider_check_to_the_agent_runtime`
  (`spawn_with` + `AgentRuntime::new` without a source → one `Failed` whose message is
  `NO_SOURCE_TO_CHECK`, not "no agent runtime in this build"); `try_serve_refuses_the_provider_check_without_a_runtime`.
- `secrets.rs`: `entering_the_same_identity_again_rebuilds_a_latched_provider` (A-4): counting
  builder over `FakeSecretProvider::new([Err(BadCredentials), Err(LoginRefusedEarlier)])`; first
  `provider()`; `note_keyring_write()`; second `provider()` → `builds.count() == 2`, `!ptr_eq`.
  The existing `an_unchanged_keyring_reuses_the_provider` stays green.
- `lib.rs`: `a_demo_session_has_no_secret_source` (`secret_source(true).is_none()`,
  `secret_source(false).is_some()`; building reads nothing).

Run: `cargo test -p htui --all-features --test secrets_settings -- --test-threads=1`;
`cargo test -p htui --all-features --lib -- --test-threads=1 secrets store_worker lib`.

### D.3 T2b (same integration file, worker half continued)

Over `Backend::memory(store)` with `store = MemStore::demo()` (writes need no keyring):
1. `set_project_secret_scope_writes_both_columns_and_answers_its_own_reply`: token from
   `hierarchy::snapshot`; reply `SecretScopeWritten { project, outcome: Applied, tree }`; the
   tree's row and `store.project(id)` carry `infisical` and the column.
2. `a_written_scope_is_what_the_next_walk_reads`: `project_scope(&row) == Ok(Some(scope))`.
3. `clearing_a_scope_nulls_both_columns`.
4. `a_spent_token_answers_stale_and_writes_nothing`: `outcome: Stale`, columns unchanged, tree is
   the current one.
5. `an_unlinked_project_is_refused_before_anything_is_written` (the `tests/hierarchy.rs:642`
   shape): `Failed { "set_project_secret_scope", .. }` containing `workspace_project`; columns
   untouched.
6. `the_scope_write_is_not_a_hierarchy_request_name`:
   `!hierarchy::REQUEST_NAMES.contains(&SET_PROJECT_SECRET_SCOPE)` (A-11).
7. `check_secret_scope_answers_a_key_count_and_no_names`: `store.set_project_secret_columns(..)`;
   `FakeSecretProvider::resolving(&[("API_KEY", V1), ("DB_URL", V2)])` → `Ok(2)`; reply `Debug`
   lacks `API_KEY`, `V1`.
8. `check_secret_scope_refuses_a_provider_less_project_without_the_source`: `Err(Config(
   NO_PROVIDER_TO_CHECK))`, `FakeSecretSource::calls() == 0`, answered as `Served::Reply`.
9. `check_secret_scope_refuses_a_column_fault_before_the_source`: provider `"vault"` →
   `Err(Config(_))`, `calls() == 0`.
10. `check_secret_scope_passes_a_provider_error_through`: `FakeSecretProvider::failing(
    PermissionDenied { detail: "…" })` → that error.
11. `a_harness_without_an_agent_runtime_refuses_the_scope_check_by_name`.
Unit (`store_worker.rs`): `the_loop_hands_the_scope_check_to_the_agent_runtime`.

Run: as T2a, plus `cargo test -p htui --all-features --test hierarchy -- --test-threads=1`.

### D.4 T3

Section half of `tests/secrets_settings.rs`, through `SectionBench` (`new()`: the demo
`Graphics` workspace). Local helpers: `tree(store)` (`htui::hierarchy::snapshot(&store, ws, None)`),
`snapshot(url, identity)`, `requests(&bench)` (the drained `Action::Store`s). **VERIFY**: the
`Graphics` workspace's project is `ids::PROJECT_VULKAN` (the hierarchy tests use it).

1. `wants_requests_names_the_keyring_read_and_the_tree`.
2. `rows_come_from_the_snapshot_and_the_tree`: rendered text holds `infisical`, `stored ·
   https://…`, `not stored`, `not checked this session`, the slug, `no secret provider`.
3. `e_on_url_sends_the_normalised_url`: typed ` https://Infisical.Example.com/api/ ` →
   `SetInfisicalUrl("https://infisical.example.com")`.
4. `a_refused_url_emits_nothing_and_the_notice_never_repeats_it`: `https://user:hunter2@x.example`
   → no request; the notice is normalisation's sentence and lacks `hunter2`; section `Debug`
   lacks it.
5. `the_identity_form_sends_one_redacted_entry`: `e` on Identity, `cid-typed-1`, `Tab`, SECRET,
   `Enter` → one `SetMachineIdentity(e)` with `e.client_id() == "cid-typed-1"` and
   `e.expose_client_secret() == SECRET`; a render taken **before** `Enter` lacks SECRET and shows
   `•` and `(28)`.
6. `a_blank_identity_half_emits_nothing`.
7. `tab_and_backtab_move_between_the_identity_fields`.
8. `esc_drops_the_identity_form`.
9. `c_on_identity_asks_then_clears` (`n` sends nothing; `c`,`y` sends `ClearMachineIdentity`);
   `c_on_url_asks_then_clears`; `c_on_a_row_with_nothing_stored_is_refused_by_its_row_text`;
   `c_on_a_half_stored_identity_is_offered`.
10. `one_write_in_flight_refuses_e_and_c_but_not_r_or_t`.
11. `t_on_a_keyring_row_sends_one_provider_check` and `a_second_t_while_checking_is_refused`.
12. `t_on_a_provider_less_project_emits_nothing`; `t_on_a_scoped_project_sends_a_scope_check`.
13. `a_provider_check_shows_on_the_health_row` (Ok, `server_ok: false`, Err).
14. `a_latching_refusal_shows_the_latch_line` (`BadCredentials`, `IdentityLocked`,
    `LoginRefusedEarlier`; not for `Unreachable`).
15. `a_scope_check_shows_a_count`: `Ok(12)` → `12 keys visible`; `Ok(1)` → `1 key visible`.
16. `e_on_a_project_opens_the_scope_form_prefilled` (column → fields; none → `["", "", "/"]`).
17. `a_refused_scope_emits_nothing_and_names_the_field`.
18. `enter_on_the_scope_form_sends_the_write_with_the_token_and_stays_open`.
19. `its_own_applied_write_closes_the_form_and_says_so`.
20. `a_stale_scope_write_keeps_the_text_and_takes_the_new_token`; `a_stale_clear_says_nothing_was_written`.
21. `c_on_a_scoped_project_asks_then_sends_scope_none`.
22. `a_hierarchy_reply_refreshes_rows_but_is_never_taken_as_the_scope_write`.
23. `a_tree_of_another_workspace_is_not_adopted`.
24. `the_hierarchy_section_adopts_a_scope_write_tree_without_its_attribution`: a
    `HierarchySection` in Browse fed `SecretScopeWritten { outcome: Stale, .. }` → its render has
    no `reloaded; press p again`, emits nothing; fed `Applied` → still nothing emitted (A-1).
25. `a_scope_change_drops_an_open_form_and_the_project_rows`.
26. `the_section_debug_holds_no_typed_text` (identity form with id and secret typed; URL form
    with `hunter2`; scope form).
27. `a_refused_read_is_unavailable_and_r_recovers`; `a_refused_write_lands_on_the_section`.
28. `demo_refuses_keyring_edits_and_checks_but_edits_scopes`.
29. `every_hint_fits_the_section` (each `HINT_*` ≤ 98 cells via `cell_width`) — unit test in
    `secrets.rs` if the constants stay private; plus `a_wide_notice_takes_the_hint_line_alone`
    (Connection's MOD-60 case, unit).
30. `the_product_registers_secrets_last`: `Harness::demo()` + `register_all`, `4`, eight `l` →
    the frame contains ` Personas  Secrets ` and `HINT_BROWSE`.
31. insta, `bench.render_section(&section, 100)`, fixed `at`s:
    `secrets_settings__not_configured`, `secrets_settings__configured_health_ok`,
    `secrets_settings__health_refused`, `secrets_settings__project_scope_checked`,
    `secrets_settings__identity_form`, `secrets_settings__demo`.

`tests/settings.rs`: `the_section_strip_fits_the_frame` with nine sections.
`personas.rs::the_product_registers_personas_last` and
`connection.rs::the_product_registers_connection_after_prompt` stay green unchanged.

Run: `cargo test -p htui --all-features -- --test-threads=1`, then
`cargo insta test -p htui --all-features` — the only new snapshots are the six above and **no**
other `.snap.new` appears (memory: snapshot impact needs a full insta run).

---

## E. Hazards

- **H-1 HIGH — latch survives re-entry (A-4).** Without the generation, re-typing the same
  identity after an `IdentityLocked` or a server-side fix leaves every walk, chat and check
  refused until restart, while the docs say "enter the identity again". Pinned by
  `entering_the_same_identity_again_rebuilds_a_latched_provider`.
- **H-2 HIGH — demo reads the real keyring (A-5).** M4 makes demo scopes writable; without
  `secret_source(demo)` a demo chat or check on a scoped project reads the developer's real
  Infisical identity. Pinned by `a_demo_session_has_no_secret_source`.
- **H-3 MEDIUM — Qdrant key behaviour change (A-6).** Today a typed key clears the stored one;
  after D9 it is stored. Say so in the T2a commit body and the phase note.
- **H-4 MEDIUM — attribution (A-1).** The Secrets section must never take `Hierarchy`/
  `HierarchyStale` as its write's answer, and the Hierarchy arm must stay passive. Pinned by D.4
  #22 and #24.
- **H-5 Debug leaks.** Every new type holding typed text has a hand-written `Debug`; `Mode`,
  forms and `Notice` print lengths. `SetInfisicalUrl(String)` prints the URL **by design** (only a
  normalised URL is ever sent, so no user info). No `tracing` field carries a value.
- **H-6 Keyring fake is process-wide.** The integration file's keyring cases take the guard
  first; the gate runs `--test-threads=1` (memory). The `secrets.rs` unit tests and the
  `secrets_settings` integration tests are different processes, so `KEYRING_WRITES` cannot
  disturb `builds.count()` assertions.
- **H-7 Keyring I/O on the loop.** `SecretsInfo` and the four writes run their `spawn_blocking`
  **awaited on the loop's arm**, like Connection's read and the Qdrant arms: an OS unlock prompt
  stalls store requests until answered. Accepted by the plan (Risks row); the checks never do this
  (spawned task; `KeyringInfisical`'s own 120 s bound).
- **H-8 `--all-features`.** `tests/*.rs` in `htui` run 0 tests without `testkit` and still say
  `ok` (memory). Every gate carries `--all-features`.
- **H-9 Duplicate `Hierarchy(ws)` read.** Superseded by R1 L-3: Secrets reads its tree as
  `SecretsTree(ws)`, its own `(origin, kind)` slot, so the shared slot is gone. Two tree reads per
  activation, one per section; harmless.
- **H-10 `.sqlx` regeneration.** From `crates/htui-store`, never `--workspace`; always
  `-- --all-targets --all-features` or the feature-gated entries are deleted (memory). Expected
  diff: exactly one `D` and one `??`. Recovery:
  `git checkout -- crates/htui-store/.sqlx && git clean -fq crates/htui-store/.sqlx`.
- **H-11 Featureless clippy.** Run `cargo clippy --workspace -- -D warnings` too: nothing new is
  test-support-gated in a lib, so no dead code is expected; `IdentityEntry::to_identity` is used by
  `serve`.
- **H-12 `Option<Option<SecretScope>>` and serde.** `Some(None)` does not survive a JSON round
  trip (serde reads `null` as `None`), as `RepoPatch.remote_url` today. Nothing serialises a
  `ProjectPatch`; do not add a JSON path for it without `#[serde(with = …)]`.
- **H-13 Shared Postgres.** T1's Postgres gate and lane B's builds must not overlap on a loaded
  box (memory: crash loop is disk or load); check `df -h .` first, re-run serially before calling
  a Postgres failure a regression.
- **H-14 Worktrees.** Lanes A and B run in separate worktrees with their own `CARGO_TARGET_DIR`
  (~10 GB each, `df -h .`); Gortex `edit` writes to the primary checkout, so implementers in a
  linked worktree edit with file tools (memory).
- **H-15 Strip snapshots.** The strip test is the only list of sections; tests that render the
  whole strip through `register_all` (personas, connection) assert substrings that stay true.
  The full insta run settles it.
- **H-16 Hint width.** At 100 columns the bordered section is 98 cells; `HINT_IDENTITY` is the
  longest (~70). The MOD-60 rule (notice wins the line) applies.

**16 hazards; H-1 and H-2 HIGH.**

---

## F. Build sequence, lanes, commits and gates

The plan's lanes are **confirmed**, with A-2, A-3, A-4, A-5 and A-7's file-set additions.

| Lane | Order | Task | Touched files |
|---|---|---|---|
| **A** | T1 | T1 | `crates/htui-core/src/model/hierarchy.rs`, `crates/htui-core/src/secret.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs` (A-3), `crates/htui-store/src/pg/write.rs`, `crates/htui-store/tests/pg_conformance.rs` (A-3), `crates/htui-store/.sqlx/` (1 out, 1 in), `crates/htui/src/ui/tabs/settings/hierarchy.rs` (literal) |
| **B** | T2a | T2a | `crates/htui/src/secrets_settings.rs` (new), `crates/htui/src/lib.rs`, `crates/htui/src/secrets.rs` (A-4), `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`, `crates/htui/src/testkit.rs` (A-2), `crates/htui/src/ui/tabs/settings/qdrant.rs`, `crates/htui-store/src/secret.rs` (A-7), `crates/htui/tests/secrets_settings.rs` (new) |
| — | T2b (after A and B) | T2b | `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`, `crates/htui/src/hierarchy.rs`, `crates/htui/src/testkit.rs`, `crates/htui/tests/secrets_settings.rs` |
| — | T3 (after T2b) | T3 | `crates/htui/src/ui/tabs/settings/secrets.rs` (new), `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/ui/tabs/settings/hierarchy.rs` (A-1 arm), `crates/htui/src/app/mod.rs`, `crates/htui/tests/settings.rs`, `crates/htui/tests/secrets_settings.rs`, `crates/htui/tests/snapshots/secrets_settings__*.snap` |
| — | T4 (after T3) | T4 | `docs/htui-secrets.md` |

**Intersections.**
- Files: A ∩ B = ∅ (checked file by file above; `htui-store` is touched in both lanes but at
  `pg/write.rs` + `tests/pg_conformance.rs` + `.sqlx/` vs `src/secret.rs`; `htui` at
  `ui/tabs/settings/hierarchy.rs` vs the eight others). T2b/T3/T4 are serial: they share
  `store_worker.rs`, `agent_worker.rs`, `testkit.rs`, `settings/hierarchy.rs` and the new test file.
- Semantics: B never constructs a `ProjectPatch` and A never touches `StoreRequest`, so neither
  lane compiles against the other's change. T2b needs both (`ProjectPatch.secret` from A, the
  module and variants from B): merge both commits first.
- Hidden coupling: both lanes build `htui-store` and `htui` (separate worktrees, H-14); the
  keyring fake is per process (H-6); Postgres is shared (H-13). No `Cargo.lock` change anywhere
  (`htui` already depends on `htui-secrets`, `zeroize`, `chrono`); a lock diff is a leak.

**Within each task** (tests first; implementers commit as they go, memory):

1. **T1** (lane A worktree): §D.1 tests red (they do not compile) → `SecretScope` serde →
   `ProjectPatch.secret` + the `settings/hierarchy.rs` literal → MemStore → count pins →
   `cargo test -p htui-core --all-features` green → Postgres SQL → `.sqlx`:
   ```bash
   psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
   cd crates/htui-store
   export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
   cargo sqlx migrate run --source migrations
   cargo sqlx prepare -- --all-targets --all-features
   cargo sqlx prepare --check -- --all-targets --all-features   # "potentially unused" warning is expected
   unset DATABASE_URL
   git status --short .sqlx    # exactly: D query-11715e10…json and ?? one new query-*.json
   cd ../..
   SQLX_OFFLINE=true cargo check -p htui-store --all-features --all-targets
   cargo test -p htui-store --all-features -- --test-threads=1        # HTUI_TEST_DATABASE_URL is preset in the sandbox
   cargo build -p htui --all-features
   ```
   Commit `feat(mod-10): M4 T1 ProjectPatch.secret writes both secret columns under CAS`.
2. **T2a** (lane B worktree): §D.2 red → `HALF_STORED_IDENTITY` → `secrets_settings.rs`
   (newtypes, snapshot, serve, names) + `lib.rs` mod line → `StoreRequest`/`StoreReply` variants,
   `name()`, `try_serve` arms, loop runtime arm, `testkit.rs` list → `check_provider` → Qdrant
   type and construction site → `secrets.rs` generation → `lib.rs` `secret_source` →
   ```bash
   cargo test -p htui --all-features --test secrets_settings -- --test-threads=1
   cargo test -p htui --all-features --lib -- --test-threads=1 secrets store_worker testkit lib
   cargo test -p htui --all-features --test settings -- --test-threads=1 qdrant
   cargo clippy -p htui --all-targets --all-features -- -D warnings
   ```
   Commit `feat(mod-10): M4 T2a keyring rows, provider check, redacted secrets on StoreRequest`
   (body names A-6's behaviour change).
3. **Merge** A and B into `hr/MOD-10` (or cherry-pick into one tree); re-run both lanes' gates on
   the merged tree before T2b.
4. **T2b**: §D.3 red → `ScopeWrite`, `fresh_tree`, the hierarchy arm, `SecretScopeWritten`,
   `SetProjectSecretScope`, `CheckSecretScope` variants, `check_scope`, `testkit.rs`, `try_serve`
   and loop arms →
   ```bash
   cargo test -p htui --all-features --test secrets_settings -- --test-threads=1
   cargo test -p htui --all-features --test hierarchy -- --test-threads=1
   cargo test -p htui --all-features --lib -- --test-threads=1 store_worker hierarchy agent_worker
   ```
   Commit `feat(mod-10): M4 T2b project scope write and scope check`.
5. **T3**: §D.4 red → `secrets.rs` section → `mod.rs` → `app/mod.rs` → Hierarchy passive arm →
   strip test →
   ```bash
   cargo test -p htui --all-features -- --test-threads=1
   cargo insta test -p htui --all-features      # review the six new; any other pending .snap.new is a bug
   ```
   Commit `feat(mod-10): M4 T3 Settings > Secrets section`.
6. **T4**: the docs → `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.
   Commit `docs(mod-10): M4 T4 Settings > Secrets in htui-secrets.md`.
7. **Merged-tree gate** (verify yourself, memory):
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo clippy --workspace -- -D warnings
   cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m4.log; grep -c SIGABRT /tmp/m4.log   # 0
   cargo insta test --workspace --all-features
   SQLX_OFFLINE=true cargo check --workspace --all-features
   bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
   ```
   `qdrant_live` timeouts are re-run serially before being called a regression (memory).
   **Host, before `scripts/hr collect MOD-10`**: the Postgres gate on the merged tree; still owed
   from M2/M3: `scripts/scrub-audit.sql` (OQ-D) and `crates/htui-secrets/tests/infisical_live.rs`
   (now configurable from the Settings section). Recorded in the phase note.

**T4 content** (`docs/htui-secrets.md`):
- `:11-13` becomes: "Store the URL and the identity, check the server, and set each project's
  scope in **Settings > Secrets** ([Settings](#settings))." and the TOC gains `[Settings](#settings)`
  after `[Keyring entries]`.
- `:70-71` becomes: "Settings > Secrets writes these entries ([Settings](#settings))."
- New `## Settings` after `## Keyring entries`: the four rows and the project rows; `e` on URL
  (normalised before it is stored; a refusal stores nothing and does not repeat the URL); `e` on
  Identity (two fields, the secret masked and never shown or read back; storing replaces both
  halves; the client ID is not shown either); `c` with confirmation (the identity's two halves go
  together); `t` on the top rows (status endpoint, then a **fresh login** — it can latch, see
  [Logins](#logins-tokens-and-lockout-safety)); `e` on a project (project ID, environment slug,
  path; validated before it is sent; a changed-elsewhere answer keeps the text); `c` on a project;
  `t` on a project (how many keys the identity can see, never names; it needs read permission like
  a run); results are kept for the session only; `--demo` shows `n/a` and never reads the keyring;
  changes take effect at the next walk, chat or check, with no restart.
- In `## Logins, tokens and lockout safety`: "To try again, enter the identity again in Settings >
  Secrets — any URL or identity write there, even of the same identity, gives the next call a new
  provider with a clean slate (M4 blueprint A-4). A health check is a login: a refused one latches
  exactly as a refused walk does."
- In `## Testing against a real server`: the live test's URL and identity can be stored from the
  Settings section.

---

## G. Tree facts relied on (re-verified at `7e17fdad`)

| Fact | Where |
|---|---|
| `StoreRequest` derives `Debug, Clone`; its doc demands a redacting newtype for secrets | `store_worker.rs:120-134` |
| `SetQdrantApiKey(zeroize::Zeroizing<String>)`; its `name()` arm; try_serve's "handled in worker loop" arm; the loop arm with `.unwrap()` and an unzeroized copy | `store_worker.rs:823`, `:1130`, `:1966-1973`, `:2637-2650` |
| `name()` is a `const fn` of literals; `StoreReply` and `Failed`; `serve` wraps `try_serve` | `store_worker.rs:1033-1160`, `:1171`, `:1727-1732`, `:1787` |
| `try_serve`'s runtime refusal list ("no agent runtime in this build"), hierarchy or-arm, connection arm | `store_worker.rs:1828-1849`, `:1854-1866`, `:1904-1907` |
| The loop's runtime arm and its `other => try_serve` | `store_worker.rs:2677-2703`, after `ListDir` |
| `spawn_hosted` hands one source to both runtimes | `store_worker.rs:2192-2216` |
| `lib.rs::run` builds `KeyringInfisical` and passes `Some` even under `--demo` | `lib.rs` (`Started::detached` for demo, then `Arc::new(secrets::KeyringInfisical::new())`, `spawn_hosted(.., Some(secrets))`) |
| `Harness::drive`'s hand-written runtime list; `settle` serves everything through `store_worker::serve` | `testkit.rs:278-297`, `settle` |
| `AgentRuntime.secrets`; `Background::reading`; `with_secret_source`; `finish_background` is `pub` | `agent_worker.rs:525`, `:549-620`, `:682`, `:894` |
| `chat_secrets` reads `backend.project(id)` then `project_scope` inline | `agent_worker.rs:1199-1216` |
| `serve`'s arms and the `other =>` refusal; `ProbeAgents`; the preview's spawn + `Background::reading` + `Deferred` | `agent_worker.rs:1229-1396`, `:1317`, `:1725-1764` |
| `Answer::at`, the last-word fns, `answering` | `agent_worker.rs:4154-4220`, `:4272`, `:4331` |
| `KeyringInfisical` reads the keyring per call and rebuilds only on URL / client ID / secret digest change | `crates/htui/src/secrets.rs:57-70`, `:131-158` |
| Keyring getters/setters; `half_identity`'s sentence; a blank slot reads as absent; pairwise write | `htui-store/src/secret.rs:445-520`, `:467-473`, `Slot::get` |
| `mock_keyring`, `mock_keyring_broken`, `BROKEN_KEYRING`, `fake_machine_identity`, `refuse_fake_store` | `htui-store/src/testkit.rs:334`, `:343`, `:360`, `~:396`, `~:412` |
| `normalise_base_url` never echoes; `ProviderHealth { base_url, server_ok }`; `health_inner` peeks the latch, GETs status, logs in fresh | `htui-secrets/src/infisical.rs:605`, `htui-core/src/secret.rs ~:265`, `infisical.rs:250-273` |
| `SecretScope` (no serde), `ScopeColumn` (serde, private, `deny_unknown_fields`, default path), `new`, `parse`, `to_column` | `htui-core/src/secret.rs:40-160` |
| `MachineIdentity` has no `Clone`; `SecretError` derives `Clone, PartialEq, Eq` and carries no value | `htui-core/src/secret.rs ~:228-260`, `~:281-420` |
| `project_scope`, `INFISICAL`, `NO_SECRET_SOURCE`; `fake::{FakeSecretProvider, FakeSecretSource}` (health always `Ok`, `list_keys` = next answer's keys) | `htui-core/src/secret.rs ~:430-480`, `:529-750` |
| `ProjectPatch` has three fields; `RepoPatch.remote_url: Option<Option<String>>` | `model/hierarchy.rs:102-110`, `:170` |
| `PgStore::update_project` COALESCEs three fields; `update_repo`'s `CASE WHEN $4 THEN $5` | `pg/write.rs:2344-2383`, `:2458-2521` |
| The current `.sqlx` entry for `update_project` | `crates/htui-store/.sqlx/query-11715e1015dcc19e7368790cae536658cd5ef884810c316cde12037bf80e3dbe.json` |
| `project.secret_provider` / `secret_scope` are `TEXT NULL`; the `updated_at` trigger | `migrations/0001_init.sql:148-149`, `:564-579` |
| `State::update_project`; `set_project_secret_columns` (tests only) | `mem.rs:2547-2590`, `:636` |
| `CASES` (148), `run_case`, `project_create_update_cas`; pins | `conformance.rs:84`, `:253`, `:2954-3029`; `htui-core/tests/mem_store.rs:36`; `htui-store/tests/pg_conformance.rs:34` |
| `ProjectPatch` literals: one full (`settings/hierarchy.rs:978`), the rest `..default()` | `conformance.rs:2982`, `mem.rs:8059`, `tests/hierarchy.rs:642` |
| `hierarchy::serve`, `UpdateProject` arm, `reread`, `cas`, `workspace_of`, `REQUEST_NAMES` (13) | `crates/htui/src/hierarchy.rs:221`, `~:313`, `~:410`, `:429`, `:491`, `~:669` |
| Hierarchy section: `RELOADED`, `on_tree` takes `busy`, `written`, `on_stale` sets `RELOADED` in Browse, `on_reply`, `Failed` matched by `REQUEST_NAMES` | `settings/hierarchy.rs:73`, `:1023`, `:1084`, `:1112-1145`, `:1302`, `~:1398` |
| Every section's reads collected; every reply to every section; shared CAS sentences | `settings/mod.rs` (`SettingsTab::wants_requests`, `on_reply`, `CHANGED_ELSEWHERE*`, `DELETED_ELSEWHERE`) |
| Staleness index keyed by `(Origin, Discriminant<StoreRequest>)` | `app/state.rs:192` |
| Connection section: modes, `busy`, `Notice` Debug, `blocked`, hint rules, `on_paste`; `DsnState`, `snapshot`, `seam_sentence`, `DEMO_SESSION` | `settings/connection.rs`; `crates/htui/src/connection.rs:46-77`, `~:141-200`, `~:203`, `~:251` |
| Qdrant key submit reads `text().unwrap_or("")` of a masked field; `text()` is `None` when masked | `settings/qdrant.rs:172`, `:198`; `text_field.rs:240`, `:695` |
| `register_all` appends Personas last; strip test lists eight | `app/mod.rs:61-79`; `tests/settings.rs:1090-1110` |
| `SectionBench` (`new`, `key`, `reply`, `drained`, `render_section`) | `crates/htui/src/testkit.rs` (`SectionBench`) |
| `htui` features: `testkit = ["htui-store/test-support"]`; dev-dep `htui-orch/test-support` brings `htui-core/test-support` (the fakes) | `crates/htui/Cargo.toml` |

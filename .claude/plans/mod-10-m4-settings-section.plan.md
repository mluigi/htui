# Plan: MOD-10 M4 — Settings section

**Source PRD**: `.claude/prds/mod-10-secret-provider.prd.md`
**Selected Milestone**: 4 — Settings section
**Complexity**: Medium
**Status**: COMPLETE 2026-10-07 (confirmed by the maintainer 2026-10-06, OQ-1..4 as recommended; fact-checked: 18 claims, 0 falsified; blueprint A-1..A-11; implemented `bdffbeeb`..`bd756fb9`; reviewed (rust-reviewer: 1 medium graded low, 8 low confirmed, L-7 refuted); all confirmed findings applied in R1 `eaf558ec`..`cc040c2e`)

## Summary

The maintainer enters the Infisical base URL and machine identity, checks provider health, and
sets each project's secret scope, all from a new `Settings > Secrets` section and without leaving
the TUI. The identity is masked on entry and never read back. Every keyring write goes through the
store worker, as the Connection and Qdrant sections' writes do. Health and scope checks run on the
process's one `KeyringInfisical`, so they share the walks' login latch. The one store change is a
writer for `project.secret_provider` / `secret_scope`: M3 reads these columns, and nothing writes
them yet.

Routed as plan on PRD milestone 4 by `/handoff-run MOD-10` (accepted 2026-10-06, sandbox run
`hr/MOD-10`, ultracode for the implement and review phases).

## Grounding (read on `hr/MOD-10`, base `fd10e494`)

- **No provider rebuild to wire.** `KeyringInfisical::current` (`crates/htui/src/secrets.rs:131`)
  reads the keyring on every `provider()` call. It rebuilds only when the normalised URL, the
  client ID or the SHA-256 of the client secret changed (`Cached::built_from`). A Settings write
  therefore takes effect at the next walk, chat or check. The PRD's "rebuild it only when M4
  stores a new identity or base URL" is already true.
- **One source per TUI process.** `lib.rs::run` (`crates/htui/src/lib.rs:170`) builds it and hands
  it to `store_worker::spawn_hosted`. That function gives it to both runtimes
  (`AgentRuntime::with_secret_source`, `agent_worker.rs:682`). The worker loop itself holds no
  copy.
- **Keyring seam exists.** `htui_store::secret::{get,set,clear}_infisical_url` and
  `{get,set,clear}_machine_identity` (`crates/htui-store/src/secret.rs:445-520`). The identity is
  written and cleared as a pair. A half-stored identity is an `Err` naming the missing slot, and a
  blank slot reads as absent.
- **Validation exists.** `htui_secrets::normalise_base_url` (`crates/htui-secrets/src/infisical.rs:605`)
  never echoes its input; its doc says "M4 calls it before storing a URL". `SecretScope::new`
  (`crates/htui-core/src/secret.rs:69`) refuses an empty project or environment, a path without a
  leading `/`, and control characters, naming the field and never the value.
- **Health.** `SecretProvider::health` checks reachability and then logs in fresh. Infisical's
  `health_inner` checks the latch and cool-down first (`infisical.rs:251`), so a refused check
  latches exactly as a refused walk does. `ProviderHealth` holds only `base_url` and `server_ok`.
  `SecretError` variants carry no values.
- **No writer for the secret columns.** `ProjectPatch` (`crates/htui-core/src/model/hierarchy.rs:102`)
  has only `slug`, `name` and `description`. `PgStore::update_project` (`pg/write.rs:2344`) and
  `State::update_project` (`mem.rs:2547`) COALESCE those three fields. The only setter is the
  test-only `MemStore::set_project_secret_columns` (`mem.rs:636`), whose doc says "M4 owns the
  writer". `RepoPatch.remote_url: Option<Option<String>>` with
  `CASE WHEN $4 THEN $5 ELSE remote_url END` (`pg/write.rs:2458`) is the precedent for a field that
  can be cleared.
- **Section shape.** `SettingsSection` (`ui/tabs/settings/mod.rs:130`) has no store handle. A
  section names its reads in `wants_requests`, every reply reaches every section, and writes leave
  through `ctx.request`. `ConnectionSection` is the secret-handling reference:
  - the masked, zeroizing `TextField`;
  - a hand-written `Debug` that prints lengths only;
  - parsing on the UI task, so a refused input emits nothing;
  - `Unreadable` is never shown as `NotStored`;
  - `--demo` never touches the keyring (D10).

  `QdrantSection` is the closest shape: a keyring URL row and a masked key row. Sections are
  appended last in `app::register_all` (`app/mod.rs:61`).
- **Staleness is per (origin, request kind)** (`app/state.rs:192`). Two Settings sections sending
  the same request kind share one slot.
- **Redaction gap (pre-existing).** `StoreRequest` derives `Debug` and `Clone`
  (`store_worker.rs:133`). Its own doc asks for a redacting newtype for any secret field.
  `SetQdrantApiKey(zeroize::Zeroizing<String>)` (`:823`) does not have one, and a compile probe
  shows `Zeroizing`'s `Debug` prints the key.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Section | `crates/htui/src/ui/tabs/settings/connection.rs` | `Mode` enum (Browse/Editing/Confirm), `busy` one write in flight, `Notice` Debug prints length only, `r` re-read never refused, hint line rules |
| Keyring rows | `crates/htui/src/connection.rs:46-77` (`DsnState`) | `NotApplicable` / `Stored` / `NotStored` / `Unreadable(sentence)`; the seam's sentence without `StoreError`'s prefix |
| Worker serve | `crates/htui/src/connection.rs:211` (`serve`), `store_worker.rs:1967` | module-level `serve` routed from `try_serve`; keyring I/O under `spawn_blocking` |
| Deferred work | `agent_worker.rs:1317` (`ProbeAgents`) | runtime-served request, spawned task, reply sent later (`R-NF-3`) |
| CAS write | `crates/htui/src/hierarchy.rs:313` (`UpdateProject`) | `workspace_of` before, `writer.update_project`, `cas(...)` re-reads the tree |
| Clearable column | `pg/write.rs:2458`, `RepoPatch` | `Option<Option<T>>`, `CASE WHEN $n THEN $m ELSE col END` |
| Errors | `crates/htui-core/src/secret.rs:285` | `SecretError` `Display` is the row text; never a value |
| Tests (section) | `crates/htui/tests/connection.rs`, `htui::testkit::SectionBench` | bench feeds replies, sends keys, asserts requests and rendered text; insta for frames |
| Tests (store) | `crates/htui-core/src/store/conformance.rs:2965` (`project_create_update_cas`) | one conformance case run by MemStore and PgStore |
| Tests (keyring) | `htui_store::testkit::{mock_keyring, mock_keyring_broken}` | process-wide fake; the gate runs `--test-threads=1` |

## Decisions (proposed; CONFIRM accepts or overrides)

- **D1 — One `Secrets` section, appended last.** It has four rows:
  - Provider (`infisical`, fixed);
  - URL;
  - Identity;
  - Health.

  Below them is one row per project of the current workspace. The project rows come from the
  `Hierarchy(workspace)` read, which the section asks for as well, since the reply reaches every
  section. The strip grows from 71 to 80 columns, within the 100-column budget.
- **D2 — Keyring rows are served by a new module `crates/htui/src/secrets_settings.rs`.** It holds
  `SecretsSnapshot` and a `serve` fn routed from `try_serve`. There is one read (`SecretsInfo`) and
  four writes:
  - `SetInfisicalUrl`;
  - `ClearInfisicalUrl`;
  - `SetMachineIdentity`;
  - `ClearMachineIdentity`.

  Every write answers a fresh snapshot. Keyring I/O runs under `spawn_blocking`, and a failed join
  is `Failed`, never a panic. `Backend::Memory` answers `NotApplicable` rows and refuses the writes
  with Connection's demo sentence (D10). The Qdrant arms' `.unwrap()` is not copied.
- **D3 — The URL is checked on the UI task.** `normalise_base_url` runs before any request, and a
  refusal emits nothing (Connection D2). The worker stores the normalised form. The URL row shows
  `stored · <normalised URL>`; a normalised URL has no user info, query or fragment.
- **D4 — Identity entry.** A two-field form:
  - the client ID as plain text;
  - the client secret masked;
  - `Tab` moves between them, and both buffers zeroize.

  It travels as `IdentityEntry`, a new newtype with `Clone` and a `Debug` that prints only
  `IdentityEntry(<redacted>)`. The worker turns it into a `MachineIdentity`. The Identity row
  shows its state only (`stored` / `not stored` / `half stored: …` / `unreadable: …`); the stored
  client ID is never read back to the UI. `c` on the row asks for confirmation, then removes both
  halves.
- **D5 — Health check.** `t` on the Provider, URL, Identity or Health row sends
  `CheckSecretProvider`. `AgentRuntime` serves it on a spawned task with its shared source:
  `source.provider().await?.health().await`. The reply is
  `StoreReply::SecretCheck(SecretCheck::Provider { at, outcome })`. The section keeps the last
  result for the session, shown as `last check 14:02:11: server ok · login ok` or the
  `SecretError` sentence; it is not persisted. A runtime with no source answers `Failed` with one
  fixed sentence. A refused login latches the shared provider (M2 D5), and storing a new identity
  clears that, because the next `provider()` rebuilds. The row's guide text says so.
- **D6 — Scope write.** `e` on a project row opens a three-field form:
  - Infisical project ID;
  - environment;
  - path, prefilled `/`.

  `SecretScope::new` validates it on the UI task and a refusal emits nothing. It is sent as
  `SetProjectSecretScope { id, expected, scope: Option<SecretScope> }`, a request kind of its own,
  so it never shares a staleness slot with the Hierarchy section's `update_project`.
  `crate::hierarchy::serve` handles it like `UpdateProject`: `workspace_of`, then `update_project`
  with `ProjectPatch { secret: Some(scope), ..default }`, then `cas`. The reply is the hierarchy
  tree, so both sections refresh, and a CAS miss uses the shared `CHANGED_ELSEWHERE*` sentences.
  `c` on a project row asks for confirmation and sends `scope: None`.
- **D7 — Store field.** `ProjectPatch` gets `secret: Option<Option<SecretScope>>`: `None` keeps
  the columns, `Some(None)` clears both, and `Some(Some(s))` writes provider `infisical` and scope
  `s.to_column()`. Both columns are written in one statement, so a provider without a scope cannot
  be stored. `SecretScope` gains serde through its column form
  (`#[serde(try_from = "ScopeColumn", into = "ScopeColumn")]`), so deserialising validates.
  Postgres uses `CASE WHEN $6 THEN $7 ELSE secret_provider END` and the same for `secret_scope`,
  and `.sqlx` is regenerated. MemStore mirrors this. A conformance case covers set, clear and a
  stale token. `set_project_secret_columns` stays as the tests' unvalidated planter.
- **D8 — Scope check.** `t` on a project row sends `CheckSecretScope { project }`. `AgentRuntime`
  reads the project, calls `project_scope`, then `provider.list_keys(scope)`. The reply is
  `SecretCheck::Scope { project, at, outcome: Result<usize, String> }`: a key **count**, never
  names or values. The row shows `14:03 · 12 keys visible` or the refusal. A project with no
  provider is refused on the UI side with no request. `list_keys` needs read-value permission
  (M2 A-6), so the check fails exactly as a walk would.
- **D9 — Fold in the Qdrant `Debug` leak.** `SetQdrantApiKey` carries a redacting newtype
  (`Redacted`, also used for the client secret inside `IdentityEntry`). This is one variant, its
  one construction site in `qdrant.rs` and its one match arm. Test: `format!("{request:?}")` of
  both requests contains no typed text.
- **D10 — Out of scope.**
  - `htui worker` gets no Settings surface.
  - Health results are not persisted.
  - There is no provider picker; `infisical` is the only kind.
  - The worker-side identity is still MOD-48's.

## Open questions (decided 2026-10-06: all four as recommended)

- **OQ-1 Placement.** Put the per-project scope in the Secrets section (D1, recommended: one place
  for secrets, and the scope check sits beside the scope)? Or add three fields to the Hierarchy
  section's project editor, with health and identity only in Secrets?
- **OQ-2 Scope check output.** Show the key count only (D8, recommended)? Or also the sorted key
  names? Names are not values, but they reveal what a project holds.
- **OQ-3 Qdrant leak.** Fold it in here (D9, recommended; it is a few lines and needs the same
  newtype)? Or file it as its own item?
- **OQ-4 Identity row.** Show state only (D4, recommended: "never echoed")? Or also show the
  stored client ID, which is an identifier rather than a secret?

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/model/hierarchy.rs` | UPDATE | `ProjectPatch.secret` (D7) | T1 |
| `crates/htui-core/src/secret.rs` | UPDATE | `SecretScope` serde via `ScopeColumn` (D7) | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `State::update_project` writes the pair (D7) | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | set / clear / stale case (D7) | T1 |
| `crates/htui-store/src/pg/write.rs` | UPDATE | `update_project` SQL (D7) | T1 |
| `crates/htui-store/.sqlx/` | UPDATE | regenerated for the new query | T1 |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | UPDATE | the one full `ProjectPatch` literal (`:978`) gains `secret: None` | T1 |
| `crates/htui/src/secrets_settings.rs` | CREATE | `SecretsSnapshot`, `serve`, request names (D2) | T2a |
| `crates/htui/src/lib.rs` | UPDATE | module declaration | T2a |
| `crates/htui/src/store_worker.rs` | UPDATE | variants, `name()`, `try_serve` routing, runtime routing, `Redacted`/`IdentityEntry`, Qdrant arm (D2, D4, D5, D9) | T2a, then T2b |
| `crates/htui/src/agent_worker.rs` | UPDATE | `CheckSecretProvider` arm (D5) | T2a |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | UPDATE | `Redacted` construction (D9) | T2a |
| `crates/htui/src/hierarchy.rs` | UPDATE | `SetProjectSecretScope` arm (D6) | T2b |
| `crates/htui/src/agent_worker.rs` | UPDATE | `CheckSecretScope` arm (D8) | T2b |
| `crates/htui/tests/secrets_settings.rs` | CREATE | worker-side cases (T2a, T2b), section cases (T3) | T2a → T2b → T3 |
| `crates/htui/src/ui/tabs/settings/secrets.rs` | CREATE | `SecretsSection` (D1, D3–D6, D8) | T3 |
| `crates/htui/src/ui/tabs/settings/mod.rs` | UPDATE | `pub mod secrets`, re-export | T3 |
| `crates/htui/src/app/mod.rs` | UPDATE | register last (D1) | T3 |
| `crates/htui/tests/settings.rs` | UPDATE | strip-width test lists nine sections | T3 |
| `crates/htui/tests/snapshots/secrets_settings__*.snap` | CREATE | section frames | T3 |
| `docs/htui-secrets.md` | UPDATE | Settings section replaces "not yet available" (`:11`, `:70`); latch note | T4 |

## Tasks

### T1: Secret-column writer (`htui-core`, `htui-store`)
- **Action**: D7. Tests first: the conformance case (set writes both columns; clear nulls both; a
  stale token answers `Stale` and writes nothing; `secret: None` leaves the columns alone), plus a
  `SecretScope` serde round trip and a refusal of an invalid column on deserialisation. Then
  MemStore and PgStore, and `cargo sqlx prepare` against a migrated scratch database (sandbox:
  `localhost:5439`; see `docs/hr-sandbox.md`).
- **Mirror**: `RepoPatch.remote_url`, `PgStore::update_repo`.
- **Validate**: `cargo test -p htui-core --all-features`;
  `cargo test -p htui-store --all-features -- --test-threads=1` (Postgres cases run with
  `HTUI_TEST_DATABASE_URL`); `SQLX_OFFLINE=true cargo check -p htui-store`.

### T2a: Keyring and health plumbing (`htui`), parallel with T1
- **Action**: D2, D4 (types only), D5, D9. Tests first, in `tests/secrets_settings.rs` through
  `Harness` / `try_serve` with `mock_keyring`:
  - every state of both rows;
  - a broken keyring is `Unreadable`, never `NotStored`;
  - demo is `NotApplicable` and its writes are refused;
  - each write answers a fresh snapshot;
  - a bad URL never reaches the keyring (UI side in T3; worker side refuses a non-normalised URL
    defensively);
  - `CheckSecretProvider` with a `FakeSecretSource` answers `server ok` and `BadCredentials`, and
    a runtime with no source answers `Failed`;
  - the `Debug` of every new request and of `SetQdrantApiKey` holds no typed text.
- **Mirror**: `connection::serve`, `QdrantSnapshot::fetch` (minus the unwraps), `ProbeAgents`.
- **Validate**: `cargo test -p htui --all-features --test secrets_settings -- --test-threads=1`.

### T2b: Scope write and check (`htui`), after T1 and T2a
- **Action**: D6 (worker half) and D8. Tests first:
  - `SetProjectSecretScope` writes, re-reads the tree and answers a CAS miss as `UpdateProject`
    does;
  - `CheckSecretScope` answers a count, a provider-less project is refused, and a provider error
    passes through as its sentence.
- **Mirror**: `hierarchy::serve`'s `UpdateProject` arm.
- **Validate**: as T2a, plus `cargo test -p htui --all-features --test hierarchy`.

### T3: `SecretsSection` (`htui`), after T2b
- **Action**: D1, D3–D6, D8 (UI half). Tests first with `SectionBench`:
  - rows from a snapshot plus a hierarchy reply;
  - `e`, `c`, `t`, `r`, `j`/`k`, `Tab`, `Esc`;
  - one write in flight;
  - a refused URL or scope emits no request;
  - the masked field never renders its text;
  - the section's `Debug` holds no typed text;
  - a scope change drops an open editor;
  - CAS sentences;
  - hint-line width (MOD-60);
  - insta frames for: not configured, configured with health ok, health refused, a project scope
    row with a check result, the identity form, and demo.
- **Mirror**: `ConnectionSection`, `QdrantSection`, `HierarchySection::on_stale`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`, then
  `cargo insta test -p htui --all-features` with **no** pending snapshots outside
  `secrets_settings__*` (memory: snapshot impact needs a full insta run).

### T4: Operator docs, after T3
- **Action**: In `docs/htui-secrets.md`:
  - a "Settings" section covering entry, health, scope and the scope check;
  - what a refused check does to later walks (the latch), and how a new identity clears it;
  - remove the "not yet" lines.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

### Parallel lanes

| Lane | Tasks | Files (disjoint across lanes) |
|---|---|---|
| A | T1 | `htui-core/src/{model/hierarchy,secret,store/mem,store/conformance}.rs`, `htui-store/src/pg/write.rs`, `htui-store/.sqlx/`, `htui/src/ui/tabs/settings/hierarchy.rs` |
| B | T2a | `htui/src/{secrets_settings,lib,store_worker,agent_worker}.rs`, `htui/src/ui/tabs/settings/qdrant.rs`, `htui/tests/secrets_settings.rs` |
| — | T2b → T3 → T4 (serial: they share `store_worker.rs`, `agent_worker.rs` and the new test file) | as listed above |

Lanes A and B share no file. They still share the `target/` directory and the process-wide
keyring fake, so each runs in its own worktree, or lane B gates only after lane A commits (memory:
shared-tree fan-out coupling).

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate (memory)
cargo test -p htui-core --all-features
cargo test -p htui-store --all-features -- --test-threads=1  # Postgres cases with HTUI_TEST_DATABASE_URL
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features                      # no unexpected pending snapshots
SQLX_OFFLINE=true cargo check --workspace --all-features
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

On the host, before merge: the Postgres gate on the merged tree. Still owed from M2/M3:
`scripts/scrub-audit.sql` (OQ-D) and `crates/htui-secrets/tests/infisical_live.rs`. Once M4
lands, the live test can be set up from the Settings section.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A typed secret reaches a log through `Debug` | Medium | D4/D9 newtypes; `Debug` tests on every request and on the section's modes and notices |
| A keyring read that hangs stalls the store worker loop (the Connection and Qdrant reads already accept this) | Low | Reads are `spawn_blocking`; health and scope checks run on spawned tasks, never in the loop |
| A health check latches the identity and later walks refuse | Medium (by design) | The Health row and docs say so; storing a new identity rebuilds the provider (verified `Cached::built_from`) |
| Two Settings writes of one kind race in the staleness slot | Low | D6's own request kind |
| The process-wide keyring fake flakes under parallel tests | Medium | `--test-threads=1` gate (memory) |
| MOD-67 (configurable hotkeys) lands keymap changes that conflict with `t` | Low | Section keys are local `KeyCode` matches like every other section's; rebase on merge |
| Strip snapshots move | Low | Only the strip-width test lists sections; full insta run in T3 |

## Acceptance

- [ ] URL, identity and health can be set, cleared and checked from `Settings > Secrets`; nothing
      typed is drawn or reaches a `Debug`
- [ ] A project's scope can be set, cleared and checked; M3 walks pick it up with no restart
- [ ] `--demo` never touches the keyring
- [ ] All validation passes; `docs/htui-secrets.md` updated
- [ ] Host steps listed in the phase note

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `KeyringInfisical` re-reads the keyring per call and rebuilds only on a changed URL, ID or secret digest | VERIFIED | `secrets.rs:131-158`; test `a_new_identity_or_url_rebuilds_the_provider` |
| One `KeyringInfisical` per TUI process, handed to both runtimes, not held by the loop | VERIFIED | `lib.rs:170-182`; `store_worker.rs:2192-2216` |
| `AgentRuntime` holds the source and serves runtime requests as deferred tasks | VERIFIED | `agent_worker.rs:682` `with_secret_source`; `:1317` `ProbeAgents` arm |
| Keyring get/set/clear for URL and identity exist; identity is a pair | VERIFIED | `htui-store/src/secret.rs:445-520` summary; `clear_removes_both_halves` test |
| `normalise_base_url` is public, never echoes, refuses userinfo/query/fragment | VERIFIED | `infisical.rs:595-640` |
| `health` peeks the latch, then status, then fresh login | VERIFIED | `infisical.rs:250-273` |
| No production writer for `secret_provider`/`secret_scope`; `ProjectPatch` has three fields | VERIFIED | `model/hierarchy.rs:102-109`; `pg/write.rs:2344-2383`; `mem.rs:2547-2590`; text search `secret_scope` |
| `Option<Option<T>>` + `CASE WHEN` precedent | VERIFIED | `RepoPatch` (`model/hierarchy.rs:170`); `pg/write.rs:2458-2521` |
| Only one full `ProjectPatch` literal (others use `..default()`) | VERIFIED | `settings/hierarchy.rs:978` full; `conformance.rs:2982`, `mem.rs:8059`, `tests/hierarchy.rs:642` use `..ProjectPatch::default()` |
| `SecretScope` has no serde derive; `ScopeColumn` does | VERIFIED | `secret.rs:40-56` |
| `MachineIdentity` has no `Clone`, so it cannot sit in `StoreRequest` (derives `Clone`) | VERIFIED | `secret.rs:228-245`; `store_worker.rs:133` |
| `Zeroizing<String>`'s `Debug` prints the content (Qdrant leak) | VERIFIED (compile probe) | zeroize 1.9.0: `Zeroizing("sk-SECRET-123")` |
| Staleness index is per (origin, request kind) | VERIFIED | `app/state.rs:192` |
| Every reply reaches every Settings section; sections are appended last | VERIFIED | `settings/mod.rs` `on_reply`; `app/mod.rs:61-79` |
| Strip fits: 71 → 80 columns of a 100-column budget | VERIFIED | `tests/settings.rs:54` `SECTION_WIDE = 100`; titles summed |
| `t` is unbound globally and in Settings | VERIFIED | no `Char('t')` in `keymap.rs`, `keys/`, `app/`; Connection's comment lists the global keys |
| `docs/htui-secrets.md` says the Settings section is not yet available | VERIFIED | `:11`, `:70` |
| Lanes A and B touch disjoint files | VERIFIED | Files table intersection: empty |

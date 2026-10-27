# Plan: MOD-158 — Qdrant: URL setting and API key

**Source**: HANDOFF `MOD-158` (from ANA-37, `docs/ANA-37.md` §7.5, §8 Q1, Q2; amended 2026-10-20: one
Qdrant for the whole database). Decision record: `docs/decisions/ana/ana-37.md`.
**Routing**: verdict was PRD at threshold (C2 + C4); **routed as plan by maintainer override**
(2026-10-27), with ultracode for the implement phase. MOD-160 stays a separate item (maintainer,
2026-10-27).
**Requirements**: `R-STO-1`, `R-STO-8`, `R-SEC-2`, `R-TUI-8`, `R-NF-2` (texts already amended on
ANA-37 and 2026-10-20; this item needs no requirement edit).
**Complexity**: Large (store, secrets, agent spawn, orchestration shell, TUI section, CLI, worker).
**Status**: confirmed 2026-10-27. Q1-Q3 answered at CONFIRM as recommended: provision includes the
key credential (D11); `c` clears the focused row (D9); the worker's `search_concepts` gate is checked
at worker start (D7).

## Summary

Today the Qdrant URL and API key are two entries in each box's OS keyring (`secret.rs:43-45`), read
per request by the TUI's index, once at start by the worker's index job, and per call by
`--search-items`/`--index-items`. A headless worker has no keyring, so it has no Qdrant. This item
moves the URL to one database-wide `app_setting` row (no migration), keeps the API key in the keyring
and adds the worker's non-keyring sources (stdin, systemd credential) and a development/CI environment
variable that no agent subprocess inherits. The old per-box keyring URL is offered once as the
setting, then never read again. A URL or key changed while htui runs applies to the next search or
index cycle.

Out of scope (MOD-160): "knowledge search unavailable" wording, warnings in Settings/first
run/worker start, `search_concepts` advertised per session, CI running `qdrant_live`. Out of scope
(MOD-159): collections per workspace, the indexer lease, the 60 s cycle.

## Decisions

- **D1 — The URL is the `app_setting` row `qdrant_url`** (a JSON string), no migration, no `workspace`
  column (amended Q1). Read and written by **inherent** methods on `PgStore` and `MemStore`, as
  `app_settings()` already is (`concepts.rs` `IndexSource` impls), not trait verbs: no forwarder in
  `writer.rs`, the conformance spies or `SpyStore` changes.
  - `qdrant_url() -> Result<Option<StoredQdrantUrl>>` (`{ url: QdrantUrl, token: DateTime<Utc> }`,
    the token is `app_setting.updated_at`).
  - `set_qdrant_url(url: &QdrantUrl, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<StoredQdrantUrl>>`:
    `None` means "I expect no row". `clear_qdrant_url(expected: DateTime<Utc>) -> Result<CasOutcome<…>>`.
  - The SQL reuses the existing `app_setting` statements of `set_queue_setting`/`clear_queue_setting`
    (App arm, `pg/write.rs:5522-5548`) and their read, **byte for byte**, so the `.sqlx` offline
    cache already holds them and no `cargo sqlx prepare` is needed.
  - A row whose value is not a string that parses as a `QdrantUrl` reads as an error naming the key,
    never as "no URL".
- **D2 — `QdrantUrl` (new `htui-core/src/qdrant.rs`)**, the one validation, used by the setter, the
  env override, the CLI and the TUI field. Trimmed; scheme `http://` or `https://`; a non-empty host;
  **refused**: any userinfo (`user@`, `user:pass@`), any query string, any fragment ("a URL that
  carries credentials is refused", `R-STO-1`). Hand-written: `htui-core` has no `url` dependency and
  needs none for these four rules. `QdrantSettings::new` (`htui-store/src/qdrant_settings.rs`) takes a
  `QdrantUrl`, so no unchecked URL reaches the client.
- **D3 — URL resolution, per process, per request.** `HTUI_QDRANT_URL` when set and non-empty (the
  development/CI override, `R-STO-1`; validated by D2, an invalid one is an error, never a fallback),
  else the `qdrant_url` row, else "no Qdrant URL". The keyring `qdrant-url` is never read here. An
  override in force is logged once at `info` and shown in Settings > Qdrant.
- **D4 — API key resolution.** `HTUI_QDRANT_API_KEY` when set and non-empty (development/CI); else:
  - TUI and the CLI verbs: this box's keyring entry `qdrant-key`, read per request as today, so a key
    stored in Settings applies to the next search;
  - `htui worker`: `--qdrant-key-stdin` when given (one line, read on the DSN reader's thread right
    after the DSN line when `--dsn-stdin` is also given, held in `Zeroizing` memory only); else the
    keyring; else, on Linux, `$CREDENTIALS_DIRECTORY/htui-qdrant-key`. Mirrors `headless_dsn`
    (`htui-store/src/secret.rs`), with one difference: no key anywhere is `Ok(None)`, not a refusal,
    because a Qdrant without a key is allowed (`docs/ANA-37.md` §8 Q2).
  - Never Postgres, `argv` or a plaintext file. A TUI session's stdin for the key (`R-STO-1` allows it)
    is not in this item's text and is not built.
- **D5 — Nothing htui-owned reaches an agent subprocess (`R-SEC-2`).** `unsafe_code = "forbid"`
  (workspace `Cargo.toml:194`) rules out `remove_var`, so the variable is removed at each spawn: one
  list, `htui_core::secret::HTUI_OWN_ENV` (`["HTUI_QDRANT_API_KEY"]`; the URL holds no secret and
  stays visible), applied with `env_remove` **after** `.envs(&launch.env)` in
  `htui-agent/src/launch.rs::spawn_supervised` (so a project secret of that name is removed too) and
  in both shell spawns of `htui-orch/src/verify.rs` (`spawn_and_wait`, `run_shell`). `htui mcp` is
  started by the agent and inherits the agent's environment, so it is covered.
- **D6 — Live pickup: a store handle the process-wide index reads through.** `concepts_worker` gains a
  settings source beside `shared_qdrant()`: a slot holding the current `PgStore` (or none), published
  by the store worker when it starts online and in `go_online` (`store_worker.rs:3501`, the only
  in-place swap), and by `htui worker` after its connect. `QdrantIndex::{search, sync}` resolve D3 and
  D4 per request through it; the existing connection cache keyed by URL and key
  (`concepts_worker.rs` `Connections`) keeps a reconnect to a changed setting only. No published store
  (offline TUI, `--demo`) answers "the Qdrant URL cannot be read: Postgres is not connected".
- **D7 — The worker's index job** is spawned whatever the setting says at start and resolves D3/D4 at
  every cycle: no URL is one `info` line, then a sleep of the interval; a URL set later is indexed on the
  next cycle without a restart. The model is still loaded only once a URL resolves.
  `search_concepts` on a worker stays offered exactly when a URL resolves at worker start (today:
  when the index job exists); MOD-160 moves that check to session start.
- **D8 — The CLI verbs.** `--search-items`: DSN, read-only connect, then D3 and D4 (the URL now needs
  Postgres, so "no DSN" and "no Postgres" now come before "no URL"); the `no_qdrant_url` and
  `qdrant_url_unusable` codes stay, their sentences name Settings > Qdrant and `htui --set-qdrant-url`.
  `--index-items`: the same through its headless connect. New flags, each exiting after it acts:
  `--set-qdrant-url <URL>` (the URL is no secret, so `argv` is allowed; refused by D2),
  `--clear-qdrant-url`, `--set-qdrant-key` (one line from stdin into this box's keyring, as
  `--set-dsn`), `--clear-qdrant-key`. URL writes connect headless with the keyring DSN and go through
  D1's compare-and-set, read-then-write.
- **D9 — Settings > Qdrant.** The URL row edits the database-wide setting through store requests (the
  section still holds no store, `R-NF-3`): the snapshot carries the row, its token, any override in
  force and its source, and the key row's keyring state. A write with a stale token is refused and
  re-read ("changed elsewhere"). The row says it applies to every box. `c` clears the **focused row**
  (the URL for every box, or this box's key), each behind its own question, instead of today's
  "clear all" (proposal Q2). Offline the URL row reads "cannot be read while Postgres is offline";
  `--demo` keeps `n/a in a demo session`. Every new key goes through `keys.toml`.
- **D10 — The old keyring URL, offered once.** When the snapshot finds the keyring `qdrant-url`
  entry: with no `qdrant_url` row, an offer row under the URL row: "this box's keyring holds
  `<url>` from before: use it for every box, or discard it"; using it is D1's insert expecting no row,
  then the keyring entry is removed; discarding removes the entry. With a row already set, the offer
  is discard only, saying the old entry is no longer used. A stored entry that fails D2 can only be
  discarded. The TUI says once per launch on the status line that Settings > Qdrant has an offer;
  `htui worker` logs it once. Nothing reads the entry for any other purpose (retired). Its keyring
  constants and functions stay for the offer.
- **D11 — `htui provision`** (proposal Q1): when this box's keyring holds a Qdrant key, provision
  also encrypts it as `htui-qdrant-key` beside `htui-dsn` and adds its `LoadCredentialEncrypted=`
  line to the unit; with no key, the unit is unchanged.
- **D12 — Sandbox.** `scripts/hr` exports `HTUI_QDRANT_URL` to the run's own Qdrant beside
  `HTUI_TEST_QDRANT_URL`, so a sandbox TUI never reads the database row for another Qdrant
  (`docs/ANA-37.md` §8 Q1: "so an hr sandbox can point at its own Qdrant"); `docs/hr-sandbox.md`
  says so.

## Proposals for the maintainer (answer at CONFIRM)

- **Q1 — Provision the key credential (D11)?** Recommended: **yes**. Without it a headless worker's
  credential is a manual `systemd-creds encrypt`. Cost: `provision/{plan,script}.rs` and the unit pins
  in `tests/provision.rs`. Alternative: the worker reads the credential, docs show the manual command,
  provisioning is filed as a follow-up.
- **Q2 — `c` clears the focused row (D9)?** Recommended: **yes**. "Clear all" would now erase a
  setting every box uses together with one box's key. Alternative: keep clear-all, its question naming
  both.
- **Q3 — Worker `search_concepts` gate (D7)?** Recommended: **checked at worker start**, today's
  behaviour with the new source; MOD-160 moves it to session start. Alternative: always offer it, as the
  TUI already does (`mcp_search::production` is always `Some`), leaving the wording to MOD-160.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Headless secret sources | `htui-store/src/secret.rs:189-301` `DsnSources`, `headless_dsn`, `read_dsn_line`, `CREDENTIAL_NAME` | Sources struct with injected `credentials_dir`; `Zeroizing`; error enum carries no secret |
| Keyring entries + test fake | `secret.rs:303-379`, `FakeSlots` :62-75 | One `get/set/clear` triple per entry; `test-support` fake first |
| `app_setting` compare-and-set | `pg/write.rs:5507-5560` `set_queue_setting` App arm; `cas_miss` | `INSERT … ON CONFLICT DO NOTHING RETURNING updated_at` / `UPDATE … WHERE updated_at = $3`; reuse text byte for byte |
| MemStore settings | `htui-core/src/store/mem.rs:684` `set_app_setting`, `State.app_settings` | `(Value, DateTime<Utc>)` map |
| Process-wide index | `concepts_worker.rs:212-271` `qdrant_settings`, `shared_qdrant`, `Connections` | Settings resolved per request; connection cached by URL + key |
| Worker startup | `worker_cmd.rs:68-118` `serve`, `read_dsn_apart` | Each startup read raced against the shutdown signal |
| Settings section | `ui/tabs/settings/qdrant.rs`, `qdrant_settings_info.rs` | Kit form rows; requests through `ctx.request`; `Redacted` key; demo refusal |
| CLI flag that exits | `cli.rs:28-34` `--set-dsn`/`--clear-dsn`, `lib.rs:137-144` | `conflicts_with_all`; acts and returns before the TUI |
| Spawn env | `htui-orch/src/isolate/git.rs:96` `SCRUBBED_ENV` + `env_remove` | A const list removed at the spawn |
| Tests | `tests/qdrant_settings.rs` (literal snapshots, no keyring), `tests/qdrant_worker.rs` (gated on `HTUI_TEST_QDRANT_URL`), `htui-store/tests/pg_*` | No real keyring; Pg cases on `HTUI_TEST_DATABASE_URL` |

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/qdrant.rs` | CREATE (`QdrantUrl`, `StoredQdrantUrl`, key name `qdrant_url`) | T1 |
| `crates/htui-core/src/lib.rs` | UPDATE (module) | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE (inherent `qdrant_url`/`set_`/`clear_`) | T1 |
| `crates/htui-store/src/pg/write.rs`, `pg/read.rs` | UPDATE (inherent methods, reused SQL) | T1 |
| `crates/htui-store/src/qdrant_settings.rs` | UPDATE (`QdrantSettings::new(QdrantUrl, …)`) | T1 |
| `crates/htui-store/tests/qdrant_url_pg.rs` | CREATE | T1 |
| `crates/htui-core/src/secret.rs` | UPDATE (`HTUI_OWN_ENV`, env names) | T2 |
| `crates/htui-agent/src/launch.rs` | UPDATE (`env_remove` in `spawn_supervised`) | T2 |
| `crates/htui-orch/src/verify.rs` | UPDATE (`env_remove` in both shells) | T2 |
| `crates/htui-store/src/secret.rs` | UPDATE (`QdrantKeySources`, `qdrant_api_key`, `htui-qdrant-key`, env override readers taking the value as an argument) | T2 |
| `crates/htui/src/concepts.rs` | UPDATE (D3/D4 resolution, index job D7, CLI verbs D8 reads) | T3 |
| `crates/htui/src/concepts_worker.rs` | UPDATE (settings source D6) | T3 |
| `crates/htui/src/worker_cmd.rs` | UPDATE (key sources, publish store, gate D7) | T3 |
| `crates/htui/src/store_worker.rs` | UPDATE (publish on start and `go_online`) | T3, then T4 |
| `crates/htui/src/cli.rs` | UPDATE (`WorkerArgs.qdrant_key_stdin`) | T3, then T5 |
| `crates/htui/tests/qdrant_worker.rs`, `tests/state_cli_pg.rs` | UPDATE | T3 |
| `crates/htui/src/qdrant_settings_info.rs` | UPDATE (snapshot, requests, offer) | T4 |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | UPDATE (rows, per-row clear, offer row) | T4 |
| `crates/htui/src/keys/**` | UPDATE only if a new act is needed | T4 |
| `crates/htui/src/app.rs` (status-line offer notice) | UPDATE | T4 |
| `crates/htui/tests/qdrant_settings.rs`, `tests/settings.rs`, snapshots | UPDATE | T4 |
| `crates/htui/src/lib.rs`, `crates/htui/src/cli.rs` (Args flags) | UPDATE | T5 |
| `crates/htui/src/qdrant_cmd.rs` | CREATE (the four flags) | T5 |
| `crates/htui/tests/qdrant_cli_pg.rs` | CREATE | T5 |
| `docs/htui-cli.md` | UPDATE (flags, changed refusal order) | T5 |
| `crates/htui/src/provision/{plan,script}.rs`, `tests/provision.rs` | UPDATE (if Q1 = yes) | T6 |
| `scripts/hr`, `docs/hr-sandbox.md` | UPDATE (D12) | T6 |

The code-architect's blueprint may move a file between tasks; step 3.5's intersection is re-run on
the blueprint's sets before fan-out.

## Tasks (TDD: the failing test first, in every task)

### Task T1 — `QdrantUrl` and the `qdrant_url` row (D1, D2)
- **Action**: `QdrantUrl::parse` with the four refusals; inherent store methods on Mem and Pg;
  `QdrantSettings` takes a `QdrantUrl`.
- **Tests**: unit cases for every refusal and a plain `http://host:6334`; Mem and Pg cases for
  insert-expecting-none, update by token, stale token, clear, a malformed row read as an error.
- **Validate**: `cargo test -p htui-core qdrant`; `cargo test -p htui-store --all-features --test qdrant_url_pg -- --test-threads=1`.

### Task T2 — Key sources and the spawn strip (D4 store half, D5)
- **Action**: `HTUI_OWN_ENV`; `env_remove` at the three spawn sites; `QdrantKeySources`
  (env value, stdin, credentials dir passed in) and `qdrant_api_key`.
- **Tests**: a launch whose `launch.env` carries `HTUI_QDRANT_API_KEY` starts a child that does not
  see it (`env` as the program); the same for `run_shell`; source order and every error, without
  touching the real keyring (fake).
- **Validate**: `cargo test -p htui-agent launch`; `cargo test -p htui-orch verify`;
  `cargo test -p htui-store --all-features secret`.

### Task T3 — Resolution, live pickup, worker and CLI-verb wiring (D3, D6, D7, D8 reads)
- **Depends on**: T1, T2.
- **Action**: the settings source; `QdrantIndex` resolves per request; the store worker and `htui
  worker` publish their store; the index job resolves per cycle; `--qdrant-key-stdin`;
  `--search-items`/`--index-items` read the row.
- **Tests**: over `MemStore`, a URL set after the index job starts is used by the next cycle; a
  changed URL makes the next search reconnect; no store published reads as the offline sentence;
  `search_items` refusal order (no DSN before no URL); the worker gate.
- **Validate**: `cargo test -p htui --all-features concepts -- --test-threads=1`; `qdrant_worker`,
  `state_cli_pg` against the sandbox's Postgres and Qdrant.

### Task T4 — Settings > Qdrant and the offer (D9, D10)
- **Depends on**: T3 (shares `store_worker.rs`).
- **Action**: snapshot and requests; URL row on the database setting; per-row clear (Q2); the offer
  row; the status-line notice.
- **Tests**: `tests/qdrant_settings.rs` rewritten for the new rows (literal snapshots, no keyring),
  stale-token refusal, offer use and discard, an invalid old entry discard-only, an override shown,
  offline and demo rows; insta snapshots reviewed.
- **Validate**: `cargo test -p htui --all-features --test qdrant_settings --test settings`; full insta run.

### Task T5 — CLI flags (D8)
- **Depends on**: T3. **Parallel with**: T4 and T6 (disjoint files).
- **Action**: four flags, `qdrant_cmd.rs`, docs.
- **Tests**: clap conflicts; Pg cases for set, refuse-credentials, clear, stale; key set/clear on
  the fake keyring.
- **Validate**: `cargo test -p htui --all-features cli`; `--test qdrant_cli_pg -- --test-threads=1`.

### Task T6 — Provision and sandbox (D11 if Q1, D12)
- **Depends on**: T2. **Parallel with**: T4, T5.
- **Validate**: `cargo test -p htui --all-features --test provision`; `bash -n scripts/hr`.

Order: **W1** T1 ∥ T2 → **W2** T3 → **W3** T4 ∥ T5 ∥ T6. Then the reviewer gate (`rust-reviewer`),
then close-out.

## Validation

```bash
bash scripts/gate.sh
cargo clippy --workspace -- -D warnings          # featureless: code dead without test-support
cargo test -p htui --all-features -- --test-threads=1   # suite green is scheduling-dependent
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A reused SQL text drifts by one byte and needs a `.sqlx` entry | Medium | Copy the statements verbatim; `SQLX_OFFLINE=true cargo check` in T1; prepare against a migrated scratch DB if it fails |
| The env strip is missed at a spawn added later | Medium | One list in `htui-core`; a test per site; the reviewer checks every `Command::new` in agent and orch crates |
| A per-search `app_setting` read adds latency | Low | One indexed single-row select per user-paced search; the connection cache is unchanged |
| `--search-items` refusal order changes what a script sees | Medium | Codes unchanged; order change documented in `docs/htui-cli.md`; `state_cli_pg` pins updated deliberately |
| Settings section snapshots move widely | High | Full insta run per memory note; T4 owns every Qdrant snapshot |
| Keyring fake is process-wide | High | Tests run `--test-threads=1` where they touch it, as today |
| An operator relies on the per-box keyring URL on a box nobody opens Settings on | Medium | The worker logs the offer at start; the status line says it once per TUI launch |

## Acceptance

- [ ] The URL lives in `app_setting.qdrant_url`; Settings, the CLI and the worker read and write it there.
- [ ] A URL with userinfo, a query or a fragment is refused everywhere.
- [ ] `HTUI_QDRANT_URL` overrides per process; `HTUI_QDRANT_API_KEY` supplies the key and no agent or
      step shell sees it.
- [ ] A worker reads the key from `--qdrant-key-stdin`, the keyring or `htui-qdrant-key`.
- [ ] The old keyring URL is offered once and nothing else reads it.
- [ ] A URL or key changed while running applies to the next search and the next index cycle.
- [ ] Gate green; featureless clippy green; reviewer findings applied or deferred with the maintainer.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| URL and key are keyring entries `qdrant-url`, `qdrant-key` | true | `htui-store/src/secret.rs:43-45`, getters :303-355 |
| The keyring URL is read in 5 places | true | `search_text get_qdrant_url`: `concepts.rs:120, 362, 507`, `concepts_worker.rs:220`, `qdrant_settings_info.rs:51` |
| The TUI index re-reads settings per request and caches the connection by URL + key | true | `concepts_worker.rs` `QdrantIndex`, `Connections::get`, `qdrant_settings` |
| The worker's index job reads the keyring URL once at start | true | `concepts.rs` `spawn_index_job_over` → `keyring_settings` |
| The worker offers `search_concepts` only when the index job exists; the TUI always does | true | `worker_cmd.rs` `worker_search`, test `the_worker_searches_only_where_it_indexes`; `mcp_search.rs` `production` always `Some`, `lib.rs:209` |
| `app_settings()` is inherent on `PgStore` and `MemStore` | true | `concepts.rs` `impl IndexSource for PgStore/MemStore` call `self.app_settings()` |
| `MemStore` has `set_app_setting` | true | `htui-core/src/store/mem.rs:684` |
| The `app_setting` CAS statements exist and are themselves reused text | true | `pg/write.rs:5503-5548`, comment "byte for byte … reuse its offline entries" |
| Byte-identical SQL reuses a `.sqlx` entry | true | memory `sqlx-offline-hash-is-literal-query`; comment at `pg/write.rs:5504-5506` |
| `go_online` is the store worker's only in-place backend swap | true | `search_text "*backend = "` in `crates/htui/src`: one hit, `store_worker.rs:3501` |
| Agent children inherit the parent environment (no `env_clear`) | true | `htui-agent/src/launch.rs:1184-1200` `.envs(&launch.env)` only |
| Step shells inherit the process environment | true | `htui-orch/src/verify.rs:370-386` comment and builder; second spawn at `:552` |
| `remove_var` cannot be used | true | `Cargo.toml:194` `unsafe_code = "forbid"`; `verify.rs:373-375` comment |
| `headless_dsn` is the pattern for stdin / keyring / credential | true | `htui-store/src/secret.rs:262-301` |
| A second stdin line after the DSN is not lost | true | Rust `Stdin` shares one process-wide buffer; T3 reads both on the DSN reader's thread (`worker_cmd.rs` `read_dsn`) |
| `htui-core` has no `url` dependency | true | `crates/htui-core/Cargo.toml` |
| No CLI flag sets Qdrant today | true | `cli.rs` search `(?i)qdrant`: only `--index-items`/`--search-items` docs |
| Provision pins the unit's `LoadCredentialEncrypted=htui-dsn` line | true | `provision/script.rs:179, 263`; `tests/provision.rs:302` |
| The Settings section tests never touch the keyring | true | `tests/qdrant_settings.rs` module doc |
| `scripts/hr` exports no Qdrant URL for htui itself | true | `scripts/hr:636, 872` export only `HR_QDRANT` (storage dir) |
| `R-STO-1`, `R-TUI-8`, `R-NF-2` already describe the database-wide URL, the key's sources and the env strip | true | `docs/REQUIREMENTS.md:375-391, 755-773, 892-904` |
| Task independence: W1 T1 ∥ T2 | true | T1 {htui-core qdrant.rs, lib.rs, store/mem.rs; htui-store pg/write.rs, pg/read.rs, qdrant_settings.rs, tests/qdrant_url_pg.rs} ∩ T2 {htui-core secret.rs; htui-agent launch.rs; htui-orch verify.rs; htui-store secret.rs} = ∅ |
| Task independence: W3 T4 ∥ T5 ∥ T6 | true | T4 {qdrant_settings_info.rs, ui/tabs/settings/qdrant.rs, store_worker.rs, keys/**, app.rs, tests/qdrant_settings.rs, tests/settings.rs, snapshots}; T5 {cli.rs, lib.rs, qdrant_cmd.rs, tests/qdrant_cli_pg.rs, docs/htui-cli.md}; T6 {provision/plan.rs, provision/script.rs, tests/provision.rs, scripts/hr, docs/hr-sandbox.md}: pairwise ∅ |
| T3 cannot run beside T4 or T5 | true | T3 ∩ T4 = {store_worker.rs}; T3 ∩ T5 = {cli.rs} |

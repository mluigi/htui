# Plan: MOD-92 - Read-only `--json` CLI for htui state

**Source**: `HANDOFF.md` MOD-92 (from ANA-29, `docs/ANA-29.md` §5.3, option S4)
**Requirements**: `R-TUI-11`, `R-ID-2`, `R-ID-6`, `R-STO-1`, `R-NF-2`
**Route**: plan path (routing verdict accepted by the maintainer, 2026-10-08)
**Complexity**: Medium
**Status**: done (2026-10-13) - `docs/decisions/mod/mod-92.md`

## Summary

Three read-only verbs that print one versioned JSON document and exit: `htui status --json`,
`htui waiting --json`, and `--json` on `--search-items`. They connect over a new **read-only**
Postgres session (the existing headless connect writes), compute the waiting list through the
TUI's own classifier (`htui_worker::waiting`), resolve scope from the first workspace, `--project`
or `--cwd`, and answer an unreachable store with a typed `{"store":"offline"}` document and exit 3.
The document shapes are documented in `docs/htui-cli.md` and pinned by insta snapshots.

## Requirements restatement

1. `htui status [--json]` - the top bar's facts: store state, workspace, box, working count,
   waiting count (`ui/top_bar.rs:30`).
2. `htui waiting [--json]` - `R-TUI-11`'s rows, built by `htui_worker::waiting` over
   `waiting_candidates` + `active_runs` + `open_permissions`, exactly as `serve_waiting`
   (`store_worker.rs:2041`) does: reason, item key, run, step, **age**.
3. `htui --search-items <Q> --json` - the hits `format_hit` prints today, as JSON.
4. Scope: the active workspace (default), `--project <slug>`, or `--cwd <path>` (the project whose
   repo, registered on this box, contains the path).
5. DSN from the keyring (`secret::get_dsn`), as every other verb; never argv or env.
6. An unreachable store -> `{"version":1,"kind":...,"store":"offline",...}` on stdout, exit **3**.
   No mirror, no cache: never stale data presented as live.
7. **No writes** - not to Postgres, not to Qdrant, not to the local cache.
8. The schema is documented and pinned by tests (MOD-93 parses it).

## Grounding findings that shape the plan

| # | Finding | Evidence | Consequence |
|---|---|---|---|
| G1 | `PgStore::connect_headless` **writes**: `bootstrap` runs `seed_if_empty` and `register_box` (an `INSERT INTO box ... ON CONFLICT`, a `FOR UPDATE`). `--search-items` uses it today. | `htui-store/src/pg/mod.rs:285-326`, `:740-751`, `:767-788` | A new `PgStore::connect_read_only` that skips `bootstrap` and opens the pool with `default_transaction_read_only=on`; the CLI verbs use it, `--search-items` included. |
| G2 | `identity::load_or_mint` writes `box.toml` when missing or when the hostname changed. | `htui-store/src/identity.rs:83-112` | A read-only `identity::load` returning `Option<Identity>`; a box with no `box.toml` reports `"box": null`. |
| G3 | `WaitingRow` carries no timestamp, so "age" is not derivable from the view. | `htui-worker/src/views.rs:520-536` | Add `since: DateTime<Utc>` to `WaitingRow`, set in `waiting()` per reason (D5). |
| G4 | The TUI has no persisted "active workspace": it scopes to the **first** entry of `workspaces()`. | `htui/src/app/update.rs:516-531` | `status`/`waiting` default to `workspaces().first()`, the same rule. |
| G5 | `serve_waiting` needs a `Backend`; `Backend::Online` requires a `CacheStore` (a local file). | `htui-store/src/backend.rs:51-71`, `store_worker.rs:2041-2061` | The CLI calls the same three reads on `PgStore` and the same `htui_worker::waiting`, without a `Backend`. |
| G6 | `--cwd` needs nothing new in SQL: `PgStore::repo_paths(box_id)` + `WriteStore::repos(project)`. | `pg/read.rs:2033`, `pg/write.rs:2661` | No `.sqlx` churn for scope resolution. |
| G7 | `project.slug` is globally `UNIQUE`; a project can sit in several workspaces. | `migrations/0001_init.sql:145` | `--project` picks the first workspace (in `workspaces()` order) that lists it. |
| G8 | `QdrantStore::connect` **writes** too: "creating it and its payload indexes on first use", and stamps an unstamped collection with the embedder. | `htui-store/src/vector.rs` `connect` doc + `ensure_collection` | A read-only `QdrantStore::open_existing` (no `ensure_collection`, no stamp; same mismatch check). A missing collection answers `"index":"empty"`, `hits: []`, exit 0. |

## Decisions (maintainer may amend at CONFIRM)

- **D1 Command shape.** `status` and `waiting` are clap **subcommands** (`Command::Status(StateArgs)`,
  `Command::Waiting(StateArgs)`), each with `--json`, `--project <SLUG>`, `--cwd <PATH>` (the last two
  conflict). `--search-items` stays a flag and gains `--json` and `--cwd` (`--cwd` conflicts with
  `--project`). Without `--json`, `status` prints the top-bar line and `waiting` one row per line,
  overlay-style - cheap, and a subcommand that prints nothing without a flag would be odd.
- **D2 Read-only session.** `PgStore::connect_read_only(dsn, box_id: Option<BoxId>, timeout)`:
  `open_pool` with `PgConnectOptions::options([("default_transaction_read_only", "on")])`, then the
  headless migration check (`migrations_table_exists` / `applied_state` / `stored_target`, all
  SELECTs), **no** `bootstrap`. `this_box` = the `box.toml` id (or nil); `this_user` nil. Postgres
  itself then refuses any write (probe: `CREATE TABLE` -> "cannot execute CREATE TABLE in a read-only
  transaction"). `PoolSize` 1-2.
- **D3 Exit codes and documents.** Every outcome prints exactly one JSON document on stdout under
  `--json` (and a human line on stderr for failures):
  | Outcome | `store` | exit |
  |---|---|---|
  | answered | `"online"` | 0 |
  | Postgres unreachable (`StoreError::Unreachable`) | `"offline"` | **3** |
  | Qdrant unreachable (search only) | `"online"`, `"index":"offline"` | **3** |
  | refused: no DSN stored, keyring unreadable, migrations pending, below target / drifted schema, unknown `--project`, `--cwd` in no registered repo | `"refused"` + `error.code` | 2 |
  | anything else (a read failing mid-way with a non-Unreachable error) | `"error"` + `error.code` | 1 |
  A typed `StateExit { code }` error carries the code to `main::exit_code`; exit 2 and 3 do not go to
  Sentry (`main.rs` `reports_to_sentry`).
- **D4 Envelope.** `{"version": 1, "kind": "status"|"waiting"|"search", "store": ..., ...}`.
  `version` bumps only on a breaking change (field removed/renamed/retyped); added fields do not bump.
  Timestamps are RFC 3339 UTC; ids are UUID strings; `null` for absent.
- **D5 Age.** `WaitingRow.since`: Gate -> the parked step's `updated_at`; JudgeFailed -> the judge's
  `updated_at`; Selection / Interrupted -> the run's `updated_at`; Unblock -> the run's `updated_at`,
  or the item's `updated_at` for a Reopen row; Permission -> `permission.created_at`. The JSON row
  carries `since` and `age_seconds` (`now - since`, floored at 0). The TUI overlay does not show it
  (out of scope).
- **D6 `--search-items` scope.** Keeps today's default (every workspace) - changing it would change a
  shipped verb's output. `--project` as today; `--cwd` added. `status`/`waiting` default to the active
  workspace (G4).
- **D8 Model download is not a state write.** `--search-items` may still fill the embedding-model
  cache (`model::ensure_model_noting`, notes on stderr): a local download cache, not htui state, and
  needed to embed the query at all. Documented, not suppressed.
- **D7 No `Backend`, no mirror.** The CLI never opens the cache (G5), so "offline" can only mean
  "Postgres did not answer this call".

## Document shapes (pinned by T3 snapshots; documented in `docs/htui-cli.md`)

```json
// htui status --json
{"version":1,"kind":"status","store":"online",
 "workspace":{"id":"…","name":"Platform"},
 "projects":[{"id":"…","slug":"htui"}],
 "box":{"id":"…","hostname":"desk","registered":true},
 "working":1,"waiting":2,"permissions_known":true}

// htui waiting --json
{"version":1,"kind":"waiting","store":"online",
 "workspace":{…},"projects":[…],"working":1,"permissions_known":true,
 "rows":[{"reason":"gate","item_key":"MOD-4","item_id":"…","run_id":"…","step_id":"…",
          "step":"prd 0.1","text":"approve the PRD","since":"2026-10-08T20:01:02Z","age_seconds":93}]}

// htui --search-items q --json
{"version":1,"kind":"search","store":"online","index":"online","query":"q","decisions":false,
 "hits":[{"key":"ANA-11","place":"item","document_kind":null,"resolution":"done",
          "requirement_state":null,"score":0.731,"snippet":"…"}]}

// any verb, Postgres unreachable                       // a refusal
{"version":1,"kind":"status","store":"offline",         {"version":1,"kind":"waiting","store":"refused",
 "error":{"code":"unreachable","message":"…"}}           "error":{"code":"no_dsn","message":"…"}}
```

`reason` uses `WaitingReason::label` (`gate`, `judge failed`, `selection`, `unblock`,
`interrupted`, `permission`) - rendered snake_case in JSON (`judge_failed`).

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Verb module | `htui/src/concepts.rs:1-15`, `worker_cmd.rs` | one module per CLI verb, module doc names the MOD and requirement |
| Typed exit | `htui/src/worker_cmd.rs:20-35`, `main.rs` `exit_code` | an error enum with `const fn code()`, downcast in `main` |
| Headless refusal text | `htui/src/concepts.rs:133-155` | `headless_refusal` / `store_context`: only `Unreachable` is "cannot reach" |
| Never stdout panic | `htui/src/lib.rs` `print_keys` | write + flush, `BrokenPipe` is not an error |
| Scope building | `htui/src/concepts.rs:157-180` `scopes` | `Scope::from_workspace`, narrowed by slug |
| Pg test | `htui/tests/*_pg.rs`, `htui_store::testkit::{fresh_db, SKIP}` | `fresh_db()` -> `None` prints `SKIP` and returns |
| Snapshots | `insta` with `filters` (htui dev-dep) | ids / timestamps filtered |

## Files to change

| File | Action | Task |
|---|---|---|
| `crates/htui-store/src/pg/mod.rs` | UPDATE - `connect_read_only`, read-only pool options | T1 |
| `crates/htui-store/src/identity.rs` | UPDATE - `load(root) -> Result<Option<Identity>>` | T1 |
| `crates/htui-store/src/vector.rs` | UPDATE - `QdrantStore::open_existing` (read-only) | T1 |
| `crates/htui-store/tests/read_only_pg.rs` | CREATE - a write on the read-only store is refused; no `box` row appears | T1 |
| `crates/htui-worker/src/views.rs` | UPDATE - `WaitingRow.since`, set in `waiting()`; tests | T2 |
| `crates/htui/src/store_worker.rs` | UPDATE - `WaitingRow` literals in tests gain `since` | T2 |
| `crates/htui/src/ui/top_bar.rs` | UPDATE - test literal | T2 |
| `crates/htui/src/ui/overlay/waiting_list.rs` | UPDATE - test literals | T2 |
| `crates/htui/src/cli.rs` | UPDATE - `Status`/`Waiting` subcommands, `StateArgs`, `--json`/`--cwd` on search | T3 |
| `crates/htui/src/state_cmd.rs` | CREATE - connect, scope resolution, reads, documents, human output, `StateExit` | T3 |
| `crates/htui/src/concepts.rs` | UPDATE - `search_items` over `connect_read_only`, JSON hits, `--cwd` | T3 |
| `crates/htui/src/lib.rs` | UPDATE - `pub mod state_cmd;`, dispatch before `keys::resolve` | T3 |
| `crates/htui/src/main.rs` | UPDATE - `exit_code` / `reports_to_sentry` know `StateExit` | T3 |
| `crates/htui/tests/state_cli_pg.rs` | CREATE - end-to-end over `fresh_db`; insta snapshots | T3 |
| `docs/htui-cli.md` | CREATE - the schema, exit codes, versioning rule | T4 |
| `README.md` | UPDATE - `Command-line options` table, a short "Scripting" section | T4 |

## Tasks (TDD: tests first in each)

### T1 - Read-only store session (htui-store) - **independent of T2**
- Tests first: `tests/read_only_pg.rs` over `fresh_db()` - (a) `connect_read_only` on a migrated
  db answers `workspaces()`; (b) a raw `INSERT` through its pool fails with a read-only error;
  (c) the `box` table row count is unchanged after connect; (d) a pending-migration db is
  `HeadlessError::MigrationsPending`. Unit: `identity::load` on a missing file is `Ok(None)` and
  writes nothing; on a valid file returns the id without rewriting a changed hostname.
- Implement D2 and `identity::load`.
- `QdrantStore::open_existing` (G8): tested beside the existing Qdrant cases (`HTUI_TEST_QDRANT_URL`,
  throwaway collection) - a missing collection is reported, not created; an existing one answers
  `search`; an embedder mismatch is still `embedder mismatch`.
- **Files**: `pg/mod.rs`, `identity.rs`, `vector.rs`, `tests/read_only_pg.rs`.
- **Validate**: `cargo test -p htui-store --all-features --test read_only_pg -- --test-threads=1`.

### T2 - `WaitingRow.since` (htui-worker) - **independent of T1**
- Tests first in `views.rs`: one per reason asserting `since` per D5.
- Add the field, set it in `waiting()`, fix every `WaitingRow { .. }` literal (16, in the four files
  listed).
- **Files**: `views.rs`, `store_worker.rs`, `ui/top_bar.rs`, `ui/overlay/waiting_list.rs`.
- **Validate**: `cargo test -p htui-worker`; `cargo test -p htui --all-features waiting top_bar`.

### T3 - The verbs (htui) - **after T1 and T2**
- Tests first: `cli.rs` parse tests (subcommands, conflicts `--project`/`--cwd`, `--json` requires
  `--search-items` at top level); `state_cmd` unit tests for the pure parts - document builders from
  fixed inputs (insta, `version`/`kind`/field names pinned), `--cwd` longest-prefix resolution,
  exit-code mapping; `tests/state_cli_pg.rs` over `fresh_db()` - seeded workspace, a parked gate,
  `status`/`waiting` documents, `--project`, `--cwd`, unknown slug -> refused/2, an unreachable DSN
  (`postgres://…@127.0.0.1:1/x`) -> offline/3.
- `state_cmd::run(cmd, out: &mut dyn Write) -> Result<(), StateExit>` reads the keyring;
  `run_with_dsn(...)` is the testable seam.
- **Files**: `cli.rs`, `state_cmd.rs`, `concepts.rs`, `lib.rs`, `main.rs`, `tests/state_cli_pg.rs`.
- **Validate**: `cargo test -p htui --all-features --test state_cli_pg -- --test-threads=1`, unit tests.

### T4 - Documentation - **after T3** (shapes come from its snapshots)
- `docs/htui-cli.md`: every document, every field, D3's table, D4's versioning rule, "no writes"
  and how it is enforced. README rows for `status`, `waiting`, `--json`, `--cwd`.

## Validation (the gate)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo sqlx prepare --workspace --check   # only if T1 adds a query! macro
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A read used by the verbs is not a plain SELECT (e.g. a `FOR UPDATE`) and fails read-only | Low | T1/T3 Pg tests run every read on the read-only pool; a failure surfaces as `error`/1, never a write |
| `WaitingRow` gains a field the TUI's `Eq` / snapshots depend on | Low | `DateTime<Utc>` is `Eq`; the overlay does not render it, so no `.snap` changes |
| `updated_at` is not exactly "parked at" for a gate (a later note edit bumps it) | Medium | Documented as "the row's last change" in `docs/htui-cli.md`; exact park time is a later schema item |
| Keyring fake is process-wide (memory: suite green is scheduling-dependent) | Medium | the Pg tests go through `run_with_dsn`, not the keyring; gate with `--test-threads=1` |
| Qdrant / model load for `--search-items --json` prints download notes | Low | notes stay on stderr; stdout carries only the document |

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `QdrantStore::connect` creates the collection / indexes | TRUE | `vector.rs` `connect` doc "creating it and its payload indexes on first use"; `connect_to` calls `ensure_collection` |
| `connect_headless` writes (seed + box registration) | TRUE | `pg/mod.rs:324` `bootstrap`; `:741` `seed_if_empty`; `:770` `INSERT INTO box` |
| `load_or_mint` writes `box.toml` | TRUE | `identity.rs:97-104` `store(root, …)` |
| `WaitingRow` has no timestamp | TRUE | `views.rs:520-536` |
| TUI's startup scope is `workspaces().first()` | TRUE | `app/update.rs:519-522` |
| `Backend::Online` requires a `CacheStore` | TRUE | `backend.rs:55-60` |
| `PgConnectOptions::options` exists in the pinned sqlx | TRUE | sqlx-postgres 0.9.0 `src/options/mod.rs:436`; workspace pins `0.9.0` |
| `default_transaction_read_only=on` refuses writes, temp tables included | TRUE | psql probe on `localhost:5439`: "cannot execute CREATE TABLE in a read-only transaction" |
| headless migration checks are reads | TRUE | `migrations_table_exists` (`to_regclass` SELECT, `:858`), `stored_target` (SELECT, `:910`), `applied_state` doc "Writes nothing" (`:815-817`) |
| `open_permissions` on PgStore is a SELECT | TRUE | `pg/relay.rs:672-` single `SELECT … EXISTS (SELECT …)` |
| `active_runs`, `box_row`, `repo_paths` are `pub` on `PgStore` | TRUE | `pg/read.rs:1408`, `:1992`, `:2033` |
| `repos(project)` exists on PgStore | TRUE | `pg/write.rs:2661` (`WriteStore`) |
| `project.slug` globally unique | TRUE | `0001_init.sql:145` |
| permission timestamp available for D5 | TRUE | `relay.rs:107` `StepPermission.created_at`; `WaitingPermission.permission` (`waiting.rs:133`) |
| `WaitingRow` literals: 16 in 4 files | TRUE | grep over `crates/` - `views.rs`, `store_worker.rs`, `top_bar.rs`, `waiting_list.rs` |
| Pg test harness `fresh_db` / `SKIP` | TRUE | `htui-store/src/testkit.rs:34`, `:151` |
| htui has `serde`, `serde_json`, `chrono`, `insta` | TRUE | `crates/htui/Cargo.toml` |
| exit 2 skips Sentry already | TRUE | `main.rs` `reports_to_sentry`: `code != 2 && …` |
| **T1 ∩ T2 file sets** | **∅** - parallel OK | T1 `{pg/mod.rs, identity.rs, vector.rs, tests/read_only_pg.rs}` vs T2 `{views.rs, store_worker.rs, top_bar.rs, waiting_list.rs}` |
| T3 depends on T1 and T2 | TRUE (API) | uses `connect_read_only`, `identity::load`, `WaitingRow.since`; T3 shares no file with T1 and only reads T2's |
| T4 depends on T3 | TRUE | documents T3's snapshots |

## Acceptance

- [ ] `htui status --json`, `htui waiting --json`, `htui --search-items q --json` print one document each
- [ ] Unreachable Postgres -> `"store":"offline"`, exit 3; nothing from the cache
- [ ] Read-only enforced by the session (`default_transaction_read_only`), pinned by a test
- [ ] `--search-items` creates no Qdrant collection, index or stamp
- [ ] Waiting rows match the TUI's (same `htui_worker::waiting`), with `since`/`age_seconds`
- [ ] Scope: first workspace / `--project` / `--cwd`, each tested
- [ ] Every document shape pinned by an insta snapshot and documented in `docs/htui-cli.md`
- [ ] Gate green (fmt, clippy x2, serial tests, validator)

## Amendments after the blueprint (2026-10-08)

The blueprint (`.claude/plans/mod-92-json-cli.blueprint.md`, "Plan amendments and clarifications")
records A-1 to A-11; they are accepted as written. Two needed the maintainer:

- **A-3 accepted:** `--search-items` uses the same exit codes with and without `--json` (3 offline,
  2 refused, 1 error); messages unchanged; the README notes the change.
- **Sentry:** exit 1 from the new verbs stays reported (plan D3 as written); 2 and 3 never report.

Execution: T1 on the primary tree, T2 in worktree `target/wt/mod-92-t2` (branch `mod-92-t2`) to avoid
the htui-store compile coupling, merged before T3.

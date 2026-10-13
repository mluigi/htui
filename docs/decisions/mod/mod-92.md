# MOD-92 - Read-only `--json` CLI for htui state (done, 2026-10-13)

**Requirements:** `R-TUI-11`, `R-ID-2`, `R-ID-6`, `R-STO-1`, `R-NF-2`.
**Origin:** ANA-29 (`docs/ANA-29.md` §5.3, option S4; `docs/decisions/ana/ana-29.md`).
**Artifacts:** plan `.claude/plans/mod-92-json-cli.plan.md` (decisions D1-D8, verified-claims table,
post-blueprint amendments), blueprint `.claude/plans/mod-92-json-cli.blueprint.md` (amendments
A-1..A-11). Routed as **plan** (C2 only; C4 borderline), no ultracode.
**User guide and schema:** `docs/htui-cli.md`; README "Command-line options" and "Scripting htui".

## The problem

Nothing outside the TUI could read htui's state: the CLI had no `status`, no `waiting` and no
`--json`. MOD-93 (a Claude Code plugin) and any agent working beside htui need the top bar's facts
and the waiting-on-you list as a stable, parseable document, without a daemon and without the DSN
leaving the keyring.

## What shipped

- **`htui status [--json] [--project SLUG | --cwd PATH]`** - the top bar: store state, workspace,
  projects in scope, this box (`id`, `hostname`, `registered`), `working` and `waiting` counts,
  `permissions_known`.
- **`htui waiting [--json] [--project SLUG | --cwd PATH]`** - `R-TUI-11`'s rows, classified by the
  TUI's own `htui_worker::waiting` over `active_runs` + `waiting_candidates` + `open_permissions`
  (the reads `serve_waiting` makes), each with `reason`, item key and id, run, step, text, `since`
  and `age_seconds`.
- **`htui --search-items Q [--json] [--cwd PATH]`** - the hits as JSON; default scope stays every
  workspace (D6).
- **Envelope** `{"version":1,"kind":"status"|"waiting"|"search","store":…}`; `version` bumps only on
  a breaking change. Every document shape is pinned by insta snapshots
  (`crates/htui/src/snapshots/htui__state_cmd__tests__*.snap`,
  `crates/htui/tests/snapshots/state_cli_pg__*.snap`).
- **Exit codes** (both modes, maintainer-accepted A-3): 0 answered; 3 Postgres or Qdrant unreachable
  (`"store":"offline"` / `"index":"offline"`, never cached data); 2 refused (no DSN, keyring
  unreadable, schema not ready, unknown project, `--cwd` unmatched or ambiguous, embedder mismatch,
  Qdrant auth refused, a pooler rejecting the read-only parameter, malformed `box.toml`); 1 any other
  failure. 2 and 3 never reach Sentry; 1 does (maintainer). A usage error, an unopenable `--log` and
  a panic print no document (A-1).
- **Scope:** the TUI's startup workspace (the first of `workspaces()`; htui keeps no "active"
  workspace), `--project` (slugs are globally unique), or `--cwd` (deepest registered checkout of
  this box; ties across projects refuse).

## Read-only, enforced

The headless paths the CLI used to share both wrote: `PgStore::connect_headless` seeds an empty
database and registers this box, `identity::load_or_mint` and `config_root()` create files, and
`QdrantStore::connect` creates and stamps the collection. MOD-92 adds read-only twins:

- `PgStore::connect_read_only` - `default_transaction_read_only=on` as a startup option (a DSN asking
  for `off` is still read-only, tested), the headless refusal checks factored into a shared
  `headless_checks`, no `bootstrap`; `PgStore::close` ends every verb so a poller leaves no EOF lines.
- `identity::load` - reads `box.toml`, never writes; the verbs use `config_root_path()`.
- `QdrantStore::open_existing` / `ExistingCollection::find` - no create, index or stamp; the
  collection is found before the embedding model loads, so Qdrant down or no collection never
  downloads the model (review M3). The model cache may still fill on a real search (D8).
- No `Backend`, no mirror: "offline" only ever means "this call could not reach Postgres".

`WaitingRow` gained `since: DateTime<Utc>` (the row's last change: gate/judge step `updated_at`, run
`updated_at` for selection/interrupted/unblock, item `updated_at` for a Reopen row, permission
`created_at`); the TUI overlay does not render it.

## Review

`rust-reviewer`: approve with fixes, no CRITICAL/HIGH. All 13 findings applied (`d62744cb`,
`2c29336f`, `e491de50`, `4662626d`, `2d00215b`): 08P01 from a pooler is a refusal not "offline"
(M1); Qdrant auth refusal is `qdrant_refused`/2 and a query-embed failure `model_unavailable`/1
(M2); Qdrant before the model (M3); a `search_answer_with` seam with real composition tests (M4);
README D8 exception, read-only override test, `box.toml` behaviour documented, `--cwd` compares
stored (already canonical) paths lexically, pools closed, `human_text` prints nothing, doc example
and help-order fixes (L5-L13).

## Commits

Plan `8e620755`, blueprint `bfe37c00`, amendments `040dd18d`; T1 `45ad8084` `28eb3736` `e40cd2ef`
`b83c6baf`; T2 `d45c3567` `d105cef7` (merged `53819dae`); T3 `ec4dd29d` `3c3b0b79` `10f08ef3`
`1b5e5388`; T4 `9c80d2b7`; review `d62744cb` `2c29336f` `e491de50` `4662626d` `2d00215b`.

## Gate

Final workspace gate on `2d00215b` (sandbox Postgres and Qdrant, serial): `cargo fmt --all --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and the featureless clippy clean;
`cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`: 5,957 passed, 30 ignored, 1 failed -
the pre-existing MOD-95 test (fails identically on `main`); no Postgres suite skipped. Workflow-docs validator: 0
errors, 0 warnings.

## Follow-ups

- **MOD-95** filed: `auto_queue::a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` fails on `main`
  `19b708fa` too (batch closed as `drained`), not caused by MOD-92.
- Owed on the host: the Postgres gate on the merged tree; on Windows, the verbs with a keyring DSN
  (MOD-16's ANA-29 note). This sandbox has no secret service, so a real-keyring smoke answered
  `keyring_unreadable`/2.
- MOD-93 consumes `docs/htui-cli.md`; MOD-94 builds its `permission list --json` on the same envelope.

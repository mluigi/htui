# Blueprint: MOD-40, milestones 2 and 3 (T3-T6)

**Status**: proposed (2026-09-29). Milestones 2 and 3. Continues
`.claude/plans/mod-40-multi-writer-hardening.blueprint.md` (milestone 1): findings continue at
**F-13**, decisions at **B10**. A **Blocker** means the plan, read literally, fails an existing
test, does not compile at a task boundary, or cannot pin its own claim. The Fix column is what the
implementer builds.

**Plan**: `.claude/plans/mod-40-multi-writer-hardening.plan.md`, CONFIRMED 2026-09-26, OQ-1 and
OQ-2 as recommended. Its D4-D9 and T3-T6 are the scope here, amended only where §0 says so.
**PRD**: `.claude/prds/mod-40-multi-writer-hardening.prd.md`; PRD D3 wins over this blueprint.

**Verified at**: `fbd7d43` (milestone 1 T1 red: the fence is threaded through every signature).
Paths are relative to `crates/`. **Line numbers are at `fbd7d43`**; milestone 1's green commits
move `pg/write.rs`, `mem.rs`, `writer.rs`, `record.rs` and `engine.rs` by a few dozen lines, so the
implementer re-greps the quoted anchor text, never the number alone. Every fact was read with
`grep`/`sed`. `graphify-out/` does not exist and Gortex is not reachable.

**Layout**: §0 findings and decisions; Milestone 2 = §1 build order, §2 T3, §3 T4; Milestone 3 =
§4 build order, §5 T5, §6 T6; §7 call-site sweep groups; §8 `.sqlx`; §9 file-by-file checklist;
§10 gates.

**Probes** (Postgres 16.13, `localhost:5432`, scratch database `mod40_m23c` built from migrations
`0001`–`0007` with `psql`, dropped afterwards):
- P-6. The §2.2 quota statement on one `agent_box` row: `quota_at` `NULL` + `t2` → `(written t,
  present t)`; then `t1 < t2` → `(f, t)`, the stored pair still `t2`; `t2` again → `(t, t)`, the
  document replaced; `t3` → `(t, t)`; a missing key → `(f, f)`.
- P-7. The §3.3 agent statements: `INSERT … ON CONFLICT (id) DO NOTHING RETURNING …` on a new id →
  one row; on a stored id → `INSERT 0 0`, **also when the name is held by another id** (the id
  arbiter is checked first); a new id under a held name → `23505 agent_name_key`. `UPDATE … WHERE
  id = $1 AND updated_at = $t` on the current token → one row, `updated_at` moved by the trigger;
  a spent token → `UPDATE 0`, **also when the new name is held** (no row, no unique check); the
  current token with a held name → `23505`; an absent id → `UPDATE 0`.
- P-8. `sqlx-postgres-0.9.0/src/types/chrono/datetime.rs:36-42` encodes a `DateTime<Utc>` as
  `num_microseconds()` since 2000, i.e. **truncates** to µs, the same on every bind; decoding is
  exact µs.
- P-9. `CREATE TABLE IF NOT EXISTS _sqlx_migrations (…)` run by a role with `USAGE` but no
  `CREATE` on `public`, **with the table already present**: `ERROR: permission denied for schema
  public`. (sqlx 0.9's `ensure_migrations_table` is exactly that statement,
  `sqlx-postgres-0.9.0/src/migrate.rs:122-144`.) The scratch role was dropped.
- P-10. The §6.2 target statements: `pg_prepare` reports parameter types `{text}` for the
  `SELECT value … FOR UPDATE` and `{text,text}` for the insert and the update; `INSERT … VALUES
  ($1, to_jsonb($2::text)) ON CONFLICT (key) DO NOTHING` on an absent key → `INSERT 0 1`, the
  value `"0.1.0"` (a JSON string, `jsonb_typeof = string`); again → `INSERT 0 0`, the row
  unchanged; the conditional `UPDATE` → `UPDATE 1` and `updated_at` moved by the trigger.

**Scope**: no migration; `TABLES` 39, the migration pins and the commented-column pins do not
move. Store `CASES` 89 → **95** (T3 three, T4 three). `.sqlx/` 281 → **288** (§8). New public
items: `PgStore::touch_box`, `PgStore::connect_headless`, `PgStore::below_target`,
`HeadlessError`, `TARGET_VERSION_KEY`, `connect::BOX_HEARTBEAT`, `Started::box_heartbeat`,
`StoreReply::StoreState::below_target`, `htui_core::fixtures::edit_agent`.

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn and clippy runs `-D warnings`;
rustdoc denies broken/private intra-doc links. `max_width = 100`. Every commit compiles; red first,
then green. `every_cross_referenced_test_name_exists` (`htui-core/src/store/conformance.rs`, its
`tests` module) fails on a backticked snake_case name of ≥4 underscores that `conformance.rs` /
`mem.rs` do not define: in `htui-core` docs, backtick only names those two files define.

---

## 0. Findings (continued from milestone 1's F-1 … F-12)

| # | Severity | Plan says | Tree / probe | Fix |
|---|---|---|---|---|
| **F-13** | Minor (wording; no code) | D6: "`edit_box` is the only `box.settings` writer and is already a CAS"; T3 adds `a_stale_box_edit_diverges` "if absent". | `edit_box` never writes `settings`: its own doc lists `settings` among the columns it never touches (`traits.rs:414-417`) and its `SET` names `declared_tags`, `quirks`, `edit_version` only (`pg/write.rs:1351-1356`). **Nothing** writes `box.settings`: `register_box` (`pg/mod.rs:389-391`, `:459-461`), `record_box_probe` (`traits.rs:390-393`) and the inserts (`pg/mod.rs:559`, `pg/demo.rs:67`) leave it at its default. The stale case already exists: `edit_box_is_cas_on_edit_version` asserts a spent token is `Stale` with the row as it is now and writes nothing (`conformance.rs:7450-7470`), and `edit_box_survives_a_probe_between_read_and_write` (`:7516`) pins the probe interleaving. | B10: **no new case**; `a_stale_box_edit_diverges` is not added. The C7 invariant goes on `edit_box`'s trait doc, phrased as what is true: `box.settings` has no writer, and the first one extends `BoxEdit` under this same CAS (§2.4). |
| **F-14** | Major (the D4 split is a TOCTOU) | D4: `NotFound` "only when the row is absent (one follow-up read)". | A read after a zero-row `UPDATE` is a second snapshot: a probe that inserts the row in between turns "absent" into `Ok(false)` ("a newer quota is stored") on a row whose `quota_at` is `NULL`. Milestone 1 removed the same shape from `append_events` for the same reason (F-2, B1). | B11: one statement answers both facts from one snapshot, `(written, present)` (P-6). No follow-up read, and the statement **replaces** the old one, so `.sqlx` does not grow for T3 (§2.2). |
| **F-15** | Minor (sweep size) | T3 lists every quota caller as a file to change. | The return moves `()` → `bool`, and `bool` is not `#[must_use]`: every call site written `.await.expect("…");` still compiles and discards it (`htui-orch/src/conformance.rs:2469`, `htui/src/agent_worker.rs:5101`, `htui-core/src/store/conformance.rs:1672`, `:1737`, `:1775`, `htui-store/tests/pg_criteria.rs:1608`, `:1757`, `htui-core/src/store/mem.rs:6878`, `:7215`). What breaks is the five implementors and the one `Ok(()) =>` arm (`htui-agent/src/record.rs:1201`). | §2.6 names the sites that **should** assert `true` anyway (the latch lands on a row just written), so a regression to "older" is caught where it would bite; the rest stay byte-for-byte. |
| **F-16** | Major (D5 has no stored row to answer with) | D5: `CasOutcome<Agent>`, "`Stale` with the stored row". | `WriteStore` has no agent read, and `PgStore` has no single-agent read either: `agents()` (`pg/read.rs:1285-1340`) is the registry joined to this box. `Applied` must carry the row **as stored** (trigger-stamped `updated_at`) and `Stale` the row as it is now. | B12: both Pg branches `RETURNING` the whole row into `query_as!(Agent, …)`, and a private `stored_agent(id)` reads it on a miss (`set_setting`'s `stored_setting` + `cas_miss` shape). `.sqlx`: −1 +3. |
| **F-17** | Major (a trap for every test that chains a token) | D5: "`Some(t)` = update `WHERE … updated_at = $t`". | The insert keeps the caller's `updated_at` (the trigger is `BEFORE UPDATE` only, `0001_init.sql:566-580`). Postgres stores it truncated to µs (P-8) while `MemStore` keeps nanoseconds (`mem.rs:1532-1534`). A test that passes its **own** struct's `updated_at` as the next token happens to work on both today (the bind truncates the token the same way), but only by accident, and it fails the moment a caller normalises one side. | Every token in this blueprint comes from a row the store answered: `Applied(row).updated_at`, `Stale(row).updated_at`, or `agents()`. The conformance cases (§3.6) are written that way, and `edit_agent` (B13) takes the token from the row it is handed, which callers read from the store. |
| **F-18** | Minor (precedence must match across stores) | D5 does not say what wins between `Stale`, `NotFound` and the name's `Constraint`. | Postgres decides by statement semantics (P-7): `None` on a stored id is `Stale` even under a held name; `Some(t)` spent is `Stale` even under a held name; `Some(t)` current + held name is `Constraint`; `Some(_)` on no row is `NotFound`. `MemStore` today checks the name **first** (`mem.rs:1517-1526`). | B12: `MemStore` reorders to Postgres's order: id/token, then name, then write. The order is in the trait doc (§3.1) and pinned by the three cases (§3.6). |
| **F-19** | Minor (the helper D5 asks for) | D5: edit sites "go through a test helper that reads the row's `updated_at` first". | All 11 edit sites outside the two rewritten cases (§3.7) already hold the row they read from `agents()` (`summary.agent`), so its `updated_at` **is** the token; none rebuilds an `Agent` for an existing id. The three that re-upsert a constructed struct are the two cases being rewritten (`conformance.rs:1560`, `:1582`, `pg_criteria.rs:1238`). The sites span four crates' test targets. | B13: one helper, `htui_core::fixtures::edit_agent(store, &row) -> Result<Agent>`, passes `Some(row.updated_at)` and panics on `Stale` with the row named. `fixtures` is behind `demo`, which every site already has (`MemStore::demo()`, `htui`'s normal dependency `features = ["demo"]`, `htui/Cargo.toml:29`). |
| **F-20** | Major (T5's worker test cannot be written) | D7: `BOX_HEARTBEAT = 60 s`; T5: "a store-worker test that the arm is gated on a writer". | The period is a constant in the loop, so a test would wait 60 s. Pausing tokio time is not an option with a real pool: auto-advance fires sqlx's `acquire_timeout` while the runtime waits on the socket. The loop's other periods are injected (`runs.sweep_every()`, `store_worker.rs:1419-1431`; `Started::settings.interval`, set by a test at `:2349`). | B14: `pub const BOX_HEARTBEAT` in `htui-store/src/connect.rs` beside `RECONNECT` (`:34`) and a `Started::box_heartbeat` field the two constructors fill (`:271-283`, `:387-400`). T5's file list gains `htui-store/src/connect.rs`. |
| **F-21** | Minor (record) | D7: `UPDATE box SET last_seen_at = clock_timestamp()`. | `box` is in the trigger loop (`0001_init.sql:575-580`), so every beat also moves `box.updated_at`. Harmless: the box editors' token is `edit_version` (`traits.rs:412-430`), and the Boxes tab already treats a moved `updated_at`/`last_seen_at` as "not stale" (`htui/tests/box_settings.rs:404-415`, `reconnected`); the mirror re-reads the own box row whole every pass (`cache/refresh.rs:624-640`). | Said in `touch_box`'s doc (§5.1). |
| **F-22** | Major (a headless connect writes DDL) | D8: `connect_headless` "sharing `connect_with`'s pool and `schema_state`"; T6: "`_sqlx_migrations` unchanged". | `schema_state` starts with `ensure_migrations_table` (`pg/mod.rs:590-592`), a `CREATE TABLE IF NOT EXISTS`. On an empty database a "never migrates" connect would create the migrations table; under a role without `CREATE` on `public` it fails **even when the table exists** (P-9), so MOD-41's least-privilege worker could never connect. | B15: `schema_state` splits into the TUI's `ensure` + a read-only `applied_state`; the headless path asks `to_regclass` and never ensures (§6.3). A missing table is `MigrationsPending(<embedded count>)`. |
| **F-23** | Major (the TUI never hears it) | D9: `Connected.below_target: Option<String>`; "the TUI shows it once … through the existing `StoreState` reply". | `connect::dial` turns `Connected` into `ConnEvent::Online(store)` (`connect.rs:433-440`); the worker only ever holds the `PgStore`. The `ApplyMigrations` path (`store_worker.rs:1449-1494`) never sees a `Connected` at all, and it is where a migrating TUI learns the target it could not raise. Carrying it on `Connected` needs `ConnEvent`'s shape changed at 15 sites and still misses the apply path. | B16: the fact lives on the store: a private `PgStore::below_target` field set by `connect_with` and by `apply_migrations`, read by `PgStore::below_target()`. `Connected` is **unchanged**. The worker copies it into `StoreReply::StoreState`, and the shell sets `status` once (§6.6). |
| **F-24** | Major (two first appliers collide) | D9: "`SELECT … FOR UPDATE`, compare with `semver`, write only if ours is greater or the row is absent". | `FOR UPDATE` locks rows, and an absent row is none: two TUIs applying the first migration after this lands both read nothing and both `INSERT`; the second fails `23505` on `app_setting_pkey`, and its `apply_migrations` answers an error after the schema is already applied. | B17: `INSERT … ON CONFLICT (key) DO NOTHING` first (the second waits for the first's commit, then does nothing), then `SELECT … FOR UPDATE`, then the conditional `UPDATE`, one transaction (§6.2). |
| **F-25** | Minor (maintainer: PRD text) | D9: key `htui_target_version`. | PRD D3 spells it `htui.target_version`. Neither collides: no migration, seed or source names either (grep of `crates/` for `target_version`, `htui_target`, `htui.target`: none), `app_setting.key` has no `CHECK`, and all 22 keys a migrated database holds are snake_case (probe listing). No reader iterates the map where a stranger key would surface (§6.7). | Blueprint uses the plan's `htui_target_version` (`TARGET_VERSION_KEY`). The PRD's D3 sentence is corrected at bookkeeping. |
| **F-26** | Minor (maintainer: version source) | D9: "compare with `semver`" against "ours". | "Ours" is `HTUI_VERSION = env!("CARGO_PKG_VERSION")` **of `htui-store`** (`pg/mod.rs:54`). The five crates each declare `version = "0.1.0"` on their own (`crates/*/Cargo.toml:3`); there is no `version.workspace`. A release that bumps only `crates/htui/Cargo.toml` leaves `HTUI_VERSION` and therefore the target where it was. Today every crate is `0.1.0`, so every target this code writes is `"0.1.0"`. | No code change in MOD-40. The maintainer decides at release time: bump in lockstep, or move to `version.workspace = true` (a CLEAN item). Named in the handback. |
| **F-27** | Minor (file sets) | T6 files: `pg/mod.rs`, `lib.rs`, `connect.rs`, `concepts.rs`, `store_worker.rs` "+ the shell's status notice", two test files, `.sqlx`. | `htui-store/Cargo.toml` has no `semver` (only `htui-agent/Cargo.toml:40` does). `StoreReply::StoreState` is built at `store_worker.rs:1226`, `:1445`, `app/update.rs:625`, `testkit.rs:261`, `:492`, `htui/tests/shell.rs:22`, and destructured without `..` at `app/update.rs:253`. `connect.rs` is **not** touched by T6 under B16 (T5 touches it, F-20). | §9's lists. |
| **F-28** | Minor (undefined input) | D9 does not say what a malformed stored target means. | `app_setting.value` is `jsonb`; a hand edit can store `42` or `"banana"`. | B18: `apply_migrations` replaces a malformed target with ours (a migrating TUI is the authority) and logs `warn!`; `connect_with` logs and treats it as absent (a TUI runs); `connect_headless` refuses with `HeadlessError::Store(Backend(…))` naming the key (a headless process never guesses). |
| **F-29** | Minor (record) | D8: "never calls `MIGRATOR.run`". | Today's non-interactive caller already **writes** on connect: `PgStore::connect` bootstraps (`seed_if_empty_as`'s `LOCK TABLE app_user` and inserts, `register_box`), `pg/mod.rs:188-190`, `:524-535`. "Never migrates" is not "never writes". | B19: `connect_headless` keeps the bootstrap (MOD-41's worker needs `this_box`/`this_user`), **after** every refusal: a refused headless process has written nothing. Said in its doc. |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B10** (F-13): C7 is a doc on `edit_box`; no case is added.
- **B11** (F-14): the quota write's verdict `(written, present)` comes from the statement itself.
- **B12** (F-16, F-18): `upsert_agent` answers the stored row, `Applied` from `RETURNING`, `Stale`
  from `stored_agent`; precedence on both stores is id/token → name → write.
- **B13** (F-19): `htui_core::fixtures::edit_agent` for every edit of a row read back.
- **B14** (F-20): the heartbeat period rides `Started`; one beat in flight at a time; a failed
  beat never moves the backend.
- **B15** (F-22): `schema_state` = `ensure` + read-only `applied_state`; headless never ensures.
- **B16** (F-23): `below_target` is a `PgStore` fact; `Connected` and `ConnEvent` unchanged.
- **B17** (F-24): insert-if-absent, lock, compare, update, in one transaction.
- **B18** (F-28): malformed target: replaced by a migrator, ignored by a TUI, refused headless.
- **B19** (F-29): headless order is pool → read-only schema check → target → bootstrap.
- **B20**: the T3 and T4 cases are appended to `CASES` after
  `"a_missing_step_keeps_its_old_error_not_fenced"` (`conformance.rs:133`), T3's three then T4's
  three; arms before `other => panic!` (`:310`); bodies after
  `a_missing_step_keeps_its_old_error_not_fenced` (before `fn fixture_box`, `:5958`).

---

## Milestone 2 — ordered writes (T3, T4)

### 1. Build order

| Task | Crates | Commits (each compiles) | Gate |
|---|---|---|---|
| T3 | htui-core, htui-store, htui-agent (+ one `htui-orch` / `htui` test site each only if the implementer chooses to assert, §2.6) | (1) red: `set_agent_box_quota -> Result<bool>` on the trait and all five implementors, still **unguarded** (Pg answers `rows_affected() == 1`, Mem always writes), `latch_quota`'s arm, the three store cases, the Mem and Pg read-back tests (they fail: an older latch overwrites); (2) green: the guard on both stores, the one-statement Pg verdict, `.sqlx`, docs (incl. the C7 invariant, B10). | §10 T3 |
| T4 | htui-core, htui-store, htui-agent, htui (tests) | (1) red: the new signature on the trait, five implementors and every call site (§3.7), `fixtures::edit_agent`, the three store cases; Pg and Mem **still overwrite** under `Some(_)` and treat `None` as the old upsert, answering `Applied(<row as stored>)` (the cases fail on `Stale`/`NotFound`); (2) green: the CAS on both stores, `stored_agent`, `.sqlx`, docs. | §10 T4 |

The call-site sweep of T4 commit 1 is mechanical and parallel over four disjoint file groups
(§7). Everything else in T3/T4 is serial (shared `traits.rs`, `mem.rs`, `pg/write.rs`).

### 2. T3 — quota order (D4) and the C7 pin (D6)

#### 2.1 `htui-core/src/store/traits.rs` — `set_agent_box_quota`

Signature (`:382-388`):

```rust
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool>;
```

Doc (`:354-381`): keep the first four paragraphs and `# Nothing can clear them yet (review L-6)`;
insert before `# Errors`, and replace `# Errors`:

```rust
    /// # Newest wins (MOD-40 plan D4)
    ///
    /// The write lands only when `quota_at` is at least the stored one: `quota_at IS NULL OR
    /// quota_at <= $quota_at`. Two chats on one box latch the same row from two processes, and a
    /// report that arrives late must not overwrite a newer allowance with an older one. `<=`, not
    /// `<`: a second latch of the same instant rewrites the document, which is how one session
    /// refreshes the spend under an unchanged `observed_at`. Callers pass microseconds, as the
    /// recorder does (`stamp`), so the comparison means the same on every store.
    ///
    /// Answers `true` when the pair was written and `false` when an equal-or-newer `quota_at`
    /// was already stored and nothing was written. `false` is not an error: the latch is
    /// best-effort, and "somebody newer got there first" is the ordering working.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "agent_box"` and
    /// id `"<agent_id>/<box_id>"` when no row has that key, whatever `quota_at` is.
```

#### 2.2 `htui-store/src/pg/write.rs` — `set_agent_box_quota` (`:1101-1139`), whole replacement

```rust
    /// The two-column quota latch of `docs/ANA-4.md` §7 (MOD-2 plan D67), keyed on the composite
    /// primary key, newest `quota_at` winning (MOD-40 plan D4).
    ///
    /// Two columns and no more: `probe` and the four discovery columns belong to the probe, which
    /// may be re-running beside this write. `updated_at` is the migration's `BEFORE UPDATE`
    /// trigger's - `agent_box` is in the `0001_init.sql:577` loop, and "no write path may set
    /// `updated_at` by hand" is that loop's own rule.
    ///
    /// One statement answers both facts the trait tells apart (MOD-40 blueprint B11): `written`
    /// from the guarded `UPDATE`, and `present` from the same statement's snapshot, so "absent"
    /// and "a newer quota is stored" can never be confused by a row that appears between two
    /// reads. Under `READ COMMITTED` the `UPDATE` re-checks its `WHERE` on the newest version of
    /// a row a concurrent latch just committed, so of two racing latches the older one loses.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when no `agent_box` row has that key.
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool> {
        let verdict = sqlx::query!(
            r#"
            WITH latched AS (
                UPDATE agent_box
                   SET quota = $3, quota_at = $4
                 WHERE agent_id = $1 AND box_id = $2
                   AND (quota_at IS NULL OR quota_at <= $4)
                RETURNING 1
            )
            SELECT EXISTS (SELECT 1 FROM latched) AS "written!",
                   EXISTS (SELECT 1 FROM agent_box
                            WHERE agent_id = $1 AND box_id = $2) AS "present!"
            "#,
            agent_id.as_uuid(),
            box_id.as_uuid(),
            &quota,
            quota_at,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if !verdict.present {
            return Err(StoreError::NotFound {
                entity: "agent_box",
                id: format!("{agent_id}/{box_id}"),
            });
        }
        Ok(verdict.written)
    }
```

Why `present` is right under concurrency: the outer `SELECT` reads the statement's snapshot, the
one the `UPDATE` started from, so `present = false` means the row did not exist for this statement
at all, and a zero-row `UPDATE` with `present = true` means the guard refused it (P-6).

Doc line `:1063` (`upsert_agent_box`'s "[`WriteStore::set_agent_box_quota`] is their only
writer.") stays.

#### 2.3 `htui-core/src/store/mem.rs`

Inner fn (`:1585-1606`): return `Result<bool>`; doc gains "…and newest `quota_at` wins (MOD-40
plan D4): an older one is `Ok(false)` and writes nothing, not even `updated_at`." Body after the
`get_mut … ?`:

```rust
        if row.quota_at.is_some_and(|stored| stored > quota_at) {
            return Ok(false);
        }
        row.quota = Some(quota);
        row.quota_at = Some(quota_at);
        row.updated_at = now;
        Ok(true)
```

Wrapper (`:5609-5618`): `-> Result<bool>`, body unchanged.

#### 2.4 C7 (D6, B10): `edit_box`'s trait doc (`traits.rs:412-428`)

Insert after the first paragraph (ending "…neither can stale an open editor."):

```rust
    ///
    /// **Every human writer of `box` is this compare-and-set** (MOD-40 plan D6, `docs/ANA-16.md`
    /// C7). `box.settings`, which admission reads under `claim_run`'s row lock, has **no** writer
    /// at all today: registration, the probe and this editor all leave it at its default. The first
    /// one extends [`BoxEdit`] and rides this statement's `edit_version` guard; a second, unguarded
    /// `UPDATE box SET settings` would let two editors overwrite each other silently.
```

`BoxEdit` is in scope in `traits.rs` (`:40`). No case is added: `edit_box_is_cas_on_edit_version`
already pins a spent token (F-13).

#### 2.5 Test doubles and `Writer`

- `htui-store/src/writer.rs:416-432`: `-> Result<bool>`; both arms forward unchanged.
- `htui-agent/src/conformance.rs:770-792` (`UsageSpy`): `-> StoreResult<bool>`;
  `let written = self.inner.set_agent_box_quota(…).await?;` then the existing `push`, then
  `Ok(written)`. The log keeps "every call the store accepted", written or not.
- `htui-agent/tests/recorder.rs:454-485` (`SpyStore`): `-> StoreResult<bool>`; the `None` arm
  forwards unchanged (it already returns the inner result).

#### 2.6 `htui-agent/src/record.rs` — `latch_quota` (`:1197-1220`)

The `match` (`:1197-1202`) becomes:

```rust
        match self
            .store
            .set_agent_box_quota(latch.agent_id, latch.box_id, quota.to_value(), at)
            .await
        {
            Ok(true) => {}
            // MOD-40 plan D4: another session on this box latched a newer allowance first. The
            // latch stays armed: this session's next report may well be the newest again.
            Ok(false) => tracing::debug!(
                "a newer quota is already latched on this agent_box row; this one is older"
            ),
```

The three `Err` arms are unchanged. The module doc (`record.rs:52`) needs no change.

Call sites that assert `true` (a row was just written for them, so `false` would be a regression
the case should name): `conformance.rs:1672` (`set_agent_box_quota_updates_two_columns_or_not_found`,
"the latch lands on a row"), `mem.rs:6878`, `pg_criteria.rs:1608` — each becomes
`assert!(store.set_agent_box_quota(…).await.expect(…), "{…}: a first latch writes")`. Every other
caller (F-15) stays byte-for-byte.

#### 2.7 T3 tests

**Store conformance (both stores)**, `CASES` += (B20):

```text
"an_older_quota_is_a_no_op",
"an_equal_quota_at_rewrites_idempotently",
"a_missing_agent_box_is_not_found",
```

Shared private helper beside them:

```rust
/// MOD-40 D4: `AGENT_CLAUDE`'s probed `agent_box` row on the fixture box, with no quota yet, as
/// `set_agent_box_quota_updates_two_columns_or_not_found` plants it.
async fn quota_row<S: WriteStore>(case: &str, store: &S, t0: DateTime<Utc>) { /* upsert_agent_box(&AgentBox {
   agent_id: ids::AGENT_CLAUDE, box_id: ids::BOX, enabled: true, version: Some("1.2.3"),
   path: Some("/usr/bin/claude"), probed_at: Some(t0), quota: None, quota_at: None,
   updated_at: t0, probe: Some(json!({"status": "ready", "source": "probe"})) }) .expect(case) */ }
```

with `epoch = t0 = seam_clock()` (`conformance.rs:4056`, already truncated to
`TIMESTAMPTZ_DIGITS`, so the two stores compare the same instants).

| Case | Setup | Asserts |
|---|---|---|
| `an_older_quota_is_a_no_op` | `quota_row`; `t1 = t0 + 1 min`, `t2 = t0 + 2 min`, `mid = t0 + 90 s`. | `set(t2, {"v": 2})` → `Ok(true)`; `set(t1, {"v": 1})` → `Ok(false)` ("an older quota is a no-op, not NotFound"); `set(mid, {"v": 15})` → `Ok(false)` ("t2 is still the stored instant: had t1 overwritten it, t1.5 would have landed"); `set(t0 + 3 min, {"v": 3})` → `Ok(true)` ("a newer one still lands"). |
| `an_equal_quota_at_rewrites_idempotently` | `quota_row`; `t = t0 + 1 min`. | `set(t, {"v": 1})` → `true`; `set(t, {"v": 2})` → `true` ("`<=`: the same instant rewrites, which is how one session refreshes its spend"); `set(t, {"v": 2})` → `true` (idempotent); `set(t - 1 µs, {"v": 0})` → `false` ("one microsecond older is older"). |
| `a_missing_agent_box_is_not_found` | none (no row planted). | For `(AgentId::new(), ids::BOX)`, `(ids::AGENT_CLAUDE, BoxId::new())` and `(ids::AGENT_CLAUDE, ids::BOX)` with `quota_at = t0 - 3650 days` (older than anything): `Err(NotFound { entity: "agent_box", id })` with `id == format!("{agent}/{box}")` ("absence is never mistaken for an older quota"). Then `quota_row`, and the same `(AGENT_CLAUDE, BOX, t0 - 3650 days)` → `Ok(true)` (a `NULL` `quota_at` accepts anything). |

(The fixture holds no `agent_box` row at all: `fixtures.rs` has none, and `MemStore::from_demo`
starts `agent_boxes` empty, `mem.rs:263`; `demo_db` loads the same fixture.)

**Read-back, per store** (the conformance suite has no `agent_box` read):

- `htui-core/src/store/mem.rs` tests, after `set_agent_box_quota_leaves_probe_and_version_alone`
  (`:6844`): `an_older_quota_leaves_the_stored_pair_alone`. Plant as that test does; latch
  `({"v": 2}, t2)` → `true`; read `before = state.agent_boxes[(CLAUDE, BOX)]` via `store.read`;
  latch `({"v": 1}, t1 < t2)` → `false`; `after` equals `before` **whole** (`quota`, `quota_at`
  and `updated_at`: "a refused latch writes nothing, not even the trigger's stamp").
- `htui-store/tests/pg_criteria.rs`, after `an_upsert_can_neither_set_nor_clear_the_quota_columns`:
  `an_older_quota_leaves_the_row_byte_identical`. `demo_db()`; plant the probe row as
  `set_agent_box_quota_leaves_probe_byte_identical` does (`:1540-1590`); latch `t2` → `true`;
  `before = SELECT to_jsonb(ab) FROM agent_box ab WHERE agent_id = $1 AND box_id = $2`; latch
  `t1` → `false`; `after == before` (the whole row as `jsonb`, `updated_at` included: a guarded
  `UPDATE` that matched nothing fires no trigger).

**Pins**: `htui-core/tests/mem_store.rs:37` 89 → 92 (T3's commit), message gains ", MOD-40
milestone 2's three for the quota order (plan D4)"; `htui-store/tests/pg_conformance.rs:19`
`EXPECTED_CASES` 89 → 92. T4 moves both to 95.

### 3. T4 — `upsert_agent` as a CAS on `updated_at` (D5)

#### 3.1 `htui-core/src/store/traits.rs` (`:324-333`), whole replacement

```rust
    /// Creates or edits one `agent` row, keyed by `agent.id` (`docs/ANA-4.md` §4.1, §5.7), as a
    /// compare-and-set on `agent.updated_at` (MOD-40 plan D5, `docs/ANA-16.md` C6) — the
    /// [`set_setting`](WriteStore::set_setting) `App` rung's shape.
    ///
    /// `expected: None` is "I expect no row": every column is inserted as given, `created_at` and
    /// `updated_at` included (the migration's trigger is `BEFORE UPDATE` only). An id that is
    /// already stored is [`CasOutcome::Stale`] with the stored row, and nothing is written — so
    /// an agent seeded or created by another process is never overwritten by a create.
    ///
    /// `Some(t)` is the `updated_at` of the row the caller read and edited. Every column but the
    /// two stamps is written where the stored `updated_at` is still `t`; `created_at` is never
    /// rewritten and `updated_at` becomes the store's clock. A token that no longer matches is
    /// `Stale` with the row as it is now, and nothing is written.
    ///
    /// `Applied` carries the row as stored; its `updated_at` is the next token. Take tokens from
    /// a row the store answered (this outcome, or a registry read), never from a struct the caller
    /// built: Postgres keeps microseconds (MOD-40 blueprint F-17).
    ///
    /// Order, the same on every store: the id and the token first (`Stale`, `NotFound`), then the
    /// name, then the write. A stale edit is `Stale` even when it would also take another agent's
    /// name.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "agent"` for
    /// `Some(_)` on an id no row has; [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// when the write would give this id a name another id holds (`agent.name` is `UNIQUE`).
    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>>;
```

`DateTime`, `Utc` and `CasOutcome` are in scope in `traits.rs` (`set_setting`, `:855-861`).

#### 3.2 `htui-core/src/fixtures.rs` — `edit_agent` (B13)

Insert before `#[cfg(test)] mod tests` (`:2000`); add `use crate::store::{CasOutcome, Result as
StoreResult, WriteStore};` to the imports (`:14-31`). Module doc (`:1-11`): append "One async
helper sits here too, [`edit_agent`], for tests that edit a fixture row (MOD-40 plan D5); it is
the only item in this module that touches a store."

```rust
/// Writes `edited` over the stored `agent` row it was read from, passing the `updated_at` it was
/// read with as the compare-and-set token (MOD-40 plan D5, blueprint B13), and answers the row
/// as stored.
///
/// For tests that read a registry row (`agents()`), change some columns and write it back — the
/// "disable every fixture agent" and "make every row unresolvable" set-ups. The token is the one
/// the row carries, so `edited` must come from a store read, not be built by hand.
///
/// # Errors
///
/// Whatever [`WriteStore::upsert_agent`] refuses with: `NotFound` for an id with no row,
/// `Constraint` for a name another id holds.
///
/// # Panics
///
/// When the token is spent (`Stale`): something wrote the row between the test's read and this
/// write, which in a test is a bug in the test. The message names the row and what is stored.
pub async fn edit_agent<S: WriteStore + ?Sized>(store: &S, edited: &Agent) -> StoreResult<Agent> {
    match store.upsert_agent(edited, Some(edited.updated_at)).await? {
        CasOutcome::Applied(row) => Ok(row),
        CasOutcome::Stale(stored) => panic!(
            "agent `{}` ({}) changed since it was read; stored now: {stored:?}",
            edited.name, edited.id
        ),
    }
}
```

#### 3.3 `htui-store/src/pg/write.rs` (`:1014-1051`), whole replacement (B12)

```rust
    /// Creates or edits one `agent` row as a compare-and-set on `updated_at` (`docs/ANA-4.md`
    /// §4.1, ANA-9 §5.7; MOD-40 plan D5).
    ///
    /// Two statements, one per branch, `set_setting`'s `App` rung's shape (`:2995-3031`):
    /// `expected: None` is `INSERT … ON CONFLICT (id) DO NOTHING`, so a stored id is a miss and
    /// never an overwrite — and the `id` arbiter is checked before the name's unique index, so a
    /// stored id is `Stale` even under a held name; `Some(t)` is `UPDATE … WHERE id = $1 AND
    /// updated_at = $t`, whose `SET` list names neither stamp: `created_at` stays the insert's and
    /// the `BEFORE UPDATE` trigger writes `updated_at`, which `RETURNING` sees. A spent token
    /// matches no row, so no unique check runs and a stale rename is `Stale`, not `Constraint`
    /// (blueprint P-7). A miss reads the row once ([`cas_miss`]): present is `Stale`, absent is
    /// `NotFound`.
    ///
    /// The insert keeps the caller's stamps at the column's resolution, microseconds; `Applied`
    /// carries them as stored, which is the token the next edit passes.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for `Some(_)` on an id no row has; [`StoreError::Constraint`]
    /// when another id holds the name (`23505` on `agent_name_key`).
    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>> {
        let landed = match expected {
            None => sqlx::query_as!(
                Agent,
                r#"
                INSERT INTO agent (id, name, transport, launch, models, default_model, billing,
                                   enabled, settings, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT (id) DO NOTHING
                RETURNING id            AS "id: AgentId",
                          name,
                          transport     AS "transport: htui_core::model::Transport",
                          launch,
                          models,
                          default_model,
                          billing       AS "billing: htui_core::model::Billing",
                          enabled,
                          settings,
                          created_at,
                          updated_at
                "#,
                agent.id.as_uuid(),
                agent.name,
                agent.transport.as_str(),
                &agent.launch,
                &agent.models[..],
                agent.default_model.as_deref(),
                agent.billing.as_str(),
                agent.enabled,
                &agent.settings,
                agent.created_at,
                agent.updated_at,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
            Some(token) => sqlx::query_as!(
                Agent,
                r#"
                UPDATE agent
                   SET name          = $2,
                       transport     = $3,
                       launch        = $4,
                       models        = $5,
                       default_model = $6,
                       billing       = $7,
                       enabled       = $8,
                       settings      = $9
                 WHERE id = $1 AND updated_at = $10
                RETURNING id            AS "id: AgentId",
                          name,
                          transport     AS "transport: htui_core::model::Transport",
                          launch,
                          models,
                          default_model,
                          billing       AS "billing: htui_core::model::Billing",
                          enabled,
                          settings,
                          created_at,
                          updated_at
                "#,
                agent.id.as_uuid(),
                agent.name,
                agent.transport.as_str(),
                &agent.launch,
                &agent.models[..],
                agent.default_model.as_deref(),
                agent.billing.as_str(),
                agent.enabled,
                &agent.settings,
                token,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
        };
        match landed {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.stored_agent(agent.id).await?, "agent", agent.id),
        }
    }
```

`stored_agent` — a private helper in `pg/write.rs`'s own inherent block (`impl PgStore`, `:5704`,
beside `delete_workspace_once`), the counterpart of `read.rs`'s `stored_setting`
(`pg/read.rs:2536`) that `set_setting`'s miss reads:

```rust
    /// One `agent` row by id, for [`WriteStore::upsert_agent`]'s miss (MOD-40 blueprint B12).
    ///
    /// `agents()` is the registry joined to this box and answers every row; the compare-and-set
    /// needs exactly the one it missed, as stored now.
    async fn stored_agent(&self, id: AgentId) -> Result<Option<Agent>> {
        sqlx::query_as!(
            Agent,
            r#"
            SELECT id            AS "id: AgentId",
                   name,
                   transport     AS "transport: htui_core::model::Transport",
                   launch,
                   models,
                   default_model,
                   billing       AS "billing: htui_core::model::Billing",
                   enabled,
                   settings,
                   created_at,
                   updated_at
              FROM agent
             WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }
```

The `RETURNING` column list is the `SELECT` list of `agents()` (`pg/read.rs:1288-1298`), whose
type overrides already decode on this crate (`Transport`/`Billing` derive `sqlx::Type` under
`htui-core`'s `sqlx` feature, `model/mod.rs:34-35`). A race in which the row is deleted between
the miss and the read answers `NotFound`, which is true by then; nothing deletes `agent` rows
outside `pg/demo.rs:158`.

#### 3.4 `htui-core/src/store/mem.rs`

Inner fn (`:1512-1538`), whole replacement:

```rust
    /// Creates or edits one agent under compare-and-set on `updated_at` (MOD-40 plan D5), in
    /// Postgres's order (blueprint F-18): the id and the token, then the name, then the write.
    ///
    /// `None` inserts the row as given, stamps included, and is `Stale` on a stored id. `Some(t)`
    /// writes every column but the stamps where the stored `updated_at` is `t`: `created_at` is
    /// the stored row's and `updated_at` is the clock, which is what Postgres's `BEFORE UPDATE`
    /// trigger does, written out.
    fn upsert_agent(
        &mut self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Agent>> {
        match (self.agents.get(&agent.id), expected) {
            (Some(stored), None) => return Ok(CasOutcome::Stale(stored.clone())),
            (Some(stored), Some(token)) if stored.updated_at != token => {
                return Ok(CasOutcome::Stale(stored.clone()));
            }
            (None, Some(_)) => {
                return Err(StoreError::NotFound {
                    entity: "agent",
                    id: agent.id.to_string(),
                });
            }
            (None, None) | (Some(_), Some(_)) => {}
        }
        if self
            .agents
            .values()
            .any(|row| row.id != agent.id && row.name == agent.name)
        {
            return Err(StoreError::Constraint(format!(
                "agent_name_key: another agent is already named `{}`",
                agent.name
            )));
        }
        let row = match self.agents.get(&agent.id) {
            Some(stored) => Agent {
                created_at: stored.created_at,
                updated_at: now,
                ..agent.clone()
            },
            None => agent.clone(),
        };
        self.agents.insert(agent.id, row.clone());
        Ok(CasOutcome::Applied(row))
    }
```

Wrapper (`:5599-5602`):

```rust
    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>> {
        let now = Utc::now();
        self.write(|state| state.upsert_agent(agent, expected, now))
    }
```

`CasOutcome` is already imported in `mem.rs` (`set_setting`, `:3085-3134`).

#### 3.5 `Writer` and the test doubles

- `htui-store/src/writer.rs:402-407`: `async fn upsert_agent(&self, agent: &Agent, expected:
  Option<DateTime<Utc>>) -> Result<CasOutcome<Agent>>`, both arms forwarding `(agent, expected)`.
  `DateTime`/`Utc` are imported for `set_agent_box_quota` (`:416-421`); `CasOutcome` at `:53`.
- `htui-agent/src/conformance.rs:750-752` (`UsageSpy`) and `htui-agent/tests/recorder.rs:434-436`
  (`SpyStore`): the same signature with `StoreResult<CasOutcome<Agent>>`, forwarding both
  arguments. Both files already import `CasOutcome`, `DateTime` and `Utc`.

#### 3.6 T4 tests

**Store conformance (both stores)**, `CASES` += (B20, after T3's three):

```text
"upsert_agent_with_none_inserts_and_is_stale_when_present",
"upsert_agent_with_a_spent_token_is_stale_and_writes_nothing",
"upsert_agent_with_the_current_token_applies",
```

Shared helper beside them (the existing `test_agent`, `conformance.rs:1306`, stamps
`Utc::now()` in nanoseconds, which Postgres truncates; these cases need exact equality):

```rust
/// MOD-40 D5: `test_agent` stamped at `at`, a microsecond instant, so the row a store answers is
/// the row given, on every store.
fn stamped_agent(id: AgentId, name: &str, model: &str, at: DateTime<Utc>) -> Agent {
    Agent { created_at: at, updated_at: at, ..test_agent(id, name, model) }
}
```

`at = seam_clock()` in every case. `applied` / `stale` are the existing helpers
(`conformance.rs:2002-2015`).

| Case | Setup | Asserts |
|---|---|---|
| `upsert_agent_with_none_inserts_and_is_stale_when_present` | `a = stamped_agent(AgentId::new(), "m40-none", "small", at)`. | (1) `upsert_agent(&a, None)` → `applied(..) == a` ("the insert stores the row as given, both stamps included"). (2) `upsert_agent(&Agent { models: vec!["large"], default_model: Some("large"), ..a.clone() }, None)` → `stale(..) == a` ("a create on a stored id is Stale with the stored row and writes nothing"). (3) `upsert_agent(&Agent { name: "claude".into(), ..a.clone() }, None)` → `Stale` (not `Constraint`: "the id is decided before the name", P-7). (4) A seeded row: `upsert_agent(&Agent { id: ids::AGENT_CLAUDE, name: "m40-seed".into(), ..a.clone() }, None)` → `stale(..)` with `.id == ids::AGENT_CLAUDE` and `.name == "claude"` ("a fixture agent is present too: every edit of one passes its token"). (5) `upsert_agent(&a, Some(a.updated_at))` → `applied` (the three refusals left the token current). |
| `upsert_agent_with_a_spent_token_is_stale_and_writes_nothing` | `r0 = applied(upsert_agent(&stamped_agent(new, "m40-spent", "small", at), None))`; `r1 = applied(upsert_agent(&Agent { models: vec!["large"], ..r0.clone() }, Some(r0.updated_at)))`. | `r1.updated_at != r0.updated_at` ("the edit spent the token"; `!=`, not `>`: the stamps come from two clocks on Postgres). `r1.created_at == r0.created_at`. `upsert_agent(&Agent { models: vec!["largest"], ..r0.clone() }, Some(r0.updated_at))` → `stale(..) == r1` ("Stale carries the row as it is now"). The same spent token under the held name `"claude"` → `Stale`, not `Constraint` (P-7). `upsert_agent(&r1, Some(r1.updated_at))` → `applied(..).models == ["large"]` ("the stale writes wrote nothing: r1's token was still current"). |
| `upsert_agent_with_the_current_token_applies` | `r0 = applied(upsert_agent(&stamped_agent(new, "m40-apply", "small", at), None))`; `far = at + 3650 days`; `edited = Agent { name: "m40-renamed", transport: Transport::Acp, launch: json!({"command": "renamed", "args": ["--acp"], "env": {}}), models: vec!["small", "large"], default_model: Some("large"), billing: Billing::Subscription, enabled: false, settings: json!({"acp": {}}), created_at: far, updated_at: far, ..r0.clone() }`. | `r1 = applied(upsert_agent(&edited, Some(r0.updated_at)))`; `r1 == Agent { created_at: r0.created_at, updated_at: r1.updated_at, ..edited.clone() }` ("every column but the stamps took the edit"); `r1.updated_at != far` ("updated_at is the store's, never the caller's"). `upsert_agent(&Agent { name: "claude".into(), ..r1.clone() }, Some(r1.updated_at))` → `Err(Constraint(_))` ("a current token under a held name is the name's refusal"). `upsert_agent(&r1, Some(r1.updated_at))` → `Applied` ("the refused rename wrote nothing"). `upsert_agent(&stamped_agent(AgentId::new(), "m40-ghost", "small", at), Some(at))` → `Err(NotFound { entity: "agent", .. })`. |

**Existing cases rewritten, meaning unchanged**:

- `upsert_agent_by_id_name_unique` (`conformance.rs:1544-1587`): (1) `let first =
  applied(CASE, upsert_agent(&test_agent(id, "tester", "small"), None)…)`; (2) `let second =
  applied(CASE, upsert_agent(&test_agent(id, "tester", "large"), Some(first.updated_at))…)`, and
  now it **can** assert what the doc said it could not: `second.models == ["large"]`,
  `second.created_at == first.created_at`; (3) and (4) pass `None` and keep their
  `Constraint` asserts; (5) `upsert_agent(&test_agent(id, "tester", "largest"),
  Some(second.updated_at))` → `applied`. Doc (`:1544-1550`): replace the second paragraph with
  "Since MOD-40 plan D5 the write answers the row it stored, so the update-in-place and the kept
  `created_at` are asserted here; `pg_criteria.rs::upsert_agent_updates_in_place_and_keeps_created_at`
  keeps the registry read-back as a second reader."
- `htui-store/tests/pg_criteria.rs::upsert_agent_updates_in_place_and_keeps_created_at`
  (`:1182-1280`): `:1205` → `upsert_agent(&inserted, None)`, asserting `Applied(row)` with
  `row == inserted` (the literal is already µs); `:1238` → `upsert_agent(&updated,
  Some(inserted.updated_at))` asserting `Applied`. The `agents()` read-back and every assertion
  after it stay. Doc `:1178-1180`: "can only assert that the second write is *accepted*" becomes
  "reads the row back from the outcome; this is the registry read-back".

**Pins**: `mem_store.rs:37` 92 → 95, message gains "and three for the agent compare-and-set (plan
D5)"; `pg_conformance.rs:19` 92 → 95.

#### 3.7 T4 call sites, complete

`grep -rn 'upsert_agent(' crates --include=*.rs | grep -v upsert_agent_box` = **65** at `fbd7d43`:
12 are the trait, the implementors and their forwards (`traits.rs:333`; `pg/write.rs:1024`;
`mem.rs:1516`, `:5599`, `:5601`; `writer.rs:402`, `:404`, `:405`; `htui-agent/src/conformance.rs:750`,
`:751`; `htui-agent/tests/recorder.rs:434`, `:435`), and **53 are callers, all in tests**: 39
create a fresh id (`None`), 14 edit a stored row (a token). No production caller exists (plan's
verified claim); MOD-23 will be the first.

Two rewrites, and a rule for each:

- **Create** (`None`): `.upsert_agent(&x)` → `.upsert_agent(&x, None)`. The outcome is discarded
  by the existing `.expect(…)`, as today (a fresh `AgentId::new()` cannot be `Stale`).
- **Edit** (token): `store.upsert_agent(&row).await.expect(M)` →
  `edit_agent(&store, &row).await.expect(M)` (the same message), with `edit_agent` added to the
  file's `use htui_core::fixtures::{…}` (or `use htui_core::fixtures::edit_agent;`). Every edit
  site's `row` is `summary.agent` from `agents()`, so its `updated_at` is the stored token. Where
  the edit was the file's only `WriteStore` method call, drop the now-unused `WriteStore` import
  (clippy runs `-D warnings`).

| Group | File | Create (`None`) | Edit (`edit_agent`) |
|---|---|---|---|
| G0 (serial, with the trait) | `htui-core/src/store/conformance.rs` | `:1554`, `:1565`, `:1573` | `:1560`, `:1582` — rewritten with `Applied` tokens (§3.6), **not** `edit_agent` |
| G0 | `htui-store/tests/pg_criteria.rs` | `:1205` | `:1238` — `Some(inserted.updated_at)` (§3.6) |
| G1 | `htui/src/agent_worker.rs` (tests) | `:3972`, `:4982`, `:5043`, `:5089`, `:5170`, `:5249`, `:5362`, `:5422`, `:5697`, `:5770`, `:5852`, `:5900`, `:5997`, `:6091`, `:6186`, `:6408`, `:6643`, `:6687`, `:6732`, `:6798`, `:7982` (21) | `:5489` (`make_unresolvable`, `pub(crate)`, used by the probe tests) |
| G1 | `htui/src/run_worker.rs` (tests) | `:2503` | `:2499` (`seeded_store`) |
| G1 | `htui/src/store_worker.rs` (tests) | `:2820` (`WriteStore::upsert_agent(&store, &agent, None)`) | — |
| G2 | `htui/tests/backlog.rs` | `:316` | `:312` |
| G2 | `htui/tests/box_probe.rs` | — | `:40` |
| G2 | `htui/tests/box_probe_pg.rs` | — | `:141` (`store: &PgStore`) |
| G2 | `htui/tests/auth.rs` | `:264` | — |
| G2 | `htui/tests/chat_live_cli.rs` | — | `:199` |
| G2 | `htui/tests/chat.rs` | `:110`, `:954`, `:1694` | `:107`, `:950` |
| G2 | `htui/tests/runs_pg.rs` | `:183` | `:178` (`store: &PgStore`) |
| G2 | `htui/tests/chat_offline.rs` | `:239` | `:236` |
| G2 | `htui/tests/settings.rs` | `:167` (a copy of a fixture row under a **new** id) | — |
| G2 | `htui/tests/install.rs` | `:370` | — |
| G2 | `htui/tests/probe.rs` | — | `:32` |
| G3 | `htui-agent/tests/extensibility.rs` | `:136`, `:430`, `:465` (`zeta_row()` mints its id) | — |

Totals: G0 4 + 2 (rewritten), G1 23 + 2, G2 9 + 9, G3 3 + 0 → 39 creates, 14 edits. A site that
seeds a fixture agent under its **own** id with `None` would now be `Stale`: there is none (every
fixture-id write reads the row first). The PgStore seed (`seed_if_empty_as`, `pg/mod.rs:340-361`)
does not go through `upsert_agent` and is unchanged.

No site asserts on the `()` the old method answered, so no assertion moves.

---

## Milestone 3 — box and schema (T5, T6)

### 4. Build order

| Task | Crates | Commits (each compiles) | Gate |
|---|---|---|---|
| T5 | htui-store, htui | (1) red: `PgStore::touch_box` answering `Ok(false)` without a statement, `BOX_HEARTBEAT`, `Started::box_heartbeat`, the loop arm and `beat_once`, the two `pg_criteria` cases and the two worker cases (the Pg ones fail); (2) green: the statement, `.sqlx`, docs. | §10 T5 |
| T6 | htui-store, htui | (1) red: `semver` dep, `TARGET_VERSION_KEY`, `HeadlessError`, `connect_headless` delegating to `connect_with` and mapping `Pending` (so "never migrates" and "below the target" fail), `PgStore::below_target` answering `None`, `StoreState.below_target` at every constructor, the shell's `note_below_target`, all T6 tests; (2) green: `schema_state` split, `raise_target`, the reads, `concepts.rs`, `.sqlx`, docs. | §10 T6 |

### 5. T5 — box heartbeat (D7, B14)

#### 5.1 `htui-store/src/pg/mod.rs` — `touch_box`

Insert after `register_box` (ends `:480`), before `pool` (`:482`):

```rust
    /// The box heartbeat (MOD-40 plan D7, `docs/ANA-16.md` C4): stamps `box.last_seen_at` with the
    /// server's `clock_timestamp()` and answers whether a row had `id`.
    ///
    /// One column, the only one registration refreshes that no reconnect-free session would
    /// otherwise move: never `hostname`, the probe columns, the tags, `quirks`, `settings`,
    /// `machine_fingerprint` or `edit_version`, so a beat cannot stale an open box editor. The
    /// migration's `BEFORE UPDATE` trigger moves `updated_at` with it (`0001_init.sql:575-580`),
    /// which nothing keys on: the editors' token is `edit_version`, and the mirror re-reads the
    /// own box row whole each pass (MOD-40 blueprint F-21).
    ///
    /// The time is the database's, like [`PgStore::register_box`]'s: a box whose clock is off
    /// still reports when the server last heard from it. Nothing reads `last_seen_at` yet (PRD out
    /// of scope: liveness is ANA-2's `DeadWalks`, not this stamp).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn touch_box(&self, id: BoxId) -> Result<bool> {
        let touched = sqlx::query!(
            "UPDATE box SET last_seen_at = clock_timestamp() WHERE id = $1",
            id.as_uuid(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(touched == 1)
    }
```

#### 5.2 `htui-store/src/connect.rs` — the period (F-20)

After `RECONNECT` (`:34`):

```rust
/// How often the store worker stamps this box's `last_seen_at` while it is online (MOD-40 plan
/// D7): [`PgStore::touch_box`] on its own task, first one period after the loop starts.
pub const BOX_HEARTBEAT: Duration = Duration::from_secs(60);
```

`Started` (`:225-251`) gains, after `settings`:

```rust
    /// The box heartbeat's period, [`BOX_HEARTBEAT`] from both constructors. A field rather
    /// than the constant in the loop so a test can beat in milliseconds against a real server,
    /// where a paused clock would also expire the pool's acquire timeout (MOD-40 blueprint F-20).
    pub box_heartbeat: Duration,
```

`box_heartbeat: BOX_HEARTBEAT,` in `Started::detached` (`:274-282`) and in `start`'s literal
(`:387-400`); `Debug` (`:253-262`) gains `.field("box_heartbeat", &self.box_heartbeat)` after
`settings`. `PgStore` is already imported (`:30`). No `pub use` change: `connect::BOX_HEARTBEAT`
is reached through the public module.

#### 5.3 `htui/src/store_worker.rs` — the arm

- Destructure (`:1377-1391`): add `box_heartbeat,` after `settings,`.
- Loop locals, after the sweeper (`:1419-1424`):

  ```rust
          // MOD-40 plan D7 (C4): the box heartbeat's ticker, first beat one period out and `Delay`
          // on a missed one like the two above, and the beat in flight, if any (blueprint B14).
          let mut box_beat = tokio::time::interval_at(
              tokio::time::Instant::now() + box_heartbeat,
              box_heartbeat,
          );
          box_beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
          let mut beat: Option<tokio::task::JoinHandle<()>> = None;
  ```

- Arm, after the sweeper's (`:1808-1810`):

  ```rust
                  // D7: only a backend with a server to beat against, and never a second beat
                  // while one is in flight: a touch that waits on a registration's or an editor's
                  // row lock is not joined by another every period. Spawned, so the loop never
                  // waits on the server (`R-NF-3`).
                  _ = box_beat.tick(),
                      if backend.writable().is_some()
                          && beat.as_ref().is_none_or(tokio::task::JoinHandle::is_finished) =>
                  {
                      if let Some(pg) = backend.writable().cloned() {
                          beat = Some(tokio::spawn(beat_once(pg)));
                      }
                  }
  ```

- Exit, beside the refresher's abort (`:1837-1839`): `if let Some(beat) = beat { beat.abort(); }`.
- Free fn after `sweep_ticker` (`:1843-1848`):

  ```rust
  /// One box heartbeat (MOD-40 plan D7), on its own task.
  ///
  /// A failure is logged and nothing else. A lost server is the refresher's to report
  /// ([`lost_the_server`]); a heartbeat that swapped the backend itself would be a second path to
  /// [`go_offline`] racing the first, from a task that does not own the backend (blueprint B14).
  async fn beat_once(pg: PgStore) {
      let id = pg.this_box();
      match pg.touch_box(id).await {
          Ok(true) => {}
          Ok(false) => tracing::warn!(box_id = %id, "the box heartbeat found no box row to touch"),
          Err(err) => {
              tracing::debug!(%err, "the box heartbeat failed; the refresher reports a lost server");
          }
      }
  }
  ```

- Docs: `spawn`'s list (`:1265-1291`) gains a bullet after the ticker's: "- A **box heartbeat**
  every `Started::box_heartbeat` ([`connect::BOX_HEARTBEAT`]) while the backend is `Online`:
  `PgStore::touch_box` on a spawned task, one at a time, its failure only logged (MOD-40 plan
  D7)." `spawn_with_runtimes`' doc (`:1365-1369`) "Beyond [`spawn_with`]'s three sources" stays
  true (the heartbeat is in `spawn`'s list).

`backend.writable()` (`htui-store/src/backend.rs:126-131`) is `Some` only for `Online`: `Memory`
and `Offline` never beat, and a `held` store (pending migrations) is not the backend, so it does
not either. `Option::is_none_or` is stable since 1.82 (MSRV 1.98).

#### 5.4 T5 tests

**`htui-store/tests/pg_criteria.rs`** (after `an_older_quota_leaves_the_row_byte_identical`, T3):

| Test | Setup | Asserts |
|---|---|---|
| `touch_box_advances_last_seen_at` | `fresh_db()` (a registered box, `db.store.this_box()`); `row = SELECT last_seen_at, to_jsonb(b) - 'last_seen_at' - 'updated_at' AS rest FROM box b WHERE id = $1`; `(seen0, rest0) = row`; `server = SELECT clock_timestamp()`. | `db.store.touch_box(id)` → `Ok(true)`; `(seen1, rest1) = row`; `seen1 > seen0`; `seen1 >= server` ("the stamp is the server's, taken after the read"); `rest1 == rest0` ("a heartbeat writes `last_seen_at` and nothing a person, the probe or registration owns; `edit_version` included"). |
| `touch_box_on_an_unknown_box_is_false` | `fresh_db()`; `before = common::count(&db.pool, "box")`. | `touch_box(BoxId::new())` → `Ok(false)`; `count(box) == before` ("no insert path: an unknown id is `false`, never a row"). |

**`htui/src/store_worker.rs` tests** (after `an_unreachable_read_drops_an_online_backend_onto_the_mirror`, `:2448`):

| Test | Setup | Asserts |
|---|---|---|
| `the_box_heartbeat_stamps_last_seen_at_while_online` | `let Some(db) = htui_store::testkit::fresh_db().await else { return };` `seen = SELECT last_seen_at FROM box WHERE id = $1` (`db.store.this_box()`), `before = seen`; `cache = CacheStore::open(tempdir, "heartbeat", 1)`; `started = Started::detached(Backend::Online { pg: db.store.clone(), cache: cache.clone() })`; `started.reconnect = None`; `started.box_heartbeat = Duration::from_millis(50)`; `spawn(started, req_rx, rep_tx)`. | Polling every 25 ms for at most 5 s, `seen > before` ("a beat stamped the box while online"); then `SELECT edit_version` is `0` ("no editor token moved"). Teardown: `drop(req_tx)`, `worker.await`, `cache.close()`, `db.drop_db()`. |
| `a_failed_box_heartbeat_leaves_the_backend_online` | As `an_unreachable_read_drops_an_online_backend_onto_the_mirror` (`:2448-2474`): `PgStore::lazy("postgres://nobody:nothing@127.0.0.1:1/none", …, 250 ms)` under `Backend::Online` over a throwaway mirror; `reconnect = None`; `box_heartbeat = 20 ms`. | `tokio::time::sleep(300 ms)` (several failed beats; at most one in flight); `StoreState` → `label == "online"` ("a failed heartbeat is logged, never a backend swap: the refresher owns that"). No Postgres needed. |

### 6. T6 — headless connect and the target version (D8, D9)

#### 6.1 Dependency

`htui-store/Cargo.toml` `[dependencies]` (`:24-43`): `semver = { workspace = true }` after
`serde_json`. `semver 1.0.28` is already in the lock (`Cargo.toml:81-83`); only `Cargo.lock`'s
dependency list for `htui-store` changes.

#### 6.2 `htui-store/src/pg/mod.rs` — the key, the verdict, the write (B17, B18)

After `HTUI_VERSION` (`:52-54`):

```rust
/// The `app_setting` key holding the **target version** (MOD-40 plan D9, PRD D3): the highest
/// [`HTUI_VERSION`] that has applied migrations to this database, as a JSON string.
///
/// Raised by [`PgStore::apply_migrations`] and never lowered. A TUI below it runs and says so
/// ([`PgStore::below_target`]); a headless process below it refuses
/// ([`HeadlessError::BelowTarget`]). Snake case like every other key; no reader iterates
/// `app_setting`, so the row is invisible to the settings resolvers and the Settings tab.
pub const TARGET_VERSION_KEY: &str = "htui_target_version";
```

Private helpers, after `this_os_family` (`:630`):

```rust
/// This build's version. `CARGO_PKG_VERSION` is semver by cargo's own rule, so the parse cannot
/// fail on a build cargo produced.
fn this_version() -> semver::Version {
    semver::Version::parse(HTUI_VERSION).expect("CARGO_PKG_VERSION is a semver version")
}

/// A stored target, parsed: the version, or the stored JSON as text when it is not a string
/// holding one (MOD-40 blueprint B18).
fn parse_target(stored: &Value) -> core::result::Result<semver::Version, String> {
    stored
        .as_str()
        .and_then(|text| semver::Version::parse(text).ok())
        .ok_or_else(|| stored.to_string())
}

/// The stored target document, if any.
///
/// The statement is `stored_setting`'s `App` text byte for byte (`read.rs:2544`), so the two share
/// one `.sqlx` entry.
async fn stored_target(pool: &PgPool) -> Result<Option<Value>> {
    let row = sqlx::query!(
        "SELECT value, updated_at FROM app_setting WHERE key = $1",
        TARGET_VERSION_KEY,
    )
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(row.map(|row| row.value))
}

/// What a TUI makes of the stored target: the target when this build is below it, `None`
/// otherwise. A malformed one is logged and ignored: a TUI runs (B18).
async fn tui_below_target(pool: &PgPool) -> Result<Option<String>> {
    let Some(stored) = stored_target(pool).await? else {
        return Ok(None);
    };
    match parse_target(&stored) {
        Ok(target) if this_version() < target => Ok(Some(target.to_string())),
        Ok(_) => Ok(None),
        Err(text) => {
            tracing::warn!(
                stored = %text,
                "app_setting.{TARGET_VERSION_KEY} is not a version; ignoring it"
            );
            Ok(None)
        }
    }
}

/// Raises the target to this build's version, never lowering it (MOD-40 plan D9, blueprint
/// B17), and answers the target when it is still above this build.
///
/// One transaction, three statements. The insert goes first because a `FOR UPDATE` on a row that
/// does not exist locks nothing: two first appliers would both read "absent" and the second
/// insert would fail on the primary key after its migrations had already run. `ON CONFLICT DO
/// NOTHING` makes the second wait for the first's commit and then do nothing; the locked read
/// that follows sees the committed row, and the comparison and the update run under that lock,
/// so two appliers of different versions leave the higher one whichever commits first.
///
/// A stored value that is not a version is replaced with this build's (B18): the TUI that has
/// just migrated the database is the authority on what it now needs.
async fn raise_target(pool: &PgPool) -> Result<Option<String>> {
    let ours = this_version();
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let inserted = sqlx::query!(
        "INSERT INTO app_setting (key, value) VALUES ($1, to_jsonb($2::text)) \
         ON CONFLICT (key) DO NOTHING",
        TARGET_VERSION_KEY,
        HTUI_VERSION,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    let mut above = None;
    if inserted == 0 {
        let stored = sqlx::query_scalar!(
            "SELECT value FROM app_setting WHERE key = $1 FOR UPDATE",
            TARGET_VERSION_KEY,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let raise = match stored.as_ref().map(parse_target) {
            // Deleted by hand between the two statements: the next apply inserts it.
            None => false,
            Some(Ok(target)) if target > ours => {
                above = Some(target.to_string());
                false
            }
            Some(Ok(target)) => target < ours,
            Some(Err(text)) => {
                tracing::warn!(
                    stored = %text,
                    "app_setting.{TARGET_VERSION_KEY} is not a version; replacing it with this build's"
                );
                true
            }
        };
        if raise {
            sqlx::query!(
                "UPDATE app_setting SET value = to_jsonb($2::text) WHERE key = $1",
                TARGET_VERSION_KEY,
                HTUI_VERSION,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }
    }
    tx.commit().await.map_err(map_sqlx)?;
    Ok(above)
}
```

`use serde_json::Value;` joins the imports (`:12-23`); `serde_json` is a dependency (`Cargo.toml:38`).

#### 6.3 `schema_state` split (B15)

`schema_state` (`:579-627`) becomes the TUI's ensure plus the read-only half:

```rust
/// Compares `_sqlx_migrations` against the embedded set (ANA-9 §5.0), creating the empty
/// bookkeeping table first when there is none — the TUI's path, which may go on to migrate.
///
/// `list_applied_migrations` takes the table name in sqlx 0.9 and lives on `PgConnection`, not on
/// `PgPool`, so a connection is acquired first (blueprint H.7).
async fn schema_state(pool: &PgPool) -> Result<MigrationState> {
    let mut conn = pool.acquire().await.map_err(map_sqlx)?;
    conn.ensure_migrations_table(MIGRATOR.table_name.as_ref())
        .await
        .map_err(map_migrate)?;
    applied_state(&mut conn).await
}

/// The read-only half of [`schema_state`] (MOD-40 blueprint B15): refuses a dirty version, a
/// newer applied version and checksum drift, and counts what is pending. Writes nothing, so a
/// headless process may run it under a role that cannot create a table.
async fn applied_state(conn: &mut PgConnection) -> Result<MigrationState> {
    let table = MIGRATOR.table_name.as_ref();
    // … today's body from `dirty_version` (`:594`) to the end, unchanged, minus `drop(conn)` …
}

/// Whether the migrations table exists at all, without creating it (blueprint B15, F-22).
async fn migrations_table_exists(conn: &mut PgConnection) -> Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT to_regclass($1) IS NOT NULL AS "exists!""#,
        MIGRATOR.table_name.as_ref(),
    )
    .fetch_one(conn)
    .await
    .map_err(map_sqlx)
}

/// How many embedded up-migrations there are: a database with no migrations table has all of
/// them pending.
fn embedded_migrations() -> usize {
    MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .count()
}
```

`use sqlx::PgConnection;` joins the imports (`write.rs:57` has the same). `open_pool` factors the
first seven lines of `connect_with` (`:173-179`), which then calls it:

```rust
/// The pool both connects open: eight connections, `connect_timeout` to acquire one.
async fn open_pool(dsn: &str, connect_timeout: Duration) -> Result<PgPool> {
    let options = PgConnectOptions::from_str(dsn).map_err(map_sqlx)?;
    PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(connect_timeout)
        .connect_with(options)
        .await
        .map_err(map_sqlx)
}
```

#### 6.4 `PgStore` field, `connect_with`, `apply_migrations` (B16)

- Struct (`:88-95`): `below_target: Option<String>,` after `registration` (private; `Debug`
  derives it). Both literals (`:182-188`, `:216-222`) gain `below_target: None`.
- Accessor, after `registration()` (`:520-524`):

  ```rust
      /// The database's target version when this build's [`HTUI_VERSION`] is below it (MOD-40
      /// plan D9, blueprint B16): read by [`PgStore::connect`] over an up-to-date schema, and
      /// recomputed by [`PgStore::apply_migrations`], which may raise the target but never lowers
      /// it. `None` when this build is at or above the target, when none is stored, and on a
      /// store whose pending schema has not been applied yet.
      ///
      /// A TUI below the target runs and says so once (PRD D3); a headless process never gets a
      /// store to ask ([`PgStore::connect_headless`] refuses).
      #[must_use]
      pub fn below_target(&self) -> Option<&str> {
          self.below_target.as_deref()
      }
  ```

- `connect_with` (`:168-193`): `let pool = open_pool(dsn, connect_timeout).await?;` and the
  `UpToDate` branch becomes

  ```rust
          if migrations == MigrationState::UpToDate {
              store.below_target = tui_below_target(&store.pool).await?;
              store.bootstrap().await?;
          }
  ```

  Doc (`:126-152`): add after the **Registration** paragraph: "**Target version (MOD-40 plan
  D9)**: over an up-to-date schema the stored `htui_target_version` is read, and a build below it
  is recorded in [`PgStore::below_target`] and otherwise carries on: a TUI warns, it does not
  refuse (PRD D3). A pending schema reads nothing; `apply_migrations` decides."
- `apply_migrations` (`:225-237`):

  ```rust
      pub async fn apply_migrations(&mut self) -> Result<()> {
          MIGRATOR.run(&self.pool).await.map_err(map_migrate)?;
          self.bootstrap().await?;
          self.below_target = raise_target(&self.pool).await?;
          Ok(())
      }
  ```

  Doc gains: "Then raises `app_setting.htui_target_version` to this build's [`HTUI_VERSION`] and
  never lowers it (MOD-40 plan D9), so a headless process older than the last migrator refuses to
  run against the schema it migrated. A target that stays above this build is kept in
  [`PgStore::below_target`]."

#### 6.5 `connect_headless` and `HeadlessError` (D8, B15, B19)

After `Connected` (`:97-104`):

```rust
/// Why [`PgStore::connect_headless`] refused (MOD-40 plan D8, `R-STO-5` as amended 2026-09-26: a
/// headless process never migrates; it refuses and reports).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeadlessError {
    /// The store refused as [`PgStore::connect`] would: unreachable, a newer schema, checksum
    /// drift, a partially applied version, or a target that is not a version.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// This many embedded migrations are not applied. A TUI asks; a headless process refuses.
    #[error(
        "{0} schema migration(s) are pending, and a headless process never migrates; start \
         `htui` once to apply them"
    )]
    MigrationsPending(usize),
    /// This build is below the database's target version ([`TARGET_VERSION_KEY`]).
    #[error(
        "this htui is {ours}, below {target}, the version that last migrated this database; \
         upgrade htui on this box"
    )]
    BelowTarget {
        /// [`HTUI_VERSION`].
        ours: String,
        /// The stored target, as `semver` prints it.
        target: String,
    },
}
```

(`StoreError` derives `Clone, PartialEq, Eq`, `htui-core/src/store/error.rs:9`.) In `impl
PgStore`, after `connect_with`:

```rust
    /// Connects for a process with no one to ask (MOD-40 plan D8): refuses rather than migrates.
    ///
    /// Order (blueprint B19), every refusal before any write: open the pool; read the schema
    /// state **without** creating the migrations table (a missing table is every embedded
    /// migration pending; a role that cannot create tables can still connect, F-22); refuse a
    /// dirty, newer or drifted schema as [`PgStore::connect`] does, and a pending one; refuse a
    /// build below the stored target version, and a target that is not a version; then seed and
    /// register exactly as `connect` does, so `this_box` and `this_user` are filled. "Never
    /// migrates" is not "never writes": the bootstrap writes `app_user`, `capability_tag`,
    /// `app_setting` defaults, the seeded agents and this box's row, as `connect` always has.
    ///
    /// # Errors
    ///
    /// [`HeadlessError::Store`] for everything [`PgStore::connect`] refuses, and for a malformed
    /// target; [`HeadlessError::MigrationsPending`]; [`HeadlessError::BelowTarget`].
    pub async fn connect_headless(
        dsn: &str,
        identity: &Identity,
        connect_timeout: Duration,
    ) -> core::result::Result<Self, HeadlessError> {
        let pool = open_pool(dsn, connect_timeout).await?;
        let mut conn = pool.acquire().await.map_err(map_sqlx)?;
        let migrations = if migrations_table_exists(&mut conn).await? {
            applied_state(&mut conn).await?
        } else {
            MigrationState::Pending(embedded_migrations())
        };
        drop(conn);
        if let MigrationState::Pending(n) = migrations {
            return Err(HeadlessError::MigrationsPending(n));
        }
        if let Some(stored) = stored_target(&pool).await? {
            let target = parse_target(&stored).map_err(|text| {
                StoreError::Backend(format!(
                    "app_setting.{TARGET_VERSION_KEY} holds {text}, which is not a version; a \
                     headless process does not guess"
                ))
            })?;
            if this_version() < target {
                return Err(HeadlessError::BelowTarget {
                    ours: HTUI_VERSION.to_owned(),
                    target: target.to_string(),
                });
            }
        }
        let mut store = Self {
            pool,
            identity: identity.clone(),
            this_box: BoxId::default(),
            this_user: UserId::default(),
            registration: None,
            below_target: None,
        };
        store.bootstrap().await?;
        Ok(store)
    }
```

`htui-store/src/lib.rs:37`: `pub use pg::{Connected, HTUI_VERSION, HeadlessError, MigrationState,
PgStore, Registration, TARGET_VERSION_KEY};`.

**`htui/src/concepts.rs`** (`:15`, `:52-68`): imports become
`use htui_store::pg::CONNECT_TIMEOUT;` and `use htui_store::{HeadlessError, PgStore, identity,
secret};` (`MigrationState` goes). `open` becomes:

```rust
    let identity = identity::load_or_mint(&identity::config_root()?)?;
    let pg = match PgStore::connect_headless(&dsn, &identity, CONNECT_TIMEOUT).await {
        Ok(pg) => pg,
        // The bail text is today's, kept byte for byte (plan D8).
        Err(HeadlessError::MigrationsPending(n)) => {
            bail!("{n} schema migration(s) are pending; start `htui` once to apply them")
        }
        Err(HeadlessError::Store(err)) => {
            return Err(anyhow::Error::new(err).context("cannot reach Postgres"));
        }
        Err(below @ HeadlessError::BelowTarget { .. }) => return Err(below.into()),
    };
```

and returns `Ok((pg, store))`. The module doc (`:1-8`) gains: "Both connect headless
(`PgStore::connect_headless`, MOD-40 plan D8): a pending schema or a build below the database's
target version is refused, never migrated."

#### 6.6 How `below_target` reaches the TUI (B16)

The one-time status notices the shell already has are `StoreReply::MigrationsApplied` and
`StoreReply::BoxProbed`, both written to `App::status` in `observe_reply`
(`htui/src/app/update.rs:241-270`), which the next key clears (`update.rs:577-586` pins that).
`StoreState` is the reply the shell re-reads every fourth tick (`update.rs:95-101`) and already
carries one "say it once" fact, the pending count, with `migration_prompt_shown` as its once-flag
(`app/state.rs:185-189`). The target rides the same reply with the same kind of flag.

1. `htui/src/store_worker.rs:820-827`, `StoreReply::StoreState` gains:

   ```rust
          /// The database's target version when this build is below it (MOD-40 plan D9, PRD D3):
          /// `PgStore::below_target` of an `Online` backend, `None` for every other. The shell
          /// says it once on the status line; the session runs regardless.
          below_target: Option<String>,
   ```

2. Both constructors in `store_worker.rs` — `try_serve`'s (`:1226-1229`) and the loop's
   (`:1445-1448`) — set
   `below_target: backend.writable().and_then(PgStore::below_target).map(str::to_owned),`.
   (The loop swaps the `Online` store in on `ConnEvent::Online` and after `ApplyMigrations`,
   whose `apply_migrations` has just recomputed the field; a `held` store is not asked.)
3. `htui/src/app/state.rs`: field after `migration_prompt_shown` (`:189`):

   ```rust
       /// Whether the below-the-target notice has been shown this session (MOD-40 plan D9).
       ///
       /// `StoreState` is re-read every fourth tick; without this a notice the next key cleared
       /// would come back a second later, `migration_prompt_shown`'s reason.
       pub(super) below_target_shown: bool,
   ```

   and `below_target_shown: false,` in `App::new` (`:236`).
4. `htui/src/app/update.rs`, `observe_reply`'s `StoreState` arm (`:253-261`):

   ```rust
              StoreReply::StoreState {
                  label,
                  migrations_pending,
                  below_target,
              } => {
                  self.top_bar.store = label.clone();
                  if migrations_pending.is_some_and(|n| n > 0) {
                      self.offer_migration_prompt();
                  }
                  if let Some(target) = below_target {
                      self.note_below_target(target);
                  }
              }
   ```

   and, after `offer_migration_prompt` (`:272-290`):

   ```rust
      /// Says once per session that this build is below the database's target version (MOD-40
      /// plan D9, PRD D3): a TUI warns and runs, where a headless process refuses.
      fn note_below_target(&mut self, target: &str) {
          if self.below_target_shown {
              return;
          }
          self.below_target_shown = true;
          self.status = Some(below_target_notice(target));
      }
   ```

   with a free function beside it, so the test can name the text:

   ```rust
   /// The status line's below-the-target sentence (MOD-40 plan D9).
   fn below_target_notice(target: &str) -> String {
       format!(
           "htui {} is older than {target}, which last migrated this database; upgrade this box",
           htui_store::HTUI_VERSION
       )
   }
   ```

5. The other `StoreState` literals get `below_target: None`: `app/update.rs:625` (test helper
   `store_state`), `htui/src/testkit.rs:261`, `:492`, `htui/tests/shell.rs:22`. The one
   destructure without `..` in a test, `store_worker.rs:2314`
   (`store_state_reports_the_backend_label_and_no_pending_count`), names `below_target` and
   asserts it `None` ("a memory backend has no target"). The others already
   have it (`migration_prompt.rs:109`, `store_worker.rs:2400`, `:2421`, `:2474`, `:2494`,
   `:2685`, `htui/tests/connection.rs:878`).

`connect.rs`, `Connected` and `ConnEvent` do **not** change (F-23): the fact travels inside the
`PgStore` that `ConnEvent::Online` already carries.

#### 6.7 The key collides with nothing, and no reader surfaces it (plan D9's claim, verified)

- **Names**: no file under `crates/` names `target_version`, `htui_target` or `htui.target`
  (grep). A migrated database holds 22 keys, all snake_case; the seed adds
  `cache_refresh_seconds` and `cache_overlap_seconds` (`pg/mod.rs:47-50`); the box probe spec is
  `box_probe_spec` (`htui-agent/src/box_probe/spec.rs:46`). `app_setting.key` has no `CHECK`.
- **`app_settings()` readers** (`pg/read.rs:1568-1574` answers the whole table as a map;
  `Backend::app_settings`, `backend.rs:399-405`, refuses offline): every consumer looks keys up —
  `htui-orch/src/engine.rs:6022` (→ `EngineParts.app`, read through `min_budget`,
  `graph.rs:616` `app_positive(app, key)`, and the `settings::resolve_*` functions,
  `htui-core/src/prompt/settings.rs:640-732`, each a `get(key)`); `htui/src/run_worker.rs:898`,
  `:1229`, `:1958` (`lease_period(&app)`); `htui/src/agent_worker.rs:2102` and
  `htui/src/box_settings.rs:66` (`get(spec::SETTING_KEY)`); `htui/src/preview.rs:215` (the
  resolvers). No `iter()`, `keys()`, `values()` or `len()` over an app map in `htui-core/src`,
  `htui-orch/src`, `htui-store/src` or `htui/src` (grep).
- **The Settings tab** builds its `App` rows from `SettingKey::ALL`, not from the table
  (`htui/src/prompt_settings.rs:141-142`; `ui/tabs/settings/prompt.rs:281` counts those entries),
  and `SettingsSnapshot::app_map` is built from the same entries (`prompt_settings.rs:90-100`), so
  an unknown key has no row to render.
- **The mirror** holds no `app_setting` table (`htui-store/cache_migrations/0001_mirror.sql`,
  `0002_agent_mirror.sql`: no match), and the refresher reads only the two cache keys by name
  (`connect.rs:615-617`).
- **Counts**: `a_second_run_is_a_no_op` (`htui-store/tests/migrations.rs:119-139`) compares the
  `app_setting` count across a second `apply_migrations`; the first (`fresh_db`) already wrote the
  target, the second writes nothing new, so it holds. `load_demo_round_trips_a_count_per_table`
  (`:1422`) counts deltas per table and does not list `app_setting`.
- **`MemStore`** has no migrations and no target: `connect_headless` and `below_target` are
  `PgStore`-only, and `Backend::Memory` answers `below_target: None` through `writable()`.

#### 6.8 T6 tests

Shared helpers in `htui-store/tests/migrations.rs` (private, beside the new tests; `serde_json`
and `sqlx::Row` are already used there):

```rust
/// The stored `htui_target_version` document, if any (MOD-40 plan D9).
async fn target(pool: &sqlx::PgPool) -> Option<serde_json::Value> { /* SELECT value FROM
   app_setting WHERE key = $1, bound TARGET_VERSION_KEY, fetch_optional */ }

/// Plants `value` as the target, over whatever is stored: another box's newer build, or a hand
/// edit.
async fn plant_target(pool: &sqlx::PgPool, value: serde_json::Value) { /* INSERT INTO
   app_setting (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value =
   EXCLUDED.value */ }
```

Runtime `sqlx::query` in tests, as the file already does (`:891`, `:918`): no `.sqlx` entry.
`HEADLESS_WAIT = Duration::from_secs(5)` is the `connect_timeout` every headless call passes.

**`htui-store/tests/migrations.rs`** (after `a_checksum_mismatch_is_refused`, `:913-935`):

| Test | Setup | Asserts |
|---|---|---|
| `a_headless_connect_never_migrates` | `bare_db()` (its `PgStore::connect` ensured an **empty** `_sqlx_migrations`; `migrations_at_connect == Pending(7)`). | `connect_headless(&db.url, &db.identity, HEADLESS_WAIT)` → `Err(HeadlessError::MigrationsPending(7))`; `count(_sqlx_migrations) == 0` and `SELECT count(*) FROM pg_tables WHERE schemaname = 'public'` `== 1` ("a headless connect applies nothing"). Then `DROP TABLE _sqlx_migrations`; the same call → `Err(MigrationsPending(7))`; `SELECT to_regclass('_sqlx_migrations') IS NULL` is `true` and the public table count is `0` ("it does not even create the bookkeeping table", F-22). |
| `a_headless_connect_refuses_a_newer_schema` | `fresh_db()`; `boxes = count(box)`. | Plant version 9999 (the `:891` statement) → `Err(HeadlessError::Store(StoreError::Backend(t)))`, `t` contains `"schema is newer"`. `DELETE … WHERE version = 9999`; `UPDATE _sqlx_migrations SET success = false WHERE version = 7` → `Store(Backend(t))`, `t` contains `"partially applied"`. `success = true` again; the `:918` checksum corruption → `t` contains `"checksum"`. After each, `count(box) == boxes` (refused before the bootstrap, B19). |
| `applying_migrations_raises_the_target_and_never_lowers_it` | `bare_db()` (`mut db`; no tables yet). | `apply_migrations()` → `target == Some(json!(HTUI_VERSION))` ("the first migrator writes its version"), `below_target() == None`. `plant_target(json!("0.0.1"))`, apply → `json!(HTUI_VERSION)` ("raised"), `None`. `plant_target(json!("99.0.0"))`, apply → still `json!("99.0.0")` ("never lowered"), `below_target() == Some("99.0.0")`. `plant_target(json!(42))`, apply → `json!(HTUI_VERSION)` ("a malformed target is replaced", B18). `DELETE` the row, apply → `json!(HTUI_VERSION)` ("re-inserted"). `count(_sqlx_migrations)` is 7 throughout (the later applies migrate nothing). |
| `a_headless_connect_below_the_target_refuses` | `fresh_db()`; `stranger = Identity { box_id: BoxId::new(), hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()) }`, the `:1167` pattern. | `plant_target(json!("99.0.0"))`; `connect_headless(&db.url, &stranger, …)` → `Err(HeadlessError::BelowTarget { ours: HTUI_VERSION.into(), target: "99.0.0".into() })`; `SELECT count(*) FROM box WHERE id = $stranger` is `0` ("refused before the bootstrap: nothing registered"). `plant_target(json!("banana"))` → `Err(Store(Backend(t)))`, `t` contains `"htui_target_version"` and `"not a version"`. `plant_target(json!(HTUI_VERSION))` → `Ok(store)`, `store.this_box() == stranger.box_id` ("at the target it connects and registers as `connect` does"), `store.below_target() == None`. `plant_target(json!("0.0.1"))` → `Ok`. |
| `a_tui_connect_below_the_target_reports_it` | `fresh_db()`. | `plant_target(json!("99.0.0"))`; `PgStore::connect(&db.url, &db.identity)` → `Ok`, `migrations == UpToDate`, `store.below_target() == Some("99.0.0")` ("a TUI runs below the target and knows it", PRD D3). `plant_target(json!(HTUI_VERSION))` → `None`. `plant_target(json!(42))` → `Ok`, `None` ("a TUI ignores a malformed target", B18). |

Imports: `use htui_store::{HeadlessError, HTUI_VERSION, TARGET_VERSION_KEY, …}`,
`use htui_store::Identity;`, `use std::time::Duration;` (then `cargo fmt`).

**`htui-store/tests/connect.rs`** (after
`start_opens_the_mirror_offline_and_reports_online_over_a_migrated_database`, `:41-94`):
`start_hands_over_a_store_that_knows_it_is_below_the_target` — the same set-up with
`plant_target`'s statement run on `db.pool` first (`"99.0.0"`); the `ConnEvent::Online(pg)` arm
asserts `pg.below_target() == Some("99.0.0")` ("the fact rides the store the worker receives",
B16). The file's `Pending(7)` pins (`:102`, `:204`) do not move.

**`htui/src/store_worker.rs` tests**: `store_state_carries_below_target_from_an_online_store` —
`fresh_db()`; `plant_target` via `sqlx::query` on `db.pool` (`"99.0.0"`);
`let pg = PgStore::connect(&db.url, &db.identity).await?.store`; `Started::detached(Backend::Online
{ pg, cache })` over a throwaway mirror, `reconnect = None`; `StoreState` →
`below_target == Some("99.0.0")`. Plus the `None` assert on the memory backend (§6.6 item 5).

**`htui/src/app/update.rs` tests**: helper `fn store_state_below(label: &str, target: &str) ->
ReplyEnvelope` beside `store_state` (`:621`); case `a_store_state_below_the_target_says_so_once`
— `shell()`; reply `store_state_below("online", "99.0.0")` → `app.top_bar.store == "online"`,
`app.status == Some(below_target_notice("99.0.0"))`; `app.on_key(KeyEvent::from(KeyCode::Char('x')))`
→ `status == None`; the same reply again → `status == None` ("said once per session, not every
fourth tick"). A reply with `below_target: None` before it sets no status (the existing
`a_store_state_reply_writes_the_top_bar_and_opens_the_prompt_once` keeps passing with
`below_target: None` in its helper).

**Pins that do not move**: `Pending(7)` (`migrations.rs:877`, `connect.rs:102`, `:204`), `TABLES`
39, the commented columns, the conformance `CASES` (T5/T6 add none), every `htui` snapshot (no
existing test sets a target, so no status line changes).

---

### 7. Call-site sweep groups (T4 only)

T3 changes a return type that every caller already discards through `?`/`.expect` (and
`latch_quota`'s `Ok(()) => {}` arm, §2.6), so it needs no sweep: its five implementors and one
arm are the whole change. T5 and T6 add methods and fields and change no signature. **Only T4
sweeps.**

T4's commit (1) must build the workspace, so the trait change and all 53 callers land
together. The groups are disjoint by file, so after G0 fixes the signature they can run in
parallel (one agent each) and merge without conflict:

| Group | Owner | Files | Sites | Depends on |
|---|---|---|---|---|
| G0 | serial, first | `htui-core/src/store/{traits,mem,conformance}.rs`, `htui-core/src/fixtures.rs` (`edit_agent`), `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`, `htui-store/tests/pg_criteria.rs`, the two pins | 12 defs/forwards + 4 conformance + 2 `pg_criteria` | — |
| G1 | parallel | `htui/src/agent_worker.rs`, `htui/src/run_worker.rs`, `htui/src/store_worker.rs` (tests only) | 23 creates + 2 edits | G0's signature and `edit_agent` |
| G2 | parallel | the 13 `htui/tests/*.rs` files in §3.7 | 9 creates + 9 edits | G0 |
| G3 | parallel | `htui-agent/tests/extensibility.rs` | 3 creates | G0 |

The gate is the compiler: `cargo build --workspace --all-features --all-targets` fails on any
site left with one argument, because the old arity no longer exists. It does **not** catch an
edit site wrongly given `None`: that call answers `Ok(Stale(..))`, which `.expect` accepts, and
the edit silently does not happen. So the one grep that matters is:

```bash
grep -rn 'edit_agent(' crates --include=*.rs | grep -v '^crates/htui-core/src/fixtures.rs'   # → exactly the 11 edit sites of §3.7
```

and a reviewer reads every remaining `upsert_agent(…, None)` in G1/G2 against §3.7's create
column (each builds a row with `AgentId::new()` or `zeta_row()`).

### 8. `.sqlx` accounting

`crates/htui-store/.sqlx/` holds **281** files at `fbd7d43` (after milestone 1). Hashes are of
the query text, so a byte-identical text shares a file and an edited text is a new file.

| Task | Removed | Added | Net | Total |
|---|---|---|---|---|
| T3 | the old `set_agent_box_quota` `UPDATE` (`-query-957e07a5…`) | the §2.2 `WITH latched …` statement | 0 | 281 |
| T4 | the old `upsert_agent` `INSERT … ON CONFLICT (id) DO UPDATE` (`-query-844671b3…`) | the `None` insert, the `Some` update, `stored_agent`'s `SELECT` (§3.3) | +2 | 283 |
| T5 | — | `touch_box`'s `UPDATE` (§5.1) | +1 | 284 |
| T6 | — | `raise_target`'s insert, `SELECT … FOR UPDATE` and conditional update, `migrations_table_exists`'s `to_regclass` (§6.2, §6.3) | +4 | **288** |

`stored_target`'s `SELECT value, updated_at FROM app_setting WHERE key = $1` is byte-identical to
`pg/read.rs:2544`'s and reuses its file (§6.2): if the implementer reflows it, it becomes a
fifth T6 file and the total is 289, so keep the text exactly as written. `applied_state` reads
`_sqlx_migrations` through sqlx's `Migrate::list_applied_migrations` (as `schema_state` does
today), which is not a macro and has no file. Test-side statements are runtime `sqlx::query`
and add none.

Expected per task: `git status --short crates/htui-store/.sqlx` shows exactly the removed (`D`)
and added (`??`) files in the table, and `ls crates/htui-store/.sqlx | wc -l` the total.

### 9. File-by-file checklist

**T3**
- [ ] `htui-core/src/store/traits.rs` — `set_agent_box_quota -> Result<bool>` + doc (§2.1); `edit_box` C7 doc (§2.4)
- [ ] `htui-core/src/store/mem.rs` — guard + `Ok(bool)` (§2.3); wrapper `:5609`; `an_older_quota_leaves_the_stored_pair_alone`
- [ ] `htui-core/src/store/conformance.rs` — three cases, `quota_row`, arms, `CASES` (§2.7)
- [ ] `htui-core/tests/mem_store.rs:37` — 92
- [ ] `htui-store/src/pg/write.rs` — `:1101-1139` replaced (§2.2)
- [ ] `htui-store/src/writer.rs:416-432` — `Result<bool>`
- [ ] `htui-store/.sqlx/` — −1 +1; 281 files
- [ ] `htui-store/tests/pg_conformance.rs:19` — 92
- [ ] `htui-store/tests/pg_criteria.rs` — `an_older_quota_leaves_the_row_byte_identical`
- [ ] `htui-agent/src/conformance.rs:770-792` — `UsageSpy` answers `bool`
- [ ] `htui-agent/tests/recorder.rs:454-485` — `SpyStore` answers `bool`
- [ ] `htui-agent/src/record.rs` — `latch_quota`'s arm (§2.6)

**T4**
- [ ] `htui-core/src/store/traits.rs:324-333` — new signature + doc (§3.1)
- [ ] `htui-core/src/fixtures.rs` — `edit_agent` before `mod tests` (§3.2)
- [ ] `htui-core/src/store/mem.rs` — CAS (§3.4); wrappers `:5599-5601`
- [ ] `htui-core/src/store/conformance.rs` — three cases, `stamped_agent`, arms, `CASES`; `upsert_agent_by_id_name_unique` rewrite + doc; `:1554`, `:1565`, `:1573` (§3.6)
- [ ] `htui-core/tests/mem_store.rs:37` — 95
- [ ] `htui-store/src/pg/write.rs:1014-1051` — two statements + `cas_miss`; `stored_agent` in `impl PgStore` (`:5704`) (§3.3)
- [ ] `htui-store/src/writer.rs:402-407` — forward both arguments
- [ ] `htui-store/.sqlx/` — −1 +3; 283 files
- [ ] `htui-store/tests/pg_conformance.rs:19` — 95
- [ ] `htui-store/tests/pg_criteria.rs:1178-1280` — `None` / `Some(inserted.updated_at)`, doc
- [ ] `htui-agent/src/conformance.rs:750-752`, `htui-agent/tests/recorder.rs:434-436` — forwards
- [ ] `htui-agent/tests/extensibility.rs` — three `None`
- [ ] `htui/src/{agent_worker,run_worker,store_worker}.rs` tests — §3.7 G1
- [ ] `htui/tests/{backlog,box_probe,box_probe_pg,auth,chat_live_cli,chat,runs_pg,chat_offline,settings,install,probe}.rs` — §3.7 G2; drop unused `WriteStore` imports

**T5**
- [ ] `htui-store/src/pg/mod.rs` — `touch_box` + doc (§5.1)
- [ ] `htui-store/src/connect.rs` — `BOX_HEARTBEAT`, `Started::box_heartbeat`, both constructors, `Debug` (§5.2)
- [ ] `htui-store/.sqlx/` — +1; 284 files
- [ ] `htui-store/tests/pg_criteria.rs` — two `touch_box` tests (§5.4)
- [ ] `htui/src/store_worker.rs` — destructure, `box_beat`, `beat`, the `select!` arm, `beat_once`, abort at exit; two tests (§5.3, §5.4)

**T6**
- [ ] `htui-store/Cargo.toml` — `semver = { workspace = true }`; `Cargo.lock` — `htui-store`'s list gains `semver` (§6.1)
- [ ] `htui-store/src/pg/mod.rs` — `TARGET_VERSION_KEY`, `this_version`, `parse_target`, `stored_target`, `tui_below_target`, `raise_target` (§6.2); `schema_state` → ensure + `applied_state`, `migrations_table_exists`, `embedded_migrations`, `open_pool` (§6.3); `below_target` field + accessor, `connect_with`, `lazy`, `apply_migrations` (§6.4); `HeadlessError`, `connect_headless` (§6.5)
- [ ] `htui-store/src/lib.rs:37` — export `HeadlessError`, `TARGET_VERSION_KEY`
- [ ] `htui-store/.sqlx/` — +4; 288 files
- [ ] `htui-store/tests/migrations.rs` — `target`, `plant_target`, five tests (§6.8)
- [ ] `htui-store/tests/connect.rs` — `start_hands_over_a_store_that_knows_it_is_below_the_target`
- [ ] `htui/src/concepts.rs` — the headless/target paragraph (§6.5)
- [ ] `htui/src/store_worker.rs` — `StoreState.below_target` (variant, `:1226`, `:1445`); `:2314` asserts `None`; `store_state_carries_below_target_from_an_online_store` (§6.6, §6.8)
- [ ] `htui/src/app/state.rs` — `below_target_shown` (+ init `:236`)
- [ ] `htui/src/app/update.rs` — `observe_reply` (`:253`), `note_below_target`, `below_target_notice`; test helpers `:621-630`; `a_store_state_below_the_target_says_so_once`
- [ ] `htui/src/testkit.rs:261`, `:492`; `htui/tests/shell.rs:22` — `below_target: None`

### 10. Gates

Environment and sqlx tooling as milestone 1's §6: `source /home/user/htui-env.sh`; `sqlx-cli
0.9.0`; `htui_sqlx` is already migrated to `0007` and needs no new migration (no task adds one).
Regenerate the offline data after each task's commit (2):

```bash
(cd crates/htui-store && DATABASE_URL=postgres://htui:htui@localhost:5432/htui_sqlx \
  cargo sqlx prepare -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l      # T3 281, T4 283, T5 284, T6 288 (§8)
```

Every task ends with:

```bash
cargo fmt --all -- --check
cargo build --workspace --all-features --all-targets
cargo clippy --workspace --all-features --all-targets -- -D warnings
(cd crates/htui-store && DATABASE_URL=postgres://htui:htui@localhost:5432/htui_sqlx \
  cargo sqlx prepare --check -- --all-targets --all-features)
```

plus, per task:

**T3:**

```bash
cargo test -p htui-core  --all-features -- --test-threads=2
cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=2
cargo test -p htui-agent --all-features -- --test-threads=2
```

**T4:**

```bash
cargo test -p htui-core  --all-features -- --test-threads=2
cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=2
cargo test -p htui-agent --all-features -- --test-threads=2
cargo test -p htui --all-features --lib -- --test-threads=2
cargo test -p htui --all-features --test backlog --test box_probe --test box_probe_pg --test auth \
  --test chat_live_cli --test chat --test runs_pg --test chat_offline --test settings \
  --test install --test probe -- --test-threads=2
```

Milestone 2 closes with `cargo test --workspace --all-features --no-fail-fast -- --test-threads=2`.

**T5:**

```bash
cargo test -p htui-store --all-features --test pg_criteria --test connect -- --test-threads=2
cargo test -p htui --all-features --lib store_worker -- --test-threads=2
```

**T6:**

```bash
cargo test -p htui-store --all-features -- --test-threads=2
cargo test -p htui --all-features --lib -- --test-threads=2
cargo test -p htui --all-features --test shell --test connection -- --test-threads=2
```

Milestone 3 closes with `cargo test --workspace --all-features --no-fail-fast -- --test-threads=2`.
As in milestone 1, `every_provider_failure_leaves_a_valid_prompt` fails on main already and is
the only accepted failure. With `HTUI_TEST_DATABASE_URL` unset the Pg tests return early and
pass vacuously; the gates above assume it is set by `htui-env.sh`.

# Plan: MOD-163 M1 — Entry store

**Source PRD**: `.claude/prds/mod-163-memory-entry-store.prd.md`
**Selected Milestone**: 1 — Entry store
**Complexity**: Large (a new entity across the store seam: a migration, MemStore and PgStore, five
forwarding implementors, conformance, project-delete reach; no UI or prompt change)
**Status**: complete 2026-10-21 (confirmed 2026-10-20, D4 answered at CONFIRM: on a subject clash the person is asked: edit the existing entry, or replace it). T1-T4 and review-fix round R1 landed on `hr/MOD-163`; rust-reviewer: approve with fixes, R1 applied, deferrals below.

## Summary

Add the memory entry as a store entity: project-owned rows with kind, subject, prose, state, a
compare-and-set version, a freshness stamp and an append-only revision history. Every backend can
create, amend, retire, supersede and list entries. Text reaches the store only through a type that has already
been capped and scrubbed fail-closed (`R-ID-7`). M1 ships the store and nothing that calls it: the
prompt section is M2, and the TUI pane, store-worker requests and capture switch are M3.

## Decisions

- **D1 — Tables (migration `0022_memory.sql`, mirroring `0006_requirements.sql`).**
  - `memory_entry`:
    - `id UUID PK` (client-minted, UUIDv7);
    - `project_id → project ON DELETE CASCADE`;
    - `kind TEXT CHECK IN ('fact','decision','gotcha','lesson')`;
    - `subject TEXT CHECK (char_length 1..=120)`;
    - `body TEXT CHECK (char_length 1..=2000)`;
    - `state TEXT CHECK IN ('active','suspect','superseded','retired') DEFAULT 'active'`;
    - `version INTEGER NOT NULL DEFAULT 1`;
    - `confirmed_at TIMESTAMPTZ NOT NULL` (the freshness stamp, D3);
    - `superseded_by UUID NULL REFERENCES memory_entry(id) ON DELETE SET NULL`, set only together with `state = 'superseded'` (a CHECK ties the two);
    - `created_by UUID NOT NULL REFERENCES app_user(id)` (as `requirement.created_by`);
    - `created_at`, `updated_at` (a `set_updated_at()` trigger, created explicitly).
  - Indexes on `memory_entry`:
    - `(project_id, updated_at)`;
    - a **partial unique index named `uq_memory_entry_live_subject`** on `(project_id, lower(btrim(subject))) WHERE state IN ('active','suspect')`, which allows one live entry per subject (D4). It is named so that `db.constraint()` can match it.
  - `memory_entry_revision`:
    - `PK (entry_id, version)`, with `entry_id ON DELETE CASCADE`;
    - `kind`, `subject`, `body`, `state`;
    - `author_id UUID NOT NULL REFERENCES app_user(id)`, `box_id REFERENCES box(id) ON DELETE SET NULL` (as `requirement_revision`);
    - `reason TEXT` (`created` | `amended` | `retired` | `superseded`, no CHECK, like `requirement_revision.reason`);
    - `created_at`.
  - Forward-only (`R-STO-5`), with a header naming MOD-163 and `COMMENT ON` lines.
- **D2 — Types (`htui-core/src/model/memory.rs`).**
  - `MemoryEntryId` via `id_newtype!`.
  - `MemoryKind` and `MemoryState` via `str_enum!`, with the CHECK-list pin test beside the existing ones.
  - `MemoryEntry` (the row).
  - `NewMemoryEntry { id, kind, subject: MemoryText, body: MemoryText, created_by, box_id }`.
  - `MemoryPatch { kind?, subject?, body?, author_id, box_id }`.
  - `MemoryRevision`.
  - `MemoryUpdate { Updated(MemoryEntry), Diverged { head, ancestor } }`, mirroring `RequirementUpdate`.
  - `MemorySupersede { Superseded { old: MemoryEntry, new: MemoryEntry }, Diverged { head, ancestor } }`.
  - The row types derive `Serialize`/`Deserialize` like their requirement twins. `NewMemoryEntry` and `MemoryPatch` do not derive `Deserialize`, so the `MemoryText` guarantee cannot be bypassed by decoding.
- **D3 — Freshness stamp.** `confirmed_at` is when the entry's text was last written or confirmed. Create and amend set it to now; retire leaves it as it was. `version` stays the compare-and-set token. MOD-166 adds the cited-row version stamps and pins beside it. The PRD's open question is answered this way.
- **D4 — One live entry per subject.**
  - Subjects are compared case-insensitively after trimming, within a project, among `active` and `suspect` entries.
  - The fold is `lower(btrim(subject))` in Postgres. `btrim` strips spaces only, so MemStore folds with `trim_matches(' ')` plus `to_lowercase()` to agree with it.
  - A create, or an amend that changes the subject, whose subject collides with another live entry is refused: `StoreError::Constraint` via a `memory_subject_taken` helper that names the existing entry's id. Pg matches `db.constraint() == Some("uq_memory_entry_live_subject")`, as `persona_name_taken` does (`pg/write.rs:787-795`).
  - **Decided at CONFIRM (maintainer, 2026-10-20): ask the person what they meant.** The store only refuses; the TUI (M3) turns the refusal into a choice: "An entry with this subject exists: edit it, or replace it?" Edit opens the existing entry. Replace calls `supersede_memory_entry` (D6) with the new text.
  - MOD-164's automatic writes supersede on the same key through the same verb, with no question.
- **D5 — `MemoryText`: cap and scrub at construction (`R-ID-7`).**
  - Built with `MemoryText::subject(raw, &dyn Scrubber)` / `MemoryText::body(raw, &dyn Scrubber)`.
  - Construction trims the text, refuses empty, and caps the length in characters (`MEMORY_SUBJECT_MAX = 120`, `MEMORY_BODY_MAX = 2000`; the DB CHECKs are the backstop).
  - It then runs the scrubber over `Value::String`: a known secret is masked, and anything credential-shaped returns the scrubber's `Unmasked` error.
  - Errors: `MemoryTextError { Empty, TooLong { max }, Unmasked(Unmasked) }`.
  - The field is private, read through `as_str()`.
  - A person's write in M3 uses `MinimalScrubber::new([])`, which has no secrets to mask and still refuses credential-shaped text.
- **D6 — Store verbs.** All on `ReadStore` / `WriteStore`, with no default impls.
  - `memory_entries(project, states: Option<Vec<MemoryState>>) -> Result<Option<Vec<MemoryEntry>>>`:
    - ordered `updated_at DESC, id`;
    - `None` means "not available here" (the cache, D7).
  - `memory_revisions(id) -> Result<Option<Vec<MemoryRevision>>>`:
    - in `version` order;
    - `Some(vec![])` for an unknown id, as for `requirement_revisions`.
  - `create_memory_entry(project, new) -> Result<MemoryEntry>`:
    - writes revision 1, `reason = "created"`;
    - refusals: `NotFound` for an unknown project, `Constraint` for a duplicate id or a subject clash.
  - `amend_memory_entry(id, expected_version, patch) -> Result<MemoryUpdate>`:
    - checks run in the order NotFound → divergence → Constraint;
    - refused (`memory_entry_closed`) on a `retired` or `superseded` entry;
    - keeps the state, re-stamps `confirmed_at`, and writes a revision at `version + 1` with `reason = "amended"`.
  - `retire_memory_entry(id, expected_version, author_id, box_id) -> Result<MemoryUpdate>`:
    - moves `active | suspect → retired`;
    - writes a revision with `reason = "retired"`;
    - an already-retired or superseded entry is refused (`memory_entry_closed`).
  - `supersede_memory_entry(old, expected_version, new: NewMemoryEntry) -> Result<MemorySupersede>`:
    - one transaction: the old entry (`active | suspect`, compare-and-set on `expected_version`) moves to `superseded` with `superseded_by = new.id` and a revision `reason = "superseded"`; then the new entry is created in the old one's project, with revision 1;
    - order: NotFound (old) → divergence → Constraint (`memory_entry_closed` on an old entry that is already retired or superseded; `memory_subject_taken` if the new subject clashes with a third live entry);
    - the new subject may differ from the old one.
  - Nothing in M1 sets `suspect`.
- **D7 — Not mirrored.**
  - The SQLite cache gets no migration.
  - `CacheStore` answers `Ok(None)` to both reads, so the M3 pane can say it is not available offline.
  - `R-STO-3`'s list is unchanged. The PRD's open question is answered this way.
- **D8 — Project delete.**
  - `DeleteReach` gains `memory_entries` and `memory_revisions`.
  - Both are counted in `project_reach` (Mem and Pg).
  - Mem's `delete_project` drops the rows, matching Pg's cascade.
  - `reach_parts` (`htui/src/hierarchy.rs`) prints them.
- **D9 — Deferred columns.** Each later item migrates its own columns, so M1 ships no dead columns:
  - provenance (run, agent, item, commit) → MOD-164 (`superseded_by` is in M1 because the person's "replace" needs it, D4);
  - usage counters → MOD-164;
  - pin and pinned-as-trusted → MOD-166;
  - `created_by` stays a person in M1, and MOD-164 relaxes it for model writes.
  - The PRD's open question is answered this way.
- **D10 — No demo rows.** Conformance cases create their own entries in a demo project, so `DemoData` and `pg/demo.rs` are untouched.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Enums | `htui-core/src/model/mod.rs:25` `str_enum!`; `requirement.rs:12-29` | Doc'd variants; CHECK-list pin test in `model/mod.rs` |
| Ids | `htui-core/src/model/ids.rs:15` `id_newtype!` | UUIDv7 `new()`, serde/sqlx transparent |
| Row / New / Patch / Revision / Update | `htui-core/src/model/requirement.rs:105-291` | Field docs name the column; `Update::{Updated, Diverged{head, ancestor}}` |
| Trait verbs | `store/traits.rs:241-258` (reads), `:1806-1842` (writes) | Doc gives decision order and `# Errors`; refusal helpers like `requirement_withdrawn` |
| MemStore | `store/mem.rs` `State::mint_requirement` :7133, `revise_requirement` :7199; wrappers :8346-8358, :9165-9193 | `self.write(\|state\| …)`; append-only revision `Vec` in `State` |
| PgStore | `pg/write.rs` mint :6973 (one CTE), `revise_requirement` :639 (`FOR UPDATE`, version check, `Diverged`); `pg/read.rs` :916-1001 | `query!`/`query_as!` with `AS "col: Type"`; `.sqlx` regenerated |
| Migration | `migrations/0006_requirements.sql` | `TEXT CHECK IN (…)`, cascade FKs, revision PK `(id, version)`, `updated_at` index, explicit trigger |
| Cache refusal | `cache/read.rs:1216` `requirement_revisions` → `Ok(None)` | Same for both memory reads |
| Forwarders | `writer.rs:274-292, 1369-1400`; `backend.rs:1059-1088` (reads only); `htui-agent/src/conformance.rs` UsageSpy :642/:751; `htui-agent/tests/recorder.rs` SpyStore :329/:439 | Forward to the inner store, both arms |
| Delete reach | `traits.rs:3069` `DeleteReach`; `pg/write.rs:511` `project_reach`; `mem.rs:4696, 4934`; `htui/src/hierarchy.rs:797` | One `u64` per table; one count subselect |
| Conformance | `store/conformance.rs` `CASES` :61, `run_case` :248, `READ_CASES` :675; example :13631 | Generic `async fn x<S: WriteStore>(store: &S)`; run by `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`, `htui-store/tests/cache.rs` |
| Scrub | `htui-core/src/scrub.rs:162-180` `Scrubber::scrub(&mut Value)`; `htui-mcp/src/tools/note.rs:55` | Scrub, then cap, then write; never persist on `Unmasked` |

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/model/memory.rs` | CREATE | T1 |
| `crates/htui-core/src/model/mod.rs`, `model/ids.rs` | UPDATE | T1 |
| `crates/htui-core/src/store/traits.rs` | UPDATE (verbs, helpers, `DeleteReach`) | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE (stubs in T1; logic, reach, delete in T2) | T1, T2 |
| `crates/htui-store/src/pg/read.rs`, `pg/write.rs` | UPDATE (stubs in T1; SQL in T3) | T1, T3 |
| `crates/htui-store/src/cache/read.rs` | UPDATE (`Ok(None)`) | T1 |
| `crates/htui-store/src/writer.rs`, `backend.rs` | UPDATE (forward) | T1 |
| `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | UPDATE (forward / stub) | T1 |
| `crates/htui/src/hierarchy.rs` | UPDATE (`labelled()` destructure and array 28 → 30) | T1 |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE (`every_cascade_table_loses_exactly_what_the_report_names`: `TABLES`/`claimed` 28 → 30, by position) | T1 |
| `crates/htui-store/tests/cache.rs` | UPDATE (cache-only test: both memory reads → `None`) | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE (cases) | T2 |
| `crates/htui-core/tests/mem_store.rs` | UPDATE (`CASES.len()` 175 → n) | T2 |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE (`EXPECTED_CASES` 175 → n) | T2 |
| `crates/htui-store/migrations/0022_memory.sql` | CREATE | T3 |
| `crates/htui-store/tests/migrations.rs` | UPDATE (`TABLES` +2 → 48, versions 1..20, `Pending(20)`, `MigrationsPending(20)`, `_sqlx_migrations` 20, a `MOD163_COLUMN_COMMENTS` chained into the comments test) | T3 |
| `crates/htui-store/tests/connect.rs` | UPDATE (19 → 20 at :140, :156, :242) | T3 |
| `crates/htui-store/.sqlx/query-*.json` | CREATE/UPDATE (prepare) | T3 |

## Tasks

Every task leaves `cargo check --workspace --all-targets --all-features` green. The order is
**T1 → (T2 ∥ T3) → T4**. T2 and T3 have disjoint file sets (fact-checked below), so they are the
parallel fan-out.

### Task 1: Types, verbs and the compile-green skeleton (serial)
- **Action**: D2, D5 and D6 signatures; `DeleteReach` fields (D8). Forwarders in Writer, Backend
  (reads), UsageSpy, SpyStore; `CacheStore` → `Ok(None)` with a cache-only test in
  `htui-store/tests/cache.rs` (not a `READ_CASES` entry: those also run on Mem and Pg, which answer
  `Some`). `labelled()` in `hierarchy.rs` and the positional `TABLES`/`claimed` arrays of
  `pg_criteria.rs` gain both fields; their counts are zero until T2/T3. MemStore and PgStore
  get **stub bodies** (`Err(StoreError::Backend("MOD-163 M1: not yet".into()))`) and zero reach counts.
  Unit tests first, for `MemoryText` (empty, too long by one character, a non-ASCII length counted in
  characters, a masked known secret, a credential-shaped refusal whose error does not repeat the
  text) and for the CHECK-list pin of both enums.
- **Mirror**: requirement model and trait docs; `RequirementText`'s length-only `Debug` for
  `MemoryText`.
- **Validate**: `cargo test -p htui-core model::memory`; `cargo check --workspace --all-targets
  --all-features`; `cargo clippy --workspace -- -D warnings` (featureless).

### Task 2: MemStore and the conformance cases (parallel with T3)
- **Action**: conformance cases first, red on MemStore:
  - create writes revision 1 and the row;
  - list order and state filter;
  - a stale amend → `Diverged` with the ancestor;
  - an amend re-stamps `confirmed_at` and writes revision `v+1`;
  - retire → `retired`, and a second retire is refused;
  - amending a retired entry is refused;
  - a subject clash on create and on amend, case and whitespace folded, naming the existing id;
  - supersede: the old entry becomes `superseded` with `superseded_by` and a `superseded` revision, and the new entry is live, in one step; a stale `expected_version` → `Diverged` with nothing written; superseding a retired entry is refused; the new entry may reuse the old subject;
  - a retired subject can be reused;
  - an unknown id or project → `NotFound`;
  - `memory_revisions` of an unknown id → `Some(vec![])`;
  - project delete reach counts and removes the rows.

  Then the `State` logic, the reach counts and the delete in `mem.rs`, with memory assertions added
  to `delete_project_leaves_no_row_in_any_map` (`mem.rs:11763`). Bump the case counts in
  `htui-core/tests/mem_store.rs:36` and `htui-store/tests/pg_conformance.rs:39`.
- **Mirror**: `State::mint_requirement` / `revise_requirement`; `CASES` + `run_case` registration
  (`run_case_accepts_every_name_in_cases` guards it).
- **Validate**: `cargo test -p htui-core --test mem_store`; `cargo test -p htui-core
  conformance::run_case_accepts_every_name_in_cases`.

### Task 3: Migration and PgStore (parallel with T2)
- **Action**: `0022_memory.sql` (D1). The `pg/read.rs` reads and `pg/write.rs` writes (create as
  one CTE; amend and retire through one `revise_memory_entry` with `FOR UPDATE`), plus the
  `project_reach` subselects. Map the unique-index violation to `memory_subject_taken` and the CHECK
  violations to `Constraint`. Update the migration pins in `tests/migrations.rs` and
  `tests/connect.rs` (19 → 20, `TABLES` → 48, `MOD163_COLUMN_COMMENTS`). Regenerate `.sqlx`
  against a migrated scratch database (sandbox: `psql -h localhost -p 5439 -U postgres`, see
  `docs/hr-sandbox.md`).
- **Mirror**: `revise_requirement` (:639), mint CTE (:6973), `project_reach` (:511).
- **Validate**: in `crates/htui-store`, `DATABASE_URL=<migrated scratch db> cargo sqlx prepare
  --check -- --all-targets --all-features`; `cargo test -p htui-store --all-features --test migrations
  --test connect -- --test-threads=1`; `--test pg_conformance` once T2's cases have landed.

### Task 4: Integration gate (serial, after T2 and T3)
- **Action**: merge the lanes. Run the whole workspace gate and fix what it finds. No new behaviour.
- **Validate**: see Validation.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m1-gate.log
grep -c SIGABRT /tmp/m1-gate.log   # expect 0
(cd crates/htui-store && DATABASE_URL=<migrated scratch db> cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| T2 and T3 both need `traits.rs` changes discovered late | Medium | T1 fixes every signature, and T2/T3 may not touch `traits.rs`; a needed change goes back through T1's owner |
| `lower(btrim(subject))` in Postgres folds differently from Mem's Rust fold (non-ASCII case, tabs and newlines) | Medium | Mem folds with `trim_matches(' ')`, as `btrim` does; `MemoryText` already trims all whitespace on the way in; the conformance case uses ASCII case and spaces; document the non-ASCII difference as `RequirementFilter::text` does |
| `.sqlx` drift between lanes | Medium | Only T3 touches `.sqlx`; T4 runs `prepare --check` |
| A new Pg migration with no cache migration trips a schema-version check | Low (verified, C3) | It does not trip anything: each box rebuilds its mirror once, an older binary meeting a version-20 database is refused (`schema_is_newer`), and a headless worker waits for a TUI to migrate, all by design |
| UsageSpy / SpyStore missed, so a test crate fails to compile only under `--all-features` | Medium | T1 validates with `--all-targets --all-features` |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented

## Verified claims

Plan fact-check, 2026-10-20, at `325922f1`. Postgres probes ran on the sandbox's PostgreSQL 16.15 in a
scratch database, which was then dropped.

| Claim | Verdict | Evidence |
|---|---|---|
| C1 `requirement.created_by` → `app_user`; revision `author_id` NOT NULL, `box_id` SET NULL; shared `set_updated_at()`, triggers per table | TRUE | `0006_requirements.sql:84,101-103,135-142`; `0001_init.sql:21` |
| C2 nothing enumerates migrations or tables | **FALSE → amended** | `tests/migrations.rs` (`TABLES` :84, versions :103-105, 46 :126, `Pending(19)` :1903/:2004, `MigrationsPending(19)` :2013/:2036, count :2189); `tests/connect.rs:140,156,242`; `pg_criteria.rs:3188-3300`; `mem_store.rs:36,73`; `pg_conformance.rs:39`. Now in T1, T2 and T3 |
| C3 Pg 0020 without a cache migration is safe | TRUE | `pg/mod.rs:724-726`; `cache/mod.rs:140-152` rebuilds on mismatch; `schema_is_newer` `pg/mod.rs:912` |
| C4 `NewMemoryEntry`/`MemoryPatch` may skip `Deserialize` | TRUE | `StoreRequest` derives `Debug, Clone` only (`store_worker.rs:151-152`); `WorkerStore` in-process (`worker.rs:141`); no serialized `WriteStore` wire |
| C5 enum pin test binds to the CHECK | PARTIAL | `check_enum` `model/mod.rs:212-232` pins `ALL` to a hand-copied string list, not the SQL; both are kept in step by hand |
| C6 `StoreError::Backend(String)` exists for stubs | TRUE | `store/error.rs:46-47` |
| C7 partial unique `lower(btrim())` index behaves as D4; `char_length` counts characters | TRUE (probe) | `Foo`/` foo ` → 23505; `retired` and another project insert; `char_length('é')=1`; CHECK refuses 4×`é` at 3. Caveat: `btrim` strips spaces only → D4 Mem fold amended. Mapping: `map_sqlx` (`htui-store/src/error.rs:17-52`), `persona_name_taken` (`pg/write.rs:787-795`) → index named in D1 |
| C8 T2 ∥ T3 file sets disjoint | PARTIAL → amended | Runner loops `CASES` (`pg_conformance.rs:57-66`) but pins the count (:39) → T2; `project_reach`/`delete_project` Mem-only (`mem.rs:4696,4934`); Pg `project_reach` is `query!` (`pg/write.rs:512`) → T3 `.sqlx`; cache read case cannot be a `READ_CASES` entry (`cache.rs:1598`, `pg_conformance.rs:84`) → cache-only test in T1. Final sets: T2 {`store/mem.rs`, `store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`}; T3 {`migrations/0022_memory.sql`, `pg/read.rs`, `pg/write.rs`, `.sqlx/*`, `tests/migrations.rs`, `tests/connect.rs`}; intersection empty |
| C9 `self.write(\|state\| …)`; NotFound → divergence → Constraint | TRUE | `mem.rs:1355`, :9165-9190; `revise_requirement` :7209, :7216, :7232 |
| C10 `reach_parts` reads fields by name | **FALSE → amended** | `labelled()` `hierarchy.rs:729-794` destructures exhaustively into `[(u64,&str); 28]`; no insta snapshots, tests use `..Default` |
| C11 `RequirementText` length-only `Debug` | TRUE | `requirements.rs:72, 88-94` (byte length) |
| C12 `cargo sqlx prepare --workspace --check` | **FALSE → amended** | `.sqlx` is `crates/htui-store/.sqlx`; README:705-719 runs prepare from `crates/htui-store` |

## Review outcome (rust-reviewer, 2026-10-21)

Verdict **Approve with fixes**: no critical or high findings. Each finding was checked by its own
adversarial verifier before any fix.

| # | Finding | Verifier | Outcome |
|---|---|---|---|
| 1 | `MemoryText` did not record its cap; a body could be passed as a subject (Mem stores it, Pg refuses) | real, low | **Fixed (R1, `7e9ab951`)**: split into `MemorySubject` / `MemoryBody`, with `compile_fail` doctests |
| 2 | Pg-only supersede/unknown-box refusal paths untested; duplicate-id check matched `Constraint(_)` only | real, low | **Fixed (R1, `c38a0ab3`)**: tests only; both stores already behaved |
| 3 | `confirmed_at` vs `updated_at`: Pg strictly less after an amend, Mem equal | real, nit | **Documented + pinned (R1, `e7a14e11`)**: `confirmed_at <= updated_at`; carried to MOD-166 below |
| 4 | Pg CAS `UPDATE` has no `AND version = $expected` | refuted (the row lock under READ COMMITTED decides; as `revise_requirement`) | Deferred hardening, below |
| 5 | Early returns rely on the transaction's drop to roll back | refuted (the pattern of the other 36 transaction sites) | No change |
| 6 | Fold doc overclaimed ASCII agreement (Turkic locales) | real, nit | **Fixed (R1, `e7a14e11`)** |
| 7 | Refusal precedence differs between stores when several inputs are wrong | real, nit | **Documented (R1, `e7a14e11`, `91718de0`)**: refusals are not ordered across backends; `already_exists`, `memory_subject_taken` and `memory_entry_closed` have backend-independent text |
| 8 | Nits: two `.expect()` in `mem.rs`, clones, `MemoryEdit` field names, `Eq` derives, no `CHECK (version >= 1)`, no `memory_entry_revision(box_id)` index | real, nit | Deferred, below |

### Deferred (needs the maintainer's OK; proposed at M1 close-out)

- **F4:** add the version predicate to the Pg memory `UPDATE`, plus a two-writer `tokio::join!` race test. This is hardening only, because the row lock already decides the compare-and-set. Take it when M2/M3 next touch `pg/write.rs`.
- **F8 nits:** a check-free insert helper in `mem.rs` (removes the second `.expect()`), `apply_memory_edit(&MemoryEdit)`, `&[&str]` binding in `memory_entries`, one field name across both `MemoryEdit`s, `Eq` on `MemoryEntry`/`MemoryRevision`, `CHECK (version >= 1)` and a `memory_entry_revision(box_id)` index in a later migration (`requirement_revision` lacks both too).
- **Carried to MOD-166:** freshness must not compare `confirmed_at` with `updated_at` for equality; Postgres makes `confirmed_at` strictly earlier after an amend.

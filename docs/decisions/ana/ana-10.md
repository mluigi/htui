# ANA-10 - Local-only mode: SQLite as a primary writable store (done, 2026-09-08)

## Summary

A box that has never been given a Postgres DSN already starts — `NO_DSN` is a message, not a fatal
(`crates/htui-store/src/connect.rs:36`) — but it starts into an empty **read-only** mirror.
`CacheStore` is "`ReadStore` only: it can never accept a write" (`cache/mod.rs:74`; `HANDOFF.md`
cited `:69`, which was stale), its schema is derived from Postgres, so the user sees nothing and can
create nothing, and a chat refuses with "this box is not registered". ANA-10 settles what a writable
local store is, what schema it carries, who mints ids with no server, what happens when the box is
later pointed at a server, and the first-run experience.

Full analysis: `docs/ANA-10.md` (2 968 lines) — thirteen sections, 33 maintainer questions, 26 risks,
31 validation criteria.

Research ran as a 14-agent sweep (five repo readers over the store backends, three web surveys over
local-first prior art, dual-dialect Rust practice and offline id minting, five per-question option
designers, one adversarial consistency critic), then a single writer agent, then two amendment
research passes after the maintainer's decisions. The critic's seven cross-verdict conflicts and
eight unsupported claims are applied as amendments in the document itself (§4.6, §4.7) rather than
recorded as commentary — two verdicts changed as a result.

## What was decided

**Q1 - shape.** A fourth `Backend::Local` arm over a **separate** file,
`<config_root>/local/local.sqlite`, backed by a new concrete `LocalStore: WriteStore`. The deciding
fact is mechanical, not aesthetic: `cache.sqlite`'s contract is "delete me on mismatch" —
`CacheStore::open` deletes the file plus its `-wal`/`-shm` and re-migrates whenever
`cache_meta.schema_version` or `db_fingerprint` differs, and `schema_version` is the binary's own max
embedded Postgres migration. Putting user-authored rows there means a routine `htui` upgrade erases
the only copy of the user's work, and setting a DSN orphans it (the directory name changes from
`offline` to `sha256(host:port/db)`).

**Q2 - schema.** Local-only gets its own forward-only migration set (`local_migrations/`), authored
against the Postgres schema's semantics rather than the mirror's subset. `cache_migrations/` and
`cache/refresh.rs` are untouched, which is what keeps `R-STO-3`'s word "read-only" literally true.

**Q3 - ids.** Generalise the contract, reject the code. Item keys are minted locally through a
**project-origin-scoped counter**: `R-ENT-7`'s "never reused" guards against two writers on one
`(project, prefix)` counter, and a project created on a box with no server has exactly one writer by
construction for its whole local life. UUID-keyed rows generalise the existing client-side UUIDv7
mint; `upload_pending`'s adoption *contract* generalises but its chat-specific code does not.

**Q4 - transition.** Local rows **stay local** when a DSN is later configured. Adoption is explicit,
confirmed, one-shot and server-bound, because it is irreversible in a way nothing else in the store
is (`no_delete_path`) and must not be a side effect of setting a DSN.

**Q5 - UX.** A modal first-run overlay triggered by an explicit no-DSN signal out of `connect::start`,
marked shown-once by a `#[serde(default)]` field in `box.toml` — the only local persistence site not
destroyed by an unrelated routine operation. A distinct top-bar state, never conflated with
`offline · <age>` ("server known, unreachable").

**Two verdicts the consistency pass changed.** `local_only` is removed entirely — neither lock nor
preference, not a mode flag at all; `box.toml` records three *facts* instead. And `Backend::Online`
and `Backend::Offline` gain `local: Option<LocalStore>`, without which "keep local" would have meant
"unreadable next launch".

## Maintainer decisions taken at close-out (2026-09-08)

| # | Question | Decision |
|---|---|---|
| §10.3 | May local-only create items at all? | **Yes.** `R-ENT-7` is amended; the origin-scoped counter ships. |
| §1.3 | What is local-only *for*? | A **temporary, lite mode**. The full product is `htui` against Postgres; the path to online usage must exist. Parity is over features, not over server-shaped infrastructure (no mirror, no refresh cursor, no warm-cache budget). |
| §10.5 | Is adoption funded, deferred or declared never? | **Funded** as MOD-18 — answered "deferred" first, then superseded the same day by the framing above: it is the exit the mode promises. **MOD-17 must not ship copy promising migration before MOD-18 exists.** |
| §10.4 | Does `R-STO-6`'s sub-second budget bind a local-only start? | **No** — it is one of the guarantees that means nothing without a server. `R-STO-6` left unamended; §12 criterion 13 kept as a recorded figure rather than a limit. |
| §10.8 | Does local-only cover graph runs? | **Yes — full parity** with a server-backed box. Overrides the document's own recommendation. |
| §10.14 | Is an in-app DSN field permitted? | **Required, not optional** — it is the item's stated objective. |
| §10.13 | Must leaving local-only be in-process? | Restart boundary retained for the first MOD; the in-process path is MOD-19. |

**The runs answer's consequences** are §4.8, §5.3, §7.1, §9.1 and risks 22-26 of `docs/ANA-10.md`.
`LocalStore`'s surface goes from 22 async methods to **61** — the document's own earlier estimate
("roughly 16 writes and 14 inherent reads") counted table rows rather than method names and omitted
the `ReadStore` set; ANA-2's summary sentence undercounts its own table by two and `HANDOFF.md:186`
inherited the error. It is affordable only because ANA-2 §2 invariant 10 makes `htui-orch` depend on
`htui-core`, so the engine is generic over `WriteStore` and never learns a third store exists.
Because `migrations/0003_orchestration.sql` does not exist yet, the local run schema splits into
**Set A** (buildable today from `0001_init.sql`) and **Set B** (reserved to MOD-4), with a hard
ordering rule: MOD-17's M3 must land before MOD-4's build step 1.

**The DSN-field answer's consequences** are §4.9, §9.1's M2b and risks 16-21. It reverses the
`--set-dsn`-only posture as *the only path*, not as a path; `--set-dsn` is retained for a terminal
that cannot enter raw mode and for scripted setup. It is defensible rather than merely ordered
because the decision it reverses had already traded away echo suppression (`lib.rs:117` prints "it
will be visible"), and because `PgConnectOptions` exposes no password getter, so the post-parse
confirmation is password-free by construction. The two new exposure axes — in-process lifetime and
`Debug` derives on `StoreRequest`/`RequestEnvelope` — are closed by a `Dsn` newtype with a
hand-written `Debug` and a `Zeroizing<String>` buffer.

## Corrections this analysis made to the tree's own record

- `HANDOFF.md` cited `cache/mod.rs:69` for the "`ReadStore` only" line; it is at `:74`.
- ANA-2 §8's write table is 17 rows naming **18** methods; its own summary at `docs/ANA-2.md:1760`
  and `HANDOFF.md:186` both say sixteen.
- `docs/ANA-10.md`'s first draft claimed an in-process transition contradicts `connect.rs:66-70`'s
  "the worker never holds a DSN". It does not: the `Reconnect` closure at `connect.rs:198-208`
  captures the DSN **by move**, so the worker already holds one inside an `Arc<dyn Fn>` it cannot
  read. The property that clause actually buys — no `store_worker` type or signature names a DSN — is
  preserved exactly by a `connect::reconnect_for` factory.
- The first draft also claimed the `Composer`-as-mode pattern is blocked by the Settings tab's
  single-letter bindings. `ChatTab` already solves that by consulting the composer first
  (`chat/mod.rs:398-402`); `SettingsTab::on_key` simply has the delegation order backwards
  (`settings/mod.rs:197-213`), which a default-bodied `captures_input` predicate fixes in five lines.
- `StoreError::ReadOnly` is declared (`error.rs:23-25`) and **never constructed** anywhere in the
  workspace; every write refusal today is `Unreachable`, which callers read as "retry later may
  work". ANA-10 is its first use.
- `WriteStore` has no hierarchy-creation method at all — `mint_item` exists, but nothing creates
  workspace, project or item kind, and runs additionally need `create_repo` and
  `set_repo_box_path`. A first run on a server-less box has nothing to mint into (§5.6, §7.1).

## Open for the maintainer (not blocking MOD-17's first milestones)

`docs/ANA-10.md` §10 carries all 33 questions with options and recommendations. The unanswered ones
are milestone-scoped: §10.17 (multi-process guard on `local.sqlite` — it gates local
`shared_serialized`), §10.22/§10.32 (how exactly the local schema mirrors Postgres's constraints,
one-way doors under SQLite), §10.7 (may adoption rewrite authorship), §10.33 (how adoption remaps
`agent`), and the requirement-amendment shape questions §10.1, §10.19, §10.20.

**`docs/REQUIREMENTS.md` was amended on 2026-09-08**, by explicit maintainer decision taken after
this analysis concluded and applied as its own commit rather than as close-out bookkeeping, per
`.claude/rules/workflow-docs.md`. `R-STO-7` was added (local-only mode as a complete box) and
eleven statements amended in place on §6.1's wording: `R-STO-1` s.1, `R-STO-4` (one word, so every
document citing it for "server known, unreachable" still cites it correctly), `R-ENT-7`, `R-ID-3`,
`R-HIS-1`, `R-STO-5`, `R-AGT-4`, `R-PRM-4`, `R-SKL-1`, `R-TUI-1` and `R-TUI-8`. `R-STO-1` s.2,
`R-STO-2`, `R-STO-3`, `R-SEC-1..4`, `R-ID-7`, `R-ORCH-1..13` and `R-HIS-2` are unaffected and are
recorded as such. `R-STO-6` was deliberately left unamended (§10.4): its sub-second budget is
conditioned on a reachable server, and a lite mode users are meant to leave does not carry the full
product's performance contract — the measurement is kept as a recorded figure so a regression stays
visible.

## Downstream items

- **MOD-17** - the implementation (M0-M6 plus M2b). Not blocked. M6 gated on the `R-ENT-7`
  amendment; M3 must precede MOD-4's build step 1.
- **MOD-18** - adoption (M7), **funded**: it is the exit a temporary mode promises. Blocked on
  MOD-17's M6, and MOD-17's first-run copy may not promise the move until it ships.
- **MOD-19** - in-process transition out of local-only (M9), deferred. Blocked on MOD-17's M3.
- **MOD-4** - gains M8 (local graph runs) and is now blocked on MOD-2 **and** MOD-17's M3; its
  migration list gains `local_migrations/0002_orchestration_local.sql` in the same commit.
- **MOD-12** - `ready_items` must be rewritten for SQLite, not only implemented for Postgres.
- **MOD-13** / **MOD-15** - their server-less create and edit paths belong to MOD-17; MOD-15 owns the
  Settings connection *section*, MOD-17 owns the masked DSN *field* inside it.

## Commits

- `docs/ANA-10.md`, this write-up, the `DECISIONS.md` index line and the `HANDOFF.md` close-out
  (ANA-10 line removed, MOD-17/18/19 opened, MOD-4/12/13/15 cross-links amended, summary table and
  status line updated).

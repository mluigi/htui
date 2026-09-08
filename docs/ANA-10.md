# ANA-10 - Local-only mode: SQLite as a primary writable store

> **Scope note:** Design authority for what `htui` does on a box that has never been given a
> Postgres DSN: the shape of a writable local store and where it attaches to the `Backend` seam,
> the schema that store carries, who mints identifiers and item keys with no server in reach, what
> happens when such a box is later pointed at a server, and the first-run experience that tells the
> user which of the two states they are in, including the in-app path by which a DSN is defined
> without leaving the program (§4.9). Amends the store model of `docs/ANA-9.md` (§2 invariants 1, 2
> and 4, §4.2 step 4, §4.4, §6.1, §11 criteria 5 and 7), implemented as MOD-6
> (`docs/decisions/mod/mod-6.md`). Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md` and
> `docs/REQUIREMENTS.md`.
>
> **Requirements addressed:** `R-STO-1`, `R-STO-3`, `R-STO-4`, `R-STO-5`, `R-TUI-8`, `R-NF-3`.
> Touched at their seams and named where a maintainer decision is owed: `R-ID-3`, `R-ID-7`,
> `R-ENT-5`, `R-ENT-6`, `R-ENT-7`, `R-ENT-10`, `R-HIS-1`, `R-HIS-2`, `R-SEC-2`, `R-SEC-3`,
> `R-SEC-4`, `R-STO-2`, `R-STO-6`, `R-USR-2`, `R-BOX-1`, `R-TUI-1`, `R-AGT-4`, `R-AGT-6..7`,
> `R-PRM-4`, `R-SKL-1..2`, `R-ORCH-1..13`.
>
> **Status (2026-09-08): concluded, with the gating maintainer decisions taken the same day.**
> **Local-only is a temporary, lite mode; the full product is `htui` against Postgres, and the path
> to online usage must exist** (§1.3) — so adoption is **funded** (§10.5 → MOD-18) and the first-run
> copy may not promise the move before that item ships (§4.5). Parity is over *features*, not over
> server-shaped infrastructure: no mirror, no refresh cursor, no `db_fingerprint`, no
> `offline · <age>`, and no warm-cache startup budget (§10.4 → a, `R-STO-6` left unamended).
> Local-only covers **graph runs at parity** with a server-backed box (§10.8 → b, §4.8); local item
> creation is **in** and `R-ENT-7` is amended (§10.3 → b); **in-app DSN entry is a requirement of the
> item**, not an option (§10.14 → b, §4.9). Implementation is tracked as MOD-17 (M0-M6 plus M2b);
> §9.1's M7 is MOD-18 (adoption, funded) and M9 is MOD-19 (in-process transition, deferred); M8
> (local graph runs) belongs to MOD-4 under §9.1's ordering rule.
>
> **`docs/REQUIREMENTS.md` was amended on 2026-09-08 by explicit maintainer decision**, after this
> document concluded and on the wording §6.1 proposes: `R-STO-7` added, and `R-ID-3`, `R-ENT-7`,
> `R-STO-1`, `R-STO-4`, `R-STO-5`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`, `R-SKL-1`, `R-TUI-1` and
> `R-TUI-8` amended in place. `R-STO-6` was deliberately left alone (§10.4). §6.1 remains the
> rationale for each; the requirements file carries the resulting text.

---

## 1. Context and problem statement

### 1.1 What a first run does today

A box that has never run `htui --set-dsn` starts. That is deliberate and documented:

```rust
/// The cache directory name used when there is no DSN to fingerprint.
///
/// A box that has never run `htui --set-dsn` still gets a mirror, so `--offline` and a first
/// launch render the same empty shell instead of failing to open one.
pub const NO_DSN_FINGERPRINT: &str = "offline";
```

(`crates/htui-store/src/connect.rs:34-38`.) `connect::start` computes a fingerprint from the DSN or
falls back to the literal `"offline"`, opens `<config_root>/cache/offline/cache.sqlite`, and always
produces `Backend::Offline`:

```rust
let fingerprint = dsn
    .as_deref()
    .map_or_else(|| NO_DSN_FINGERPRINT.to_owned(), identity::db_fingerprint);
let cache = CacheStore::open(&root, &fingerprint, PgStore::schema_version()).await?;

let connecting = !opts.offline && dsn.is_some();
let backend = Backend::Offline {
    cache,
    since: if connecting { None } else { Some(Utc::now()) },
};
```

(`connect.rs:184-193`.) So the first run does not fail. It opens into an empty **read-only** mirror,
and five separate mechanisms then conspire to make it useless.

**One. The store it opens can never accept a write, by type.**

```rust
/// The per-box read-only mirror (§4.4). `ReadStore` only: it can never accept a write.
```

`crates/htui-store/src/cache/mod.rs:74`. (`HANDOFF.md` cites `cache/mod.rs:69` for this line; the
sweep re-read the working tree and the line is at `:74`. Cite `:74`.) The refusal is a compile
error, not a runtime flag: `CacheStore` implements `ReadStore` only (`cache/read.rs:205`), and there
is no runtime write path on it to reuse.

**Two. Its schema is derived from Postgres and is a strict subset.**
`cache_migrations/0001_mirror.sql:24-25` states it in as many words: "Not mirrored, and deliberately
without a table: revisions, skills, templates, graphs, phases, agents, counters, settings,
capability tags, command queue." The three tables a writable store needs first — `item_revision`,
`item_key_counter`, `step_graph` — are all on that list. `0001_mirror.sql:19-20` drops the integrity
too, on a premise that is exactly false with no server: "no CHECK constraints and no foreign keys:
one writer feeds this file from a database that already enforced them".

**Three. The file is disposable by design.** `CacheStore::open` deletes `cache.sqlite` plus its
`-wal` and `-shm`, recreates, re-migrates and writes a fresh `cache_meta` whenever
`cache_meta.schema_version` differs, `cache_meta.db_fingerprint` differs, or `cache_meta` is missing
(`cache/mod.rs:137-149`, `:261-279`). `schema_version` is the binary's own maximum embedded
*Postgres* migration version (`pg/mod.rs:387-389`) and is passed even when there is no DSN at all
(`connect.rs:187`). `CacheStore::rebuild()` — documented in-tree as `Settings > Rebuild cache` and
assigned to MOD-15 by `HANDOFF.md:251` — issues `DELETE FROM {table}` over all sixteen
`MIRRORED_TABLES` (`cache/mod.rs:164-193`).

**Four. Nothing local survives a server appearing.** The refresher holds exactly six `DELETE`
statements (`cache/refresh.rs`, verified by grep at `:413`, `:466`, `:516`, `:552`, `:924`,
`:1356`): four whole-table replaces of `app_user`, `agent`, `workspace` and `workspace_project`; the
`item_link` tombstone-to-hard-delete; and the `session_event` trim against a server-derived wanted
list. Every cursor-driven row is written by `upsert_sql` as
`ON CONFLICT (<pk>) DO UPDATE SET <every non-pk column>` (`refresh.rs:355-376`) with no column that
could exclude a locally authored row. And the fingerprint directory renames from `offline` to
`sha256(host:port/dbname)` the moment a DSN is set, so anything under `cache/offline/` is not
adopted, not refused — just never opened again. That already happens, today, silently, to
`cache/offline/pending/*.jsonl`.

**Five. The shell says nothing.** `Backend::label()` has four outputs — `memory`, `online`,
`connecting`, `offline · <age>` (`backend.rs:79-91`) — and the top bar renders `state.store`
verbatim as one span with no branching (`crates/htui/src/ui/top_bar.rs:34-43`). A first run with no
DSN and a box whose server just died are byte-identical: both read `offline · 0s`, one from
`connect.rs:191` and one from `Backend::went_offline()`, and both are pinned by tests
(`crates/htui-store/tests/connect.rs:179` and `:241`). The `NO_DSN` message
(`"no DSN stored; run \`htui --set-dsn\`"`, `connect.rs:41`) is worse than unrendered: `attempt` is
the only producer of it and `start()` never spawns `attempt` when `connecting` is false
(`connect.rs:210-222`), so on the real first-run path it is never produced at all. And
`ConnEvent::Failed`'s reason is consumed with a `tracing::warn!` that never reaches stdout
(`crates/htui/src/store_worker.rs:525-528`, `crates/htui/src/lib.rs:131-134`).

The visible consequences are the two the HANDOFF item names. The workspace switcher opens on the
empty first `Workspaces` reply and reads `"no workspaces — creating one arrives with MOD-15"`
(`crates/htui/src/ui/overlay/workspace_switcher.rs:110-117`). And a chat refuses:

```rust
let box_id = backend.box_info().await?.ok_or_else(|| StoreError::NotFound {
    entity: "box",
    id: "this box is not registered".to_owned(),
})?.box_id;
```

(`crates/htui/src/agent_worker.rs:344-351`, asserted at `:1234-1238`.) Note what that refusal
actually is: `CacheStore::box_info()` is `SELECT id, hostname, os_family FROM box ORDER BY id
LIMIT 1` over an empty mirror (`cache/read.rs:640-659`). It is a **read** failure, not a write
refusal — `backend.writer()` already answers `Some(Writer::Buffered(..))` offline
(`backend.rs:138-146`) and the code gets past it.

### 1.2 What the contract says, and where it breaks

Four `must` requirements block a writable local store as written, and they cross-reference each
other, so amending one alone leaves the contract self-contradictory:

- `R-STO-1` (`docs/REQUIREMENTS.md:111-113`): "Postgres is the only writable store."
- `R-STO-3` (`:115-117`): "Each box keeps a **read-only** cache …"
- `R-STO-4` (`:118-121`): "When Postgres is unreachable, the TUI opens in offline read-only mode
  from the cache … **No item creation, no runs.**"
- `R-ENT-7` (`:89-90`): "Item keys are minted online from a per-project, per-prefix sequence. Never
  reused. Offline creation is not supported (see R-STO-4)."

Two more fall with them: `R-ID-3` (`:31-32`, "Postgres is the single source of truth") and
`R-HIS-1` sentence 2 (`:195`, "Nothing about a run exists only on one box"). `CONCEPTS.md` restates
the same contract in three bullets, one of which — "Offline is read-only from a per-box cache; keys
are minted online, so no offline collision and **no sync engine**" (`CONCEPTS.md:25-26`) — is the
strongest single argument against the automatic-adoption answer to Q4, and ANA-9 §1 lists a
`sync_state` engine among the four things the current schema exists to retire (`docs/ANA-9.md:22-25`).

Two things temper that. First, `R-ID-2` (`:29-30`) makes `htui` "a local-first, developer-guided
harness", so the tension is internal to §1 of the requirements rather than between §1 and §4.
Second, the invariant has already been narrowed once, in code, with the reasoning recorded:

```
Since MOD-2 milestone 4 those two differ on [`Backend::Offline`] (plan D34): `writer()` answers
`Some(Writer::Buffered(..))` there, so there **is** an offline write path — and it reaches a file
under `<cache_dir>/pending/`, never the server. The invariant that survives is narrower and still
true: nothing writes to Postgres unless the backend is [`Backend::Online`], which is what
`writable()` still answers for.
```

(`crates/htui-store/src/backend.rs:11-17`.) This document extends a breach that is already open and
takes that narrowed sentence as the replacement wording for ANA-9 §2 invariant 1. It does not
pretend the tension is untouched.

### 1.3 What this document settles

The five questions the HANDOFF item names, as §4.1 to §4.5:

1. **Shape** — what a writable local store *is*, and where it attaches.
2. **Schema** — whether it reuses `cache_migrations/` or gets its own set.
3. **Ids** — who mints identifiers and item keys with no server.
4. **Transition** — what happens when such a box is later pointed at a server, and whether
   `local_only` is a lock or a preference.
5. **UX** — the first-run experience, the state vocabulary, and the seam into Settings.

Then §4.6 applies a consistency pass over the five verdicts, because the five were reached
independently and seven of their conclusions contradicted each other; §4.7 records the claims this
document deliberately does **not** assert. Two further subsections were added after the maintainer
answered §10's gating questions on 2026-09-08: §4.8 (runs on a local-only box) and §4.9 (in-app DSN
entry).

**What local-only is for, decided by the maintainer on 2026-09-08 and stated first because every
other decision reads differently under it.** Local-only is a **temporary, lite mode**. The full
product is `htui` against Postgres; a box with no DSN is where a user starts, tries the program, or
works until a server exists, and **the path to online usage must exist**. Two consequences run
through this whole document. Adoption (§4.4, §9.1's M7, MOD-18) is therefore **funded** — it is the
exit the mode promises, not a deferrable nicety — and **the first-run copy must not promise
migration before that path exists** (§4.5). And the parity of §4.8 is parity over **features**, not
over infrastructure: a local-only box does everything the product does — hierarchy, items, chat,
graph runs — while the server-shaped machinery is simply absent, because it is meaningless without a
server. There is no mirror to refresh, no per-table cursor, no `db_fingerprint`, no
`offline · <age>` state and no warm-cache startup budget (§10.4). That distinction is what "lite"
means here, and it is why the local store is authored against Postgres's *semantics* (§4.2) while
copying none of the mirror's *mechanism*.

Scope was fixed by the two consumers `HANDOFF.md` names — **MOD-13** (item editing) and **MOD-15**
(workspace, project, repo and kind management) — and by §10's question 8, which the maintainer has
since answered: **local-only covers graph runs, at parity with a server-backed box**. MOD-4 is
therefore a third consumer, §4.8 registers the answer, and `docs/ANA-2.md` §3's warrant clause, §4.9
and §8's placement column reopen as §6.3 anticipated. The maintainer also decided that entering a
DSN from inside the running program is a requirement of the item rather than an option, which is
§4.9.

---

## 2. Invariants

Restated from `CONCEPTS.md`, `docs/ANA-9.md` §2 and the code, split by what this document does to
each. Every one has a mechanical enforcement point named in §5 or §7.

### 2.1 Invariants this document preserves verbatim

1. **Nothing writes to Postgres unless the backend is `Online`.** `Backend::writable()` returns
   `Option<&PgStore>` — a *concrete* type, so no other store can be smuggled through it
   (`backend.rs:113-118`). Unchanged, and it is the type-level expression of what survives of
   `R-STO-1`.
2. **One cache writer, one direction.** "Only the refresh task. The offline chat buffer writes
   `pending/*.jsonl`, not `cache.sqlite`. User actions never touch the cache … That is the whole
   reason the mirror never diverges: it has one writer and one direction" (`docs/ANA-9.md:873-878`).
   Preserved **because** §4.1 puts the writable rows in a different file. This is the payoff of the
   separate-file verdict and `CONCEPTS.md:31-32` ("One cache writer") stays verbatim.
3. **No silent merge.** Every spec edit is a compare-and-set on `item.version`
   (`docs/ANA-9.md` §4.2). Ports to SQLite unchanged. Nothing in this document reconciles two
   versions of one row; the local store and the mirror hold **disjoint** row sets, never two copies
   of the same row.
4. **No agent in a bookkeeping path** (ANA-9 §2 invariant 5) and **logical identity never depends on
   a path** (invariant 6). Untouched.
5. **Keys are never reused.** ANA-9 §2 invariant 2's second clause. §4.3 preserves it mechanically
   rather than by policy; its first clause ("minted online") is amended.
6. **Nothing in the store crate runs on the UI task** (`R-NF-3`, `lib.rs:9-10`, `cache/mod.rs:8`).
   Binding on every write this document adds, including the `box.toml` marker, which is a blocking
   syscall issued from a key handler and therefore goes through the worker like everything else.
7. **No `DELETE FROM item`, ever.** "There is **no `DELETE FROM item`** in this file and none may be
   added" (`pg/write.rs:12-15`), pinned by the conformance case `no_delete_path`. The local store
   inherits it, which is why §4.4 refuses to make adoption a side effect of anything.

### 2.2 Invariants this document amends

| Invariant | Current text | This document |
|---|---|---|
| ANA-9 §2 invariant 1 (`:48-50`) | "**One writable store.** Only Postgres accepts writes. … Enforced by the store trait split: `ReadStore` for both backends, `WriteStore` implemented only by the Postgres backend." | Sentences 1, 2 and 4 amended. Replacement wording is already in-tree at `backend.rs:11-17`: nothing writes to Postgres unless the backend is `Online`. Sentence 3 ("The cache is written by the refresh task alone") is **unaffected**. |
| ANA-9 §2 invariant 2 (`:51-53`) | "**Keys are minted online and never reused.**" | Clause 1 amended to "minted by the store that owns the project"; clause 2 preserved and given a mechanical guarantee (§4.3). |
| ANA-9 §2 invariant 4 (`:56-58`) | "**Nothing about a run exists only on one box.** … the offline chat buffer is a queue in front of that table, not a second store." | Amended and **bounded in time**: on a box that has never been given a DSN, the local store *is* the only copy, and the TUI says so. The bound is "until the box adopts into a server", not a repeal. |
| ANA-9 §4.2 step 4 (`:187`) | "Offline mode never edits, so there is no queued write to reconcile (`R-STO-4`)." | Amended. True of `Backend::Offline`; false of `Backend::Local`. The compare-and-set mechanism itself (`:177`) is unaffected. |
| ANA-9 §6.1 (`:851-853`) | "`PgStore: WriteStore`. `CacheStore: ReadStore` only. … The TUI holds a `Backend` enum { Online(PgStore, CacheStore), Offline(CacheStore) } and every write path is unreachable in `Offline`, at the type level rather than by a runtime flag." | Enum shape amended (§5.2). "`CacheStore: ReadStore` only" stays **verbatim**; the final clause becomes "every write path to *Postgres* is unreachable outside `Online`". |
| ANA-9 §4.4 (`:309-311`) | "Not mirrored: revisions, skills, templates, graphs, `agent_box`, counters, settings, command queue - none is browsed offline." | List unchanged; the **warrant clause** lapses. Those tables are now *created locally* on a local-only box, not browsed offline. §6.3 tracks every sentence in that position. |
| ANA-9 §11 criterion 7 (`:1051-1052`) | "A chat session started offline lands in Postgres on reconnect with `seq` order preserved and the pending file removed …" | Restated. It describes `Backend::Offline` and continues to hold there. It does **not** describe the no-DSN case any more, because §4.6's conflict 2 moves local-only chat off `BufferedWriter` and into `local.sqlite`. |
| `CONCEPTS.md:20-21`, `:25-26` | "Postgres holds everything …"; "Offline is read-only from a per-box cache; keys are minted online, so no offline collision and no sync engine …" | Both need a one-or-two-sentence amendment at close-out, within the file's 6 144-byte cap. `:31-32` ("One cache writer") stays verbatim. |

---

## 3. Surface as read

Read from the working tree on 2026-09-08, branch `main`, HEAD `2257e87`. Facts below are the ones
the verdicts turn on; every other citation in this document was produced by the same read.

**The `Backend` seam.** Three arms today — `Memory(MemStore)`, `Online { pg, cache }`,
`Offline { cache, since }` — an enum and not a `Box<dyn ReadStore>` deliberately: "native `async fn`
in traits is not object safe, and a concrete type keeps the spawned worker's futures
`Send`-inferable" (`backend.rs:3-4`, `:38-58`). Dispatch is an exhaustive `match self` at 18 sites
in `backend.rs` alone and 18 more in `writer.rs`. Across the workspace there are 45
`Backend::{Memory,Online,Offline}` mentions in 11 files, all exhaustiveness-checked by rustc.
`Writer` is `{ Memory(MemStore), Online(PgStore), Buffered(BufferedWriter) }` with
`label() -> "memory" | "online" | "buffered"` (`writer.rs:44-66`).

**The trait surface.** `ReadStore` is seven `async fn`s (`items`, `item`, `links`, `documents`,
`notes`, `runs`, `step_events`) under `#[allow(async_fn_in_trait)]` (`traits.rs:27-45`).
`WriteStore: ReadStore` is nine (`mint_item`, `update_item`, `transition`, `append_events`,
`set_step_usage`, `upsert_agent`, `upsert_agent_box`, `start_chat_run`, `finish_chat_run`,
`traits.rs:57-152`) and ends at a comment:

```rust
    // links, notes, documents, skills, templates, box ...
}
```

`traits.rs:153` — verified. There is **no** `create_workspace`, `create_project` or
`create_item_kind` anywhere on the seam, on either backend. §5.6 addresses this directly.

**The conformance gate.** `CASES` is 20 named cases, written against `WriteStore` alone with no
concrete store named (`conformance.rs:23-43`, `run_case<S: WriteStore>` at `:56`, documented as
"Runs one case by name against an already-loaded store"). `CacheStore` implements `ReadStore` only,
so the suite structurally cannot reach it. Four of the twenty are mint cases;
`mint_writes_revision_v1` and `update_cas_diverged` require real `item_revision` rows, because
`UpdateOutcome::Diverged { head, ancestor: ItemRevision }` returns one (`traits.rs:181-191`).

**Identity and local state.** `box.toml` is the only file-based local persistence the product has:
`struct BoxToml { box_id: Uuid, hostname: String }` (`identity.rs:30-34`), minted as UUIDv7 on first
launch, written atomically temp-file-then-rename (`:104-124`), rewritten on a hostname change
(`:76-78`) and by `connect::try_connect` under the adopt-DB-id rule (`connect.rs:267-284`).
`BoxToml` derives `Deserialize` with **no** `#[serde(default)]`, and `load_or_mint` turns a parse
failure into `StoreError::Backend` (`:68-71`) which `connect::start` propagates before the terminal
opens. `app_setting` is Postgres-only, seeded with exactly two keys, and explicitly not mirrored —
so a `local_only` row there is unreadable in precisely the state it governs. `local_only` appears
nowhere in the tree except `HANDOFF.md`.

**Item keys.** `item.key_number INTEGER NOT NULL CHECK (key_number >= 1)`,
`key TEXT GENERATED ALWAYS AS (key_prefix || '-' || key_number::text) STORED`, and
`UNIQUE (project_id, key_prefix, key_number)` (`migrations/0001_init.sql:310-331`, verified).
`item_key_counter (project_id, prefix, last_value)` at `:299-304`. `item.created_by UUID NOT NULL
REFERENCES app_user(id)` at `:322`; `item_revision.author_id UUID NOT NULL REFERENCES app_user(id)`
at `:346` and `item_revision.box_id UUID REFERENCES box(id) ON DELETE SET NULL` at `:347`. Every
UUID primary key in the product is already minted client-side as UUIDv7: `id_newtype!` gives all 17
id types `::new() -> Self(Uuid::now_v7())` (`ids.rs:31-33`, `:77-112`), `NewItem.id` is caller-minted
(`item.rs:150-152`), and `PgStore::mint_item` binds it as `$1` rather than using the column default.
The only server-allocated value in the entire schema is `item.key_number`.

**CLI precedence.** `--set-dsn` and `--clear-dsn` carry `conflicts_with_all` against each other and
`--demo`; `--offline` carries **no** `conflicts_with` at all (`crates/htui/src/cli.rs:20-32`,
verified). `--demo` short-circuits before `connect::start` runs (`lib.rs:70-71`). `--set-dsn` reads
stdin and returns *before* the terminal is initialised, by a stated decision: "the DSN is typed into
a normal shell and never into a raw-mode terminal", with echo suppression explicitly declined
(`lib.rs:42-44`, `:109-129`).

**A test fact the consistency pass needed.** `offline_never_dials_and_starts_at_an_age`
(`crates/htui-store/tests/connect.rs:227-241`) passes
`dsn: Some("postgres://nobody:nothing@127.0.0.1:1/none")` with `offline: true` and asserts
`"offline · 0s"`. Its fixture **does** store a DSN, so §4.5's rule — `--offline` with a DSN keeps
`offline · <age>` — leaves that test green. Verified rather than assumed.

**SQLite capability floor.** `htui` statically links SQLite 3.51.3 (`libsqlite3-sys 0.37.0` via
sqlx's `sqlite-bundled`), so generated columns (3.31.0), the generalised UPSERT (3.35.0) and STRICT
tables (3.37.0) are guaranteed on every box with no floor to check. `ALTER TABLE … ALTER COLUMN
SET/DROP NOT NULL` needs 3.53.0 and is **not** available.

---

## 4. Settled questions

### 4.1 Q1 - The shape of a writable local store (`R-STO-1`, `R-STO-3`, `R-NF-3`)

**Need.** A store a local-only box can write, reachable through the existing seam, without making
the mirror writable and without stopping the process from ever dialling Postgres.

**Constraints that bind before any option is weighed.** `Backend::is_writable()` must stay `false`
for every non-`Online` arm: the store worker's re-dial ticker is guarded on
`reconnect.is_some() && !backend.is_writable() && held.is_none()` (`store_worker.rs:537`) and
`backend.rs:93-104` states that a `true` there stops the process dialling forever. `writable()` must
keep answering `None`; its concrete `Option<&PgStore>` return type is the type-level expression of
`R-STO-1`. Whatever the arm holds must be `Clone + Debug` and hold cheap handles only, because
`Backend` must stay a concrete enum for `Send`-inference.

**Options.**

| Option | Mechanism | Verdict |
|---|---|---|
| A. A fourth arm over the **same** `cache.sqlite` | `Backend::Local { cache: CacheStore, .. }`; local rows go into the mirrored tables, or into new tables inside the same file | **Rejected.** Genuinely cheapest in code and it has real merits: one file to back up, one pool, one set of hand-written encoders (`cache/read.rs:54-111`), no second Migrator, no second set of sqlx-dedicated OS threads, and `testkit::seed_mirror` already proves the six bootstrap tables accept hand-written rows with no server (`testkit.rs:237-257`). It is rejected on the deciding reason below. |
| B. A fourth arm over a **separate** file, backed by a new `LocalStore: WriteStore` | `Backend::Local { local: LocalStore }` over `<config_root>/local/local.sqlite`, its own forward-only migration set, `CacheStore` untouched | **Adopted.** |
| C. One store type subsumes `CacheStore` with a writable/read-only mode | Replace `CacheStore` with a single SQLite store carrying a mode flag | **Rejected.** Its pros are real — one SQLite read path instead of two, one file, and reads over local and mirrored rows unify for free with no merge layer ever needed. But it converts "an offline write is a compile error rather than a runtime flag" (`cache/mod.rs:3-6`, `:74`) into a runtime property, which is a larger move than narrowing it again; it inherits every con of A because the rows still live in the mirror file; it is structurally untestable, since `CASES` is `WriteStore`-shaped and would have to pass in one mode and fail to compile the trait in the other; and it is the largest diff of the four. |
| D. No new arm: widen `Writer` with a `Writer::Local` variant | `Backend::Offline` already answers `Some` to `writer()` and `false` to `is_writable()`; add a fourth `Writer` arm, exactly as `BufferedWriter` was added in MOD-2 milestone 4 | **Rejected.** A tenth of B's cost, zero `Backend` churn, and `BufferedWriter` is the exact working precedent for a store that implements `WriteStore`, delegates `ReadStore` to the mirror and refuses most writes (`writer.rs:67-88`, `:156-319`). Fatal on reads: `Backend`'s `ReadStore` impl and its six inherent reads all dispatch `Offline` to `cache` (`backend.rs:222-296`, `:304-360`), so a locally created workspace, project or item would be written and then **invisible** — the backlog, the switcher and the top bar would all still read the empty mirror. It also cannot express Q5's distinction, because keeping the `Offline` arm keeps the `offline · <age>` conflation, and it does not silence "this box is not registered", which is a read refusal. |

**Verdict.** Option B. A fourth `Backend::Local` arm over a separate file, backed by a new concrete
`LocalStore: WriteStore` at `<config_root>/local/local.sqlite`, outside `cache/<fingerprint>/`
entirely, with its own forward-only migration set. `CacheStore` and its file are untouched.

**Deciding reason, stated as a fact.** `cache.sqlite`'s contract is *delete me on mismatch*.
`CacheStore::open` deletes the file plus its `-wal`/`-shm`, recreates, re-migrates and writes a fresh
`cache_meta` whenever `cache_meta.schema_version` or `cache_meta.db_fingerprint` differs from the
arguments (`cache/mod.rs:104-113`, `:137-149`, `:261-279`); `schema_version` is the binary's own
maximum embedded Postgres migration version (`pg/mod.rs:387-389`) and is passed even with no DSN
(`connect.rs:187`); and the directory name changes from the literal `"offline"` to
`sha256(host:port/dbname)` the moment a DSN is set (`connect.rs:34-38`, `:184-187`). So under option
A or C, shipping any future `migrations/000N_*.sql` — a routine `htui` upgrade with nothing to do
with local-only mode — silently erases the only copy of the user's work, and configuring a DSN
silently orphans it. A file that is deleted by design cannot hold the only copy of user-authored
rows, and no carve-out makes that safe without inverting the rule the mirror exists to obey. That is
mechanical, not a preference.

Two supporting reasons rank the rest. A separate file removes writer contention with
`cache::refresh` entirely, so the 5 s `busy_timeout` (`cache/mod.rs:65`) can never stall a local
write behind a refresh pass and `R-NF-3` holds structurally rather than by scheduling luck. And
`LocalStore: WriteStore` can be held to the 20-case conformance suite at all, where a writable
`CacheStore` is the one shape the suite structurally cannot reach.

External evidence converges independently, which is worth recording because the in-tree argument is
about one file's lifecycle and the external one is about a category. Linear keeps its local
IndexedDB "a strict subset of the server database (the SSOT)" that "cannot contain unapproved
changes, since premature writes would make reverting server rejections error-prone", with pending
mutations in a separate `__transactions` table; PowerSync keeps a separate `ps_crud` queue;
ElectricSQL documents the one-table-two-origins shape (synced table + shadow table + combining view
+ `INSTEAD OF` triggers) as its **highest-complexity** pattern whose specific cost is "loss of
context when handling rollbacks". `htui` already reached the same answer in miniature, in a
comment: "`pending` is the offline chat buffer of §4.3, which lives next to the file rather than
inside it" (`cache/mod.rs:3-6`).

**Amended by the consistency pass (§4.6, conflicts 1, 2, 3 and 4).** The verdict as first reached
put `Backend::Local` behind the predicate `dsn.is_none()` and gave the other arms no `LocalStore`
handle. Both are changed here. The arm predicate is defined once, in §5.1, as `dsn.is_none()`, and
`--offline` is a no-op on a box with no DSN. All three non-`Memory` arms carry the local store
(§5.2): `Online` and `Offline` gain `local: Option<LocalStore>`, opened whenever
`<config_root>/local/local.sqlite` exists. Without that field, §4.4's "the local rows stay local"
would mean "the local rows become unreadable the next launch after a DSN is set" — the same silent
disappearance that already happens to `cache/offline/pending/*.jsonl`, dressed as a policy.

**What the arm answers.**

| Accessor | `Backend::Local` | Why |
|---|---|---|
| `label()` | `"local-only"`, with no age | §4.5. An age measured from `since: Some(Utc::now())` at start means nothing when no server was ever known. |
| `is_writable()` | `false` | It means "the server is reachable" (`backend.rs:93-106`), which is permanently and correctly false here. A `true` would disarm the re-dial ticker. |
| `writable()` | `None` | Concrete `&PgStore` return type; `R-STO-1` expressed in the type system. |
| `writer()` | `Some(Writer::Local(LocalStore))` | §5.2. |
| `cache()` | `None` | A local-only box does not open `cache/offline/` at all. |
| `local()` | `Some(&LocalStore)` | New accessor, `Some` on `Local` and on `Online`/`Offline` whenever the file exists. |

**Cost this prices, deliberately.** A second SQLite pool and a second set of sqlx-dedicated OS
threads — sqlx gives every SQLite connection its own thread (`sqlx-sqlite-0.9.0`
`connection/worker.rs:26-29`), so the local pool is sized 1-2, not the mirror's 4. A third migration
set on the forward-only rule (`R-STO-5`), which unlike `cache_migrations/` can never answer a
mismatch by deleting the file, so it must migrate data forward for the product's lifetime. And no
compile-time query checking: one crate has one `DATABASE_URL` and it is the Postgres one
(`cache/read.rs:18-19`; `sqlx.toml` is per-crate, and its only multi-database key is
`database-url-var`, whose own documented example is "break it up into multiple crates"). Every local
statement is untyped `sqlx::query().bind()` with hand-written encoders, roughly doubling the
unchecked SQL surface unless a second crate is split out — which is a bigger structural change than
the arm itself and is left as a maintainer question (§10.15).

**Implementation notes.**

- New module `crates/htui-store/src/local/{mod,read,write}.rs`, mirroring `CacheStore`'s shape but
  with **no** rebuild-on-mismatch rule, plus `crates/htui-store/local_migrations/0001_local.sql` and
  a third `pub static LOCAL_MIGRATOR` beside `MIGRATOR` (`lib.rs:36`) and `CACHE_MIGRATOR` (`:44`).
  Its doc comment must state the inverse of `CACHE_MIGRATOR`'s: this file is never rebuilt and
  always migrates its data forward.
- `LocalStore` implements all seven `ReadStore` and all nine `WriteStore` methods with the
  signatures verbatim from `traits.rs:30-45` and `:57-152`, even where several refuse.
  `BufferedWriter` (`writer.rs:186-319`) is the in-tree template for a partial implementor.
- Refusals construct `StoreError::ReadOnly(&'static str)` (`error.rs:23-25`), which is declared and
  **never constructed anywhere in the workspace** today. `BufferedWriter` uses
  `StoreError::Unreachable` (`writer.rs:322-331`), which callers read as "retry later may work"
  (`error.rs:26-32`) — the wrong reading for a refusal that is permanent until a server appears.
- Concurrency guard rails for the local pool: `max_connections(1..=2)`, WAL, a busy timeout,
  `foreign_keys` **ON** (the mirror's `foreign_keys(false)` at `cache/mod.rs:244-247` is justified by
  "the server already enforced every constraint this file would repeat", which is false locally),
  `BEGIN IMMEDIATE` for write transactions, and never an `.await` on non-DB work — an agent spawn, a
  network call — while holding a transaction. The chat-run write path is exactly where that rule
  would be broken.
- Blast radius, measured: 45 `Backend::` arm mentions across 11 files — `backend.rs` (19 match
  sites), `connect.rs` (6), `store_worker.rs` (5), `tests/connect.rs` (4), `writer.rs` (2),
  `crates/htui/src/testkit.rs` (2), `tests/cache.rs` (2), `tests/chat_offline.rs` (2),
  `tests/settings.rs` (1), `tests/writer_buffered.rs` (1), `agent_worker.rs` (1). All
  exhaustiveness-checked by rustc.
- `Backend::Local` answering `None` to `cache()` makes `go_online`'s "no mirror to go online over"
  arm (`store_worker.rs:565-568`) reachable in principle. It is unreachable today because
  `Started::reconnect` is `None` whenever there was no DSN at `start()` (`connect.rs:198-208`) and
  `--set-dsn` exits before the TUI. **When** M9 lands (§9.1, §10.13), `go_online` must learn to
  *open* a mirror rather than move one. It stays unreachable through M8, and M3 must not
  pre-emptively give `Backend::Local` a `CacheStore` to prepare for it — that would re-open every
  failure this section's separate-file verdict closes.

### 4.2 Q2 - Schema: reuse the mirror set or author a new one (`R-STO-3`, `R-STO-5`, `R-ENT-7`)

**Need.** A schema for the local store; a ruling on which mirror tables and columns are unusable for
locally created rows; whether `db_fingerprint` applies; how `refresh()` must behave when a table
holds both mirrored and local rows; how the forward-only rule of `R-STO-5` applies to a second
SQLite schema; and the maintenance cost of two schemas against one.

**Options.**

| Option | Mechanism | Verdict |
|---|---|---|
| A. Reuse `cache_migrations/` as-is | Local rows land in the 16 `MIRRORED_TABLES` | **Rejected.** Zero new DDL and the type mapping and encoders already exist and are tested (`0001_mirror.sql:5-12`; `cache/read.rs:54-111`; `refresh.rs:379-393`); the item/run half of the column shapes is adequate as written. Rejected because the file is disposable (§4.1's deciding reason), because four tables are `DELETE`-then-replaced whole on every pass, and because the mirror is a strict subset that has no `item_revision`, no `item_key_counter` and no `step_graph`. |
| C. Reuse the mirror tables plus an origin/sidecar column | `origin TEXT CHECK (origin IN ('server','local'))` per row, WatermelonDB's `_status`/`_changed` pattern, so `refresh()` can exclude local rows | **Rejected.** Genuinely attractive: one file, one pool, one migrator, one encoder set, reads merge for free with no `ATTACH` and no `UNION`, `box_info()`'s single-row assumption becomes a filter, adoption gets a natural work queue (`WHERE origin='local'`), and it is externally attested. Rejected for three reasons. It does not avoid a second schema, it adds one to the mirror — `item_revision`, `item_key_counter` and `step_graph` still have to be created and the dropped CHECKs and FKs still have to come back, for local rows only, which SQLite cannot express as a per-row constraint. The per-row upsert is built as `ON CONFLICT (<pk>) DO UPDATE SET <every non-pk column>` (`refresh.rs:355-376`): when a server row arrives with the primary key of an adopted local row, "update every column" is exactly wrong and there is no rollback context — the failure ElectricSQL names for its own equivalent pattern. And an origin column does nothing about the two whole-**file** destroyers (`cache/mod.rs:137-149`, `:164-193`), because the destruction is of the file, not of rows. |
| B. Its own forward-only migration set, authored **for SQLite** against the Postgres schema's semantics | `local_migrations/0001_local.sql`, a third Migrator, STRICT tables, real CHECKs, FKs on, `item.key` generated, `UNIQUE (project_id, key_prefix, key_number)`, `item_revision`, `item_key_counter`, a local meta table. `cache_migrations/` and `refresh.rs` untouched. | **Adopted.** |

**Verdict.** Option B, derived from `migrations/0001_init.sql`'s semantics rather than from
`cache_migrations/0001_mirror.sql`'s subset.

**Deciding reason, stated as a fact.** It is a lifecycle mismatch, not a column mismatch. The mirror
schema's defining property is that it is **disposable**, and the codebase says so and acts on it:
"a mismatch … is what rebuilds it (plan D8), so the mirror never has to migrate its own data
forward" (`lib.rs:39-44`), enforced by `CacheStore::open`'s delete-and-recreate (`cache/mod.rs:137-149`)
and by `CacheStore::rebuild()` emptying all sixteen `MIRRORED_TABLES` behind a shipped one-keystroke
button (`:164-193`, MOD-15's per `HANDOFF.md:251`). A primary writable store's defining property is
that it is **durable**. Those two lifecycles cannot be reconciled by adding a column, because the
destruction is of the whole file.

Three facts corroborate rather than decide. The mirror is a strict subset, so "reuse" is not reuse:
`0001_mirror.sql:24-25` lists revisions, counters and graphs among the deliberately absent, MOD-13
needs the first two and MOD-15 needs the third because `item_kind.default_graph_id` is `TEXT NOT
NULL` in the mirror (`0001_mirror.sql:70`) mirroring `UUID NOT NULL REFERENCES step_graph(id)`
(`0001_init.sql:288`) while `step_graph` has no mirror table at all. The integrity a writable store
needs was dropped on a premise local-only falsifies (`0001_mirror.sql:19-20`), and the pragma is set
once at connect time for the whole pool (`cache/mod.rs:247`), so FKs cannot be on for a local writer
and off for the refresher. And the conformance gate is written against `WriteStore` alone and cannot
reach `CacheStore`, so a distinct store type is the only shape the existing 20 cases can hold to
account.

**`refresh()`: exactly as it does today.** That is the answer to the sub-question, and it is the
point of the verdict. There are exactly six `DELETE` statements in `cache/refresh.rs` — four
whole-table replaces at `:413` (`app_user`), `:466` (`agent`), `:516` (`workspace`), `:552`
(`workspace_project`); the per-project `session_event` trim at `:1356`; and the `item_link`
tombstone-to-hard-delete at `:924` — plus `upsert_sql`'s blanket `DO UPDATE SET` at `:355-376` with
no column to exclude a local row by. Under option B none of them is touched, no origin predicate is
added anywhere, and ANA-9 §6.3 and `CONCEPTS.md:31-32` stay verbatim.

**`db_fingerprint`: it does not apply and must not be carried.** It is the lowercase hex sha256 of
`host:port/dbname` parsed out of a DSN (`identity.rs:126-144`), it is both the cache directory name
and a `cache_meta` key, and its value with no DSN is the literal `"offline"`. A local-only store has
no DSN by definition, so the field has no input. It is also an *endpoint* hash rather than a
database identity: it matches a dropped-and-recreated database at the same coordinates and differs
for the same database reached via another hostname. The local store carries a `local_meta` table
with its own `schema_version`, a minted `store_id` and `built_at`, and **no** `db_fingerprint` and
**no** `last_full_refresh_at` (§5.4). Its path must not be under `cache/<fingerprint>/`, whose name
is keyed to a server it does not have.

**Amended by the consistency pass (§4.6, conflict 5).** The verdict as first reached described
`store_id` as "the Fossil project-code / Postgres `system_identifier` analogue, which is what an
adoption gate should compare". That is withdrawn. A Fossil project code and a Postgres
`system_identifier` identify the **shared peer**, which is why comparing them detects "wrong
project"; a locally minted UUID identifies only the local file and detects nothing about a server.
`store_id` keeps a narrower, real job — pairing the local file with the `box.toml` that belongs to
it — and the adoption gate compares `adopted_into` instead (§5.7).

**`R-STO-5`: forward-only is inherited; rebuild-instead-of-migrate is not.** `CACHE_MIGRATOR` runs
at `CacheStore::open` with no server and no prompt, so "on connect after confirmation" is already
false of the second migrator. The third migrator runs without confirmation too, but can never answer
a mismatch by deleting the file, so it migrates data forward for the product's lifetime. That is the
one real recurring cost of this verdict: SQLite's `ALTER TABLE` supports only rename-table,
rename-column, add-column and drop-column, so every future constraint change on the local schema is
the documented 12-step create-copy-drop-rename procedure, and `ALTER COLUMN SET/DROP NOT NULL` needs
3.53.0 while `htui` bundles 3.51.3. Atomicity is not a concern: sqlx wraps each SQLite migration in
a transaction unless the file opens `-- no-transaction` (`sqlx-sqlite-0.9.0/src/migrate.rs:165-175`).

**Table by table, for the first-need set.** Scope is hierarchy + items + free-standing chat; the run
half is §5.3's Set A, added after §10.8 was answered.

| Table | Mirror shape usable? | Ruling |
|---|---|---|
| `app_user` | Yes | Local table. `DELETE FROM app_user` every pass (`refresh.rs:413`) rules out sharing. Must restate `R-USR-2`'s single-row rule, which Postgres enforces with `LOCK TABLE app_user IN SHARE ROW EXCLUSIVE MODE`; locally that is a fixed single-row `CHECK`. |
| `box` | Yes | Local table. Refresh only upserts `WHERE id = $1`, but `cache/read.rs:640-659` reads `ORDER BY id LIMIT 1` on a stated "at most one row" assumption, so a local row plus a later mirrored one would make the smaller UUID win arbitrarily. Seven `NOT NULL` columns (`os_family`, `os_version`, `arch`, `htui_version`, `registered_at`, `last_seen_at`, `updated_at`) have no local source; `box.toml` holds only `box_id` and `hostname` and the probe is MOD-7's. §5.3 fills them from what is knowable at launch. |
| `workspace`, `workspace_project`, `agent` | Yes | Local tables. `DELETE`-then-replace every pass (`:516`, `:552`, `:466`); no exception is possible. |
| `project`, `repo` | Yes | Local tables. Cursor-driven with no bulk delete, but a locally created project is outside the watched server scope, so its rows would be neither refreshed nor deleted — permanent orphans in a file the rebuild rule deletes anyway. `project` additionally carries `sealed_at` (§4.6, conflict 7). |
| `item_kind` | No | `default_graph_id` is `NOT NULL` and its referent has no mirror table. Local table, and it drags in `step_graph`. |
| `step_graph`, `step_graph_phase`, `prompt_template` | Absent | New local tables. ANA-9 §5.10 (`:820-824`) seeds, per new project, "the five `item_kind` rows with their default graphs and phases from the `R-ENT-6` table, and one `prompt_template` version 1 per phase name". They are included because a local graph run reads all three at stage 3 (§4.8), *and* because relaxing `item_kind.default_graph_id`'s `NOT NULL` locally would produce rows the server rejects at adoption — silent locally, loud later, which is the wrong way round. `phase_agent` and `capability_tag` are **required**: without `phase_agent` every seeded graph carries zero candidates and every run refuses at admission (`docs/ANA-2.md:2077`), and `capability_tag` is `R-BOX-3`'s vocabulary for the tags `R-ORCH-10` matches on (§5.3 Set A). |
| `item` | No | The sharpest case. Mirror has `key` as a plain `TEXT` column (`0001_mirror.sql:77`) where Postgres generates it (`0001_init.sql:316`); mirror has two plain non-unique indexes (`0001_mirror.sql:83-84`) where Postgres has `UNIQUE (project_id, key_prefix, key_number)` (`:330`); mirror has no `CHECK (key_number >= 1)` and no status `CHECK`. Local table with all three restored. |
| `item_revision` | Absent | New local table. `mint_writes_revision_v1` and `update_cas_diverged` both require real rows. |
| `item_key_counter` | Absent | New local table (§4.3's verdict; Q2 records only that the mirror cannot host it and the local schema can). Its absence from the mirror is the schema-level statement of `R-ENT-7`. |
| `item_link` | No | Mirror deliberately drops `deleted_at` (`0001_mirror.sql:16-18`) because a tombstone becomes a hard delete (`refresh.rs:924`), so a locally deleted link could not be represented for later upload. Local table with `deleted_at` restored. |
| `item_note`, `document` | Yes | The two cleanest. Copy the shapes as-is. |
| `run`, `run_step`, `run_step_commit`, `session_event` | Yes | Local tables. `session_event` is swept by the trim at `:1356` against a server-derived list, which rules out sharing. |
| local settings and flags | Absent | `app_setting` is Postgres-only and explicitly not mirrored, so it is unreadable in the state it governs. §5.4 gives the local store a `local_setting` table with `app_setting`'s shape, so MOD-15's Settings sections have somewhere to write on a local box rather than inventing a second location. |

**Maintenance cost, honestly stated.** Two schemas is the status quo, not a new burden: `Cargo.toml`
already pins sqlx 0.9 with both `postgres` and `sqlite`, and the repo already ships 32 Postgres
tables and a 17+1-table mirror. Vaultwarden — the mainstream Rust multi-backend precedent —
duplicates outright, with three generated schema files and three migration directories, and
documents cross-backend data migration as unsupported. No project surveyed runs one *portable*
schema across Postgres and SQLite with compile-time checking on both; every precedent either
duplicates or abandons dual support. The recurring cost of B is drift between `local_migrations/`
and `migrations/0001_init.sql`, caught **late** — at adoption, against Postgres, on the user's data —
with no compile-time query checking to help. §12 criterion 6 turns that into a test. The recurring
cost of A or C is larger and lands on a module that currently works.

**A shape the survey turned up and this document rejects on evidence rather than omission.** Embed
Postgres itself: `postgresql_embedded` (real Postgres binaries as a child process, ~30-52 MB of
dependencies, runtime download by default) or `pglite-rs` (a single-process Postgres fork linked
in-process). Both keep one schema and all 86 compile-time-checked queries. Rejected: `pglite-rs` is
0.2.x with two reverse dependencies, its prebuilt Windows targets are `windows-gnu` while `htui`
targets msvc, and its `socket` feature is unix-only; `postgresql_embedded` turns a TUI into a
process supervisor for a database server on a box whose whole premise is that it has no server.

### 4.3 Q3 - Who mints ids offline (`R-ENT-5`, `R-ENT-6`, `R-ENT-7`, `R-ENT-10`)

**Need.** Three sub-questions. (a) UUID-keyed rows: generalise the client-side UUIDv7 mint or not.
(b) Human-readable per-project per-prefix item keys under `R-ENT-7`. (c) Whether MOD-2's
`upload_pending` adoption path is the precedent to generalise or reject. Plus provenance columns and
what a local-only box may create on day one.

**The question is narrower than it looks.** Row identity is *already* client-minted for every entity
in the product: `id_newtype!` gives all 17 id types `::new() -> Self(Uuid::now_v7())`
(`ids.rs:31-33`, `:77-112`), `NewItem.id` is caller-supplied (`item.rs:150-152`),
`PgStore::mint_item` binds it as `$1` rather than using the column default, and the 18
`DEFAULT gen_random_uuid()` columns are dead fallbacks. ANA-9 §7.1's "Postgres-side mint" is
therefore narrower than its name: the only server-allocated value in the entire schema is
`item.key_number`. `ChatRunSpec::mint` is not an exception to be generalised — it is one instance of
the standing rule. So (a) is **generalise**, and it is barely a generalisation.

That leaves one integer per `(project, prefix)`. `R-ENT-7` reduces to it entirely.

**Options for the key.**

| Option | Mechanism | Verdict |
|---|---|---|
| 1. Refuse offline item creation | Local-only creates hierarchy and chat but no items; `mint_item` refuses locally | **Rejected**, but it is the maintainer's to reinstate (§10.3). It needs no `R-ENT` amendment at all, has the largest external precedent (Jira's keys are strictly server-assigned, the REST API ignores a client-supplied `key`, and cross-instance sync never preserves them), and carries zero collision risk. Rejected because it leaves the stated problem 80% unsolved — `HANDOFF.md:60-61` frames the defect as "the user sees nothing and can create nothing" — and strands MOD-13, whose `new` action would simply remain unavailable. It also cannot pass four of the twenty conformance cases, so it needs a case-subset escape the suite does not have. |
| 2. Deferred key (Linear / Todoist split) | Mint the row offline with no key; the server assigns `key_number` at adoption and returns a mapping | **Rejected.** The best-attested external shape: Linear treats the human-readable `number` as a mutable server-assigned display label beside a client-minted UUID, and Todoist's `temp_id` → `temp_id_mapping` does exactly this. It preserves `R-ENT-7`'s first two clauses verbatim and removes the collision hazard rather than managing it. Rejected because an item with no key is unusable on a box that never connects — which is the exact population the mode exists for: the backlog key column renders blank, key search misses, `{{item_key}}` renders empty. It also needs a Postgres forward migration making `key_number` nullable (today `INTEGER NOT NULL CHECK (key_number >= 1)` with `key` generated from it), i.e. a change to the primary store to serve a secondary mode, and it turns `Item.key`/`key_number` into `Option` across htui-core, the backlog list and render, the detail body, the link graph and MemStore's search and sort. Todoist's own `INVALID_TEMPID` state — "the object exists only locally" — is where a never-connected box would live permanently. |
| 3. Provisional local key, renumbered on adoption | Mint `FEAT-1`, `FEAT-2` locally; the adoption pass rewrites `key_number` against the server's counter | **Rejected, decisively.** The user sees a normal key from birth with no model change, and GitLab shipped the closest analogue (letting the importer set `iid` explicitly, MR !3759). It breaks ANA-5 invariant 2: `{{item_key}}` is `project.slug + ':' + item.key` and is a prompt input (`docs/ANA-5.md:323`), and the invariant is that the prompt is a pure function of its inputs with `prompt_digest` reproducible for the same item version (`:128-133`). Renumbering changes the prompt bytes with no version bump, so a digest recorded before adoption can never be reproduced after it. It also rewrites history that is already durable in `session_event` — a transcript naming `FEAT-7` keeps naming it after the row becomes `FEAT-23`, and there is no scrub pass for transcripts — and ANA-9 §4.1 forbids it outright ("No `UPDATE` of `key_prefix`/`key_number`", pinned by `update_kind_keeps_key_and_project`). GitLab's own defect class is the named cost: stale references become wrong-but-valid rather than erroring, and adopting rows without fast-forwarding the counter produces 409s and 500s indefinitely (gitlab#519457). |
| 4. Per-box prefix, stride, or leased block (hi/lo) | Give each box a disjoint slice of the number space | **Rejected.** Keys are real from birth and never renumbered, so ANA-5's digest and the transcripts are safe, and both mechanisms are automated in production elsewhere (MySQL/Galera's `auto_increment_increment` + `auto_increment_offset`; hi/lo in Hibernate). Fatal for the leased variant: the lease comes from a server, and a box that has never had a DSN has never had a lease — the option cannot serve the population it is for. Per-box prefix breaks `R-ENT-5`'s stated shape and the generated expression `key_prefix \|\| '-' \|\| key_number::text`, because the prefix would have to encode the box and the kind would no longer own it. Stride needs the number of boxes known up front and protects `INSERT`s only. Both destroy the chronology users read into `PROJ-1 < PROJ-2`, which the ticketing literature names as the fatal cost, and hi/lo's interoperability drawback is decisive: every other writer must know the scheme, and Postgres is a writer that does not. |
| 5. Project-origin-scoped local counter | The local store carries its own `item_key_counter` with the identical §4.1 rule, and may mint **only** into projects that originate in the local store | **Adopted.** |

**Verdict.** Option 5, with (a) generalise and (c) generalise-the-contract-reject-the-code.

**Deciding reason, stated as a fact.** `R-ENT-7`'s hazard is not "a key was minted without a
server". It is "two writers minted into one `(project, prefix)` counter". A project created on a box
with no server configured **exists on no server**, so its counter has exactly one writer by
construction for the whole time the project is local. "Never reused" is preserved mechanically
rather than by policy. Option 5 is consequently the only one of the five that leaves `R-ENT-5`'s
`<PREFIX>-<N>` shape, the `Item` model, the generated `key` column, all four mint conformance cases
and ANA-5's `prompt_digest` reproducibility untouched.

The mechanism is already written twice and proven. `MemStore` reimplements the exact §4.1 counter
rule in process with a `HashMap<(ProjectId, String), i32>` (`mem.rs:73-74`, `:548-575`), including
the comment "a §4.1 key number a refused mint consumed is never given back". And SQLite can
reproduce the key semantics exactly, since `htui` bundles 3.51.3 and STORED generated columns landed
in 3.31.0, so `key TEXT GENERATED ALWAYS AS (key_prefix || '-' || key_number) STORED` is available
locally where the mirror had to downgrade it to a plain copied column.

**Amended by the consistency pass (§4.6, conflicts 2, 6 and 7), and this is the most consequential
set of amendments in the document.**

1. **The origin rule is vacuous until the coexistence field lands.** As first reached, this verdict
   rested its collision-freedom entirely on "legal only in locally-originated projects, refused in
   mirrored ones" — but under Q1's original shape there was no state in which the rule could fire:
   `Backend::Local` has no mirror and therefore no mirrored projects, while `Online` and `Offline`
   had no `LocalStore` and therefore no local mint. The safety property that decides Q3 was
   unreachable code. §5.2 fixes it by giving `Online` and `Offline` a `local: Option<LocalStore>`
   field, at which point the rule is trivially implementable because local and mirrored projects
   live in physically different files. **Until that field exists (M3) and a local mint exists (M6),
   the rule has no case to fire on, and the implementation MOD must not cite it as a shipped safety
   property before both.** It is a precondition on coexistence, not a property of the mint.
2. **Adoption seals the project.** The one-writer argument holds only while the project is local. If
   a box may still mint locally into a project it has already adopted, two counters serve one
   `(project_id, prefix)` and `UNIQUE (project_id, key_prefix, key_number)` (`0001_init.sql:330`)
   rejects the row at the *next* adoption — long after the user created it, with no
   `DELETE FROM item` available to clean it up. So adoption must seal the local project **in the
   same transaction** that fast-forwards `item_key_counter`: a `sealed_at` column on the local
   `project` table (§5.3) plus a mint guard, enforced in the schema rather than by policy. This also
   answers "can a box go back to local-only": not for an already-adopted project.
3. **"Chat already works via `BufferedWriter`" is withdrawn.** The verdict as first reached asserted
   that free-standing chat "already works through `ChatRunSpec::mint` and `BufferedWriter` once the
   box and `app_user` rows exist". It does not, in the adopted arm shape.
   `BufferedWriter::new(cache: CacheStore)` takes its directory from `cache.dir()`
   (`writer.rs:90-110`) and `Backend::writer()` constructs `Writer::Buffered` only from the
   `Offline { cache, .. }` arm (`backend.rs:138-146`). `Backend::Local` deliberately holds no
   `CacheStore` and opens no `cache/offline/` directory, so `Writer::Buffered` is **unconstructible**
   there. Local-only chat writes `run`, `run_step` and `session_event` rows straight into
   `local.sqlite` through `Writer::Local` — all three are local tables per §4.2 — and
   `Writer::label()` gains a fourth string that rides the existing
   `StoreReply::ChatAccepted.writer_label` channel (`store_worker.rs:209-216`). The consequence is
   recorded in §2.2 and §6.2: ANA-9 §11 criterion 7 no longer describes the no-DSN case.

**(c) `upload_pending`: reject the code, generalise the contract.** The code is irreducibly
chat-specific — the line format is `session_event` columns only, `run` and `run_step` are
synthesised at upload from the file name and the events, the identity foreign keys
(`target_box_id`, `executing_box_id`, `started_by`) are supplied at adoption from the connected
server as arguments, and `usage`/`prompt_digest` are recomputed. None of that generalises to an
item. Four contract rules do, and are lifted verbatim into the adoption pass of §9's M7:

1. Adoption is **id-preserving** and never remaps. The ids are UUIDv7 and cannot collide, which is
   why WatermelonDB #216's orphaning failure mode — "if the server ignores it and generates its own,
   the local record will never receive updates from the server" — does not apply here.
2. Idempotence comes from `ON CONFLICT (id) DO NOTHING` on client-minted primary keys
   (`pending.rs:507`, `:543`, `:562`), so the pass is re-runnable end to end.
3. Whichever path lands first owns the columns, and the resulting asymmetry is **documented rather
   than merged** (`run.rs:185-198`, pinned by `chat_run_rows_converge_with_the_offline_mint` and
   `an_offline_first_chat_keeps_the_uploaded_columns`).
4. A row the server refuses with `StoreError::Constraint` is quarantined, not retried, and never
   fails the connection (`pending.rs:331-346`).

**One rule the item path must not inherit.** `pending.rs` quarantines a poisoned buffer with a
`warn!` to a log file the user never sees, forever. That is tolerable for a chat buffer and
intolerable for a user-authored item, because `pg/write.rs:12-15` forbids any `DELETE FROM item`
(pinned by `no_delete_path`), so a refused item is **unremovable as well as invisible**. A refused
adoption must surface in the UI with a per-row status (§12 criterion 12).

**Provenance columns. Amended by the consistency pass (§4.7, claim 1): this is not free.** The
verdict as first reached priced Postgres-side provenance at zero, on the grounds that
`item_revision.box_id` already exists and is bound at mint. That is half true and the half it misses
is the expensive half. Verified against `migrations/0001_init.sql`: `item.created_by UUID NOT NULL
REFERENCES app_user(id)` (`:322`) and `item_revision.author_id UUID NOT NULL REFERENCES
app_user(id)` (`:346`), with `item_revision.box_id UUID REFERENCES box(id) ON DELETE SET NULL`
(`:347`). A locally minted `app_user` cannot be inserted server-side, because `seed_if_empty_as`
takes `LOCK TABLE app_user IN SHARE ROW EXCLUSIVE MODE`, inserts only `WHERE NOT EXISTS` and returns
the **oldest** row's id, not the one matching a name (`pg/mod.rs:230-333`). So adoption must remap
**both** author columns to the server's existing user — an authorship rewrite — and `box_id` points
at a local `box` row whose server-side id is decided by `register_box`'s adopt rule. The original
local authorship is preserved by **no existing column**, and recording it needs a new forward-only
`migrations/000N_*.sql`, because `0001_init.sql` may never be edited (its checksum is compared on
every connect and drift is a hard refusal). The correct statement is therefore: **no Postgres
migration for the mint; a Postgres migration for provenance if the maintainer wants local authorship
preserved** (§10.7).

On the local store's own tables the columns are free, because the schema is new (§5.3):
`origin_box_id` `NOT NULL` — the local store has no server to ask for `this_box`/`this_user`, unlike
`upload_pending` which receives both as arguments; `adopted_at` per row and `sealed_at` per project,
making "which rows still need adopting" a query rather than a directory listing; and a monotonic
`local_seq` so the adoption pass has a deterministic parent-before-child order across tables without
relying on cross-table UUIDv7 timestamp ordering.

**What local-only may create on day one.** One `app_user` (subject to §10.7); one `box` row from
`box.toml`'s already-minted `BoxId`, which is what silences the "this box is not registered" refusal
at `agent_worker.rs:344-351`; `workspace`, `workspace_project`, `project`, `repo`, `item_kind` with
`R-ENT-6`'s seeded kind set and ANA-9 §5.10's graphs, phases and templates; `item` +
`item_revision` v1 + `item_note` + `document` + `item_link` within a locally-originated project,
with real keys; and free-standing chat (`run`, `run_step`, `session_event`) through `Writer::Local`.

**What it may not.** Create an item in a *mirrored* project while a known server is unreachable —
`R-STO-4` unchanged, and this is exactly the rule §4.6's conflict 6 makes real. Run a graph against a
*mirrored* project, or against any project while a DSN is configured (§4.8's placement table) — a
graph against a locally-originated project on a `Backend::Local` box is permitted, and ANA-2 §8's
placement rule keeps its split while its `PgStore`-inherent reads gain a `LocalStore` twin and a
fourth `Backend` arm (§7.1). Delete anything (`no_delete_path`). Or link a local item to a mirrored one: `item_link`
foreign-keys both endpoints to `item(id)` (`0001_init.sql:358-359`) and the two rows live in
different stores, so cross-store links are refused on day one.

**Implementation notes.**

- The local mint is **three statements inside one `BEGIN IMMEDIATE` transaction**, not one CTE.
  SQLite has no data-modifying CTEs and `RETURNING` cannot be used as a subquery, so ANA-9 §7.1's
  single statement does not port. Atomicity is preserved; the single-statement guarantee is not.
  Order must preserve §4.1's rule that a refused mint burns no number: validate the
  kind-belongs-to-project guard **before** the counter upsert, which is the SQLite equivalent of
  folding the guard into the CTE's `SELECT` (`pg/write.rs:57-72`; `mem.rs:562-573` is the closest
  existing model). §7.3 gives the three statements.
- Local `item` needs `UNIQUE (project_id, key_prefix, key_number)`; the mirror's two item indexes
  are plain and non-unique, which is right for a mirror and wrong for a store that mints.
- The counter's adoption statement already exists and is already tested:
  `ON CONFLICT (project_id, prefix) DO UPDATE SET last_value = GREATEST(item_key_counter.last_value, $n)`
  with an explicit `key_number` (ANA-9 §7.1's importer variant), pinned by
  `the_importer_variant_keeps_the_counter_above_max` (`tests/pg_criteria.rs:242-292`) but **not in
  the product** — `item.rs:145-148` reserves the explicit-`key_number` variant to MOD-8. Two
  consumers now want that one statement; whichever lands second owns the join, and the seam is a
  `WriteStore` method that does not exist yet.
- `item_revision` is not optional in the local schema: `mint_writes_revision_v1` and
  `update_cas_diverged` both require real rows.
- Two costs of UUIDv7 stand and are worth recording because nothing else in the tree does: RFC 9562
  §6.11's format leaks creation time, and §8 says UUIDs MUST NOT be used as security capabilities.
  Neither bears on `htui`'s use, but a later feature that treats an id as an unguessable handle would
  be wrong. There is no case for changing format: RFC 9562 §6.4 rejects a central id registry as a
  bottleneck, and two independent Postgres benchmarks measure v7 as *cheaper* than v4 as a primary
  key (≈35-49% faster inserts, ≈22% smaller index at 10M rows), with the caveats that neither
  instruments WAL or page splits and neither benchmarks a bigint baseline.

### 4.4 Q4 - The transition to a server (`R-STO-1`, `R-STO-4`, `R-HIS-1`, `R-USR-2`)

**Need.** What happens when a box holding local rows is later pointed at a server; whether
`local_only` is a lock or a preference the first successful connection clears; and where it lives
with no server to hold `app_setting` rows.

**One sub-question dissolves before the options.** "Refuse on a fingerprint mismatch" cannot be
implemented as phrased. The local store is a different file in a different directory from any
server's mirror, so there is never a mismatched `cache_meta` to compare: the mismatch is a different
*path*, not a different *value*. (Stated precisely, because the imprecise version of this sentence
would contradict §4.1's deciding reason — see §4.7, claim 6. The check at `cache/mod.rs:137-149`
fires perfectly well, for the mirror, whenever its fingerprint or schema version changes; that is
the shipped behaviour every other verdict in this document leans on. What never fires is a
comparison against a *local* store, which has no fingerprint at all.) A refusal on identity grounds
therefore has to be a new, invented gate — which §5.7 supplies — not the existing check.

**Options.**

| Option | Mechanism | Verdict |
|---|---|---|
| A. Auto-adopt on the first successful connection | Generalise `upload_pending`: at the end of the first successful refresh pass after `go_online`, push the local rows into Postgres id-preservingly. `local_only` is a preference the connection clears. | **Rejected.** The precedent is shipped and works, row identity is already client-minted everywhere, `upload_pending` already splits "the row's own id" from "which server-owned rows it points at", and first-link-uploads is what users expect (Zotero). Rejected on three grounds. It re-introduces exactly the thing `CONCEPTS.md:25-26` says does not exist ("no offline collision and **no sync engine**") and ANA-9 §1 lists `sync_state` among the four elements the schema exists to retire. The ordering inside the pass defeats it anyway: the refresher `DELETE`s-then-replaces `app_user`, `agent`, `workspace` and `workspace_project` (`refresh.rs:413`, `:466`, `:516`, `:552`) and `upload_pending` runs only *afterwards* (`:299-300`), so an automatic uploader arrives after the local rows in those four tables are already gone. And the destination would be whatever DSN happens to be in the keyring at that launch, chosen outside the UI entirely, for an operation that `no_delete_path` makes irreversible. |
| B. Keep local, permanently: the first launch's choice is irreversible | A box that has created local rows is a local box for life | **Rejected as a stated policy, but it is what ships until M7.** It is exactly what the code does today at zero cost, it needs no sync engine, no key collision story, no `app_user` remap and no rollback, and Gitea's own tracker argues honestly for this position ("Database conversion is a nightmare"; the docs should say the initial DB choice is effectively permanent). Rejected as a *decision* because it makes an irreversible product choice at first launch, in the exact state where the user has no information — and because `HANDOFF.md` binds MOD-13 and MOD-15 to this verdict, so under B a user who evaluates `htui` locally and then stands up Postgres loses every item and workspace those two features exist to deliver. |
| C. Refuse to connect at all while local rows exist | Fossil's project-code refusal and git 2.9's "refusing to merge unrelated histories", applied to the whole connection | **Rejected.** The strongest guard against silent duplication and it makes the transition impossible to miss. It holds the whole product hostage to an adoption tool that does not exist: until that tool ships, a box with one local item could not use a server at all, contradicting `R-STO-3` and `R-STO-4`'s promise of a usable connected box. Fossil's own users are the evidence against it: `Error: wrong project`, zero artefacts exchanged, and only folklore remedies. |
| D. Keep local by default; adoption is an explicit, confirmed, one-shot, server-bound operation; a second or different server is refused | Pointing a local box at a server opens the server's mirror as usual and leaves the local store readable beside it. Adoption is never a side effect of connecting. | **Adopted.** |

**Verdict.** Option D — with the framing of §1.3 applied: "keep local by default" is the **interim**
state of a mode users are expected to leave, not a destination. The mechanism is unchanged (adoption
is explicit, confirmed, one-shot and server-bound; nothing is uploaded as a side effect of
connecting), but the intent is not "these rows live here now" — it is "these rows live here until you
say move them". Adoption is funded as MOD-18 (§10.5), and the copy rule of §4.5 follows: the product
may not promise the move before the move exists.

**Deciding reason, stated as a fact.** Adoption is irreversible in a way nothing else in the store
is: "There is **no `DELETE FROM item`** in this file and none may be added" (`pg/write.rs:12-15`),
pinned by the conformance case `no_delete_path`. An operation whose mistakes cannot be undone must
not be a side effect of setting a DSN — which today happens outside the TUI entirely (`lib.rs:61-63`,
`:109-129`) and which §4.9 brings inside it. The reasoning is unchanged and its conclusion is
unchanged: adoption is an explicit, confirmed action, and moving DSN entry into the TUI must not be
read as moving adoption with it.

**Is `local_only` a lock or a preference? Neither. It is not a mode flag at all.** This is the
consistency pass's fourth amendment (§4.6, conflict 4) and it changes the verdict as first reached,
which made `local_only` a lock in `box.toml`.

- It **cannot** be "a preference the first successful connection clears", for two independent
  reasons. Mechanically, that connection is not observable in the process that would clear it:
  `Started::reconnect` is `None` for the whole process whenever no DSN was present at `start()`
  (`connect.rs:198-208`), the re-dial ticker is guarded on `reconnect.is_some()`
  (`store_worker.rs:537`), and `--set-dsn` exits before the TUI, so a local-only process can never
  reach a successful connection. The transition **is** a restart boundary in the first MOD, by
  choice rather than by necessity: of the three seams an in-process transition touches, one is paid
  by §4.9 anyway and one is a factory function; only `go_online`'s inability to open a mirror it does
  not have (`store_worker.rs:572-575`) is genuine cost, and it is deferred to M9 (§10.13).
  Semantically,
  "the first successful connection" is the exact moment the local directory stops being opened, so
  clearing a flag there would *automate* the silent stranding that already happens to
  `cache/offline/pending/*.jsonl`.
- It **need not** be a lock, because the thing a lock would guard — automatic adoption — does not
  exist under this verdict. Adoption is already an explicit, confirmed, named action. Keeping a lock
  would introduce a fourth state ("this box has a DSN but must stay local") that nothing in the
  HANDOFF item asks for, that §4.5's state vocabulary has no slot for, and that would force the arm
  predicate to become `local_only || dsn.is_none()`.

So the arm predicate is purely `dsn.is_none()` (§5.1), and what actually needs persisting is not a
mode but three **facts**: `adopted_into` (has this local store been given to a server, and which
one), `local_store_id` (does this `box.toml` belong to the local file beside it), and
`local_notice_shown` (has the first-run overlay been seen). All three live in `box.toml` (§5.5). An
explicit "stay local even though a DSN exists" preference is a separate feature and is offered to
the maintainer as §10.9, with a recommendation against.

**Why `box.toml` and not the alternatives.** `app_setting` is a Postgres table and is explicitly not
mirrored (`0001_mirror.sql:24-25`; `MIRRORED_TABLES` at `cache/mod.rs:39-56`), so the HANDOFF
sketch — `local_only` as an `app_setting` row defaulting to false — is unreadable in precisely the
state it governs. `cache_meta` sits inside the file `CacheStore::open` deletes on any
`schema_version` or `db_fingerprint` drift, where `schema_version` is the binary's own embedded
Postgres migrator maximum applied even with no DSN, so a routine `htui` upgrade would erase the
flag. `local_meta` inside `local.sqlite` is durable but circular for the `local_store_id` pairing
check and unavailable before the local store is opened. `box.toml` is atomic temp-file-then-rename
(`identity.rs:104-124`), is touched by no rebuild path, and already holds a client-minted UUIDv7 the
server later adopts under the adopt-DB-id rule — the exact precedent for local state that yields to
a server on first contact.

**What "keep local" actually means, mechanically.** Amended by the consistency pass (§4.6, conflict
3): it means the local store stays **open and readable** beside the mirror, not merely undeleted on
disk. `Backend::Online` and `Backend::Offline` carry `local: Option<LocalStore>` (§5.2), opened
whenever the file exists. Reads are never unioned inside a query — ANA-9 §2 invariant 3 forbids
silent merges — but `workspaces()` concatenates the two sources with a per-row origin tag, and
everything below a workspace dispatches to the store that holds it. That is a concatenation of
**disjoint** sets, not a merge: no row exists in both files, because a local workspace has no server
and a mirrored one is not in the local file. Without this field the verdict would be option B
wearing option D's clothes.

**The refusal, and its escape hatch.** A second adoption into a second server would *succeed* if
nothing stopped it — every key is a client-minted UUID and would insert cleanly — so duplication has
to be prevented by policy, which is the Zotero failure ("two libraries into one account produces
duplicates") and the Syncthing union surprise. The rule is git 2.9's: refuse by default, with one
explicit named opt-in and no configuration variable that turns it on permanently. The value compared
is `adopted_into` against `SELECT system_identifier FROM pg_control_system()` (§5.7), which is the
same value Postgres itself uses to refuse a standby against the wrong primary
("`FATAL: database system identifier differs between the primary and standby`"). Nothing in the tree
reads it today — ripgrep confirms zero occurrences — so it is new work, and whether to add it to the
connect handshake is §10.10. Fossil is the warning attached to this: everyone implements the
refusal, and then users need a documented way out. Ours is the adoption action itself plus the
export path of §9's M3.

**Adoption is deferred, and the deferral is visible.** M7 is a separately funded item with its own
`HANDOFF.md` id, minted at close-out. That is deliberate. The single most consistent finding of the
external survey is that an unfunded promotion path makes the first store choice permanent in
practice: Grafana still has no official SQLite→Postgres route and third-party scripts are pinned to
specific versions (an issue confirmed still failing in 2025); Gitea's tracker argues the docs should
say the choice is permanent; only Synapse, which built a resumable, re-runnable `synapse_port_db`
with documented anti-footguns, made it routine. If the maintainer will not fund M7, this document's
answer degrades to option B — and then §4.5's overlay copy must say so at the moment the user
creates their first local row, rather than implying a promotion path that does not exist. That is
§10.5, and it is the decision the external evidence is most emphatic about.

**Implementation notes for the adoption item (out of scope for the implementation MOD).**

- The transaction shape to copy is `upload_pending` (`pending.rs:286-360`): one transaction per
  unit, local artefact retired only after commit, `StoreError::Constraint` quarantined rather than
  propagated. Two things it must add: the unit is a **project subtree**, not a file, because items
  foreign-key to `project` and `item_kind`; and a **queryable per-row status**, because today a
  refusal is a `tracing::warn!` that never reaches the UI.
- Identity remap: reuse `upload_pending`'s split — the row's own id is offline UUIDv7, but
  `this_box`/`this_user` come from the connected server (`pending.rs:311-316`,
  `refresh.rs:52-55`). The box remap has a working precedent in the adopt-DB-id rule
  (`pg/mod.rs:335-375` + `connect.rs:267-284`); the user remap does not, and collides with
  `seed_if_empty_as`'s oldest-row-wins rule. Resolve foreign keys through the **same** map used for
  primary keys, or local rows are orphaned (WatermelonDB #216; Oracle Data Pump's `REMAP_DATA`
  applies one remap function across both columns of a referential constraint for exactly this
  reason).
- The counter fast-forward must run in the **same transaction** as the row inserts (§4.3), and the
  same transaction must set `sealed_at` on the local project.
- Side effects — notifications, agent launches, worktree creation — belong in an explicit
  post-commit phase so a retry cannot re-fire them.
- A pre-adoption copy of the local file is new work: nothing in the tree copies a SQLite file before
  a destructive operation. `remove_file_set` deletes outright and `rebuild` empties tables.
  Obsidian and Anki both tell users to take that copy manually; §10.11 asks whether it is mandatory.
- No `migrations/000N_*.sql` is needed for the *verdict*; one is needed if §10.7 says local
  authorship must be preserved.

**What this verdict does not fix.** Local rows written under `cache/offline/pending/` by boxes that
have already run offline are untouched. Nothing in the tree ever enumerates `<root>/cache/*` — there
are only three `join("cache")` sites — so those buffers remain stranded exactly as they are today.
The new arm prevents the class prospectively, for boxes that adopt this design; it recovers nothing.
See §4.7, claim 4, and §10.12.

### 4.5 Q5 - The first-run experience (`R-TUI-1`, `R-TUI-8`, `R-NF-3`, `R-STO-6`)

**Need.** The overlay's trigger condition and its once-only persistence with no `app_setting` to
write to; the copy split between "no server configured" and the existing `offline · <age>`; what the
top bar shows; the seam into the Settings DSN section; how the user leaves local-only mode;
`R-NF-3` compliance for every write the flow performs; and what MOD-13 and MOD-15 must not ship
ahead of this.

**Options.**

| Option | Mechanism | Verdict |
|---|---|---|
| A. Status only: a fifth top-bar label plus one line of switcher copy | No overlay, no persistence, so the once-only question never arises | **Rejected.** It is the only option needing no new local persistence at all, has zero `R-NF-3` exposure because it writes nothing, edits the screen a no-DSN first run already shows, and fixes the real test-pinned confusion. Rejected because it does not satisfy the HANDOFF item, gives the user no in-product route to a DSN — the remedy stays `htui --set-dsn` folklore, which is the exact "remedy is folklore" failure Fossil demonstrates — and cannot express a two-outcome choice later. |
| B. Modal first-run overlay, triggered by an explicit no-DSN signal out of `connect::start`, marked shown-once in `box.toml` | New `Started`/reply field, `local_only_overlay` slot, dismissal emits a request the worker writes through `identity::store` | **Adopted.** |
| C. Same overlay, marker in `cache_meta` | A fifth key beside `schema_version`, `db_fingerprint`, `built_at`, `last_full_refresh_at` | **Rejected.** `cache_meta` is already a typed key/value table needing no serde or file-format change, and the file is already opened by the worker. Rejected because the marker is destroyed by events that have nothing to do with it: `CacheStore::open` deletes the file whenever `schema_version` differs, and that version is the binary's own embedded Postgres migrator maximum, so shipping any future `migrations/000N_*.sql` would re-open a first-run notice on a months-old box. "Once" would mean "once per mirror generation". It also puts a UI preference behind `Settings > Rebuild cache`, and it forces the first non-refresher production write into `cache.sqlite`, which pre-empts §4.1 and §4.2. |
| D. Overlay with the session-only guard, no persistence | Copy `migration_prompt_shown` verbatim | **Rejected.** It copies a shipped, tested pattern exactly — that bool exists because `StoreState` is re-read every fourth tick, so without it an answered `n` reopens the prompt a second later, forever (`app/state.rs:180-184`, `app/update.rs:227-245`) — and it is defensible on its merits, since the state it announces is permanent for the process. Rejected because it directly contradicts "shown once rather than every start", and a modal on every launch of a box the user deliberately runs local-only is what users learn to dismiss without reading. |
| E. Self-clearing trigger: show while local-only **and** the local store is still empty | Make the trigger a fact about the store rather than a remembered event, in the spirit of Syncthing's `.stfolder` marker | **Rejected, and it is the genuine rejection.** It is the more elegant design: no persistence site is needed and yet "once" is real rather than per-session, the condition cannot go stale or be silently erased, and `no_delete_path` makes it monotone so it cannot flip back. Rejected because its trigger only clears if there *is* a writable local store, which makes Q5 undecidable ahead of Q1 — and this document's own HANDOFF entry names Q5 as the part MOD-15 must join against, so it has to be settleable on its own. It also does not clear for the user who reads the notice, goes off to run `--set-dsn`, decides against a server and comes back; and "empty" has no single definition across the hierarchy. |

**Verdict.** Option B.

**Deciding reason, stated as a fact.** Durability of the marker against events that have nothing to
do with it. Every other local persistence site in the product is destroyed by routine, unrelated
operations: `cache.sqlite` is deleted whole whenever `cache_meta.schema_version` differs, and that
version is the binary's own embedded Postgres migrator maximum (`pg/mod.rs:387-389`) applied even
with no DSN (`connect.rs:187`); the no-DSN mirror directory is the literal `cache/offline/` and
changes name the moment a DSN is set; and `CacheStore::rebuild()` deletes every mirrored table.
`box.toml` is touched by none of them. Decisively for sequencing, a marker there needs no writable
local store to exist, so this verdict is implementable whatever §4.1 decided.

**Five sub-verdicts.**

**(1) Trigger.** `dsn.is_none()`, evaluated inside `connect::start` and carried out explicitly. Not
"no `box` row" — `CacheStore::box_info()` returns `None` for any unsynced mirror and is the agent
worker's chat-refusal cause, not a first-run signal. Not "empty database" — equally true of a
configured server with an empty database, and already claimed by the switcher. It must be carried
explicitly because `connect::start` is the only code that sees `dsn.is_some()` and it discards it:
`let connecting = !opts.offline && dsn.is_some();` (`connect.rs:189`) collapses "no DSN" and
"`--offline` with a DSN" into one indistinguishable `Started`. `reconnect.is_none()` is **not** a
usable proxy.

*Amended by the consistency pass (§4.6, conflict 1).* The trigger as first reached was
`dsn.is_none() && !opts.offline`, which contradicted §4.1's arm predicate of `dsn.is_none()` and
would have given the same launch two different arms and two different top-bar words, because
`--offline` carries no `conflicts_with` and `--offline` with no stored DSN is reachable
(`cli.rs:20-32`, verified). The `&& !opts.offline` is dropped. A box with no DSN has nothing to be
offline *from*, so `--offline` is a no-op there. §5.1 states the full precedence.

**(2) Top bar.** A fifth `Backend::label()` output, `local-only`, with **no age**. The age in
`offline · <age>` is measured from `since: Some(Utc::now())` set at start (`connect.rs:191`), which
for a never-configured box measures nothing but how long the process has been up. `--offline` **with
a DSN** keeps `offline · <age>`, because a DSN exists and the server is merely not being dialled —
and that keeps `offline_never_dials_and_starts_at_an_age` green, since its fixture does store a DSN
(`tests/connect.rs:227-241`, verified in §3). Only the no-DSN label changes. Rendering costs one
match arm and zero render changes, because `top_bar.rs:34-43` renders `state.store` verbatim as one
span.

The fact must travel in a reply, not be inferred in a view. The precedent is explicit in-tree:
`StoreReply::ChatAccepted.writer_label` exists because "the tab must be able to say that a
conversation is only on this disk, and it cannot ask: `R-NF-3` keeps every store handle on the
worker's side, and inferring 'offline therefore buffered' from the top bar would be a guess about a
backend the tab does not hold" (`store_worker.rs:209-216`). The same rule applies here: no view may
derive local-only from the `offline · <age>` string. `StoreReply::StoreState` gains the field
(§7.2).

**(3) The handoff seam into Settings.** The missing primitive is a **section-focus action**.
`TabAction::Focus(TabId)` exists (`crates/htui/src/app/action.rs:59`) but nothing addresses a
`SectionId` within a tab; `SettingsRegistry` exposes only `cycle_next`/`cycle_prev`
(`settings/mod.rs:197-208`), which consume `h`/`l`/`[`/`]`/arrows before the active section sees
them. This document fixes the two names so that whichever of the implementation MOD and MOD-15 lands
second implements the other half against them, rather than inventing a second mechanism:

- `SectionId("connection")` — the stable identity of the Postgres-connection section.
  `SectionId(pub &'static str)` is `Copy` (`settings/mod.rs:29-30`).
- `TabAction::FocusSection(TabId, SectionId)` — the action the overlay emits.
  `TabAction` derives `Copy` and `SectionId` is `Copy`, so the variant does not change the enum's
  derives.

The overlay emits `Action::Tab(TabAction::FocusSection(SettingsTab::ID, SectionId("connection")))`
followed by `Action::Overlay(OverlayAction::Close)`, exactly as `MigrationPrompt` emits its pair
(`migration_prompt.rs:90-96`).

**(4) Leaving local-only remains a restart boundary — a chosen one, not a necessary one, and the
copy must not claim otherwise.** Three facts make the restart the cheap answer:
`Started::reconnect` is `None` for the whole process when no DSN was stored (`connect.rs:198-208`)
and the ticker is guarded on `reconnect.is_some()`; the `Reconnect` closure captures the DSN at
`start()` **by design**, "so the store worker never has to hold a DSN: `R-STO-1` keeps the secret in
the keyring and in the connect path, not in the shell" (`connect.rs:66-70`); and the mirror
directory is chosen once from the fingerprint at `start()`. §10.13 now prices those three seams
individually and finds only one expensive — `go_online` cannot open a mirror it does not have
(`store_worker.rs:572-575`) — so the restart is a **deferral to §9.1's M9**, not a property of the
design.

The security paragraph that stood here is **withdrawn and replaced by §4.9**, because the maintainer
has decided that entering a DSN inside the running program is the objective of the item. Its factual
content survives — `--set-dsn` does read stdin before raw mode with echo suppression explicitly
declined (`lib.rs:42-44`, `:109-129`), and `Composer` *is* documented as a mode rather than a widget
(`chat/composer.rs:3-4`) — but its conclusion does not, and one of its premises was wrong: the mode
pattern is not blocked by single-letter bindings. `ChatTab` already solves exactly that by consulting
the composer **first** (`chat/mod.rs:398-402`). What blocks it is `SettingsTab::on_key` consuming
`h`/`l`/`[`/`]`/Left/Right **before** delegating to the active section (`settings/mod.rs:197-213`),
which a default-bodied `captures_input` predicate fixes in five lines. See §4.9(2).

**(5) `R-NF-3`.** Every write in the flow — the `box.toml` marker, and the keyring write if a DSN
field is ever approved — is a `StoreRequest` handled on the spawned worker and emitted from the view
through `Ctx::emit`, exactly as `MigrationPrompt` emits `ApplyMigrations`. No view holds a store
handle or a channel (`lib.rs:6-9`, `app/state.rs:39-124`), so this is structural rather than
conventional. Keyring calls are blocking FFI and belong in `spawn_blocking` on the worker. Neither
write is network I/O, which is why it is worth saying: `R-NF-3` binds them because both are blocking
syscalls issued from a key handler.

**The overlay itself.** Copy `crates/htui/src/ui/overlay/migration_prompt.rs` end to end:
`is_modal() = true` so unhandled keys are swallowed (`app/state.rs:375-377`); state fetched via
`wants_requests -> vec![StoreRequest::StoreState]`, because the factory is
`Fn() -> Box<dyn Overlay>` and takes no arguments (`overlay/registry.rs:53`) so an overlay cannot be
constructed with the fact that triggered it; effects out through `ctx.emit`; never blank, with a
`loaded == false` arm. A new `local_only_overlay: Option<OverlayId>` slot beside `migration_overlay`
plus a `local_only_notice_shown: bool` session guard mirrors the shipped shape — the session guard
is required *in addition to* the persistent marker, because `StoreState` is re-read every fourth
tick and is explicitly not gated on a non-empty scope (`app/update.rs:74-87`, test at `:533-544`),
which is the exact defect `migration_prompt_shown` was added to prevent.

**The overlay slot contest, decided.** `App` has exactly one `startup_overlay` slot
(`app/state.rs:167`), already held by the workspace switcher (`app/mod.rs:61`), and there is no
priority mechanism. **The notice replaces the switcher on a local-only first run**, rather than
stacking on it. Stacking would leave `"no workspaces — creating one arrives with MOD-15"` visible
underneath as a second, differently worded first-run message on the same screen. MOD-15 must be told
which, and this is it.

**The copy rule, from §1.3's framing.** Local-only is temporary, so the copy says so — and says
nothing it cannot yet deliver. Until MOD-18 ships, the overlay states that work created here is
stored on this disk and that configuring a server does **not** move it; once MOD-18 ships, the same
copy gains the move. **The implementation MOD must not ship the second wording ahead of the second
item** (§10.5). A first-run notice that promises a migration the binary cannot perform is the exact
failure the external survey found in Grafana and Gitea, and it is worse than saying nothing, because
the user makes a filing decision on the strength of it.

**The copy split.** Three states, three sentences, and the difference is stated rather than implied:

| State | Label | What the user is told |
|---|---|---|
| No server configured | `local-only` | This box has no Postgres server. Work created here is stored on this disk only. **Press `s` to enter a connection string now — it goes straight into the OS keyring and is never written to a file. Or dismiss this and set it later under Settings › Connection.** The connection takes effect the next time `htui` starts. (`htui --set-dsn` moves out of this copy and into the connection section's help line, where a scripted or headless user looks for it: a first-run notice carrying two mechanisms teaches neither.) |
| Server known, unreachable | `offline · <age>` | Unchanged. The mirror is read-only, browsing works, item creation and runs do not, and a free-standing chat is buffered and lands on the next successful connection. |
| `--offline` with a DSN | `offline · <age>` | Unchanged. Deliberately identical to the above: a DSN exists and the box has simply been told not to dial. §10.15 asks whether it deserves a word of its own. |

**Amended by the consistency pass.** The copy is a downstream dependency of §4.3 and §4.4, not a
free variable, and the second sentence of the local-only text is contingent on §10.5: if the
adoption item is not funded, the notice must say plainly, at the moment the user creates their first
local row, that the work will stay on this disk. Promising a promotion path that is not funded is
the Grafana/Gitea outcome, made by omission.

**Is the first-run seed a write on startup?** No — and it is worth deciding explicitly because the
overlay copy cannot be drafted without it. Opening `htui` on a virgin box **creates and migrates**
`local/local.sqlite`, which is the same thing that already happens to `cache/offline/cache.sqlite`
at every launch, so it is not new behaviour. It writes **no rows**. The bootstrap rows of §9's M4 —
one `app_user`, one `box`, and the workspace/project/kind chain — are written on the first user
action that needs them (starting a chat, or creating a workspace), never at launch. A read-only
inspection of a local-only box does not mutate it beyond creating an empty schema.

### 4.6 The consistency pass

The five verdicts were reached independently and seven of their conclusions contradicted each other.
Each is applied above, under the affected question; this table is the index, so that no reader has
to reconstruct which verdict text is current.

| # | Between | The contradiction | Resolution applied |
|---|---|---|---|
| 1 | §4.1 arm predicate vs §4.5 trigger | `--offline` carries no `conflicts_with` (`cli.rs:20-32`, verified), so `--offline` with **no** stored DSN is reachable. §4.1 put that box in `Backend::Local` (predicate `dsn.is_none()`) and would label it `local-only`; §4.5 excluded it (`dsn.is_none() && !opts.offline`) and kept `offline · <age>`. The same launch got two arms and two words. | The predicate is `dsn.is_none()` **alone**, defined once in §5.1 and referenced everywhere. `&& !opts.offline` is dropped from §4.5's trigger. A box with no DSN has nothing to be offline from, so `--offline` is a no-op there. §5.1 gives the full precedence table including `--demo`, which is decided even earlier (`lib.rs:70-71`). |
| 2 | §4.1 (`Backend::Local` holds no `CacheStore`) vs §4.3 ("chat already works via `BufferedWriter`") | `BufferedWriter::new(cache: CacheStore)` copies its directory from `cache.dir()` (`writer.rs:90-110`) and `Backend::writer()` builds `Writer::Buffered` only from the `Offline { cache, .. }` arm (`backend.rs:138-146`). `Writer::Buffered` is unconstructible in the arm §4.3 assumed it ran in. | §4.3's "already works" claim is **withdrawn**. Local-only chat writes `run`/`run_step`/`session_event` into `local.sqlite` through `Writer::Local`, and `Writer::label()` gains a fourth string on the existing `writer_label` channel. Consequence recorded in §2.2 and §6.2: ANA-9 §11 criterion 7 no longer describes the no-DSN case. |
| 3 | §4.4 ("keep local by default") vs §4.1 (no arm outside `Local` carries a `LocalStore`) | Both verdicts parked "does `Online` gain a `local` field" in Unresolved. As specified, the first launch after a DSN is set made every local row unreadable — the silent-disappearance failure already documented for `cache/offline/pending/*.jsonl`, dressed as a policy. | Promoted out of Unresolved into a decision taken here. `Online` and `Offline` carry `local: Option<LocalStore>` (§5.2), opened whenever the file exists; `workspaces()` concatenates two disjoint sources with an origin tag and everything below a workspace dispatches to the store that holds it. If the maintainer will not pay for this, §4.4's verdict is option B and §4.5's copy must say so (§10.5). |
| 4 | §4.4 (`local_only` as a lock) vs §4.5 (state vocabulary) vs §4.1 (predicate "`dsn.is_none()` plus whatever Q4 rules") | A lock exists precisely for "this box has a DSN but must stay local". §4.5's vocabulary had no slot for it and §4.1 left the predicate open, so the one state the lock was designed to express was unrenderable and its arm undefined. | The lock is **downgraded out of existence**. `local_only` is not a mode flag; `box.toml` records three facts instead (§5.5), the predicate stays `dsn.is_none()`, and the fourth state disappears. An explicit "stay local despite a DSN" preference is offered separately as §10.9, recommended against. |
| 5 | §4.2 (`store_id` as the adoption gate's comparand) vs §4.4 (gate compares `system_identifier`) | Two identity mechanisms in two files, neither verdict acknowledging the other, answering different questions. §4.2's framing was wrong on its face: a locally minted UUID proves nothing about the server on the other end. | Both kept, named separately, jobs stated (§5.7). `store_id` answers "is this the same local store the `box.toml` belongs to". `adopted_into` answers "have these rows already been given to a server, and is this that server". `db_fingerprint` is used for **neither**. |
| 6 | §4.3 (mint legal only in locally-originated projects) vs §4.1 (no arm sees both kinds) | §4.3's collision-freedom rested entirely on the origin rule, but under §4.1's original shape there was no state in which it could fire. The safety property that decided Q3 was unreachable code. | The rule becomes real once §5.2's `local` field exists (M3) and a local mint exists (M6); local and mirrored projects then live in physically different files, so it is trivially implementable. **Until both, the rule is vacuous and the implementation MOD must not cite it as a shipped safety property.** It is carried forward as a precondition on coexistence, not as a property of the mint. |
| 7 | §4.3 (one counter writer per project, forever) vs §4.4 (adoption is allowed) | §4.4 adopts a project into a server but never retired the local counter. If the box can still mint locally into an adopted project, two counters serve one `(project_id, prefix)` and `UNIQUE (project_id, key_prefix, key_number)` (`0001_init.sql:330`) rejects the row at the *next* adoption — with no `DELETE FROM item` to clean it up. | Adoption **seals** the local project in the same transaction that fast-forwards `item_key_counter`: `sealed_at` on the local `project` table (§5.3) plus a mint guard, enforced in the schema. This also answers §4.4's open "can a box go back to local-only": not for an already-adopted project. |

### 4.7 Claims this document does not assert

Eight assertions appeared in the underlying verdicts that the evidence does not support, or supports
only in a narrower form. Each is either removed from the text above or carried with an explicit
flag; none is silently retained.

| # | Claim | Disposition |
|---|---|---|
| 1 | "Postgres-side provenance: none required — `item_revision.box_id` already exists, so revision 1 of an adopted item already names the authoring box." | **Corrected in §4.3.** `item.created_by` and `item_revision.author_id` are both `NOT NULL REFERENCES app_user(id)` (`0001_init.sql:326`, `:346`), and a locally minted `app_user` cannot be inserted server-side under `R-USR-2`'s oldest-row-wins rule, so adoption must remap both — an authorship rewrite. Original local authorship is preserved by no existing column and recording it needs a new forward-only migration. The verdict is restated as "no migration for the mint; a migration for provenance if §10.7 says so". |
| 2 | "`R-STO-6` pins warm-cache startup under one second, so a second SQLite file open is a cost against it." | **Flagged as not binding, and unmeasured.** `R-STO-6` verbatim (`:123-124`): "Startup with a warm cache **and reachable Postgres** is under one second on the reference workstation." It does not bind a launch with no server. Whether the maintainer *wants* the budget extended to the local-only path is a decision, not a given (§10.4). The measurement itself has not been taken by anyone and is §12 criterion 13. |
| 3 | "`store_id` is the Fossil project-code / Postgres `system_identifier` analogue, which is what an adoption gate should compare." | **Removed** from §4.2 and replaced. Both of those values identify the **shared peer**, which is why comparing them detects "wrong project"; a locally minted UUID identifies only the local file. The gate compares `adopted_into` (§5.7). Note also that `system_identifier`/`pg_control_system` appear **zero** times in `crates/` or `docs/` today, so both mechanisms are entirely new work. |
| 4 | "A `Backend::Local` arm that holds no `CacheStore` removes the stranded `cache/offline/pending/*.jsonl` class." | **Narrowed to "prospectively".** Boxes that have already run offline hold those files, nothing in the tree enumerates `<root>/cache/*`, and the new arm never opens the directory — so existing stranded buffers are untouched. §4.4 and §10.12 carry it. |
| 5 | "Option 5 costs no Postgres migration." | **Narrowed** to "no Postgres migration **for the mint**". The adoption path needs the `GREATEST` importer statement, which exists only in a test (`tests/pg_criteria.rs:242-292`) and is reserved to MOD-8 by `item.rs:145-148`; and any provenance or remap record needs a new `000N_*.sql`, because `0001_init.sql` may never be edited. |
| 6 | "The existing check at `cache/mod.rs:137-149` never fires." | **Reworded in §4.4.** As written it contradicts §4.1's and §4.2's deciding reason. That check fires perfectly well for the mirror whenever the fingerprint or schema version changes — that is the shipped rebuild behaviour this whole document leans on. What never fires is a comparison against a *local* store, which has no fingerprint. |
| 7 | "Keeping `--offline` on the old string keeps `tests/connect.rs:241` green." | **Verified true**, rather than assumed. `offline_never_dials_and_starts_at_an_age` (`tests/connect.rs:227-241`) passes `dsn: Some("postgres://nobody:nothing@127.0.0.1:1/none")` with `offline: true`. Its fixture stores a DSN, so §4.5's rule leaves it green. |
| 8 | "`LocalStore: WriteStore` joins the 20-case conformance suite **for free**." | **Corrected.** `run_case<S: WriteStore>` is documented as running "one case by name against an already-loaded store" (`conformance.rs:47`, `:56`), and the fixture the twenty cases assert against is the demo dataset — which a local store must be able to **construct**, i.e. it needs exactly the hierarchy-creation methods that do not exist on `WriteStore` (`traits.rs:153`). It joins `CASES` at the cost of one store-specific loader, modelled on the existing harnesses. §5.6 addresses the underlying gap. |

### 4.8 Runs on a local-only box (`R-ORCH-1..11`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`)

**Status.** Not a question this document weighed options for. §10.8 asked the maintainer whether
local-only covers graph runs; the answer, on 2026-09-08, is **yes, at parity with a server-backed
box**. This subsection records what that means mechanically, in the register of the five settled
questions, because the answer touches every one of them.

**What a run is, locally.** Exactly what it is on a server: a `run(kind='graph')` row, its
`run_step` rows keyed `UNIQUE (run_id, position, attempt, fanout_index)`, its `session_event` rows,
its `document` versions, its `run_step_commit` rows per repo and — once MOD-4 authors it — its
`run_step_tree` rows. All of them in `local.sqlite`, written through `Writer::Local`, driven by
`htui-orch`'s engine generic over `S: WriteStore`. The graph the run executes is
`run.graph_snapshot`, resolved at insert from the local `step_graph` / `step_graph_phase` /
`phase_agent` rows that ANA-9 §5.10's seed already writes per project (§5.3).

**Why this is affordable, stated first because it is the deciding fact.** `docs/ANA-2.md` §2
invariant 10 (`:143-146`) reads "the orchestrator crate depend[s] on `htui-core`, not on
`htui-store`". `LocalStore` lives in `htui-store` (§8). So `htui-orch` cannot name `LocalStore`, does
not need to, and never learns that a third store exists: it is generic over `WriteStore` exactly as
`FakeOrchestrator` is already generic over `MemStore` (`docs/ANA-2.md:1764-1771`). Every one of the
18 write methods of `docs/ANA-2.md:1739-1755` is therefore a per-store implementation cost and
**zero** orchestrator cost. Invariant 10 is preserved verbatim by this answer, not amended by it.

**What survives from ANA-2 unchanged on SQLite.**

| ANA-2 mechanism | Where | Port |
|---|---|---|
| Compare-and-set on status: `UPDATE … WHERE id = ? AND status = ? RETURNING *` (invariant 1, `:103-108`) | `run`, `run_step`, `item` | **Verbatim.** `RETURNING` in a top-level `UPDATE` landed in SQLite 3.35.0 and `htui` bundles 3.51.3 (§3). §7.3's caveat is only that `RETURNING` cannot be a *subquery*; here it is not one. |
| `graph_snapshot` is the executing copy, never the live tables (invariant 2, `:109-113`) | `run.graph_snapshot` | **Verbatim**, as `TEXT` under §5.3's `JSONB → TEXT` mapping. The `topology` sha256 refuse-to-resume rule (`:1478-1482`) is store-agnostic. |
| The review loop at the **same** `position` with `attempt + 1` (§4.4, `:700-703`) | `run_step` | **Verbatim.** It is pure row shape; the unique key that makes it work already exists at `0001_init.sql:494`. |
| The judge as a real step at **`fanout_index = -1`** (§4.5, `:814-820`) | `run_step` | **Verbatim, and the local schema must actively preserve the absence of a constraint.** `run_step.fanout_index` has no `CHECK` at `0001_init.sql:477`, and it must have none locally either. ANA-2 risk 12 (`:2075`) warns a later `CHECK (fanout_index >= 0)` would break every judged run; on SQLite that mistake is **irreversible without the 12-step table rebuild** (§11 risk 5). §5.3 states the prohibition. |
| Loser exclusion in `input_kinds` resolution: `s.selected IS NOT FALSE`, `ORDER BY (s.run_id = $run) DESC NULLS LAST` (§4.2, `:397-406`) | `document` ⋈ `run_step` | **Verbatim.** SQLite has supported `NULLS FIRST/LAST` since 3.30.0. |
| Order-of-allocation for `document.version` inside the insert transaction (§4.2, `:373`) | `document` | **Verbatim**, under `BEGIN IMMEDIATE` (§7.1) instead of a row lock. |
| The four isolation modes, the scratch root outside every `repo_box_path`, `before_hash`/`after_hash` capture, winner reconciliation, the git backoff (§4.6) | filesystem + git | **Untouched.** None of it is a database concern. `R-ID-4` and invariant 4 hold identically. |
| The six-stage step lifecycle, gate table, settle outcome, `GateOutcome` mapping (§4.2) | orchestrator | **Untouched.** |
| The two-order judge call, the verification prefilter, judge-failure-is-not-run-failure (§4.5) | orchestrator | **Untouched.** |
| Replay of a step (`R-HIS-2`, ANA-9 §7.5 `:968-975`) | `session_event` | **Verbatim.** ANA-9 already asserts the query text is identical against SQLite. §6.1 already records `R-HIS-2` as unaffected. |

**What does not survive unchanged, and what replaces it.**

| ANA-2 mechanism | Why it does not port | Local replacement |
|---|---|---|
| `shared_serialized`'s **Postgres advisory lock keyed `(box_id, repo_id)`**, held for the whole step (§4.6, `:916`) | SQLite has no advisory locks. | An in-process lock keyed `(box_id, repo_id)`, and it is **only sound because of §7.1's advisory single-writer file lock** `local/local.lock`: one writer process means one orchestrator, so an in-process lock is exactly as strong as the Postgres one was. **This is the sharpest dependency of runs-at-parity on an unanswered question: §10.17 must be answered before local `shared_serialized` can ship.** If the maintainer picks §10.17(b) or (c) instead, `shared_serialized` must be refused locally with a named error rather than silently unserialised. |
| `SELECT settings FROM box WHERE id = $box FOR UPDATE` as the admission critical section (§4.7, `:1096`) | SQLite has no `FOR UPDATE`. | `BEGIN IMMEDIATE`, which is §7.1's standing rule anyway. It takes the write lock on the whole file, which is *stronger* than a row lock and correct here because the local `box` table holds one row. **Free.** |
| `run.repo_scope UUID[]` queried with `&&` over a GIN index (§4.7, `:1023`, `docs/ANA-2.md:1900`, `:1912`) | No array type, no `&&`, no GIN. | `repo_scope` as a JSON array in `TEXT` under §5.3's mapping; the overlap predicate of `:1060-1072` evaluated in Rust over the non-terminal runs, or through `json_each`. Decidable and cheap at local scale — but it is a **second implementation of one safety predicate**, and that is where two implementations drift. §11 risk 23. |
| `i.required_tags <@ (b.probed_tags \|\| b.declared_tags)` (`R-ORCH-10`, ANA-9 §7.4 `:958`) and the `EXCEPT unnest` refusal message (`:966`) | Postgres array containment. | A Rust-side subset test over two decoded JSON arrays. Same answer, different code, same drift hazard. |
| ANA-9 §7.4's `ready_items` in full (`:952-963`) | `= ANY($projects)` plus the above. | Rewritten, not reused. This is MOD-12's, not MOD-4's, and it is why `R-ORCH-6`'s auto mode is **not** part of the parity answer's first cut (§9.1's M8). |
| `GREATEST(a, b)` in the counter fast-forward (§7.3) | Not a SQLite function. | `MAX(a, b)` in its scalar form. Already a known local-port item. |

**The lease, on a box that is the only box (`docs/ANA-2.md` §4.9, `:1276-1291`).** Its two jobs
separate, and only one of them is vacuous.

- Its **cross-box** job — "which box owns this run" — is vacuous. `run.lease_box_id`,
  `run.target_box_id` and `run.executing_box_id` all resolve to the one local `box` row.
  `R-ORCH-12`'s "executes only when it is the local box" is satisfied by construction.
- Its **cross-process** job is *not* vacuous but is **no longer the primary guard**. `lease_owner`
  "is minted per orchestrator process at start, so a second `htui` on the same box cannot silently
  adopt the first's runs" (`:1277-1278`), and ANA-2 risk 13 (`:2076`) is exactly that race. §7.1's
  file lock closes it earlier and harder: the second process cannot write at all. The lease is
  belt-and-braces here.
- Its **crash-recovery** job is fully live and is why the columns and the sweep are kept verbatim.
  A killed `htui` leaves an expired lease and the recovery sweep of `:1284-1299` adjudicates every
  `running` step by the artefact test — `after_hash` present for every repo in scope **and** a
  document of `output_kind` — exactly as on a server. That test has nothing to do with the number
  of boxes, and dropping the lease locally would cost a local box the one thing that makes an
  interrupted graph resumable.

**Verdict: keep `lease_box_id`, `lease_owner`, `lease_expires_at`, the TTL refresh and the sweep,
unchanged, on the local schema.** They are cheap, and the failure mode of omitting them is a run
that is permanently `running` with no owner and no way to adjudicate it.

**`box_info()` on a local store, and a defect this answer forces into the open.** Every run row
foreign-keys `target_box_id` and `executing_box_id` into `box` (`0001_init.sql:455-456`), so the
box row must be resolvable at run-start with no ambiguity. `CacheStore::box_info()` is
`SELECT … FROM box ORDER BY id LIMIT 1` on a stated at-most-one-row assumption
(`cache/read.rs:640-659`), and §4.2 already flagged that a local row plus a later mirrored one makes
the smaller UUID win arbitrarily. Runs make that intolerable rather than merely untidy.
**`LocalStore::box_info()` selects by `box.toml`'s `BoxId`, never by `ORDER BY id LIMIT 1`**, and the
local `box` table is deliberately **not** given the single-row `CHECK` that §5.3 gives `app_user`:
an adopted or restored file may legitimately hold more than one, and a constraint would turn that
into an unstartable binary rather than a resolvable ambiguity.

**Where a local run may start. This is the placement rule, and it follows §7.4 exactly.**

| Backend arm | Project originates in the local store | Project is mirrored from a server |
|---|---|---|
| `Local` | **Run allowed.** Everything resolves in `local.sqlite`. | n/a — this arm has no mirror. |
| `Online` | **Refused.** The local store is read-only in this arm (§5.2), which is the same rule §12 criterion 8 already applies to `mint_item`. | **Allowed, unchanged.** MOD-4 against `PgStore`, exactly as ANA-2 specifies. |
| `Offline` | **Refused**, by the same rule. | **Refused**, `R-STO-4` unchanged, `docs/ANA-2.md` §4.9's offline window unchanged. |
| `Memory` | `MemStore` as today. | n/a |

Two things this table settles. **There is never a cross-store run**, because `run.project_id`,
`run.item_id`, `run.target_box_id` and `run.started_by` all foreign-key inside one file; the
cross-store refusal §4.3 already states for `item_link` generalises unchanged. And **a local project
stops being runnable the moment the box is given a DSN**, until adoption (M7) moves it. That is a
direct consequence of §5.2's read-only-in-`Online` rule and of §12 criterion 8, and it is the same
rule applied consistently rather than a new one — but it sharpens §10.5 considerably: under the
"adoption never funded" answer, the unfunded case now strands **runnable work**, not merely readable
rows. §11 risk 22 carries it and §4.5's overlay copy inherits it.

**What still refuses, locally, and it is a short list.** `R-SEC-4` is unchanged: a run needing
secrets from an unreachable provider is refused with no plaintext fallback, on a local box exactly
as on a server, because the secret provider is a network service and has nothing to do with which
store holds the rows. `R-ORCH-12`'s remote dispatch stays `later` and is more obviously refused on a
box that knows one box. `R-ORCH-13`'s scheduler window stays a reserved `app_setting` key — locally,
a `local_setting` key (§5.4).

**What the maintainer's answer does *not* do.** It does not make the mirror hold graph tables.
`docs/ANA-2.md:222-227`'s **fact** survives untouched — a `Backend::Offline` box still reaches a
graph only through `run.graph_snapshot`, because a mirrored project's graphs live in Postgres and a
local project is not on the `Offline` path at all. Only that sentence's *warrant clause*
("`R-STO-4` starts no runs offline") needs the restatement §6.3 records.

### 4.9 In-app DSN entry (`R-TUI-8`, `R-STO-1`, `R-SEC-2`, `R-NF-3`)

**Need.** The maintainer's objective for the whole item is "allow the user to start the program
without defining the DSN, so to give the possibility to define it inside". §4.5's verdict routed the
user back to `htui --set-dsn` and a restart, which does not satisfy it. This question settles where
the field lives, what primitive it is built from, how the typed string reaches the keyring without
touching argv, a shell history, a file, a log or an error message, what appears on screen, what a
validation failure does, and how the value is disposed of.

**Options.**

| Option | Mechanism | Verdict |
|---|---|---|
| A. Masked single-line field in the Settings connection section, reached by `TabAction::FocusSection(SettingsTab::ID, SectionId("connection"))` | A new `MaskedField` primitive beside `Composer`; a `SettingsSection::captures_input` predicate that reorders `SettingsTab::on_key`; `StoreRequest::SetDsn` handled in `spawn_blocking` on the worker | **Adopted.** |
| B. Full-screen first-run form, its own `Overlay`, DSN plus a "test connection" button | The overlay is already modal and already swallows unhandled keys (`app/state.rs:375-377`), so it has no `h`/`l` contest at all, and `MigrationPrompt` is the copyable precedent (`migration_prompt.rs:90-96`) | **Rejected, and it is the genuine rejection.** It is the *easier* build: a modal overlay needs no `SettingsSection` trait change, no reordering of `SettingsTab::on_key`, and no contest with the section strip — the single hardest mechanical problem in option A disappears entirely. Rejected because the DSN has a **second** lifecycle the first run does not cover: replacing a rotated password, correcting a typo six months later, moving a server. A first-run-only form means that user is back on `--set-dsn` folklore, which is the exact failure §4.5 option A was rejected for. It also puts a credential field in the one `startup_overlay` slot `App` has (`app/state.rs:167`), which §4.5 has already promised to the local-only notice and which MOD-15's switcher also wants — a three-way contest for one slot. |
| C. Keep `--set-dsn` only; the section is read-only and points at the flag | Zero new primitives, zero reversal of `lib.rs:42-44`, and the section is four lines of `Paragraph` | **Rejected.** Its pros are real and were §10.14's original recommendation: it needs no masked primitive (none exists — a grep of `crates/htui/src` for `mask\|redact\|password\|secret` finds only scrubber prose and the `secret::` calls in `lib.rs`), it cannot leak through a `Debug` derive because no request carries a DSN, and it keeps the credential path entirely outside raw mode. Rejected because it does not satisfy the item's stated objective. Recorded with its pros intact because it is the fallback if §12 criteria 20-26 cannot be met. |

**Verdict.** Option A. `--set-dsn` and `--clear-dsn` are **retained unchanged**: they are the only
path that works when the terminal cannot be initialised, and they are what a scripted box uses.

**Deciding reason, stated as a fact.** The DSN has a lifecycle, not a first run. A first-run-only
mechanism (option B) and an out-of-band-only mechanism (option C) both leave the second, third and
fourth time the DSN changes outside the product.

**Seven sub-verdicts.**

**(1) Where the field lives.** In the Settings tab, in `SectionId("connection")` — the section §4.5
sub-verdict 3 already named and §9.4 already assigns to MOD-15. The overlay's action pair is
unchanged: `Action::Tab(TabAction::FocusSection(SettingsTab::ID, SectionId("connection")))` then
`Action::Overlay(OverlayAction::Close)`. **M0's two names are not renegotiated**, which was the whole
point of pinning them on day one. What changes is what the user finds on arrival — and the user must
land **on the field**, not on a section needing a second, undiscoverable keystroke. Of the two ways
to arrange that — a third action variant carrying an intent, or the section opening its own field
when it is focused and `ConnectionInfo` reports no DSN stored — **the second is adopted**: it keeps
M0's names frozen and puts the decision in the component that owns the state, where the first would
widen a `Copy` enum for one caller and need a third name agreed with MOD-15 after M0 shipped.

**(2) The masked primitive is a new sibling of `Composer`, not an extension of it — and §4.5's
stated reason for ruling it out was only half right.** `Composer` is documented as "A mode, not a
widget with focus: while it is active every printable key is text, which is what lets the tab's own
bindings (`i`, `t`, digits) stay single letters everywhere else" (`chat/composer.rs:3-4`). §4.5 read
that as meaning the mode pattern cannot work in Settings. **It can, and it already does one tab
across.** `ChatTab::on_key` consults the composer **first**, with the comment "The composer owns
every key while it is open, so the tab's own single letters stay single letters everywhere else"
(`chat/mod.rs:398-402`). `SettingsTab::on_key` has the order backwards: it consumes
`h`/`l`/`[`/`]`/Left/Right and returns `Handled::Consumed` **before** reaching the active section
(`settings/mod.rs:197-213`). The problem is a five-line delegation order in one file, not a missing
focus system.

The fix is one default-bodied trait method:

```rust
/// Whether this section is currently consuming every printable key, so the tab must not
/// take `h`/`l`/`[`/`]` for section cycling. The chat tab's rule (`chat/mod.rs:398-402`),
/// one tab across.
fn captures_input(&self) -> bool { false }
```

checked in `SettingsTab::on_key` before the cycle match. Every existing section — `AgentsSection`,
which returns `Handled::Pass` from `on_key` (`settings/agents.rs:63-65`) — is unaffected by the
default body. The enclosing key chain needs no change at all: it is overlay → `tab.on_key` →
`KeyScope::Tab` → `KeyScope::Global` (`app/state.rs:404-421`), so a `Handled::Consumed` from a
section already swallows `q` and every global binding.

Extending `Composer` is rejected on four checked grounds: it exposes `pub fn text(&self) -> &str`
(`composer.rs:42-45`), which a masked field must never offer; its `render` hard-codes the visible
`Span::styled(composer.text.clone(), theme.base)` (`:91-95`); its `Submit(String)` hands the caller a
bare `String`, which is the type sub-verdict (6) forbids; and `ChatTab` has four call sites plus five
tests pinned to its current behaviour. A second ~90-line file costs less than a widened one.
`MaskedField` lands at `crates/htui/src/ui/masked.rs` — beside `layout.rs` and `theme.rs`, not under
`tabs/chat/`, because it is a general primitive.

**(3) The typed string's route to the keyring, and every place it must not appear.**

```
MaskedField (TUI process, one Zeroizing<String>)
    └─ Ctx::emit(Action::Request(StoreRequest::SetDsn(Dsn)))    ← newtype, redacting Debug
        └─ store_worker, spawn_blocking(|| secret::set_dsn(&dsn))
            └─ keyring::Entry::set_password  (secret.rs:38-40, :83-87)
```

Five negative guarantees, each with the mechanism that makes it true rather than intended:

- **Never argv.** The field is not a CLI path at all; `cli.rs` is untouched.
- **Never a shell history.** Same.
- **Never a file.** The only DSN-adjacent file write in the tree is `identity::db_fingerprint`
  (`identity.rs:132-144`), which writes a SHA-256 hex digest as a directory name. Worth stating
  precisely: it falls back to hashing the **whole DSN** when `PgConnectOptions::from_str` fails
  (`:140`), so a malformed DSN's bytes are hash material — one-way and unrecoverable, but it means an
  unvalidated DSN reaches that function. Sub-verdict (5) prevents an unvalidated DSN from being
  stored at all, which closes it. §5.5 rule 3 and §12 criterion 17 already forbid a DSN in
  `box.toml`; criterion 20 widens the grep.
- **Never a log.** `StoreRequest` derives `Debug, Clone` (`store_worker.rs:49-50`) and
  `RequestEnvelope` derives `Debug` (`:263-264`). A `SetDsn { dsn: String }` variant would therefore
  be printed in full by any `{:?}` of an envelope. **The variant must carry a newtype with a
  hand-written `Debug`:**

  ```rust
  /// A DSN on its way to the keyring. Never `#[derive(Debug)]`: `StoreRequest` and
  /// `RequestEnvelope` both derive it (store_worker.rs:49-50, :263-264), so a derived
  /// impl would print the credential into any `{:?}` and from there into `--log`.
  #[derive(Clone)]
  pub struct Dsn(Zeroizing<String>);

  impl core::fmt::Debug for Dsn {
      fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
          f.write_str("Dsn(<redacted>)")
      }
  }
  ```

  `StoreRequest::name()` (`store_worker.rs:131-170`) returns `"set_dsn"`, which is the string that
  reaches `StoreReply::Failed` and the logs — the existing mechanism already does the right thing.
- **Never an error message.** See sub-verdict (5).

There is no route by which the DSN reaches an agent: `R-SEC-2` (`:223-225`) says `htui`'s own
credentials "are never exposed to a session", and nothing in the field's path crosses `agent_worker`.
Stated so the reversal in §10.14 is not read as widening the blast radius.

**(4) What is echoed on screen.**

- **While typing: nothing recoverable.** One `•` per typed `char`, plus a live count —
  `•••••••••••••• (34)`. Length is disclosed, which is the standard password-field trade and is what
  makes backspace usable.
- **No reveal toggle.** A toggle is a single keystroke to the exact thing the design exists to
  prevent, and in a TUI it lands in scrollback the moment the alternate screen is left improperly.
- **After a successful parse: a reconstructed, redacted summary** —
  `postgres://htui@db.example:5432/htui · sslmode=require`. Built from
  `PgConnectOptions::get_username`, `get_host`, `get_port`, `get_database`, `get_ssl_mode`. **This is
  safe by construction, not by discipline: `PgConnectOptions` has no password getter at all** (the
  full getter set is `get_host`, `get_port`, `get_socket`, `get_username`, `get_database`,
  `get_ssl_mode`, `get_application_name`, `get_options`). The summary cannot carry the password even
  if someone tries. It is also nearly free: `identity::db_fingerprint` already performs exactly this
  reconstruction for three of the five fields (`identity.rs:134-139`).
- **Rendering constraints, from the tree.** `Composer::render` uses a bare `Paragraph::new(line)`
  with no `Wrap` (`composer.rs:89-100`), so a line longer than the pane is clipped and the trailing
  `_` cursor vanishes. `MaskedField` must render a **window** ending at the cursor with a leading `…`
  when the mask overflows the pane. `Event::Resize` only sets `dirty` (`app/state.rs:324`) — there is
  no reflow hook — so the window is computed at render time from the `Rect`, never cached.
- **Paste.** Bracketed paste is **not enabled**: `terminal::init()` is bare `ratatui::init()`
  (`terminal.rs:23-28`) and `App::on_terminal_event` matches only `Event::Key(Press | Repeat)` and
  `Event::Resize`, with `_ => {}` for everything else (`app/state.rs:319-327`). So `Event::Paste`
  never arrives and a terminal paste is a burst of `KeyCode::Char`. Two consequences the field must
  handle: a pasted trailing newline arrives as `KeyCode::Enter` and would submit an incomplete line,
  and `KeyCode::Char('\t')` or control characters can arrive mid-burst. `MaskedField` therefore
  rejects control characters silently rather than pushing them, and submission requires a buffer that
  **parses** (sub-verdict 5) rather than merely being non-empty — which is what makes a truncated
  paste fail loudly instead of storing garbage. Enabling bracketed paste is a separate, larger change
  (it touches `terminal.rs`, `on_terminal_event` and every tab) and is explicitly **out of scope**;
  §11 risk 17.

**(5) Validation failure: nothing stored, nothing echoed, nothing logged.**

Validation is `PgConnectOptions::from_str(&dsn)` on the **UI side, before the request is emitted** —
a pure string parse with no I/O, so `R-NF-3` does not bind it, and doing it there means a bad DSN
never enters the request channel, never reaches the worker and never reaches `secret::set_dsn`. A DSN
is stored only after it parses.

The error text must **not** be sqlx's. What sqlx 0.9.0 actually produces was checked, because the
answer determines whether pass-through would even be a leak: `from_str` maps a URL failure through
`Error::config` (`sqlx-postgres/src/options/parse.rs:176-180`), rendered
`"error with configuration: {0}"` (`sqlx-core/src/error.rs:34-35`); the inner errors reachable from
`parse_from_url` are `url::ParseError`, `Utf8Error`, `AddrParseError` and `ParseIntError`, **none of
which quotes its input**; and exactly one case does quote its input —
`sqlx-postgres/src/options/ssl_mode.rs:48`, `format!("unknown value {s:?} for \`ssl_mode\`")` — which
quotes the **`sslmode` parameter value only**, never the userinfo.

So today the whole-DSN echo is unreachable. **That guarantee is sqlx's, not `htui`'s, nothing in the
tree asserts it, and it is one dependency bump from changing.** The design therefore does not rely on
it: the section renders a **fixed vocabulary** — `not a URL` / `no host` / `unsupported sslmode` /
`port out of range` / `unrecognised parameter` — derived by matching on the parse outcome, and never
`err.to_string()`. Nothing is logged on a validation failure; there is no `tracing` call on this path
at all. This mirrors `R-ID-7`'s "fails closed" and `R-SEC-3`'s persistence block as a *principle*
without stretching either requirement's text (§6.1).

For contrast, and because it is the live surface: a *valid-but-wrong* DSN reaches `connect::attempt`,
whose failure becomes `ConnEvent::Failed(err.to_string())` (`connect.rs:253`) and travels to **both**
the status line and `tracing::warn!(%why, "connect failed")` → the `--log` file
(`store_worker.rs:527`). That path predates this document and is unchanged by it — but an in-app
field makes it far more frequently hit, which is §11 risk 20.

**(6) Zeroing and disposal.** `zeroize 1.9.0` **is already compiled into this tree**, as an
unconditional dependency of `keyring 3.6.3` (`Cargo.lock:1834`, `:4597-4605`) and of `rustls`,
`rustls-native-certs`, `rustls-pki-types` and `dbus-secret-service`. It is **not** in
`[workspace.dependencies]` (`Cargo.toml:11-56`). Declaring it directly therefore adds one manifest
line and compiles **no new crate** — but it is a new *declared* dependency, and §12 criterion 16's
"`cargo tree` shows no new workspace dependency" is amended to say so rather than quietly violated.
It is not invented here; it is promoted from transitive to direct.

`MaskedField` holds a `Zeroizing<String>`; `Dsn` wraps one; both are wiped on drop. Three disposal
points, all required: `MaskedField::clear()` on submit, on `Esc` and on scope change, so the buffer
does not survive leaving the section; the `Dsn` moved into the `spawn_blocking` closure and dropped
there, with the worker keeping no copy; and `secret::set_dsn(&str)` (`secret.rs:38-40`) unchanged,
since `keyring` already owns `zeroize` for exactly this. **Priced honestly:** `Zeroizing<String>`
wipes the *current* allocation, and `String::push` reallocates as it grows, so a DSN typed one key at
a time leaves residue from every intermediate capacity. `String::with_capacity(256)` at construction
means no realloc occurs for any realistic DSN, making the wipe near-total instead of partial. That is
a reduction, not an elimination (§11 risk 19).

**(7) `R-NF-3` and the keyring write.** `secret::set_dsn` → `Slot::set` → `keyring::Entry::set_password`
(`secret.rs:83-87`) is blocking platform FFI — Windows Credential Manager, macOS Keychain, or a D-Bus
round trip to the Linux secret service, which is the slow one. It is issued from a key handler, which
is exactly the case `R-NF-3` binds even though it is not network I/O. Enforcement is structural: no
view holds a store handle (`lib.rs:6-9`), so the field **cannot** call it — it can only `Ctx::emit` a
request. The worker wraps it in `tokio::task::spawn_blocking`. The precedent is one crate down at
`htui-store/src/cache/pending.rs:118` and `:154`; **there are zero `spawn_blocking` calls in
`crates/htui/src/store_worker.rs` today**, so §7.2's `ConnectionInfo` and this write together
introduce the pattern to the worker. The read side (`ConnectionInfo` calling `secret::get_dsn`) takes
the same treatment.

**Security posture: today's stdin prompt against the in-app masked field.**

| Axis | `--set-dsn` on stdin (`lib.rs:109-129`) | In-app masked field |
|---|---|---|
| Echo | **Exposed.** Echoed in full, and the prompt says so: "paste the DSN and press Enter (it will be visible)" (`lib.rs:117`). Echo suppression was explicitly declined — "that would be another dependency" (`:113`). | **Protected.** `•` per character plus a count. No reveal toggle. |
| Terminal scrollback | **Exposed.** The prompt runs on the normal screen, before `ratatui::init()`, so the echoed line stays in scrollback and in any `script`/`tmux` capture. | **Protected.** Raw mode + alternate screen (`terminal.rs:23-28`); nothing recoverable is ever drawn, and `TerminalGuard` restores on the normal, error and panic paths (`:35-41`, `:58-62`). |
| Shell history | **Protected** — stdin, not argv (`lib.rs:111-112`). | **Protected** — no CLI path at all. |
| argv / process list | **Protected** — the flag is a bool (`cli.rs:21-23`). | **Protected.** |
| Config files | **Protected** — no env-var fallback, keyring only (`secret.rs:4-7`). | **Protected**, identically. Same `secret::set_dsn` sink. |
| In-process copies | **Minimal.** One `String` from `read_line`, dropped at function exit; the process exits seconds later (`lib.rs:61-63`). | **Exposed, and this is the new surface.** The value lives in a `MaskedField`, a request envelope, a channel and a `spawn_blocking` closure, inside a long-lived process. Closed by sub-verdict (6). |
| Debug / logging | **No exposure** — no type carries it. | **Exposed by default, and this is the second new surface.** `StoreRequest` and `RequestEnvelope` both derive `Debug`. Closed by the `Dsn` newtype's hand-written `Debug`, sub-verdict (3). |
| Validation errors | Equal. Neither validates before storing; `set_dsn_from_stdin` checks only `!dsn.is_empty()` (`lib.rs:121`). | **Improved.** Parse before store; a bad DSN is never written (sub-verdict 5). |
| Availability when the terminal cannot start | **Works.** Runs before `ratatui::init()`. | **Does not.** Which is why `--set-dsn` is retained. |

**The honest summary: the change is a net improvement on the two axes the stdin prompt is worst on
(echo, scrollback), and it opens two axes the stdin prompt does not have (in-process lifetime,
`Debug` derives). Both new axes are enumerable and both are closed above. The change is not a
regression only if all five of the following hold**, each a `must` with a §12 criterion:

1. **`R-STO-1` sentence 2 (`:111-113`), verbatim and unamended:** "Connection string and provider
   identities live in the OS keyring …, **never in a file**." The field's only sink is
   `secret::set_dsn`. No new file, no `box.toml` key, no cache row, no env var. §5.5 rule 3 forbids
   the `box.toml` route; §12 criterion 20 greps for every other one. This sentence is the reason the
   field is permissible at all: it constrains *where the secret rests*, and the field does not change
   that by one byte.
2. **`R-SEC-3` and `R-ID-7` are not stretched to cover the field, and the field imitates their
   discipline anyway.** Neither binds a credential-entry widget. Saying so plainly matters: a reader
   who assumes the scrubber covers the field will build a design relying on a masking pass that never
   runs on this path. The field's own guarantee is earlier and stronger — the value is never rendered
   — and its failure semantics are copied deliberately: **fail closed**.
3. **No error surfaced to the UI or the log is derived from the input.** Fixed vocabulary only.
4. **`R-NF-3`: the keyring write does not run on the UI thread.** Structural, not conventional.
5. **`--set-dsn` is retained**, and its doc comment at `lib.rs:109-115` is amended at M2b to say it is
   now one of two paths — leaving it unamended would make the binary's own documentation contradict
   the shipped UI.

---

## 5. Shapes this document fixes

### 5.1 The arm predicate, defined once

Every other section refers to this definition and none restates it.

```
Backend::Local  ⟺  the keyring holds no DSN at connect::start
                    (i.e. `dsn.is_none()`, after `--demo` has been excluded)
```

That is the whole predicate. It does not consult `--offline`, it does not consult any persisted
flag, and it does not consult the presence or emptiness of `local.sqlite`. Precedence over the four
inputs, as a table rather than prose, because four inputs and four arms is where a prose rule goes
wrong:

| `--demo` | DSN in keyring | `--offline` | Arm | Label |
|---|---|---|---|---|
| yes | — | — | `Memory(MemStore)` | `memory` |
| no | no | no | `Local { local }` | `local-only` |
| no | no | **yes** | `Local { local }` | `local-only` |
| no | yes | no | `Offline { .. since: None }` then `Online` on a successful dial | `connecting` → `online` |
| no | yes | **yes** | `Offline { .. since: Some(now) }`, never dials | `offline · <age>` |

Notes on the table, each carrying its evidence.

- `--demo` short-circuits before `connect::start` runs at all (`lib.rs:70-71`) and already conflicts
  with `--set-dsn` and `--clear-dsn` at the CLI (`cli.rs:20-27`). Nothing here changes it.
- Row 3 is the case §4.6's conflict 1 exists for. `--offline` carries no `conflicts_with`
  (`cli.rs:29-32`, verified), so it is reachable with no DSN. **It is a no-op there**: a box with no
  DSN has nothing to be offline from, and the arm and label are the same as row 2. The alternative —
  rejecting the combination at the CLI with a `conflicts_with` — is available to the maintainer
  (§10.16) and would be equally consistent; what must not happen is two answers.
- Rows 4 and 5 are unchanged behaviour. Row 5 keeps `offline · <age>` deliberately: a DSN exists and
  the box has been told not to dial, which is a different fact from having no DSN. It also keeps
  `offline_never_dials_and_starts_at_an_age` green (§3).
- **`--clear-dsn` on a box that had a server.** The flag exists (`cli.rs:25-27`) and no verdict
  addressed it, so it is decided here: clearing the DSN moves the box to row 2 on the **next**
  launch, exactly as the table says, because the predicate reads the keyring at `start()` and nothing
  else. The mirror at `cache/<old-fingerprint>/` is not deleted and not opened; the local store is
  opened and is empty unless the box created something. `cache/<old-fingerprint>/pending/*.jsonl`
  that had not uploaded will never upload, which is the pre-existing stranding of §4.7 claim 4
  reached from the other direction. The first-run notice is **not** re-shown, because
  `local_notice_shown` in `box.toml` survives (§5.5). Whether `--clear-dsn` should warn about
  unuploaded buffers is §10.12.

### 5.2 `Backend` and `Writer` variant shapes

```rust
/// The store the application runs against (ANA-9 §6.1, as amended by ANA-10 §4.1).
#[derive(Debug, Clone)]
pub enum Backend {
    /// Everything in process memory (`--demo`, tests).
    Memory(MemStore),
    /// Postgres reachable: reads go to Postgres, writes are available, the refresher is running.
    Online {
        pg: PgStore,
        cache: CacheStore,
        /// The local store, when this box has one. **Read-only in this arm**: while a DSN is
        /// configured, a locally-originated project neither mints nor runs, and is readable
        /// only, until adoption moves it (ANA-10 §4.8, §12 criterion 8).
        local: Option<LocalStore>,
    },
    /// Postgres unreachable: reads come from the mirror, and the only write path to a *mirrored*
    /// project is the offline chat buffer under `<cache_dir>/pending/`.
    Offline {
        cache: CacheStore,
        since: Option<DateTime<Utc>>,
        local: Option<LocalStore>,
    },
    /// No DSN is stored: there is no server and no mirror. The local store is the whole world.
    Local {
        local: LocalStore,
    },
}
```

`LocalStore` is `Clone + Debug` and holds a cheap `SqlitePool` handle, exactly as `CacheStore` does,
which is what keeps `Backend` a concrete enum with `Send`-inferable futures.

Accessor contract, as a table because it compresses and because getting two of these backwards is
the failure mode:

| Method | `Memory` | `Online` | `Offline` | `Local` |
|---|---|---|---|---|
| `label()` | `memory` | `online` | `connecting` \| `offline · <age>` | **`local-only`** (no age) |
| `is_writable()` | `true` | `true` | `false` | **`false`** |
| `writable() -> Option<&PgStore>` | `None` | `Some` | `None` | **`None`** |
| `writer() -> Option<Writer>` | `Memory` | `Online` | `Buffered` | **`Local`** |
| `cache() -> Option<&CacheStore>` | `None` | `Some` | `Some` | **`None`** |
| `local() -> Option<&LocalStore>` | `None` | as held | as held | **`Some`** |

`is_writable()` and `writable()` are the two that read naturally in the wrong direction — "the local
store is writable" is a true sentence and the wrong answer. `is_writable()` means *the server is
reachable* (`backend.rs:93-106`) and a `true` there stops the process ever dialling Postgres again,
because the re-dial ticker is guarded on `reconnect.is_some() && !backend.is_writable() &&
held.is_none()` (`store_worker.rs:537`). `writable()`'s concrete `&PgStore` return type is the
type-level expression of `R-STO-1` and is left alone deliberately.

```rust
/// A writable store a caller can hold.
#[derive(Debug, Clone)]
pub enum Writer {
    Memory(MemStore),
    Online(PgStore),
    Buffered(BufferedWriter),
    /// The local primary store of a box with no server (ANA-10 §4.1).
    Local(LocalStore),
}
```

`Writer::label()` gains `"local"` beside `"memory"`, `"online"` and `"buffered"`
(`writer.rs:56-66`). That label already travels to the chat tab on
`StoreReply::ChatAccepted.writer_label` so a view can say a conversation is only on this disk
without asking the backend (`store_worker.rs:209-216`), which means local-only chat needs no new
plumbing on that axis at all.

### 5.3 The local migration set

`crates/htui-store/local_migrations/0001_local.sql`, applied by a third
`pub static LOCAL_MIGRATOR: Migrator = sqlx::migrate!("./local_migrations")` beside `MIGRATOR`
(`lib.rs:36`) and `CACHE_MIGRATOR` (`:44`). Its header must state the inverse of `CACHE_MIGRATOR`'s
rationale: **this file is never rebuilt and always migrates its data forward.**

Decisions the DDL encodes, each settled in §4.2 and none of them re-openable cheaply, because SQLite
cannot `ALTER` a `CHECK`, a `UNIQUE` or a foreign key afterwards:

- **`STRICT` on every table.** SQLite's declared types are otherwise advisory: an `INTEGER` column
  silently stores the string `'wxyz'`, and `PRIMARY KEY` columns may contain `NULL`. `STRICT` makes
  a lossless-conversion failure an `SQLITE_CONSTRAINT_DATATYPE` error and makes `PRIMARY KEY`
  implicitly `NOT NULL`. The mirror has neither defence and did not need one.
- **Foreign keys ON**, at the pool. The mirror's `foreign_keys(false)` is justified by "the server
  already enforced every constraint this file would repeat" (`cache/mod.rs:244-247`), which is
  exactly false here. The pragma is set once at connect time, which is a second reason the two
  stores cannot share a pool.
- **The Postgres `CHECK` vocabularies, restored.** `item.status`'s eight values, `key_number >= 1`,
  `last_value >= 0`. One rewrite is needed: `item_kind.prefix`'s Postgres regex `CHECK` uses `~`,
  which SQLite does not have, so it becomes a `GLOB`.
- **`item.key` as a real generated column.** Available since SQLite 3.31.0 and `htui` bundles
  3.51.3. It must be in the initial `CREATE TABLE`, because a `STORED` generated column cannot be
  added later by `ALTER TABLE`. Note it must not be `PRIMARY KEY` and must not carry a `DEFAULT`,
  neither of which is wanted here.
- **The mirror's type mapping, reused verbatim** — UUID as hyphenated `TEXT`, `TIMESTAMPTZ` as
  `INTEGER` epoch microseconds, `TEXT[]` as `TEXT` holding a JSON array, `JSONB` as `TEXT`,
  `BOOLEAN` as `INTEGER` 0/1 (`0001_mirror.sql:5-12`) — so the encoders and decoders at
  `cache/read.rs:54-111` and `refresh.rs:379-393` are reusable. `TEXT` and `INTEGER` are both in
  `STRICT`'s six permitted type names, so the two decisions do not conflict.

The sketch below is the first-need set for the three consumers `HANDOFF.md` names — MOD-13, MOD-15
and, since §10.8 was answered, MOD-4 — abbreviated to the tables and columns where this document
takes a position; the remainder follow `migrations/0001_init.sql` mechanically under the mapping
above. The run half is the **Set A / Set B** split below the sketch.

```sql
-- --------------------------------------------------------------------------------------------
-- local_migrations/0001_local.sql - the writable local store of ANA-10 §4.2.
--
-- Derived from migrations/0001_init.sql's SEMANTICS, not from cache_migrations/0001_mirror.sql's
-- subset. Unlike the mirror, this file is never rebuilt: a version mismatch migrates data
-- forward. Forward-only (R-STO-5): later work adds 000N_*.sql and never edits this file.
-- --------------------------------------------------------------------------------------------

CREATE TABLE local_meta (                      -- §5.4; NOT cache_meta's key set
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL
) STRICT;

CREATE TABLE local_setting (                   -- app_setting's shape; app_setting is Postgres-only
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,                 -- JSON text, as app_setting's JSONB
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE app_user (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    -- R-USR-2's single row, which Postgres enforces with LOCK TABLE ... SHARE ROW EXCLUSIVE.
    -- Locally there is no concurrency to serialise, so the rule is a constraint:
    only_row    INTEGER NOT NULL DEFAULT 1 CHECK (only_row = 1),
    UNIQUE (only_row)
) STRICT;

CREATE TABLE box (
    id           TEXT PRIMARY KEY,             -- box.toml's already-minted UUIDv7
    user_id      TEXT NOT NULL REFERENCES app_user(id),
    hostname     TEXT NOT NULL,
    os_family    TEXT NOT NULL,                -- std::env::consts::OS at first write
    os_version   TEXT NOT NULL,                -- '' until MOD-7's probe runs; NOT NULL kept so
    arch         TEXT NOT NULL,                -- an adopted row satisfies the server's DDL
    htui_version TEXT NOT NULL,
    registered_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE (user_id, hostname)                 -- the server's rule, so adoption cannot surprise
) STRICT;

CREATE TABLE project (
    id            TEXT PRIMARY KEY,
    -- ... slug, name, settings, created_by, created_at, updated_at per 0001_init.sql ...
    origin_box_id TEXT NOT NULL REFERENCES box(id),   -- §4.3 provenance; no server to ask
    local_seq     INTEGER NOT NULL,                   -- parent-before-child order for adoption
    sealed_at     INTEGER,                            -- §4.6 conflict 7: set by adoption, in the
                                                      -- same transaction as the counter fast-forward
    adopted_at    INTEGER
) STRICT;

CREATE TABLE item_key_counter (                -- absent from the mirror by design
    project_id  TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    prefix      TEXT    NOT NULL,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0),
    PRIMARY KEY (project_id, prefix)
) STRICT;

CREATE TABLE item (
    id             TEXT    PRIMARY KEY,
    project_id     TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind_id        TEXT    NOT NULL REFERENCES item_kind(id),
    key_prefix     TEXT    NOT NULL,
    key_number     INTEGER NOT NULL CHECK (key_number >= 1),
    key            TEXT    GENERATED ALWAYS AS (key_prefix || '-' || key_number) STORED,
    title          TEXT    NOT NULL,
    body           TEXT    NOT NULL DEFAULT '',
    status         TEXT    NOT NULL DEFAULT 'open' CHECK (status IN
                     ('open','queued','in_progress','awaiting_approval','blocked','done','failed','closed')),
    priority       INTEGER NOT NULL DEFAULT 0,
    required_tags  TEXT    NOT NULL DEFAULT '[]',   -- JSON array, per the mirror's mapping
    touched_paths  TEXT    NOT NULL DEFAULT '[]',
    step_graph_id  TEXT    REFERENCES step_graph(id),
    version        INTEGER NOT NULL DEFAULT 1,
    created_by     TEXT    NOT NULL REFERENCES app_user(id),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    closed_at      INTEGER,
    origin_box_id  TEXT    NOT NULL REFERENCES box(id),
    local_seq      INTEGER NOT NULL,
    adopted_at     INTEGER,
    UNIQUE (project_id, key_prefix, key_number)     -- the mirror has two PLAIN indexes here
) STRICT;

CREATE TABLE item_revision (                   -- absent from the mirror; conformance requires it
    item_id        TEXT    NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    version        INTEGER NOT NULL,
    title          TEXT    NOT NULL,
    body           TEXT    NOT NULL,
    required_tags  TEXT    NOT NULL,
    author_id      TEXT    NOT NULL REFERENCES app_user(id),
    box_id         TEXT    REFERENCES box(id),
    reason         TEXT    NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (item_id, version)
) STRICT;

CREATE TABLE item_link (
    -- ... from_item_id, to_item_id, kind, created_by, created_at ...
    deleted_at     INTEGER                     -- the mirror DROPS this (0001_mirror.sql:16-18)
) STRICT;

-- Also, unchanged in shape from 0001_init.sql under the mirror's type mapping:
--   workspace, workspace_project, repo, item_kind, item_note, document, agent,
--   run, run_step, run_step_commit, session_event,
--   step_graph, step_graph_phase, prompt_template.
--
-- Deliberately ABSENT and why (rewritten after §10.8 was answered; every table this block used
-- to defer is now Set A below, because a local graph run reads it):
--   app_setting      - Postgres-only; local_setting above is its local counterpart.
--   item_key_counter for MIRRORED projects - a mirrored project's counter has a second writer
--                      by construction, which is exactly what §4.3's origin rule refuses.
```

**The runs delta, and the line that separates buildable from reserved.** The maintainer's answer to
§10.8 puts graph runs in scope. It cannot be specified against `migrations/0003_orchestration.sql`,
because **that file does not exist**: `crates/htui-store/migrations/` holds exactly one file,
`0001_init.sql`; `0002_agent_probe.sql` is MOD-2's and unwritten; `0003_orchestration.sql` is MOD-4's
and unwritten; there is no `htui-orch` crate; and MOD-4 is an open `HANDOFF.md` item blocked on MOD-2
(`HANDOFF.md:177-194`). ANA-2 states the ordering hazard itself (`docs/ANA-2.md:1848-1857`, risk 1 at
`:2064`). The delta therefore splits in two, and the split is the load-bearing part of this
subsection.

**Set A — authored here, at M3, because it exists in `migrations/0001_init.sql` today.** Every table
and column below is copied from the shipped Postgres file under the type mapping above. Nothing here
depends on MOD-4 having written a line.

```sql
-- --------------------------------------------------------------------------------------------
-- local_migrations/0001_local.sql, continued: the run half (§4.8).
-- Derived from migrations/0001_init.sql as it stands today. Nothing here anticipates MOD-4.
-- --------------------------------------------------------------------------------------------

-- box gains the nine columns the sketch above omitted. R-ORCH-10's capability check reads
-- probed_tags/declared_tags; ANA-2 §4.7's BoxSettings reads settings; R-BOX-3 reads quirks.
-- A local box without them cannot admit a single run. (0001_init.sql:53-75.)
--   cpu           TEXT    NOT NULL DEFAULT '',
--   ram_mb        INTEGER,
--   gpu_present   INTEGER NOT NULL DEFAULT 0,
--   gpu_vendor    TEXT,
--   probed_tags   TEXT    NOT NULL DEFAULT '[]',   -- JSON array, per the mirror's mapping
--   declared_tags TEXT    NOT NULL DEFAULT '[]',
--   quirks        TEXT    NOT NULL DEFAULT '',
--   settings      TEXT    NOT NULL DEFAULT '{}',   -- ANA-2 §4.7 BoxSettings
--   last_probed_at INTEGER
-- NO single-row CHECK on box, deliberately (§4.8): an adopted or restored file may hold more than
-- one, and box_info() resolves by box.toml's BoxId rather than by ORDER BY id LIMIT 1.

-- project gains the two columns the "per 0001_init.sql" shorthand elided, because R-SEC-1's
-- provider binding is per project and a run resolves it before stage 1 (0001_init.sql:148-149):
--   secret_provider TEXT,
--   secret_scope    TEXT

CREATE TABLE capability_tag (                  -- R-BOX-3's seeded vocabulary; 0001_init.sql:43-47
    tag         TEXT PRIMARY KEY,
    description TEXT NOT NULL DEFAULT '',
    seeded      INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE box_tool (                        -- R-BOX-2; MOD-7's probe fills it. 0001_init.sql:81-88
    box_id    TEXT NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    version   TEXT NOT NULL,
    path      TEXT NOT NULL,
    probed_at INTEGER NOT NULL,
    PRIMARY KEY (box_id, name)
) STRICT;

CREATE TABLE agent_box (                       -- R-AGT-7 quota + ANA-2 §4.2's DriverCaps interlock
    agent_id   TEXT NOT NULL REFERENCES agent(id) ON DELETE CASCADE,
    box_id     TEXT NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    enabled    INTEGER NOT NULL DEFAULT 1,
    version    TEXT,
    path       TEXT,
    probed_at  INTEGER,
    quota      TEXT,                           -- JSONB -> TEXT
    quota_at   INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, box_id)
) STRICT;

CREATE TABLE phase_agent (                     -- R-AGT-8's priority list. Without it every seeded
    phase_id TEXT    NOT NULL REFERENCES step_graph_phase(id) ON DELETE CASCADE,  -- graph has zero
    position INTEGER NOT NULL,                 -- candidates and every run refuses (ANA-2 risk 14).
    agent_id TEXT    NOT NULL REFERENCES agent(id),
    model    TEXT    NOT NULL,
    PRIMARY KEY (phase_id, position)
) STRICT;

CREATE TABLE repo_box_path (                   -- R-BOX-4; ANA-2 §4.6 resolves every tree from it
    repo_id    TEXT NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    box_id     TEXT NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    local_path TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (repo_id, box_id)
) STRICT;

CREATE TABLE workspace_box_path (              -- R-BOX-4; ANA-5's excerpt fallback root
    workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    box_id       TEXT NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    root_path    TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, box_id)
) STRICT;

CREATE TABLE command_run (                     -- R-MCP-3; 0001_init.sql:537-551. Unread until
    id          TEXT    PRIMARY KEY,           -- MOD-11, present so an adopted run arrives complete
    run_step_id TEXT    NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    box_id      TEXT    NOT NULL REFERENCES box(id),
    class       TEXT    NOT NULL,
    command     TEXT    NOT NULL,
    cwd         TEXT    NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'queued'
                  CHECK (status IN ('queued','running','done','failed','cancelled')),
    exit_code   INTEGER,
    output      TEXT,                          -- scrubbed (R-SEC-3, §10.6)
    queued_at   INTEGER NOT NULL,
    started_at  INTEGER,
    finished_at INTEGER
) STRICT;

-- skill, skill_version, skill_binding: shapes verbatim from 0001_init.sql:406-441 under the
-- mapping. R-PRM-1 puts "bound skill text" in every step prompt, so a local run cannot assemble
-- one without them. `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)` is the reason
-- Postgres 16 is the floor (0001_init.sql:429); SQLite treats NULLs as distinct in UNIQUE, so the
-- local expression is a UNIQUE over (skill_id, project_id, COALESCE(phase_id, '')).

-- run, run_step, run_step_commit, session_event, step_graph, step_graph_phase, prompt_template,
-- agent, document: already named in the "unchanged in shape" list above. Two additions to it:
--   * document.produced_by_step_id gains its REFERENCES run_step(id) ON DELETE SET NULL, and
--     item_link.proposed_by_step_id and item_note.via_step_id gain theirs, exactly as
--     0001_init.sql:582-596 adds them at the end of the file. In SQLite a forward reference is
--     legal inside CREATE TABLE with foreign_keys deferred at DDL time, so no ALTER is needed;
--     the ordering is the only constraint.
--   * run_step.fanout_index carries NO CHECK, and none may ever be added. ANA-2 §4.5 puts the
--     R-ORCH-7 judge at fanout_index = -1 (docs/ANA-2.md:814-820) and its risk 12 (:2075) names
--     the hazard; on SQLite a later CHECK is the 12-step table rebuild, so the prohibition is
--     permanent rather than merely advised.
```

**Set B — reserved, NOT authored here.** Everything below is `docs/ANA-2.md` §9's
`0003_orchestration.sql`, which does not exist. Authoring it here would fix a shape MOD-4 has not yet
written and cannot then change without a table rebuild. It belongs to MOD-4, in a
`local_migrations/0002_orchestration_local.sql` landed in the same commit as its Postgres and mirror
siblings (§9.1's ordering rule).

```sql
-- RESERVED to MOD-4. Do not author in 0001_local.sql.
--   step_graph.is_override                                     (ANA-2 §4.1; :1869)
--   step_graph_phase.judge_agent_id, judge_model,
--                    deadline_seconds                          (ANA-2 §4.5, §4.2; :1877-1880)
--   step_graph_phase CONSTRAINT ck_phase_judge_model           (:1881-1882)
--   run.repo_scope, lease_box_id, lease_owner, lease_expires_at (ANA-2 §4.7, §4.9; :1900-1903)
--   run CONSTRAINT ck_run_graph_snapshot                       (:1907-1908)
--   run_step.verify_outcome, verify_exit_code, promoted_at     (ANA-2 §4.2, §4.8; :1926-1929)
--   TABLE run_step_tree                                        (ANA-2 §4.6; :1951-1959)
```

**One decision the reservation forces, because SQLite makes it a one-way door.** SQLite's
`ALTER TABLE … ADD COLUMN` prohibits `PRIMARY KEY`, `UNIQUE`, a non-constant `DEFAULT`, `NOT NULL`
without a default, a `STORED` generated column, and a foreign key with a non-`NULL` default. A
column-level `CHECK` is **not** on that list, and a whole new table is always addable. So every
*column* and the *table* in Set B can be added later by MOD-4 at no rebuild cost. The two
**table-level** constraints cannot: `ck_run_graph_snapshot` and `ck_phase_judge_model` are
add-only-by-rebuild. The choice is therefore:

| | Option | Cost |
|---|---|---|
| (a) | Author both table-level CHECKs in `0001_local.sql` now, mirroring ANA-2 §9 exactly | The local schema is briefly ahead of Postgres, and if MOD-4 changes `0003`'s shape the local file is already frozen. Note `ck_run_graph_snapshot` can be authored **plain** rather than `NOT VALID`: ANA-2 makes it `NOT VALID` only because the demo loader writes graph runs with a NULL snapshot (`docs/ANA-2.md:2003-2005`), and `local.sqlite` has no legacy rows. |
| (b) | **Recommended.** Author `ck_run_graph_snapshot` now (its shape is fully determined by `0001_init.sql`'s existing `kind` column and cannot change); defer `ck_phase_judge_model` with its columns to MOD-4, accepting that until MOD-4 lands the local schema is one CHECK looser than Postgres | One documented divergence, in a constraint whose violation is unreachable before MOD-4 exists, because nothing writes `judge_model` until then. |

§10.22 answered "mirror Postgres's constraints exactly"; this is the one place that answer needs a
qualifier, and §10.32 puts it back to the maintainer.

`updated_at` needs an explicit decision, because Postgres maintains it with a `BEFORE UPDATE`
trigger over exactly twenty tables (`0001_init.sql:563-580`) and `pg/write.rs:14-15` forbids setting
it by hand. **The local store sets it in the write path**, not by trigger: there is no cursor riding
on it locally (nothing refreshes a local store), one writer sets it deterministically, and a trigger
would be a second place to keep the two schemas aligned. The adoption pass therefore reads it as an
ordinary column.

### 5.4 The local meta table

`local_meta` replaces `cache_meta`'s role and must **not** copy its key set. `cache_meta` holds
`schema_version`, `db_fingerprint`, `built_at`, `last_full_refresh_at` (`cache/mod.rs:68-72`).

| Key | Value | Job |
|---|---|---|
| `schema_version` | the local migrator's applied maximum | Feeds the forward migration decision. Never a delete trigger. |
| `store_id` | UUIDv7, minted once at file creation | Pairs the file with the `box.toml` beside it (§5.7). |
| `built_at` | epoch microseconds | Diagnostics. |
| *(absent)* | `db_fingerprint` | No DSN, no input; and it is an endpoint hash, not a database identity (§4.2). |
| *(absent)* | `last_full_refresh_at` | Nothing refreshes this store. |

`local_setting` exists so MOD-15's Settings sections have somewhere to write on a local box.
`HANDOFF.md:257-258` already puts MOD-15 on the hook for exposing ten `app_setting` keys, and
`app_setting` is Postgres-only, so without this table MOD-15 would invent a second local persistence
location — which is exactly what §4.5 spends a verdict avoiding.

### 5.5 `box.toml` keys

```toml
# <config_root>/box.toml
box_id   = "0192f3a1-...-...."      # existing: UUIDv7, minted on first launch, never re-minted
hostname = "workstation"            # existing: last known hostname

# ANA-10 additions. Every one is #[serde(default)].
local_store_id     = ""             # local_meta.store_id of the local.sqlite this file belongs to
adopted_into       = ""             # Postgres system_identifier this box's local rows went to
local_notice_shown = false          # the first-run overlay has been seen
```

Three rules, each of which is a live defect if broken.

1. **`#[serde(default)]` is mandatory.** `BoxToml` derives `Deserialize` with no defaults today
   (`identity.rs:30-34`) and `load_or_mint` turns a parse failure into `StoreError::Backend`
   (`:68-71`), which `connect::start` propagates before the terminal is initialised. A new required
   field would make every existing `box.toml` unreadable and the binary unstartable. The failure
   mode is "htui no longer starts", not "the notice reappears".
2. **The fields must be carried on `Identity`, not only on `BoxToml`.** `identity::store` serialises
   the whole `BoxToml` and is called by `load_or_mint` on any hostname change (`:76-78`) and by
   `connect::try_connect` under the adopt-DB-id rule (`connect.rs:267-284`). A field added to
   `BoxToml` alone survives until the first laptop rename.
3. **No DSN, ever, in this file.** `R-STO-1` sentence 2 is untouched by this document and is
   restated here as a rule about `box.toml` specifically: "Connection string and provider identities
   live in the OS keyring …, never in a file." The marker this document adds is a UI preference and
   an adoption record; adding a field to this file must not be read as licence for a config-file
   DSN. §12 criterion 17 tests for it.

### 5.6 The store-trait gap: nothing creates the hierarchy

This is the largest sizing gap in the analysis and every underlying verdict left it open, so it is
stated plainly. **`WriteStore` has no hierarchy-creation method.** The trait ends at

```rust
    // links, notes, documents, skills, templates, box ...
```

(`traits.rs:153`, verified). `mint_item` exists; nothing creates a `workspace`, a
`workspace_project`, a `project` or an `item_kind`, on **either** backend. Meanwhile
`CacheStore`'s reads require the whole chain before a single item is visible: `links()` joins
`project` and returns `NotFound` when the join misses, `projects()` requires a `workspace_project`
row joined to `project`, `workspaces()` drives off `workspace` (`cache/read.rs:323-343`, `:596-638`,
`:689-704`). As specified before this document, the implementation MOD would ship a writable store
that can mint items into a hierarchy no code can create.

It is fixed here by fixing the **seam**, in two halves, so that MOD-15 and the implementation MOD do
not each invent one:

**Half one — the three trait methods, named now, owned by MOD-15.** `HANDOFF.md:245-258` already
gives MOD-15 "create and edit workspaces, projects (seeded kinds, graphs and templates per
`docs/ANA-9.md` §5.10), repos with primary flag and per-box paths". These are its signatures, so the
implementation MOD can code against them and MOD-15 can implement them:

```rust
// crates/htui-core/src/store/traits.rs, appended to WriteStore.
async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace>;
async fn create_project(&self, new: NewProject) -> Result<Project>;
async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind>;
```

Each `NewX` carries a caller-minted UUIDv7 `id`, as `NewItem` already does (`item.rs:150-152`);
`NewProject` carries the workspace it is attached to, so `create_project` writes the
`workspace_project` row in the same transaction; `create_item_kind` carries the prefix and the
default graph, because `item_kind.default_graph_id` is `NOT NULL`. Each obliges `MemStore` and
`PgStore` and lands with its conformance case in the same commit, which is the cost ANA-2 already
priced for its sixteen and ANA-5 for its six. That cost belongs to MOD-15, and this document does
not spend it.

**Half two — the first-run bootstrap, owned by the implementation MOD, and deliberately not on the
trait.** The rows a local-only box needs before anything works are one `app_user`, one `box` from
`box.toml`'s existing `BoxId`, and one workspace/workspace_project/project/item_kind chain with
ANA-9 §5.10's seed. That is a **private, non-trait** method on the concrete type:

```rust
// crates/htui-store/src/local/write.rs
impl LocalStore {
    /// The rows ANA-9 §5.10 seeds on first connect, written locally instead. Not on `WriteStore`:
    /// it is a bootstrap, not an operation, and MOD-15 owns the general case.
    pub(crate) async fn seed_bootstrap(&self, identity: &Identity, user: &str) -> Result<()>;
}
```

`crates/htui-store/src/testkit.rs:237-380` (`seed_mirror`, behind the `test-support` feature) is the
encoding-correct template: it already writes `app_user`, the own `box` row, `workspace`,
`workspace_project`, `project` and `agent` straight into SQLite with no server, documented as
"everything an offline chat resolves before it can start". It is the proof the bootstrap is not
speculative. It should be **re-pointed** at the local schema, not promoted as-is.

**Third consequence, for the conformance harness.** `LocalStore` also needs a store-specific loader
for the demo fixture, because `run_case` asserts against an already-loaded store
(`conformance.rs:47`, `:56`). That is one function modelled on the existing harnesses, not zero —
see §4.7, claim 8.

### 5.7 Three identity mechanisms, named separately

They were conflated across two verdicts. They answer different questions, live in different files
and must not be substituted for one another.

| Mechanism | Where | Value | The question it answers |
|---|---|---|---|
| `store_id` | `local_meta` inside `local.sqlite` | UUIDv7, minted once when the file is created | *Is this the same local store the `box.toml` beside it belongs to?* Compared against `box.toml`'s `local_store_id`. A mismatch means a copied or restored file and must refuse to write until the user resolves it, on git-annex's precedent, where duplicate repository UUIDs produce "expected repository UUID … but found UUID …" and reusing one is an explicit named command. |
| `adopted_into` | `box.toml` | `SELECT system_identifier FROM pg_control_system()` of the server adopted into; empty means never adopted | *Have these rows already been given to a server, and is this that server?* This is the adoption gate. Postgres uses the same value to refuse a standby against the wrong primary. Zero occurrences in the tree today, so it is new work at the connect handshake (§10.10). |
| `sealed_at` / `adopted_at` | rows in `local.sqlite` | epoch microseconds | *Which subtrees have already gone, and may this project still mint locally?* Per project and per row, set by the adoption transaction (§4.6, conflict 7). |

`db_fingerprint` is used for **none** of these. It is `sha256(host:port/dbname)`
(`identity.rs:126-144`), an endpoint hash with no input on a DSN-less box, and it matches a
dropped-and-recreated database at the same coordinates while differing for the same database reached
via another hostname. It keeps its existing job — naming the mirror's directory — and gains no other.

---

## 6. What this amends

`docs/REQUIREMENTS.md` is edited **only by explicit maintainer decision, never at item close-out**
(`.claude/rules/workflow-docs.md`). This section therefore **proposes** exact replacement wording
and applies none of it. §10 collects the decisions in decidable form. `docs/ANA-9.md` is a different
case: its own status line pre-authorises later ANAs to amend it — "ANA-2, ANA-4, ANA-5 and ANA-7
amend this schema by forward-only migration where their verdicts need columns this document only
reserves" (`docs/ANA-9.md:11-13`) — so §6.2's amendments are in-format, and ANA-10 should be added
to that list at close-out.

### 6.1 `docs/REQUIREMENTS.md`

| ID | Line | Current wording (verbatim) | Fate | Proposed replacement |
|---|---|---|---|---|
| `R-STO-1` s.1 | `:111` | "Postgres is the only writable store." | **amend** | "Postgres is the only writable **shared** store: a box with a server configured writes nothing anywhere else. A box with no server configured writes to a local store on that box alone, whose rows reach a server only through an explicit, confirmed adoption." |
| `R-STO-1` s.2 | `:111-113` | "Connection string and provider identities live in the OS keyring (Windows Credential Manager, macOS Keychain, Linux secret service), never in a file." | **unaffected**, and load-bearing | Verbatim. The local store holds rows, never a DSN, and `box.toml` gains a preference and an adoption record, not a credential (§5.5 rule 3). |
| `R-STO-3` | `:115-117` | "Each box keeps a **read-only** cache of the projects it has opened under the user's config directory, refreshed on every successful connection. Contents: items, links, documents, notes, run and step summaries, and transcripts of the last N steps (N configurable)." | **unaffected** | Verbatim, and this is the payoff of §4.1's separate-file verdict rather than an absence of impact. "read-only" stays literally true because the writable rows are not in the cache. Under §4.1's option A or C this would need striking, which would be a second maintainer-only amendment. Confirming the intent to preserve it is §10.2. |
| `R-STO-4` | `:118-121` | "When Postgres is unreachable, the TUI opens in offline read-only mode from the cache: browse items, graph, documents and cached transcripts. No item creation, no runs. A free-standing chat session against a repo (R-TUI-6) remains allowed; its events are buffered locally, scrubbed, and persisted on the next successful connection so R-HIS-1 still holds." | **amend by splitting the state**, not by relaxing the clause | Keep R-STO-4 **verbatim** for "server known, unreachable", with one word added for precision: "When a **configured** Postgres is unreachable, …". Add a sibling clause (or a new id, §10.1) for "no server configured": "A box that has never been given a DSN opens in local-only mode against a local store held separately from the read-only cache. It is a complete box: it may create the hierarchy, items and free-standing chats described in `docs/ANA-10.md` §4.3, **and may run step graphs against its own projects under `docs/ANA-2.md`'s contract in full** (§4.8). Its rows stay local when a DSN is later configured, and a project that is still local is not runnable while a DSN is configured; they are never uploaded as a side effect of connecting. Adoption into a server is an explicit, confirmed action, recorded against the server adopted into, and refused for any second or different server without a further explicit confirmation." |
| `R-STO-5` | `:122-123` | "Schema migrations are versioned, forward-only in version one, applied by `htui` on connect after confirmation." | **extend** | Append rather than substitute, so the Postgres rule is untouched: "… applied by `htui` on connect after confirmation. The per-box SQLite schemas are versioned and forward-only on the same rule but are applied without confirmation when the local file is opened: the read-only mirror answers a version mismatch by rebuilding, and the local store, which holds rows that exist nowhere else, migrates its data forward and is never rebuilt." Note both "on connect" and "after confirmation" are **already** false of `cache_migrations/`, which `CacheStore::open` applies with no server and no prompt; the extension makes an existing gap explicit rather than changing the rule. |
| `R-STO-6` | `:123-124` | "Startup with a warm cache and reachable Postgres is under one second on the reference workstation. Cache refresh runs in the background and never blocks input." | **unaffected as written**; a decision is owed | Verbatim. It is conditioned on "a warm cache **and reachable Postgres**" and therefore does not bind a local-only start at all. Whether the maintainer wants the budget extended to that path is §10.4; if yes, the wording is "… and a local-only start is under the same budget", and §12 criterion 13 measures it. |
| `R-ENT-5` | `:75-77` | "`item`: key (`<PREFIX>-<N>`, unique per project), kind, title, body, status, required capability tags, `created_by`, `version` for optimistic locking, timestamps. …" | **unaffected**, and load-bearing | Verbatim. A locally minted key is a real `<PREFIX>-<N>`, unique in its project, from birth. It would have needed amending under §4.3's rejected option 2 (the key becomes absent until adoption) and option 4 (the shape gains a box component). |
| `R-ENT-6` | `:78-88` | "Item kinds are configurable per project. Each kind has a key prefix … Seeded set on project creation." | **unaffected**, but a dependency | Verbatim. Local project creation must seed the five kinds locally per ANA-9 §5.10, which is MOD-15's work, not an amendment. |
| `R-ENT-7` | `:89-90` | "Item keys are minted online from a per-project, per-prefix sequence. Never reused. Offline creation is not supported (see R-STO-4)." | **amend** clauses 1 and 3; clause 2 preserved and strengthened | "Item keys are minted from a per-project, per-prefix sequence held by the store that owns the project: Postgres for a project that exists on a server, the local store for a project created on a box with no server configured. Never reused: a project has exactly one counter and exactly one writer of it, and adoption seals a project's local counter in the same transaction that fast-forwards the server's. Creating an item in a project mirrored from a server, while that server is unreachable, is not supported (see R-STO-4)." **This is the single largest scope lever in the document** — §10.3 offers leaving it entirely untouched. |
| `R-ENT-10` | `:98-99` | "Item body and title changes are recorded as revisions with author, box and reason. …" | **unaffected** | Verbatim. `item_revision` is in the local schema and carries `author_id`, `box_id` and `reason`. The compare-and-set mechanism ports to SQLite unchanged. |
| `R-ID-3` | `:31-32` | "Postgres is the single source of truth. Everything `htui` knows lives there: items, documents, runs, transcripts, skills, templates, box profiles, agent registry, settings." | **amend, or scope** | "**Wherever a server is configured,** Postgres is the single source of truth. Everything `htui` knows lives there: …". A proposal that amends `R-STO-1` without this leaves the contract self-contradictory. `R-ID-2` (`:29-30`, "a local-first, developer-guided harness") is the requirement local-only mode actively supports, so the tension is internal to §1 of the requirements. |
| `R-ID-7` | `:41-42` | "Any transcript or tool output is scrubbed of secrets on the host box before it is persisted or transmitted. Scrubbing fails closed." | **unaffected in text; a seam decision is owed** | Verbatim, and binding. **No underlying verdict raised this.** A local store holding item bodies, notes, documents and local `session_event` rows would be the first durable local persistence of user text outside the recorder's scrubber seam. `append_pending` explicitly "trusts its input" and writes events verbatim, because the `Scrubber` lives in the recorder (`pending.rs:10-17`). This document's position: the seam does not move — the recorder scrubs before it hands events to any writer, so `Writer::Local::append_events` receives already-scrubbed rows exactly as `BufferedWriter::append_events` does; and item bodies, titles, notes and documents are user-typed text that the scrubber has never covered on any backend, so local-only changes nothing about them. §10.6 asks the maintainer to confirm both halves, because "fails closed" is a must and this is the first time the question has been put. |
| `R-SEC-3` | `:227-230` | "Scrubber builds exact-match masks from every resolved secret value plus a pattern rule set for known key formats, runs before any transcript row persists, and marks the step failed and blocks persistence when an unmasked pattern remains." | **unaffected in text; same seam decision** | Verbatim. "before any transcript row persists" must be read to include a row persisting into `local.sqlite`. ANA-7 owns the scrubber; this document owns the statement that the local store is downstream of it. §10.6. |
| `R-USR-2` | `:47-49` | "A `user` entity exists from day one with a single row. Boxes belong to a user. Items, notes, revisions, runs and documents carry `created_by`. …" | **extend**, for the adoption item | Verbatim locally: §5.3's `app_user` has a single-row `CHECK` where Postgres uses `LOCK TABLE app_user IN SHARE ROW EXCLUSIVE MODE`. The extension is about adoption: a locally minted `app_user` **cannot** be inserted server-side, because `seed_if_empty_as` inserts only `WHERE NOT EXISTS` and returns the oldest row's id (`pg/mod.rs:230-333`), so every local row's `created_by` and `author_id` must be remapped. Proposed addition: "Adoption of locally created rows remaps their `created_by` and revision `author_id` to the adopting server's single user; the original authoring box is preserved on the revision." That is an authorship rewrite and is §10.7. |
| `R-BOX-1` | `:52-53` | First launch on a machine registers it as a `box` under the current user. | **extend** | A local-only box registers itself in the local store from `box.toml`'s already-minted UUIDv7, rather than through `PgStore::register_box`. The adopt-DB-id rule (`pg/mod.rs:335-375`, `connect.rs:267-284`) already covers the case where the server later has a row for the same hostname. |
| `R-HIS-1` | `:194-195` | "Every session event from R-AGT-1, plus the assembled prompt, every follow-up and every permission answer, is stored as an ordered row per run step, after scrubbing. Nothing about a run exists only on one box." | **amend** sentence 2, bounded in time | "… Nothing about a run exists only on one box once that box has a server configured. On a box that has never been given a DSN the local store is the only copy — **of a graph run as much as of a chat** — and the TUI says so." The bound is unchanged; what changed after §10.8 was answered is that "a run" now means what `R-ORCH-11` means by it, not only a `run(kind='chat')` row. Sentence 1 is **unaffected**, and is what makes §10.6's scrubbing confirmation binding on `Writer::Local`. ANA-9 §2 invariant 4 pre-empts the loophole in prose ("the offline chat buffer is a queue in front of that table, not a second store", `:56-58`) and moves with the requirement. |
| `R-HIS-2` | `:197` | "The chat view can reopen any past step read-only and replay it." | **unaffected** | Verbatim. `step_events` is one of the seven `ReadStore` methods `LocalStore` implements, and ANA-9 §7.5's replay query is already asserted to be identical text against SQLite (`:968-975`). A local-only chat replays from `local.sqlite` with no new query shape. |
| `R-TUI-1` | `:247-248` | "Keyboard driven, mouse optional. Top bar: workspace or project, box, **Postgres state**, active run count. …" | **extend** the one phrase | Replace "Postgres state" with "store state, distinguishing online, connecting, offline since T, and local-only (no server configured)". Everything else unchanged. Without this the distinction has a label arm but no requirement backing, and today all three of no-DSN, `--offline` and a genuine server drop render the identical `offline · <age>` (pinned at `tests/connect.rs:179` and `:241`). |
| `R-TUI-8` | `:264-265` | "Settings tab: agent registry with quota, box profile with capability edits, item kinds and step graphs per project, secret provider, caps, scheduler window." | **extend** the enumerated list | Add: "Postgres connection: whether a DSN is stored, a masked field to enter or replace it, an action to clear it, and the state of the last attempt. What is typed goes to the OS keyring and nowhere else (`R-STO-1`); it is not echoed, not logged, and not written to any file." **This wording supersedes the earlier proposal** ("… how to set or clear it …"), which is ambiguous between instructions and controls; after §10.14 it is controls. **Stated plainly, because it matters:** today's citation of `R-TUI-8` for a DSN section — including this document's own HANDOFF entry — is an **extension of the requirement, not a reading of it.** The list names no connection or DSN section; "secret provider" is `R-SEC`/MOD-10's agent provider config, and `:225` says `htui`'s own credentials "are never exposed to a session", so it is explicitly not this. Whether an in-app DSN *field* is permitted is a separate and larger question (§10.14). |
| `R-NF-3` | `:290-291` | "All long operations (probe, cache refresh, sessions) run off the UI thread; the TUI never blocks on network or subprocess I/O." | **unaffected**, and binding | Verbatim. It binds the two writes this flow adds — the `box.toml` marker and any keyring write — even though neither is network I/O, because both are blocking syscalls issued from a key handler. Enforcement is structural (`lib.rs:6-9`): no view holds a store handle. Worth one explicit sentence because "it is just a local file, do it inline" is the obvious temptation. **After §4.9 the second clause is a certainty rather than a conditional**, and the read side (`ConnectionInfo` calling `secret::get_dsn`) takes the same treatment. |

**Rows added after §10.8 was answered (runs in scope).**

| ID | Line | Current wording (verbatim) | Fate | Proposed replacement |
|---|---|---|---|---|
| `R-AGT-4` | `:138-141` | "Agent registry **in Postgres**: name, transport (`acp` or `cli`), launch command, model list, billing mode (`subscription` or `per_token`), default model, enabled per box. Version one entries: `claude` and `agy`. …" | **amend** the two words | "Agent registry in the store that owns the box: Postgres for a box with a server configured, the local store for a box with none. Fields: name, transport …". Without this, `R-AGT-8`'s priority list has no legal home on a local box and every seeded graph refuses at admission (ANA-2 risk 14, `docs/ANA-2.md:2077`), because "MOD-2's seed fills `agent` on first connect" and a local-only box never connects. §5.6's `seed_bootstrap` seeds it instead. |
| `R-AGT-6` | `:144-145` | "Autodiscovery: on box registration and on demand, probe `PATH` for known agent binaries and ACP adapters, record version, mark enabled on that box. Manual entries allowed." | **unaffected in text; a seam consequence** | Verbatim. The consequence is on §7.1's refusal table, not on the requirement: `upsert_agent` and `upsert_agent_box` must **succeed** on `LocalStore`, where §7.1 as first written refused them. |
| `R-AGT-7` | `:146-148` | "Quota tracking per agent: … enforced by cancelling the session when exceeded." | **unaffected** | Verbatim. ANA-4 §7 puts enforcement in the recorder, which is store-agnostic; the caps live in `project.settings` and `app_setting`, whose local counterpart is `local_setting` (§5.4). |
| `R-PRM-4` | `:211-212` | "Prompt templates per phase are versioned rows **in Postgres** with a documented placeholder contract, editable in the TUI." | **amend** the two words | "… are versioned rows in the store that owns the project, with a documented placeholder contract, editable in the TUI." §5.3 already puts `prompt_template` in the local schema; this document's earlier justification for it ("included even though nothing local reads them") is withdrawn in §4.8 — a local run reads them at stage 3. |
| `R-SKL-1` | `:213` | "Skill library **in Postgres**: name, description, versioned markdown body." | **amend** the two words | "Skill library in the store that owns the project: name, description, versioned markdown body." `R-PRM-1` (`:202-206`) puts "bound skill text" in every step prompt, so a local run cannot assemble one without `skill` / `skill_version` / `skill_binding`. §5.3's Set A adds all three. MOD-9 still owns the editor. |
| `R-SKL-2` | `:214-215` | "Bindings at project level and at phase level; a phase binding overrides a project binding of the same skill. A binding pins a version or follows latest." | **unaffected in text; one schema note** | Verbatim. `skill_binding`'s `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)` is why Postgres 16 is the floor (`0001_init.sql:429`); SQLite treats `NULL`s as distinct in `UNIQUE`, so the local expression is a `UNIQUE` over `(skill_id, project_id, COALESCE(phase_id, ''))`. Same semantics, different spelling (§5.3). |
| `R-ORCH-1..11` | `:154-186` | The whole orchestration contract. | **unaffected, every one** | Verbatim, and this is the payoff of `docs/ANA-2.md` §2 invariant 10 rather than a coincidence: the orchestrator crate depends on `htui-core`, so the engine is generic over `WriteStore` and every `R-ORCH` mechanism is expressed against the seam rather than against Postgres. §4.8 records the four mechanisms whose *SQL* does not port (the advisory lock, `FOR UPDATE`, `&&` over `repo_scope`, `<@` over tag arrays) — none of them is a requirement change. |
| `R-ORCH-12` | `:187-189` | "Remote dispatch: … Version one stores the target box and executes only when it is the local box." | **unaffected** | Verbatim, and trivially satisfied locally: the local `box` table holds the one row `seed_bootstrap` writes, so `target_box_id = executing_box_id` always (§4.8). |
| `R-ORCH-13` | `:190` | "Scheduling: run the queue inside a time window on a chosen box." | **unaffected** | Verbatim. ANA-2 reserves `scheduler_window` as an `app_setting` key; locally it is a `local_setting` key (§5.4), stored and not enforced in v1 exactly as ANA-2 says. |
| `R-SEC-4` | `:229-230` | "When the provider is unreachable, runs needing its secrets are refused with a clear error; there is no plaintext fallback." | **unaffected**, and load-bearing | Verbatim. The secret provider is a network service and is orthogonal to which store holds the rows, so a local box's run refuses on exactly the same rule. Named explicitly because "local-only means no network" is the obvious wrong inference: local-only means no *Postgres*. `R-NF-2` (`:289`) is satisfied more strongly on such a box, not less. |

**Rows added after §10.14 was answered (in-app DSN entry).** Only `R-TUI-8` above needed amended
wording. The rest are recorded as unaffected because each is a useful result:

| ID | Line | Fate | Why it is stated |
|---|---|---|---|
| `R-STO-1` s.2 | `:111-113` | **unaffected, verbatim — and it is the sentence that permits the field** | It constrains *where the secret rests*, not how it is typed. The field's only sink is `secret::set_dsn` (`secret.rs:38-40`), the same sink `--set-dsn` uses (`lib.rs:122`). Its status upgrades from "unaffected" to "unaffected and now load-bearing on a second path"; §12 criterion 20 is what keeps it true. |
| `R-STO-2` | `:114` | **unaffected, verbatim** | TLS is whatever the DSN's `sslmode=` says and `PgConnectOptions::from_str` handles it; the field changes the input method, not the string. One consequence for the *implementation*: a bad `sslmode=` is the single sqlx 0.9.0 parse error that quotes its own input, so §4.9(5)'s fixed vocabulary must cover it explicitly rather than passing it through. |
| `R-SEC-1` | `:221-222` | **unaffected, verbatim** | `SecretProvider` is the agents' project secrets. The DSN is not one and must not become one — routing it through `SecretProvider` would make it resolvable into a run environment, which `R-SEC-2` forbids. |
| `R-SEC-2` | `:223-225` | **unaffected, verbatim — and reinforced** | Sentence 2 is exactly the boundary an in-app field could be imagined to weaken, and it does not: nothing in the field's path crosses `agent_worker`. Recorded so §10.14's reversal is not read as widening the blast radius. |
| `R-SEC-3` | `:227-230` | **unaffected in text — and it does not cover the field, which must be said plainly** | It binds transcript rows; the DSN field produces none. A reader who assumes the scrubber protects the field will design against a masking pass that never runs on this path. What §4.9 borrows is only the discipline: fail closed, persist nothing on failure. |
| `R-SEC-4` | `:229-230` | **unaffected, verbatim** | About the agent secret provider. Listed to close the `R-SEC-*` sweep. |
| `R-ID-7` | `:41-42` | **unaffected in text** | The DSN field is neither a transcript nor tool output. The `R-ID-7` row above — about local `session_event` rows, §10.6 — is a different question and is untouched by this. |

### 6.2 `docs/ANA-9.md`

Using the stale-premise shape ANA-2 §1 established for exactly this.

| Section | Statement | Fate |
|---|---|---|
| §2 invariant 1 (`:48-50`) | "One writable store. Only Postgres accepts writes. The cache is written by the refresh task alone … Enforced by the store trait split: `ReadStore` for both backends, `WriteStore` implemented only by the Postgres backend." | **amend** s.1, s.2, s.4. Replacement is in-tree at `backend.rs:11-17`: "nothing writes to Postgres unless the backend is `Backend::Online`, which is what `writable()` still answers for", plus "`WriteStore` is implemented by the Postgres, in-memory and local stores; only the Postgres one is reachable through `writable()`". Sentence 3 **unaffected**. |
| §2 invariant 2 (`:51-53`) | "Keys are minted online and never reused." | **amend** clause 1 → "minted by the store that owns the project"; clause 2 preserved with §4.3's mechanical guarantee and §4.6 conflict 7's sealing rule. |
| §2 invariant 4 (`:56-58`) | "Nothing about a run exists only on one box. … the offline chat buffer is a queue in front of that table, not a second store." | **amend**, bounded in time, moving with `R-HIS-1`. |
| §4.2 step 4 (`:187`) | "Offline mode never edits, so there is no queued write to reconcile (`R-STO-4`)." | **amend.** True of `Backend::Offline`; false of `Backend::Local`. The compare-and-set verdict at `:177` is **unaffected** and ports to SQLite unchanged. |
| §4.4 layout (`:284-288`) | The `<config_dir>/htui/` tree: `cache/<db_fingerprint>/cache.sqlite`, `cache/<db_fingerprint>/pending/*.jsonl`, `box.toml`. | **extend** with `local/local.sqlite`, a sibling of `cache/` and `box.toml`, explicitly **not** under a fingerprint directory. §8 gives the tree. |
| §4.4 not-mirrored list (`:309-311`) | "Not mirrored: revisions, skills, templates, graphs, `agent_box`, counters, settings, command queue - none is browsed offline." | List **unaffected**; the **warrant clause** lapses. Those tables are now created locally on a local-only box rather than browsed offline. |
| §4.4 WAL note (`:290`) | "`cache.sqlite` runs in WAL mode: the refresh task is the single writer, the TUI is a reader …" | **unaffected**, verbatim, because §4.1 put the second writer in a different file. Under option A or C it would have had to be struck. |
| §6.1 (`:851-853`) | "`PgStore: WriteStore`. `CacheStore: ReadStore` only. `MemStore: WriteStore` … The TUI holds a `Backend` enum { Online(PgStore, CacheStore), Offline(CacheStore) } and every write path is unreachable in `Offline`, at the type level rather than by a runtime flag." | **amend** the enum shape (§5.2 supplies it; the shipped enum already has three arms, so a fourth has precedent) and the final clause → "every write path **to Postgres** is unreachable outside `Online`". "`CacheStore: ReadStore` only" stays **verbatim**. Add "`LocalStore: WriteStore`". |
| §6.3 (`:873-878`) | "Who writes the cache. Only the refresh task. … it has one writer and one direction." | **unaffected**, verbatim. This is the sharpest single criterion between §4.1's options and the reason option B wins on more than one axis. |
| §7.1 (`:886-901`) | The mint CTE and the importer variant. | **extend.** The Postgres statement is unchanged. The local equivalent is three statements in one `BEGIN IMMEDIATE` transaction (§7.3), because SQLite has no data-modifying CTEs and `RETURNING` cannot be a subquery. The importer variant gains a second consumer (§4.3). |
| §5.10 (`:820-824`) | "Seed on first connect. One `app_user`, the `capability_tag` vocabulary, agents … Per new project: the five `item_kind` rows with their default graphs and phases … and one `prompt_template` version 1 per phase name." | **extend.** The same seed runs locally, on first *use* rather than first connect, minus `capability_tag` and `phase_agent` (§5.3). |
| §11 criterion 5 (`:1047-1048`) | "Warm start (cache present, Postgres reachable) renders the Backlog tab under one second …; cold start with Postgres unreachable renders from the cache." | **restate.** A third clause is needed for the local-only start, and whether it carries the same budget is §10.4. |
| §11 criterion 7 (`:1051-1052`) | "A chat session started offline lands in Postgres on reconnect with `seq` order preserved and the pending file removed; running the upload twice does not duplicate rows." | **restate.** It continues to describe `Backend::Offline` exactly and stays as MOD-6's acceptance. It no longer describes the **no-DSN** case, because §4.6 conflict 2 moves local-only chat off `BufferedWriter` and into `local.sqlite`, where there is no pending file to remove and no upload until the adoption item lands. |
| §11 criteria 1-4 and 6 | Migrations, mint concurrency, compare-and-set, replay, the overlap window. | **unaffected.** MOD-6's acceptance contract survives; `docs/decisions/mod/mod-6.md` records criteria 1-4, 6 and 7 as pinned by tests, and only 5 and 7 are touched above. |

### 6.3 Downstream sentences whose warrant lapses if `R-STO-4` is amended

Several concluded documents justify a design choice with "`R-STO-4` forbids it". Amending
`R-STO-4` silently unmoors every one of them. This is the complete set found; each needs a
one-sentence restatement at whichever item next touches the document, and none of them changes a
verdict.

| Document | Line | Sentence | What lapses |
|---|---|---|---|
| `docs/ANA-2.md` §3 | `:222-227` | "The mirror carries the run tables and not the graph tables. … Offline, therefore, the only route to a graph is `run.graph_snapshot`, which is mirrored. **That is not a defect: `R-STO-4` starts no runs offline.**" | The justification clause only. The **fact** survives, on a narrower warrant: the mirror still needs no graph tables, because a mirrored project's graphs live in Postgres and a locally-originated project is not on the `Offline` path at all (§4.8's placement table). Restate as "no run starts against a project the local store does not own, and none starts offline". |
| `docs/ANA-2.md` §4.9 | `:1325-1340` | "The offline window. `R-STO-4` forbids *starting* a run offline; it does not say what happens to one already live, and the answer is **forced by the type system** rather than chosen: 1. On `go_offline`, the orchestrator stops admitting and stops scheduling new steps. **It cannot write, so it writes nothing.**" | "Forced by the type system" and "it cannot write". Both remain true for `Backend::Offline`, which is the arm §4.9 is about, and §4.8's placement table keeps `Local` off that arm entirely — so "it cannot write, so it writes nothing" still holds wherever §4.9 asserts it. **The conditional has fired**: §10.8 is answered (b), and ANA-2 §4.9 is re-opened for one thing only — the lease and the recovery sweep now run against `LocalStore` too, with the cross-box half vacuous and the crash-recovery half fully live (§4.8). |
| `docs/ANA-2.md` §4.9 option row | `:1258` | "Cancel a live run when the store goes offline \| Rejected \| It cannot be recorded: `Backend::writable()` returns `None` offline, so the cancellation itself is unwritable." | **Unaffected.** `writable()` still returns `None` on every non-`Online` arm, including `Local` (§5.2). Listed because the sentence names `writable()` and a careless reading of §4.1 would think it moved. |
| `docs/ANA-2.md` §8 | `:1699-1705` | The read-placement rule: `step_graph`, `step_graph_phase`, `phase_agent`, `prompt_template`, `agent`, `agent_box`, `repo`, `repo_box_path`, `command_run` and `app_setting` are `PgStore`-inherent "because they are **not mirrored**, so putting their readers on `ReadStore` would oblige `CacheStore` to answer questions the mirror cannot." | The **reason** is unaffected — it is about `CacheStore`, which this document does not touch — so the `ReadStore`-versus-inherent split stands verbatim. What changes now that §10.8 is answered is the **dispatcher**: the 14-row inherent column gains a `LocalStore` twin and `Backend` gains a fourth arm on each, following the `agents()` precedent (`backend.rs:292-298`, `traits.rs:11-14`). One column added, not a re-derivation; §7.1 gives the corrected counts. |
| `docs/ANA-5.md` §4.1 | `:418-422` | "*Free-standing chat prompts.* `R-STO-4` starts no graph run offline, and `prompt_template`, `skill*` and `step_graph*` are not mirrored …, so a graph step never needs a template offline." | The warrant. The conclusion survives for `Backend::Offline`, whose arm ANA-5 §4.1 is about. It does **not** survive for `Backend::Local`: `prompt_template` is read there at stage 3, so §5.3's "unread, for adoption fidelity" is withdrawn, and `skill*` join it for the same reason (`R-PRM-1`'s bound skill text). Restate the condition as "without a store that owns the project". |
| `docs/ANA-9.md` §4.2 step 4 | `:187` | "Offline mode never edits, so there is no queued write to reconcile (`R-STO-4`)." | Amended outright — see §6.2. |
| `docs/ANA-9.md` §4.4 | `:309-311` | "… none is browsed offline." | Warrant clause — see §6.2. |
| `docs/ANA-9.md` §11 criterion 7 | `:1051-1052` | The offline-chat-upload acceptance. | Restated — see §6.2. |
| `CONCEPTS.md` | `:25-26` | "**Offline is read-only** from a per-box cache; keys are minted online, so no offline collision and no sync engine [R-STO-3, R-STO-4, R-ENT-7]." | The whole bullet. See §6.4. |

### 6.4 `CONCEPTS.md`

Edited at close-out only, and capped at 6 144 bytes, so each amendment must fit in one or two
sentences net of what it replaces.

| Line | Current | Fate |
|---|---|---|
| `:20-21` | "**Postgres holds everything**: items, documents, runs, full transcripts, skills, templates, box profiles, agent registry, settings [R-ID-3]. Credentials live in the OS keyring [R-STO-1]." | **amend** the first clause to "Wherever a server is configured, Postgres holds everything …". Sentence 2 verbatim. |
| `:25-26` | "**Offline is read-only** from a per-box cache; keys are minted online, so no offline collision and no sync engine [R-STO-3, R-STO-4, R-ENT-7]." | **amend.** Proposed: "**Offline is read-only** from a per-box cache. A box with no server configured instead writes a separate local store; its project counters have one writer each, so there is still no offline collision and no sync engine [R-STO-3, R-STO-4, R-ENT-7, ANA-10]." The "no sync engine" clause is preserved deliberately, and §4.4's verdict is what earns the right to keep it. |
| `:31-32` | "**One cache writer.** The per-box cache is a SQLite mirror written only by the refresh task, never by user actions; live reads while connected go to Postgres (`docs/ANA-9.md` §6.3)." | **unaffected**, verbatim. |
| `:28-30` | "**No silent merges.** …" | **unaffected**, verbatim. §4.4 adds no merge; the two stores hold disjoint row sets. |

---

## 7. Contracts and key statements

### 7.1 The store surface `LocalStore` must satisfy

All seven `ReadStore` methods and all nine `WriteStore` methods, with the signatures **verbatim**
from `traits.rs:30-45` and `:57-152`, all `Send + Sync`, all returning
`htui_core::store::Result<T>`, all under `#[allow(async_fn_in_trait)]`. Plus the six inherent
`Backend` reads the new arm must answer as an exhaustive `match self` (`backend.rs:222-296`):
`workspaces()`, `box_info()`, `active_runs(&Scope)`, `projects(&Scope)`, `agents()`, `this_user()`.
Plus the five non-async accessors of §5.2 and the two mutators `gave_up()` and `went_offline()`
(`backend.rs:180-208`), which on `Local` are both no-ops: there is nothing to give up on. Plus the
**fifteen further inherent reads** MOD-4 adds (`docs/ANA-2.md:1711-1724`), each with a `Local` arm —
see the surface delta at the end of this subsection.

Refusal vocabulary, and it is a real decision rather than a detail:

| Situation | Error | Why |
|---|---|---|
| A write targeting a project the local store does not own, or any local write while the backend is `Online` or `Offline` | `StoreError::ReadOnly(&'static str)` | Declared at `error.rs:23-25` and **never constructed anywhere in the workspace** today. This is its first use and its meaning is exactly right: the store refuses this write. **Corrected after §10.8 was answered** — this row previously read "a run, an agent-registry write", and both are now writes the local store must *accept*; see the corrected list below. |
| A write that needs a server the box does not have (minting into a mirrored project) | `StoreError::ReadOnly` too | Permanent until a server appears, which is not "retry later". |
| A genuine local I/O or constraint failure | `StoreError::Backend` / `Constraint` | Unchanged. |
| *Not* used | `StoreError::Unreachable` | `BufferedWriter` uses it (`writer.rs:322-331`) and callers read it as "retry later may work" (`error.rs:26-32`), which is the wrong reading for a local-only refusal. `BufferedWriter`'s own usage is left alone; this is about the new arm. |

Concurrency contract of `local.sqlite`, stated because the mirror's is stated and the two differ:
WAL, `max_connections` 1-2 (the mirror's is 4), a busy timeout, foreign keys **ON**, `BEGIN
IMMEDIATE` for every write transaction so contention is a wait at the start rather than a failure in
the middle, and no `.await` on non-database work while a transaction is held. That last rule is not
hypothetical: the chat-run write path is exactly where an agent spawn would otherwise sit inside a
transaction.

**Multi-process access, decided.** The mirror's design treats a second `htui` process as "just
another reader" (`cache/mod.rs:64-66`), which stops being true for a writable primary store: SQLite
permits many readers and exactly one writer, and the loser of a race stalls up to the busy timeout.
The rule adopted here is an **advisory single-writer lock**, `<config_root>/local/local.lock`, taken
at `LocalStore::open`. A second process that cannot take it opens the local store **read-only**, and
the top bar says so with a distinct label suffix rather than failing to start. Rejected: accepting
the busy-timeout stall (it can land near the UI path and `R-NF-3` forbids that), and a hard
single-instance refusal (a second `htui` on one box is a normal thing to do and must keep working
for reading). §10.17 puts the choice to the maintainer, because it is a usability call as much as a
correctness one.

**The runs delta on the store surface, with this document's own earlier estimate corrected.**
§10.8's original row estimated that runs-in-scope grow `LocalStore` "by roughly 16 writes and 14
inherent reads". Counted against `docs/ANA-2.md` §8 and `crates/htui-core/src/store/traits.rs` as
they actually stand, that is wrong in three ways: it counts table **rows** rather than method names,
it inherits ANA-2's own miscount, and it omits an entire table.

| Set | ANA-2 §8 location | Rows | Method names | Exists today? |
|---|---|---|---|---|
| `WriteStore` writes | `:1739-1755` | 17 | **18** — the row at `:1744` names `transition_run` **and** `transition_step` | **None.** `WriteStore` is nine methods and ends at a comment (`traits.rs:57-153`). |
| `ReadStore` reads | `:1730-1733` | 4 | **6** — `:1732` names `run(id)` and `run_steps(run)`; `:1733` names `step_trees(step)` and `step_commits(step)` | **None.** `ReadStore` is seven methods (`traits.rs:30-45`). And these six oblige `CacheStore` too, because `CacheStore: ReadStore`. |
| `PgStore`-inherent reads | `:1711-1724` | 14 | **16** — `:1717` names `agents()` and `agent_boxes(box)`; `:1719` names `repos(project)` and `repo_paths(box)` | **One of sixteen.** `agents()` exists on all three stores and on `Backend` (`backend.rs:292-298`). The other **15 are new**. |

**So `LocalStore` grows by 18 + 6 + 15 = 39 async methods**, taking it from the 22 above (seven
`ReadStore`, nine `WriteStore`, six inherent `Backend` reads) to **61**, plus `seed_bootstrap`.
ANA-2's own summary sentence — "That is sixteen methods and roughly as many cases"
(`docs/ANA-2.md:1760`) — undercounts its own table by two, and `HANDOFF.md:186` inherits the error;
§9.1 records the correction as a MOD-4 HANDOFF amendment.

**Where the 15 inherent reads go.** They become `Backend`-inherent methods with a four-arm
`match self`, `Local` answering from `local.sqlite` and `Offline` refusing. The precedent is exact
and in-tree — `traits.rs:11-14` explains why `agents()` is inherent rather than on `ReadStore`, and
MOD-2 milestone 4 later gave it an `Offline` arm once `agent` became mirrored
(`cache_migrations/0002_agent_mirror.sql`, `backend.rs:280-298`).

**Refusal vocabulary, corrected.** The table above previously called a run and an agent-registry
write "a write the local store will never accept". Both are wrong now. `upsert_agent` and
`upsert_agent_box` must **succeed** locally, or a local box has no candidate agent and ANA-2 risk 14
(`:2077`) — "the seeded graphs cannot run until agents exist … MOD-2's seed fills `agent` on first
connect" — becomes permanent on a box that never connects. `seed_bootstrap` (§5.6) therefore seeds
`agent` locally too, and `R-AGT-6`'s autodiscovery writes `agent_box` locally. What is left for
`StoreError::ReadOnly` is a genuinely short list, and it is the better list: (a) any write targeting
a **mirrored** project; (b) any write to the local store while the backend is `Online` or `Offline`
(§5.2, §12 criterion 8); (c) a mint or a run in a project whose `sealed_at` is set (§4.6 conflict 7);
(d) a `local_meta.store_id` / `box.toml.local_store_id` mismatch (§12 criterion 19).

**Two hierarchy-creation methods §5.6 missed, exposed by runs.** §5.6 names three
(`create_workspace`, `create_project`, `create_item_kind`) and assigns them to MOD-15. A graph run
resolves its trees from `repo` and `repo_box_path` (`docs/ANA-2.md` §4.6 `:905`, and its
`repo_paths(box)` read at `:1719`), and nothing creates either row on **either** backend today.
`HANDOFF.md:246-247` already puts "repos with primary flag and per-box paths" in MOD-15's scope, so
the owner is unchanged; the seam needs two more names so the implementation MOD and MOD-4 can code
against them:

```rust
// crates/htui-core/src/store/traits.rs, appended to WriteStore beside §5.6's three.
async fn create_repo(&self, new: NewRepo) -> Result<Repo>;
async fn set_repo_box_path(&self, repo: RepoId, box_id: BoxId, path: &str) -> Result<()>;
```

`NewRepo` carries `is_primary`, because `uq_repo_primary` is a partial unique index over
`repo(project_id) WHERE is_primary` (`0001_init.sql:196`) and a second primary must fail at the
schema rather than in a caller.

### 7.2 The reply that carries the state

`StoreReply::StoreState` today carries only `label: String` and `migrations_pending: Option<usize>`
(`store_worker.rs:218-227`). It gains one field:

```rust
StoreReply::StoreState {
    label: String,
    migrations_pending: Option<usize>,
    /// Whether this box has no DSN stored, so the shell can distinguish "no server configured"
    /// from "server known, unreachable". A view must never infer this from `label`
    /// (the `ChatAccepted::writer_label` rule, store_worker.rs:209-216).
    no_server_configured: bool,
}
```

Two new request/reply pairs, both modelled on `ApplyMigrations`/`MigrationsApplied`
(`store_worker.rs:127-128`, `app/update.rs:220-222`), because no view may hold a store handle:

- `StoreRequest::AckLocalOnlyNotice` → writes `local_notice_shown = true` through `identity::store`
  and answers with a plain acknowledgement. One small `std::fs` write, atomic
  temp-file-then-rename.
- `StoreRequest::ConnectionInfo` → answers whether a DSN is stored and the last attempt's outcome,
  for the Settings connection section. It reads the keyring, which is blocking FFI, so it runs in
  `spawn_blocking` on the worker.

The `StoreRequest` enum has seventeen members today and the only write-shaped ones are
`ApplyMigrations` and the four `Chat*` (`store_worker.rs:132-153`). There is no settings read or
write of any kind, and MOD-15 is separately on the hook for exposing ten `app_setting` keys, so the
two items share this missing plumbing and should agree on its shape. That is why the pairs are named
here rather than left to whichever lands first.

### 7.3 The local mint

ANA-9 §7.1's Postgres mint is one statement with a CTE. It does not port: SQLite has no
data-modifying CTEs, and `RETURNING` "cannot be used as a subquery" and is available only in
top-level `DELETE`/`INSERT`/`UPDATE`. The local equivalent is three statements in one transaction.
Atomicity is preserved; the single-statement guarantee is not, and that is the whole difference.

```sql
BEGIN IMMEDIATE;

-- 1. The kind-belongs-to-project guard FIRST, so a refused mint burns no number.
--    This is the SQLite equivalent of folding the guard into the CTE's SELECT
--    (crates/htui-store/src/pg/write.rs:57-72; crates/htui-core/src/store/mem.rs:562-573).
--    Zero rows here aborts the transaction with StoreError::Constraint and touches no counter.
SELECT k.key_prefix
  FROM item_kind k
  JOIN project p ON p.id = k.project_id
 WHERE k.id = ?kind
   AND k.project_id = ?project
   AND p.sealed_at IS NULL;          -- §4.6 conflict 7: an adopted project never mints locally

-- 2. The counter moves before the row lands, exactly as §4.1 requires.
INSERT INTO item_key_counter (project_id, prefix, last_value)
VALUES (?project, ?prefix, 1)
ON CONFLICT (project_id, prefix)
DO UPDATE SET last_value = item_key_counter.last_value + 1
RETURNING last_value;

-- 3. The row and its revision 1. `key` is generated; nothing binds it.
INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body, status,
                  priority, required_tags, touched_paths, step_graph_id, version,
                  created_by, created_at, updated_at, origin_box_id, local_seq)
VALUES (?id, ?project, ?kind, ?prefix, ?last_value, ...);

INSERT INTO item_revision (item_id, version, title, body, required_tags,
                           author_id, box_id, reason, created_at)
VALUES (?id, 1, ?title, ?body, ?tags, ?user, ?box, 'created', ?now);

COMMIT;
```

Three properties this preserves and one it gives up. Preserved: the counter is monotonic and
outlives the items; a rolled-back mint returns the number; `mint_writes_revision_v1` passes because
revision 1 exists in the same transaction. Given up: the mint is no longer a single statement, so a
crash between statements 2 and 3 is impossible only because the transaction covers them — which is
the same guarantee, reached differently.

The counter's **adoption** statement is the importer variant, unchanged from ANA-9 §7.1 and already
test-pinned though not in the product:

```sql
INSERT INTO item_key_counter (project_id, prefix, last_value)
VALUES ($1, $2, $3)
ON CONFLICT (project_id, prefix)
DO UPDATE SET last_value = GREATEST(item_key_counter.last_value, $3);
```

It must run in the **same transaction** as the adopted item inserts and the local `sealed_at` write.
Omitting it is a documented defect class: GitLab #519457 records 409 "Duplicated issue" via the API
and 500s in the UI, indefinitely, until someone bumps the counter by hand.

### 7.4 Reading across two stores

Only one read presents rows from both stores, and it is a concatenation of disjoint sets rather than
a merge, which is what keeps ANA-9 §2 invariant 3 intact.

```
Backend::workspaces():
    Memory  -> mem.workspaces()
    Local   -> local.workspaces()                       (every row origin = local)
    Online  -> pg.workspaces()   ++ local.workspaces()  (if the local store is open)
    Offline -> cache.workspaces() ++ local.workspaces() (if the local store is open)
```

No row can appear twice: a workspace created locally has never been on a server, and a mirrored one
is not in the local file. `WorkspaceSummary` gains an origin marker so the switcher can label the
local ones; everything below a workspace — `projects`, `items`, `item`, `links`, `documents`,
`notes`, `runs`, `step_events` — dispatches to the store that holds the workspace, so no other read
ever sees two sources. `mint_item` and the other writes dispatch the same way, which is where §4.3's
origin rule finally has a case to fire on.

---

## 8. Crate and module layout for the implementation MOD

```
crates/htui-store/
  local_migrations/
    0001_local.sql              # §5.3. Forward-only; never rebuilt.
  src/
    local/
      mod.rs                    # LocalStore, open(), pool, dir(), the advisory lock, local_meta
      read.rs                   # the 7 ReadStore methods + the 6 inherent Backend reads
      write.rs                  # the 9 WriteStore methods + seed_bootstrap()
    lib.rs                      # + pub static LOCAL_MIGRATOR, beside MIGRATOR and CACHE_MIGRATOR
    backend.rs                  # + Backend::Local, + `local` field on Online/Offline, + label arm
    writer.rs                   # + Writer::Local, + label() arm
    connect.rs                  # arm selection per §5.1; carry `dsn.is_none()` out on `Started`
    identity.rs                 # + three #[serde(default)] fields, threaded through Identity
  tests/
    local_conformance.rs        # the 20 CASES against LocalStore, with a fixture loader

crates/htui/
  src/
    store_worker.rs             # + StoreState.no_server_configured, + 2 request/reply pairs
    app/action.rs               # + TabAction::FocusSection(TabId, SectionId)
    app/state.rs                # + local_only_overlay slot, + local_only_notice_shown guard
    app/update.rs               # + offer_local_only_notice(), modelled on offer_migration_prompt
    app/mod.rs                  # + overlay registration; the notice replaces the switcher (§4.5)
    ui/overlay/local_only.rs    # copied from migration_prompt.rs end to end
    ui/tabs/settings/mod.rs     # + focus-by-SectionId, alongside cycle_next / cycle_prev
```

On-disk layout, extending ANA-9 §4.4's tree (`docs/ANA-9.md:284-288`):

```
<config_dir>/htui/
  box.toml                             # box_id, hostname, + §5.5's three fields
  cache/<db_fingerprint>/cache.sqlite  # unchanged; disposable by design
  cache/<db_fingerprint>/pending/*.jsonl
  local/local.sqlite                   # NEW: durable, never rebuilt, outside every fingerprint dir
  local/local.lock                     # NEW: advisory single-writer lock (§7.1)
```

The path is deliberately **not** under `cache/`. `identity::cache_dir` is
`root.join("cache").join(db_fingerprint(dsn))` (`identity.rs:152-154`), that name changes from the
literal `"offline"` to `sha256(host:port/dbname)` the moment a DSN is set, and nothing in the tree
ever enumerates `<root>/cache/*` — there are three `join("cache")` sites in total. Anything under
that directory becomes unreachable on a DSN change. The exact name and nesting of `local/` beyond
"not under `cache/`" is the maintainer's.

**The crate boundary question, priced and left open.** Local writes inside `htui-store` get no
compile-time query checking, because one crate has one `DATABASE_URL` and it is the Postgres one
(`cache/read.rs:18-19`), `sqlx.toml` is per-crate, and its only multi-database key is
`database-url-var` — whose own documented multi-database example is "break it up into multiple
crates". A second crate (`htui-store-local`, its own `sqlx.toml`, its own `LOCAL_DATABASE_URL`) buys
compile-time checking at the cost of a crate boundary through the middle of the store layer. Note
also that a shared `.sqlx` cache keys files by SHA-256 of the query **text** and records `db_name`
per file, so byte-identical SQL prepared for both dialects collides — portable SQL is actively
penalised and divergent SQL is the safe path either way. §10.15.

---

## 9. Phasing and downstream impact

### 9.1 Milestones

Ten milestones. **M0 to M6, plus M2b, belong to the implementation MOD minted at close-out. M7 is a
separately funded adoption item, with its own `HANDOFF.md` id, also minted at close-out** — the
deferral is visible rather than implied, which is the whole lesson of §4.4's external evidence.
**M8 belongs to MOD-4** (local graph runs, §4.8) and **M9 is deferred** (in-process transition out of
local-only, §10.13); neither waits on M7.

| # | Lands | Unblocks / why here |
|---|---|---|
| **M0** | **Names only, no code.** The seam names this document owes MOD-15: `SectionId("connection")` and `TabAction::FocusSection(TabId, SectionId)` (§4.5, §5.2), **and now `SettingsSection::captures_input` and `MaskedField`** (§4.9). Plus the proposed `R-TUI-1` and `R-TUI-8` wording going to the maintainer — `R-TUI-8`'s being §4.9's wording, not §6.1's original. | Unblocks MOD-15 to proceed in parallel and settles the join `HANDOFF.md:255-258` currently leaves to "whichever lands second". Costs nothing and can land the day this document closes. One addition: MOD-15 must know before it starts that the *field* inside its section is not its work (§9.4), or it will build one. |
| **M1** | **Distinguish the state, write nothing.** Carry `dsn.is_none()` out of `connect::start` (today `connecting = !opts.offline && dsn.is_some()` at `connect.rs:189` collapses it), add the fifth `Backend::label()` output, add `no_server_configured` to `StoreReply::StoreState`, fix the two pinned label tests and the 100×30 snapshot suite. | Real user value on its own: the `offline · 0s` conflation is fixed. Needs no schema, no writable store and no requirement amendment beyond `R-TUI-1`. **This is the milestone that still lands if the maintainer refuses the `R-STO-1` amendment**, which is why it is second and not fifth. |
| **M2** | **Local state file and the first-run notice.** `box.toml` gains §5.5's three `#[serde(default)]` fields threaded through `Identity`; `StoreRequest::AckLocalOnlyNotice` and `ConnectionInfo`; the overlay copied from `MigrationPrompt`, replacing the switcher in the single `startup_overlay` slot. | Still no writable store. The copy at this point says only "no server configured"; **M2b supplies the field it points at.** M2 alone still points at `htui --set-dsn`, which is why M2b is the next milestone and not a later one. Depends on M1 for the signal. |
| **M2b** | **In-app DSN entry** (§4.9). `crates/htui/src/ui/masked.rs` — `MaskedField` with `Zeroizing<String>`, `String::with_capacity(256)`, control-character rejection, a render-time window with `…` overflow and no reveal toggle. `SettingsSection::captures_input(&self) -> bool` with a `false` default body, checked in `SettingsTab::on_key` (`settings/mod.rs:197-213`) **before** the `h`/`l`/`[`/`]` cycle match. `StoreRequest::SetDsn(Dsn)` / `ClearDsn`, joining M2's `ConnectionInfo`, with `Dsn`'s hand-written `Debug` and `StoreRequest::name()` arms. The worker's first two `spawn_blocking` calls. `zeroize` promoted from transitive to a declared `[workspace.dependencies]` entry. §4.5's revised overlay copy, and `lib.rs:109-115`'s doc comment amended to say `--set-dsn` is now one of two paths. | **This is the milestone that satisfies the item's stated objective** — start with no DSN, be told so, define one inside the program. It depends on M2 and on nothing in M3+, so it lands before the store work. It also makes M0 + M1 + M2 + M2b a complete, shippable answer to the HANDOFF item on its own (§9.2). |
| **M3** | **`LocalStore` skeleton.** `local_migrations/0001_local.sql` (§5.3), `LOCAL_MIGRATOR`, the fourth `Backend` arm with its 19 match sites, the `local: Option<LocalStore>` field on `Online`/`Offline` (§4.6 conflict 3), read paths wired (§7.4), the advisory lock (§7.1), the conformance harness added and expected red on the write cases. Plus an **export path** — a file copy or `VACUUM INTO` of `local.sqlite` to a user-named location — so "temporarily local" can never become "permanently trapped" even if M7 is never funded. | Decide here, explicitly and in writing: `is_writable()` false, `writable()` `None`, `cache()` `None` on `Local`. That arm at `store_worker.rs:565-568` stays **unreachable through M8** and becomes reachable only in M9; M3 must not pre-emptively give `Backend::Local` a `CacheStore` to prepare for it. **M3 also carries §5.3's Set A in full** — the nine missing `box` columns, `project.secret_provider`/`secret_scope`, and the eleven run-half tables — deliberately front-loaded, because SQLite cannot `ALTER` a `CHECK`, a `UNIQUE` or a foreign key afterwards (§11 risk 5), so a table omitted here and needed at M8 costs the 12-step rebuild. |
| **M4** | **Bootstrap seed.** `LocalStore::seed_bootstrap` (§5.6): one `app_user`, one `box` from `box.toml`'s existing UUIDv7, and a workspace / workspace_project / project / item_kind chain with ANA-9 §5.10's graphs, phases and templates — written on the first user action, never at launch (§4.5). **Now also seeds `agent`** (the `claude`/`agy` rows MOD-2's first-connect seed writes on a server, which a local box never reaches — `docs/ANA-2.md:2077`), the `capability_tag` vocabulary, and `phase_agent` rows for the seeded graphs. | This is what silences `agent_worker.rs:344-351` and makes the shell non-empty. Without the three additions, §5.10's seed produces graphs with zero candidates and every local run refuses at admission. **Gated** on §10.7 (may a local `app_user` be minted at all) and on §5.6's hierarchy-creation split. |
| **M5** | **Local chat.** `Writer::Local` writing `run` / `run_step` / `session_event` into `local.sqlite`, retiring `Writer::Buffered` for the no-DSN case (§4.6 conflict 2), with ANA-9 §11 criterion 7 restated (§6.2) and the scrubbing seam stated (§6.1, `R-ID-7`/`R-SEC-3`). | The first thing a local-only box can actually **do**. Depends on M4 for the `box` and `app_user` rows. |
| **M6** | **Local item mint, revisions, notes, documents.** §7.3's three-statement mint, the origin-scoped guard, `item_revision`, `item_note`, `document`, `item_link`. Conformance suite green. | **Gated** on the `R-ENT-7` amendment landing (§10.3). MOD-13's `new` and `edit` land against this, and MOD-13's HANDOFF scope line "mint per §7.1" must be re-scoped first (§9.3). §4.3's origin rule becomes non-vacuous here, and not before. |
| **M7** | **Adoption. Funded** (§10.5, on §1.3's framing: it is the exit a temporary mode promises), as a separately tracked item. Explicit, confirmed, resumable; one transaction per project subtree; the `GREATEST` counter fast-forward and the `sealed_at` write inside that transaction; `created_by`/`author_id`/`box_id` remap; pre-adoption file copy; second-server refusal keyed on `system_identifier`; a UI-visible per-row status — never `pending.rs`'s log-only quarantine, because `no_delete_path` makes a refused item unremovable. | **Funded**, so §4.5's copy rule is the binding constraint instead: the overlay may not promise the move until this item ships, and gains the promise when it does. That is the Grafana/Gitea-versus-Synapse choice, made openly rather than by omission. **Its unit grew after §10.8**: a project subtree now carries `run`, `run_step`, `run_step_commit`, `run_step_tree`, `session_event`, `document` and `command_run` rows as well as items, and two new remap targets appear — `run.started_by` and `run_step.agent_id`, the second having no adopt rule at all today (§10.33, §11 risk 26). |
| **M8** | **Local graph runs. Owned by MOD-4, not by the implementation MOD.** `LocalStore` implementations of the 18 `WriteStore` methods, the 6 `ReadStore` methods and the 15 new inherent reads of §7.1, landed method-by-method beside their `MemStore` and `PgStore` twins; the fourth `Backend` arm on each inherent read; `local_migrations/0002_orchestration_local.sql` carrying §5.3's Set B; and the four ports of §4.8's second table (the in-process `(box_id, repo_id)` lock, `BEGIN IMMEDIATE` for admission, the Rust-side `repo_scope` overlap predicate, the Rust-side tag-subset test). | It is MOD-4's seam, MOD-4's migration and MOD-4's crate. Assigning it to the implementation MOD would make that MOD implement eighteen store methods whose Postgres and in-memory twins do not exist yet — the exact ordering ANA-2 §9 build step 1 forbids. **Auto mode (`R-ORCH-6`) is explicitly not in M8:** MOD-12 owns `ready_items`, which is ANA-9 §7.4 rewritten for SQLite, and it is blocked on MOD-4 anyway. |
| **M9** | **In-process transition out of local-only. Deferred, with its id minted at close-out** so the deferral is visible, on the rule §10.5 applies to adoption. `connect::reconnect_for(dsn, root, timeout) -> Reconnect` as a public factory (keeping the DSN inside `connect.rs`, which is `connect.rs:66-70`'s real property); `let mut reconnect` at `store_worker.rs:411` plus one assignment; and the expensive half — `go_online` learning to **open** `CacheStore::open(&root, &db_fingerprint(dsn), PgStore::schema_version())` rather than move an existing handle (`store_worker.rs:572-575`, `connect.rs:187`), while a `Backend::Local` and its open `local.sqlite` are still in hand. | §10.13 prices the three seams individually: one is paid by M2b anyway, one is a factory function, and only the mirror-open is genuine cost. It buys the user one avoided restart and nothing else, which is why it is last. It is **not** a prerequisite for anything — M2b delivers the objective without it. |

**The hard ordering rule against MOD-4, and it runs the opposite way from the intuition.**

> **The implementation MOD's M3 must land before MOD-4's build step 1 (`docs/ANA-2.md:1791-1794`).**

The reason is mechanical, not aesthetic. ANA-2 build step 1 is "`htui-core` first: … the sixteen
`WriteStore` methods plus the `ReadStore` reads of §8 with their conformance cases. `MemStore`
implements them all." Every `WriteStore` method obliges **every** implementor. If `LocalStore` exists
when MOD-4 starts, MOD-4 writes three implementations per method and prices them; if it does not, the
implementation MOD inherits eighteen unbudgeted methods on a store it thought was finished, and M3's
milestone note ("the conformance harness added and expected red on the write cases") silently changes
meaning from twenty cases to roughly thirty-eight.

**What must not ship before the other:**

1. **MOD-4 must not author `migrations/0003_orchestration.sql` without authoring
   `local_migrations/0002_orchestration_local.sql` in the same commit.** ANA-2 §9 already pairs the
   Postgres migration with `cache_migrations/0003_orchestration.sql`; local-only makes it a
   **three-way** fan-out. A `0003` that lands alone leaves the local schema permanently one
   generation behind, and SQLite makes catching up expensive rather than merely late.
2. **MOD-4 must not add `CHECK (fanout_index >= 0)` to any of the three schemas.** ANA-2 risk 12
   (`:2075`) names the hazard; §5.3 makes the prohibition permanent locally.
3. **MOD-4 must not ship local `shared_serialized` before §10.17 is answered.** The mode's
   serialization is a Postgres advisory lock (`docs/ANA-2.md:916`); its local substitute is sound only
   under §7.1's advisory single-writer file lock. Under §10.17(b) or (c), local `shared_serialized`
   must refuse with a named error rather than run unserialised.
4. **The implementation MOD must not ship a `Writer::Local` that refuses `upsert_agent` or
   `upsert_agent_box`**, which §7.1 as first written did. It would make M8 unreachable.
5. **The implementation MOD must not omit any Set A table from `0001_local.sql`** on the grounds that
   nothing reads it at M6. The whole point of front-loading is that SQLite cannot add the constraints
   later.

**What changes for MOD-4's own `HANDOFF.md` entry (`:177-194`), at close-out.**

1. **"sixteen `WriteStore` methods" → "eighteen"** (`HANDOFF.md:186`). ANA-2 §8's table is 17 rows and
   the row at `docs/ANA-2.md:1744` names two methods; ANA-2's own summary at `:1760` undercounts and
   this line inherits it. Add: "and the six `ReadStore` reads of `:1730-1733` and the fifteen new
   inherent reads of `:1711-1724`, each now obliging **three** stores — `MemStore`, `PgStore` and
   ANA-10's `LocalStore` — not two."
2. **The migration list gains a third file** (`:182-183`): "migration `0003_orchestration.sql`,
   `cache_migrations/0003_orchestration.sql` **and `local_migrations/0002_orchestration_local.sql`
   (ANA-10 §5.3 Set B), all three in one commit**".
3. **The blocker line changes** (`:193-194`): "Blocked on MOD-2 **and on ANA-10's implementation MOD
   milestone M3**, which authors `local_migrations/0001_local.sql` and the fourth `Backend` arm; see
   ANA-10 §9.1's ordering rule."
4. **Add the placement note:** the `PgStore`-inherent reads of ANA-2 §8 become `Backend`-inherent with
   a four-arm `match self`, following the `agents()` precedent (`backend.rs:292-298`,
   `traits.rs:11-14`). ANA-2 §8's split rule is unchanged; only its dispatcher is.
5. **Add the two prohibitions:** no `CHECK (fanout_index >= 0)` on any of the three schemas; no local
   `shared_serialized` before §10.17 is answered.

MOD-12's entry gains one line: `ready_items` is ANA-9 §7.4 rewritten for SQLite as well as
implemented for Postgres, because `<@` and `= ANY($projects)` have no SQLite spelling (§4.8).

### 9.2 The shortest useful path, if the whole plan is not funded

M0 + M1 alone fix a real, test-pinned defect — three distinct states rendering one string — and cost
one match arm, one reply field and two test updates. **M0 + M1 + M2 + M2b close the HANDOFF item's
stated objective outright** — the user starts with no DSN, is told so, and defines one inside the
program — and need no schema, no writable store and no requirement amendment beyond `R-TUI-1` and
`R-TUI-8`. Everything from M3 on depends on the `R-STO-1` decision. Stating this explicitly is
deliberate: it means the maintainer can refuse the largest amendment in §10 and still ship the part
of this document that is unambiguously right.

### 9.3 What MOD-13 and MOD-15 must not ship ahead of this

`HANDOFF.md:238-240` already forbids MOD-13 a local-only edit path and `HANDOFF.md:245-258` puts
MOD-15's create paths behind this verdict. These are the specific prohibitions.

**MOD-13** (`R-TUI-2`, `R-ENT-5`, `R-ENT-10..12`) must not ship:

1. A `new` or `edit` action reachable while the backend is `Local` or `Offline`.
2. Any local `mint_item` or local `item_key_counter` — that is M6's, gated on §10.3.
3. A compare-and-set or divergence view backed by anything but `PgStore` or `MemStore`.
4. A second copy of the local-only status word, or its own first-run notice.
5. Its HANDOFF scope line currently reads "mint per §7.1", which is the Postgres statement. It must
   be re-scoped at close-out to say which mint applies on a box with no server (§10.18a).

**MOD-15** (workspace, project, repo and kind management) must not ship:

1. Workspace or project creation on a box with no server.
2. A Settings connection or DSN section built against an ad-hoc focus mechanism instead of M0's
   `SectionId("connection")` + `TabAction::FocusSection` names.
3. **Inverted after §10.14 was answered:** any DSN field **of its own**. The field is required, and
   it is the implementation MOD's (M2b), not MOD-15's — MOD-15 owns the connection *section*, the
   implementation MOD owns the credential *path* inside it (§9.4). MOD-15 must not build a second
   one, and must not build the section against an ad-hoc input mechanism instead of M0's
   `SettingsSection::captures_input` and `MaskedField` names.
4. A second persistence location for a "shown once" marker, or for local settings — §5.4's
   `local_setting` exists so it does not have to invent one.
5. `Settings > Rebuild cache` — which it owns (`HANDOFF.md:251`, `CacheStore::rebuild()` at
   `cache/mod.rs:164-193`) — against a file this document has made writable. Under §4.1's verdict
   that button is **safe as written**, because the local store is a different file and
   `MIRRORED_TABLES` never names it. MOD-15 must not "helpfully" extend the button to clear local
   data, and its confirmation copy should say what it does and does not delete.
6. Note `HANDOFF.md` currently marks MOD-15 "Not blocked". Its create paths for a local-only box now
   depend on the implementation MOD; §10.18b records the amendment.

### 9.4 The seams they share with this document

| Seam | Owner | Contract |
|---|---|---|
| The Settings connection **section** | MOD-15 builds and registers it; this document names it | `SectionId("connection")`. Its layout, its strip position and its non-credential content are MOD-15's; its identity is fixed at M0 so the overlay can point at it. |
| The DSN **field** inside that section | **The implementation MOD (M2b), not MOD-15** | New, from §4.9: `MaskedField`, `SettingsSection::captures_input`, `StoreRequest::SetDsn(Dsn)` / `ClearDsn` / `ConnectionInfo`, the redacting `Debug`, the `spawn_blocking` keyring write, the fixed error vocabulary and the zeroing. MOD-15 registers the section and calls into the field; it does not implement it. **Why the split, stated because it reverses `HANDOFF.md:255-257`'s natural reading:** that line assigns MOD-15 "the Settings DSN section" as the overlay's *destination* — ownership of screen real estate — not a credential path. The field is security surface this document specifies, whose posture it argues and whose seven criteria (§12, 20-26) it writes. The split lets MOD-15 ship a section that is complete without the field. §10.18 gains sub-item (d′). |
| Section focus | Whichever lands second implements it | `TabAction::FocusSection(TabId, SectionId)` (`app/action.rs`, beside `Focus(TabId)` at `:59`). Both `TabId` and `SectionId` are `Copy`, so the enum's derives are unchanged. |
| The startup-overlay slot | This document decides | `App` has exactly one (`app/state.rs:167`), already held by the workspace switcher (`app/mod.rs:61`). The local-only notice **replaces** it on a local-only first run (§4.5). MOD-15 owns the switcher's own empty-state copy, which currently reads "no workspaces — creating one arrives with MOD-15". |
| Settings plumbing | Shared | There is no settings read or write in `StoreRequest` today, and MOD-15 owes ten `app_setting` keys. §7.2's two pairs and §5.4's `local_setting` are the shape both should use. |
| The `GREATEST` importer statement | MOD-8 reserves it; M7 needs it | `item.rs:145-148` reserves the explicit-`key_number` variant to MOD-8. Two consumers now want the one statement; whichever lands second owns the join, and the seam is a `WriteStore` method that does not exist yet. |
| Hierarchy creation | MOD-15 owns the trait methods; the implementation MOD owns the private bootstrap | §5.6. Named here so neither invents a second mechanism. |
| The box probe | MOD-7 | §5.3's local `box` row keeps seven `NOT NULL` columns the probe fills, defaulted to what is knowable at launch, so an adopted row satisfies the server's DDL. MOD-7 later overwrites them. |
| MOD-4 (orchestrator) | **Shared** — this document owns the local schema and the store arm; MOD-4 owns the engine, the migration and the method bodies | Runs are **in scope** (§4.8, answering §10.8). `docs/ANA-2.md` §2 invariant 10 survives verbatim and is what makes this affordable: `htui-orch` depends on `htui-core`, so the engine is generic over `WriteStore` and never names `LocalStore`. ANA-2 §3's warrant clause (`:222-227`), §4.9's offline window (`:1325-1340`) and §8's placement column (`:1699-1705`) reopen as §6.3 predicted; §8's split *rule* does not. The ordering rule of §9.1 binds both items. |

---

## 10. Open for the maintainer

Thirty-three questions, deduplicated from the five verdicts and the consistency pass. Each carries
the options actually available and this document's recommendation, so each is decidable in one
reading. **Questions 1 to 8 gate the implementation MOD's size and must be answered before it is
planned;** the rest can be answered as their milestone arrives.

**Answered by the maintainer on 2026-09-08:** question 3 → **(b)**, local-only creates items and
`R-ENT-7` is amended; question 5 → **defer adoption with its id minted**; question 8 → **(b)**, runs
in scope at parity, with its consequences in §4.8, §5.3, §7.1, §9.1 and §11.22-26; question 14 →
**(b)**, in-app DSN entry is a requirement of the item, with §4.9, §9.1's M2b and §11.16-21 following
from it, and question 13 re-answered on that premise. The rows below are updated in place; the
options each question offered are kept so the decision is legible rather than merely recorded. `docs/REQUIREMENTS.md` is edited only by
explicit maintainer decision, so every row touching it proposes and stops.

| # | Question | Options | Recommendation |
|---|---|---|---|
| 1 | **`R-STO-1` sentence 1 and `R-STO-4`: amend in place, or mint new requirement ids?** §6.1 gives replacement wording for both. | (a) Edit `R-STO-1` s.1 and `R-STO-4` in place. (b) Leave both verbatim and mint e.g. `R-STO-7` ("local-only mode") carving out the exception. Both are consistent with the ID-stability rule (`:16-17`, "IDs are stable; a withdrawn requirement keeps its number and is marked withdrawn rather than deleted"). | **(a) for `R-STO-1`, (b) for `R-STO-4`.** `R-STO-1`'s sentence needs one scope word and reads badly as a cross-reference. `R-STO-4` is genuinely two states now, and splitting it keeps the "server known, unreachable" text — which is still exactly right — undisturbed and citable by ANA-2 and ANA-5. |
| 2 | **`R-STO-3`: confirm the intent is to preserve "read-only" verbatim.** | (a) Preserve; the writable rows stay out of `cache.sqlite`. (b) Strike "read-only" and allow §4.1's option A or C. | **(a).** It is the payoff of the separate-file verdict and should be recorded as a decision rather than assumed. (b) additionally requires re-specifying `CacheStore::open`'s rebuild rule and `CacheStore::rebuild()` before a single local row is safe. |
| 3 | **`R-ENT-7`: the single largest scope lever. May local-only mode create items at all?** | (a) **No.** Local-only creates hierarchy and chat only; `LocalStore::mint_item` refuses. `R-ENT-7` is untouched entirely, §4.3 collapses to "generalise the UUIDv7 mint, reject `upload_pending`'s code", M6 leaves the plan, half the local schema drops out (`item`, `item_revision`, `item_key_counter`, `item_link`, `item_note`, `document`), and MOD-13 stays where it is. (b) **Yes**, with §4.3's project-origin-scoped counter and §6.1's replacement wording. **ANSWERED (b), by the maintainer, 2026-09-08** — local-only creates items, and `R-ENT-7` is amended per §6.1. The recommendation and its counter-case are kept below because they are why the answer is not free. **(b)**, but the case for (a) is real and is Jira's shipped position. `HANDOFF.md:60-61` frames the defect as "the user sees nothing and **can create nothing**", and under (a) a local-only box would have projects and kinds but a permanently empty backlog — arguably worse than today's empty shell, because the emptiness now looks like a bug. Decide this before the MOD is sized; it is roughly half its cost. |
| 4 | **`R-STO-6`: does the sub-second startup budget bind a local-only start?** As written it does not — it is conditioned on "a warm cache **and reachable Postgres**" (`:123-124`). One verdict assumed it binds, another assumed it does not (§4.7, claim 2). | (a) It does not bind; state so and take no measurement. (b) Extend it: a local-only start is held to the same budget, and §12 criterion 13 measures it. | **ANSWERED (a) with the measurement kept, by the maintainer, 2026-09-08.** Local-only is a temporary, lite mode (§1.3): the server-shaped guarantees — a warm cache, refresh cursors, a mirror to be warm *from* — are exactly the ones that mean nothing without a server, and `R-STO-6` is one of them. `R-STO-6` is therefore left **unamended**, with a sentence recording that a decision was taken rather than overlooked. §12 criterion 13 stays, downgraded from a limit to a **recorded figure**, so a regression on the first-run path is still visible. (This document's own recommendation was (b); the framing decision overrides it, and correctly — a benchmark on a mode users are meant to leave is a commitment with no beneficiary.) |
| 5 | **Is the adoption path funded, deferred to a named item, or declared never?** This is the decision the external evidence is most emphatic about. | (a) Fund M7 as a named item now. (b) Defer with the id minted so the deferral is visible. (c) Declare local rows permanent and say so in the UI at first local creation. **ANSWERED, twice, by the maintainer on 2026-09-08 — and the second answer supersedes the first.** First: (b), deferred with its id minted. Then, on the framing decision of §1.3 — local-only is a **temporary** mode and Postgres is the full product — adoption stops being a deferrable nicety and becomes **the exit the mode promises**, so it is **funded** as MOD-18 rather than merely named. The id was minted either way; what changed is that it is now a commitment. **The sequencing rule this creates is the operative part: MOD-17 must not ship copy promising migration before MOD-18 exists.** Either the path is there or the UI says the data stays put — an implied promotion path that never lands is the worst of the three outcomes, and is precisely what the external survey found (Grafana still has no official SQLite→Postgres path; Gitea's tracker argues the docs should say the first choice is permanent; only Synapse's resumable `synapse_port_db` made it routine). §4.8 sharpens it further: what an unfunded adoption would strand is *runnable* work, not merely readable rows (§11 risk 22). Grafana still has no official SQLite→Postgres path and third-party scripts are pinned to versions; Gitea's tracker argues the docs should say the first choice is permanent; only Synapse's resumable `synapse_port_db` made it routine. An implied promotion path is the failure mode. This also answers "may a locally-originated project ever be adopted at all": under (c) it never is, and §4.3's key question closes permanently. |
| 6 | **`R-ID-7` and `R-SEC-3`: where does the scrubbing seam sit for local writes?** No underlying verdict raised this, and scrubbing "fails closed" is a must. A local store holding item bodies, notes, documents and local `session_event` rows is the first durable local persistence of user text outside the recorder. | (a) Confirm §6.1's position: the seam does not move — the recorder scrubs before handing events to any writer, so `Writer::Local::append_events` receives already-scrubbed rows exactly as `BufferedWriter` does; and item bodies, titles, notes and documents are user-typed text the scrubber has never covered on any backend. (b) Extend the scrubber to cover local item text, which no backend does today. | **(a), confirmed explicitly.** But it must be confirmed rather than assumed, because `append_pending` "trusts its input" by design (`pending.rs:10-17`) and this is the first time the local store has been asked to hold anything but events. §12 criterion 11 pins it. |
| 7 | **`R-USR-2`: may a local `app_user` row be minted, and may adoption rewrite authorship?** `item.created_by` and `item_revision.author_id` are both `NOT NULL REFERENCES app_user(id)` (`0001_init.sql:326`, `:346`), and `seed_if_empty_as` returns the **oldest** row's id, never one matching a name (`pg/mod.rs:230-333`), so adoption cannot avoid the remap. | (a) Mint locally from `identity::os_user_name()`; adoption silently remaps both columns to the server's single user. (b) Same, but a server whose single user differs from the local one is itself a refusal condition. (c) Preserve original local authorship with a new forward-only `migrations/000N_*.sql` adding a provenance column. | **(a) for the mint, (b) for adoption, and (c) only if the maintainer wants the history.** Minting locally is a new precedent — the mirror deliberately refuses to invent a `UserId` (`cache/read.rs:773-800`) — and it is unavoidable, because every local row needs a `created_by`. The remap is an authorship rewrite and is not a decision an ANA should take. |
| 8 | **Scope: does local-only cover graph runs, or hierarchy + items + chat only?** | (a) Hierarchy + items + chat (this document's original assumption). (b) Runs too — full feature parity with a server-backed box. | **ANSWERED (b), by the maintainer, 2026-09-08.** A local-only box is a full box: it runs graphs. §1.3's exclusion is withdrawn and §4.8 is the register entry for the answer. **What it costs, stated so it is not discovered later.** `LocalStore`'s surface goes from the 22 async methods of §7.1 to **61**: the 18 `WriteStore` methods of `docs/ANA-2.md:1739-1755`, the 6 `ReadStore` methods of `:1730-1733`, and 15 of the 16 inherent reads of `:1711-1724` (`agents()` already exists). §7.1's delta corrects this document's own earlier estimate of "roughly 16 writes and 14 inherent reads", which counted table **rows** rather than method names and omitted the `ReadStore` set entirely. The local schema gains eleven tables and one column set §5.3 originally excluded, plus a reserved slot for MOD-4's `run_step_tree`. **What it reopens:** ANA-2 §3's warrant clause, §4.9's offline window and §8's placement split; `R-STO-4`'s proposed sibling clause loses the words "and may not start runs". **What it does not reopen:** ANA-2 §2 invariant 10 survives *because* `htui-orch` depends on `htui-core` and not on `htui-store` — the engine is generic over `S: WriteStore`, so it never names `LocalStore`. That is the single reason this answer is affordable. **The hard sequencing consequence is §9.1's M8 and the ordering rule against MOD-4:** `migrations/0003_orchestration.sql` does not exist, so the parity answer cannot be specified against it — §5.3 splits the delta into Set A (buildable today) and Set B (reserved to MOD-4). |
| 9 | **An explicit "stay local even though a DSN exists" preference?** §4.6 conflict 4 removes `local_only` as a mode flag. | (a) No such preference; the arm predicate stays `dsn.is_none()`. (b) Add it, accept a fourth state, and make the predicate `local_only \|\| dsn.is_none()` with two local-only renderings. | **(a).** The thing a lock would guard — automatic adoption — does not exist under §4.4. (b) buys a feature nothing has asked for and costs a fourth state in a one-line top bar. |
| 10 | **Add `system_identifier` to the connect handshake?** `SELECT system_identifier FROM pg_control_system()` is what Postgres itself uses to refuse a standby against the wrong primary; `db_fingerprint` is an endpoint hash that matches a dropped-and-recreated database at the same coordinates. Ripgrep confirms **zero** occurrences of either name in the tree. | (a) Read it once at connect, persist it as `adopted_into` on adoption. (b) Use the existing endpoint fingerprint and accept an advisory refusal. | **(a).** It is one `SELECT` in a handshake that already seeds and registers, and without it the second-server refusal of §4.4 is advisory rather than real. Only needed when M7 lands, so it can ride with it. |
| 11 | **Is a pre-adoption copy of `local.sqlite` mandatory, and where does it go?** Nothing in the tree copies a SQLite file before a destructive operation today. | (a) Mandatory, beside the file, timestamped. (b) Offered in the confirmation, not forced. (c) Not taken. | **(a).** Obsidian and Anki both tell users to do it manually and their forums are the evidence for why. §12 criterion 12 makes it checkable. M3's export path (§9.1) is a cheaper first version of the same insurance. |
| 12 | **`--clear-dsn` on a box that had a server, and the buffers it strands.** §5.1 decides the arm behaviour; this is about the warning. | (a) `--clear-dsn` warns when `cache/<fingerprint>/pending/*.jsonl` exist and would now never upload. (b) Silent, as today. (c) Additionally, sweep or recover existing stranded `cache/offline/pending/*.jsonl`. | **(a). Not (c).** The warning is cheap and honest. Recovery is a separate, unbudgeted feature: nothing in the tree ever enumerates `<root>/cache/*`, and §4.7 claim 4 records that the new arm fixes the class prospectively only. |
| 13 | **Given 14, must leaving local-only be in-process, or may it remain a restart boundary?** Re-answered on its new premise: the user can define a DSN without leaving the program, so the only remaining question is whether the *running* process must also switch backends. The keyring write and the backend swap are independent operations. | (a) **Restart boundary retained.** The field writes the keyring; the section says the connection takes effect on the next launch. (b) **In-process.** `Backend::Local` → `Online` inside one process. | **(a) for the first MOD, on a re-derived reason — and the deferral is now visible (M9) rather than definitional.** The three seams are not equally priced. **Seam 1, a request that writes the keyring: real, and paid by M2b anyway**, so it is not a cost of (b) at all. **Seam 2, a `Reconnect` closure built after `start()`: real, but this document's earlier reason for it was wrong.** It does *not* contradict `connect.rs:66-70`: the closure at `connect.rs:198-208` captures the DSN **by move**, so the worker already holds a DSN today, inside an `Arc<dyn Fn>` it cannot read. What `:66-70` buys is that no `store_worker` type, signature or reply names a DSN — which a `connect::reconnect_for(dsn, root, timeout) -> Reconnect` factory preserves exactly, at the cost of `let mut reconnect` at `store_worker.rs:411` plus one assignment. **Seam 3, re-opening the mirror mid-process: real, and the entire cost of (b).** `go_online` refuses outright when `backend.cache()` is `None` — "no mirror to go online over; keeping the current backend" (`store_worker.rs:572-575`) — and `Backend::Local` holds no `CacheStore`, so going online means `go_online` must **open** `CacheStore::open(&root, &db_fingerprint(dsn), PgStore::schema_version())` rather than move one, as an async file create-and-migrate on the worker task with a `Backend::Local` still in hand. §4.1 flagged this conditionally; it is now scheduled, not hypothetical. The copy must stop saying the restart is "necessary". |
| 14 | **Is an in-app DSN field permitted?** | (a) ~~No field.~~ **Withdrawn.** (b) A masked single-line field in the Settings connection section. (c) A full-screen first-run form. | **ANSWERED (b), by the maintainer, 2026-09-08: it is a requirement of the item, not an option.** The stated objective is "allow the user to start the program without defining the DSN, so to give the possibility to define it inside", and a design that routes the user back to a shell flag does not satisfy it. Specified in §4.9. The decision reverses `lib.rs:42-44` / `:109-129` **as the only path**, not as a path: `--set-dsn` is retained, because it is the only mechanism that works when the terminal cannot be initialised and it is what a scripted box uses. What is reversed is the *implicit* claim that raw mode is unsafe for a DSN. Three findings make the reversal defensible rather than merely ordered: the decision it reverses had already traded away echo suppression (`lib.rs:117` prints "it will be visible"), so the field is **strictly less** exposed on that axis; the risk of a raw-mode field is a `Debug` derive and a `tracing` call, both enumerable and both closed in §4.9; and `PgConnectOptions` exposes **no password getter**, so the post-parse confirmation the user needs is password-free by construction. Consequences: §9.3's MOD-15 prohibition 3 inverts, M2b joins §9.1, §6.1's `R-TUI-8` wording is superseded, and risks 16-21 / criteria 20-26 follow. |
| 15 | **A second crate for compile-time-checked local writes?** `sqlx.toml` is per-crate and its only multi-database key is `database-url-var`, whose own documented example is "break it up into multiple crates". | (a) Keep `LocalStore` in `htui-store` with runtime-checked SQL, as `cache/read.rs:18-19` already accepts for the mirror. (b) Split `htui-store-local` with its own `LOCAL_DATABASE_URL`. | **(a) first, (b) only if the unchecked surface actually bites.** (b) puts a crate boundary through the middle of the store layer, which is a bigger structural change than the arm itself. Note sqlx's SQLite nullability inference on a *write* path is unmeasured against `htui`'s SQL; (b) needs a spike, not a citation. |
| 16 | **`--offline` with no DSN, and whether `--offline` deserves its own word.** It carries no `conflicts_with` today (`cli.rs:29-32`, verified). | (a) §5.1's table: it is a no-op with no DSN, and `--offline` **with** a DSN keeps `offline · <age>`. (b) Reject the combination at the CLI with a `conflicts_with`. (c) Give `--offline` a fourth word, e.g. `offline (by request)`. | **(a), and not (c).** (b) is equally consistent and slightly more explicit if preferred; what matters is that one answer ships. (c) is three states versus four on a one-line header, and the extra word buys nothing the user did not already type. |
| 17 | **Multi-process guard on `local.sqlite`.** The mirror's "a second `htui` process is just another reader" assumption stops holding for a writable primary store. | (a) §7.1's advisory lock: the second process opens the local store read-only and says so. (b) Hard single-instance refusal. (c) Accept the busy-timeout stall. | **(a).** (b) breaks a normal workflow; (c) can put a five-second stall near the UI path, which `R-NF-3` forbids. |
| 18 | **`HANDOFF.md` amendments at close-out.** | (a) MOD-13's scope line "mint per §7.1" re-scoped to say which mint applies on a box with no server. (b) MOD-15's "Not blocked" corrected: its create paths for a local-only box now depend on the implementation MOD, and it owns `Settings > Rebuild cache`, which must state what it does and does not delete. (c) The adoption item's id minted so §10.5's deferral is visible. (d) The Settings DSN section is **MOD-15's**, per `HANDOFF.md:255-258`; MOD-13's dependency on this document is the edit path only (`:238-240`). (e) The `GREATEST` importer statement now has two consumers, MOD-8 and the adoption item; `item.rs:145-148` reserves it to MOD-8 alone. (f) §5.6's three hierarchy-creation `WriteStore` methods are assigned to MOD-15. | **All six.** (d) in particular corrects a misattribution that is easy to propagate. |
| 19 | **`R-ID-3` and `R-HIS-1`: amend, or carve out?** Amending only `R-STO-1` and `R-STO-4` leaves both standing against the result. | (a) Scope them, per §6.1's wording. (b) A narrower carve-out that names local-only as outside "everything `htui` knows". | **(a).** The amendment set must be internally consistent or the contract contradicts itself; §6.1 gives both sentences. ANA-9 §2 invariant 4 moves with `R-HIS-1`. |
| 20 | **`R-STO-5` wording.** Both "on connect" and "after confirmation" are already false of `cache_migrations/`. | (a) §6.1's appended sentence naming the two SQLite schemas. (b) A per-store rule. (c) Leave it alone and record the divergence only in this document. | **(a).** It makes an existing gap explicit without changing the Postgres rule, and a third migration set over a file that can never be rebuilt is exactly the case the current text does not describe. |
| 21 | **Is ANA-9 §5.10's per-project seed honoured in full locally?** "the five `item_kind` rows with their default graphs and phases … and one `prompt_template` version 1 per phase name" (`:820-824`). | (a) In full: `step_graph`, `step_graph_phase` and `prompt_template` join the local schema, unread locally but present so an adopted project arrives complete. (b) Partially, relaxing `item_kind.default_graph_id`'s `NOT NULL` locally. | **(a)**, which is what §5.3 assumes. (b) produces rows the server rejects at adoption — silent locally, loud later, which is the wrong way round — and SQLite cannot add the constraint back afterwards. |
| 22 | **Should the local schema mirror Postgres's constraints exactly, or deliberately relax some?** Close to a one-way door: SQLite cannot `ALTER` a `CHECK`, `UNIQUE` or foreign key, so an over-tight first cut costs the 12-step table rebuild to loosen and an under-tight one costs the same to tighten. | (a) Exact. (b) Relax where a partially built local hierarchy would otherwise be unrepresentable. | **(a).** The mirror relaxed them for a stated reason local-only falsifies. A user blocked mid-creation by a constraint a server-backed user never sees is a real cost, but it is a *visible* one, and the alternative surfaces at adoption on data that cannot then be deleted. |
| 23 | **Where does the local file live?** §5.3 and §8 assume `<config_root>/local/local.sqlite`. | (a) As assumed. (b) Another name or nesting. | **(a).** The only binding constraint is that it must not be under `cache/<fingerprint>/`, whose name is keyed to a server it does not have; beyond that the name is the maintainer's. |
| 24 | **May an adopted project ever mint locally again, and may a box return to local-only?** | (a) No: adoption seals the project permanently (§4.6 conflict 7), enforced by `sealed_at` plus a mint guard. (b) Yes, with a second counter reconciliation. | **(a).** (b) puts two writers on one `(project_id, prefix)` and the collision surfaces at the *next* adoption, long after creation, with no `DELETE FROM item` to clean it up. |
| 25 | **How strict is the second-server refusal?** | (a) Hard refusal with one explicit named opt-in, on git 2.9's model, whose docs state that because the legitimate case is rare "no configuration variable to enable this by default exists and will not be added". (b) A dismissible warning. | **(a).** Zotero's "two libraries into one account produces duplicates" and Syncthing's union-on-re-add are what happens with (b), and every key involved is a client-minted UUID that would insert cleanly. |
| 26 | **Retention and a size bound on the local store.** Nothing prunes the existing `pending/` tree — no byte cap, no file-count cap, no age prune — and `CacheStore::rebuild()` never touches it. A permanent local primary store makes retention a decision rather than a buffer detail. | (a) No bound; add the export path of M3 and a size figure in the connection section. (b) A configurable retention like `R-HIS-3`'s. | **(a) for v1, with the export path as the escape valve.** `no_delete_path` forbids deleting items anyway, so a retention sweep could only ever touch transcripts, which `R-HIS-3` already governs. §12 criterion 14 measures the growth. |
| 27 | **Local rows the user declines to adopt, permanently: readable, hidden, or exportable?** | (a) Readable beside the server's, tagged by origin (§7.4's default). (b) Hidden unless a mode is toggled. (c) Export only. | **(a).** It is the least surprising and it is what §4.6 conflict 3 already pays for. (b) recreates the silent-disappearance failure this document exists to fix. |
| 28 | **`R-TUI-1`: replace "Postgres state" with a named state vocabulary?** | (a) §6.1's wording: "store state, distinguishing online, connecting, offline since T, and local-only (no server configured)". (b) Leave it and let the label arm carry the distinction unbacked. | **(a).** Without it the distinction has an implementation but no requirement, and three states rendering one string is exactly how it got here. |
| 29 | **`R-TUI-8`: add a Postgres-connection section?** | (a) §6.1's wording. (b) Leave `R-TUI-8` alone and route the overlay somewhere else. | **(a), with the honesty clause.** This document must record plainly, and does (§6.1), that today's citation of `R-TUI-8` for a DSN section — including its own HANDOFF entry — is an extension of the requirement rather than a reading of it. Its list names "secret provider", which `R-SEC-2` makes the agent provider config and `:225` explicitly excludes `htui`'s own credentials from. |
| 30 | **Sequencing: does the implementation MOD or MOD-15 land first?** `HANDOFF.md:255-258` says "whichever lands second owns the join" but does not sequence them. | (a) The implementation MOD first; M0 ships the two names on day one so MOD-15 is unblocked immediately. (b) MOD-15 first. | **(a).** M0 costs nothing and removes the contest entirely, which is why it is a milestone rather than a footnote. |
| 31 | **Is it acceptable that one box has two item-creation behaviours** — allowed in local-origin projects, refused in mirrored ones? | (a) Yes, with the backlog and the `new` action explaining which. (b) No; refuse local item creation entirely (which is question 3(a)). | **(a).** It is the correct behaviour under the amended requirements, but it is a genuine explanation burden that lands on §4.5's copy and on MOD-13's `new` action, so it should be accepted deliberately rather than discovered. |
| 32 | **May `0001_local.sql` author ANA-2's two table-level CHECKs (`ck_run_graph_snapshot`, `ck_phase_judge_model`) ahead of MOD-4?** §10.22 answered "mirror Postgres's constraints exactly", and SQLite cannot add a table-level `CHECK` later — only columns and whole tables are cheap. MOD-4-era, not gating. | (a) Author both now, mirroring ANA-2 §9. (b) Author `ck_run_graph_snapshot` only; defer `ck_phase_judge_model` with its columns. | **(b)** (§5.3). `ck_run_graph_snapshot`'s shape is fully determined by `0001_init.sql`'s existing `kind` column and cannot change, and it can be authored **plain** rather than `NOT VALID` because `local.sqlite` has no legacy rows. `ck_phase_judge_model` guards a column nothing writes until MOD-4 exists, so the divergence is unreachable in the meantime. |
| 33 | **How does adoption remap `agent`?** `agent.name` is `UNIQUE` (`0001_init.sql:96`), so a locally seeded `claude` row and a server's are two UUIDs for one name, and `run_step.agent_id`, `phase_agent.agent_id` and `agent_box.agent_id` all point at the local one. A **third** identity remap beside `created_by`/`author_id` and `box_id`. M7-era, not gating. | (a) Match on `agent.name` and adopt the server's id. (b) Refuse adoption when the two registries differ. (c) Carry local agent rows into the server as new rows — impossible under the `UNIQUE`. | **(a)**, on the adopt-DB-id precedent (`pg/mod.rs:335-375`, `connect.rs:267-284`). It is new work, and exactly the class of thing `upload_pending` avoids by receiving the id as an argument rather than deriving it (§11 risk 26). |

---

## 11. Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | **The design degrades into "keep local forever, silently" if the adoption item is never funded.** Then §4.4's verdict is option B with ceremony, and §4.5's overlay has been promising a path that does not exist. This is the single most consistent finding of the external survey. | §10.5 forces the choice openly and §9.1 mints M7's id at close-out so the deferral is visible in `HANDOFF.md` rather than implied. M3's export path is the floor: even with no adoption tool, the data can leave. If the maintainer answers "never", §4.5's copy must say so at first local creation. |
| 2 | **`box.toml`'s new fields are silently erased.** `identity::store` serialises the whole `BoxToml` and is called on any hostname change (`identity.rs:76-78`) and by the adopt-DB-id rewrite (`connect.rs:267-284`). A field added to `BoxToml` alone survives until the first laptop rename. | §5.5 rule 2 requires the fields on `Identity`, not only on `BoxToml`, and §12 criterion 3 tests exactly the rename path. |
| 3 | **A new `box.toml` field makes `htui` refuse to start.** `BoxToml` derives `Deserialize` with no defaults and `load_or_mint` turns a parse failure into `StoreError::Backend`, which `connect::start` propagates before the terminal is initialised. The failure mode is "the binary no longer starts", not "the notice reappears". | `#[serde(default)]` on every added field (§5.5 rule 1), plus §12 criterion 3's downgrade case: an older binary reading a newer file and a newer binary reading an older one must both start. |
| 4 | **Drift between `local_migrations/` and `migrations/0001_init.sql` is caught at adoption, on the user's data.** The mirror's equivalent drift at least fails loudly in the refresher's bind lists; the local store's fails against Postgres, months later, on rows that cannot be deleted. Nothing in the build checks that the two schemas still express the same constraints. | §12 criterion 6 asserts the local `CHECK` vocabularies against the Postgres ones as a test rather than a convention. §10.22's "exact, not relaxed" is the lower-regret direction for the same reason. |
| 5 | **The local schema's constraints are effectively permanent.** SQLite cannot `ALTER` a `CHECK`, `UNIQUE` or foreign key, cannot add a `NOT NULL` column without a non-`NULL` default, cannot `ADD` a `STORED` generated column, and 3.53.0's `ALTER COLUMN SET/DROP NOT NULL` is unavailable at the bundled 3.51.3. Every future constraint change is the 12-step create-copy-drop-rename procedure, forever, under `R-STO-5`. | Priced, not mitigated. The counterweight is that the alternative — a schema that can be rebuilt — is the property that makes the mirror unsuitable in the first place. §10.22 makes the first cut a deliberate decision. |
| 6 | **No compile-time query checking for local writes**, roughly doubling the unchecked SQL surface. All 86 `.sqlx` descriptions are PostgreSQL and `cache/read.rs:18-19` fixes the rule. sqlx's SQLite nullability inference on a write path is unmeasured against `htui`'s SQL. | The mirror already runs this way in production and the read paths are hand-decoded with `bad()` reporting corruption as a store error. §10.15 keeps the crate split available. §12 criterion 1 requires the conformance suite green, which is 20 cases of coverage the mirror never had. |
| 7 | **The one-writer-per-project guarantee is a property of the deployment, not of the schema.** Nothing in SQLite or Postgres enforces "this project has one counter writer"; it holds because a local project exists on no server. Two `htui` processes on one box, or a local store copied to a second machine, break it silently, and Postgres's `UNIQUE (project_id, key_prefix, key_number)` is the only backstop — firing late. | §7.1's advisory lock closes the two-process case. §5.7's `store_id`/`local_store_id` pairing closes the copied-file case by refusing to write until the user resolves it. §4.6 conflict 7's sealing closes the double-adoption case. None of them is free and all three are needed. |
| 8 | **No prior art exists for the adopted key strategy.** No surveyed system mints a human-readable per-project sequence key offline and keeps it: every one either refuses (Jira), demotes the key to a mutable server label (Linear, GitLab's `id` vs `iid`) or accepts renumbering. §4.3 works because of a structural property `htui` has and they do not — a project that exists on no server — but there is no design to copy for the adoption path, only failure modes to price. | The failure modes *are* priced (§4.3, §4.4) and the counter fast-forward is already test-pinned. The residual is that M7 is genuinely novel work and should be scoped as such rather than as "generalise `upload_pending`". |
| 9 | **A refused adoption is invisible and unremovable.** `pending.rs` quarantines with a `warn!` to a log the user never sees, forever, and `pg/write.rs:12-15` forbids `DELETE FROM item`, so a refused item is stranded in both stores. | §4.3 explicitly forbids inheriting that policy for items; §12 criterion 12 requires a UI-visible per-row status. It is listed as a risk because the cheapest implementation of M7 is to copy `upload_pending` wholesale, including this. |
| 10 | **A local-only user cannot delete anything.** `no_delete_path` is a conformance case, so a user experimenting on a first run will create items they wanted to discard and the only remedy is closing them by status. This is correct per ANA-9 §4.1 and will read as a bug. | Documented rather than fixed; the rule is older and larger than this document. The `new` action's copy should say that closing, not deleting, is how an item goes away. |
| 11 | **The prompt-digest argument that kills §4.3's option 3 constrains the adopted option at one boundary.** An adopted item's key is unchanged, but `{{item_key}}` is `project.slug + ':' + item.key` (`docs/ANA-5.md:323`), so the *qualified* key is only stable if the project slug is immutable across adoption. | The key is safe by construction; the slug is a Q4-adjacent property. M7 must either preserve the local slug or record that a re-slugged project's pre-adoption digests are not reproducible. §12 criterion 12. |
| 12 | **Two writers, one file, if anything ever puts the local store back under `cache/`.** The whole verdict rests on the two files being separate; a later convenience change that "tidies" the layout would re-introduce every rejected failure at once. | §5.3, §8 and §12 criterion 5 state the prohibition three times, and criterion 5 makes it a test: a `schema_version` bump must not touch `local.sqlite`. |
| 13 | **The overlay reopens every second.** `StoreState` is re-read every fourth tick and is explicitly not gated on a non-empty scope — which is exactly the shell a first launch shows (`app/update.rs:74-87`, test at `:533-544`). A persistent marker alone is not enough if the in-session guard is forgotten. | §4.5 requires both: the `box.toml` marker for "once ever" and the session bool for "not again this second". It is the exact defect `migration_prompt_shown` was added to prevent, so the precedent and the test are both in tree. |
| 14 | **Adding a fifth `label()` output changes a value the tests treat as a contract**, and the 100×30 snapshot suite renders the top bar (`crates/htui/tests/settings.rs:36-40`). | Keeping `--offline` on `offline · <age>` limits the churn to the no-DSN path and leaves `tests/connect.rs:227-241` green (verified, §3). Snapshot churn is expected and is what the suite is for. |
| 15 | **Local writes are unscrubbed user text if §10.6 is answered carelessly.** `R-ID-7` and `R-SEC-3` are `must`s and scrubbing fails closed, and no verdict raised this until the consistency pass. | §6.1 states the position, §10.6 asks for it to be confirmed, and §12 criterion 11 tests that a local chat's events are scrubbed on the same path as a buffered one. |

---

**Risks 16 to 21 — from §4.9 (in-app DSN entry).**

| # | Risk | Mitigation |
|---|---|---|
| 16 | **The masked field renders clipped, with the cursor off-screen, in a narrow pane.** `Composer::render` is a bare `Paragraph::new(line)` with no `Wrap` (`composer.rs:89-100`), so a line longer than the pane is silently truncated and the trailing `_` disappears — and a 60-character DSN in a section pane inside a bordered Settings block is routinely longer than the pane. `Event::Resize` only sets `dirty` (`app/state.rs:324`); nothing recomputes on resize by itself. | §4.9(4) requires a render-time window ending at the cursor with a leading `…`, computed from the `Rect` on every frame and never cached. §12 criterion 21 tests 20, 40 and 100 columns. |
| 17 | **Paste is a key burst, not a paste event, and a pasted newline submits an incomplete DSN.** Bracketed paste is not enabled (`terminal.rs:23-28` is bare `ratatui::init()`), and `on_terminal_event` drops every event that is not `Key(Press\|Repeat)` or `Resize` (`app/state.rs:319-327`), so `Event::Paste` never arrives even if a terminal sends it. A multi-line clipboard therefore submits at the first `\n`. | §4.9(4): control characters are rejected rather than pushed, and submission requires a buffer that **parses**, so a truncated paste fails loudly instead of storing a prefix. Enabling bracketed paste is out of scope — it touches `terminal.rs`, `on_terminal_event` and every tab's key handling. §12 criterion 21. |
| 18 | **The DSN leaks through a derived `Debug`.** `StoreRequest` derives `Debug, Clone` (`store_worker.rs:49-50`) and `RequestEnvelope` derives `Debug` (`:263-264`). A `SetDsn { dsn: String }` variant would be printed in full by any `{:?}`, and from there into the `--log` file. This is the single most likely way the change becomes a regression, because the leak is invisible until someone adds a `tracing::debug!(?envelope)` months later. | §4.9(3)'s `Dsn` newtype with a hand-written `Debug` printing `Dsn(<redacted>)`. §12 criterion 26 makes it a unit test rather than a convention, so a future `#[derive(Debug)]` on the newtype fails the suite. |
| 19 | **Zeroing is partial, and claiming otherwise would be worse than not doing it.** `Zeroizing<String>` wipes the current allocation only; `String::push` reallocates as the buffer grows and the freed allocations are never wiped, so a DSN typed one key at a time leaves residue from every intermediate capacity. | `String::with_capacity(256)` at construction so no realloc occurs for any realistic DSN (§4.9(6)). **Priced, not eliminated.** Note `zeroize` is promoted from transitive to declared — it is already compiled in via `keyring 3.6.3` (`Cargo.lock:1834`, `:4597-4605`) — so the honest cost is one manifest line, not a new crate. |
| 20 | **The field makes a wrong-but-valid DSN a common event, and its error text is sqlx's to control, not `htui`'s.** A DSN that parses but does not connect reaches `connect::attempt`, whose failure becomes `ConnEvent::Failed(err.to_string())` → the status line (`connect.rs:253`) **and** `tracing::warn!(%why, "connect failed")` → the `--log` file (`store_worker.rs:527`). sqlx 0.9.0 does not echo the DSN on any reachable path, but nothing in `htui` asserts that and it is one dependency bump from changing. | §4.9(5) puts a fixed error vocabulary in front of the parse path, so `htui` never renders `err.to_string()` for a DSN the user just typed. §12 criterion 23 pins it, including a fixture whose password is a distinctive token asserted absent from both the rendered frame and the log file. |
| 21 | **Reordering `SettingsTab::on_key` regresses section cycling.** The fix at `settings/mod.rs:197-213` moves `h`/`l`/`[`/`]`/Left/Right behind a `captures_input()` check. Get the predicate wrong — a section reporting `true` when idle — and the Settings strip becomes unnavigable, which the 100×30 snapshot suite (`crates/htui/tests/settings.rs:36-40`) will not catch because it renders rather than keys. | The trait method's default body is `false`, so every existing section (`AgentsSection`, which already returns `Handled::Pass` at `settings/agents.rs:63-65`) is unaffected by construction. §12 criterion 25 tests both directions. |

**Risks 22 to 26 — from §4.8 (runs in scope).**

| # | Risk | Mitigation |
|---|---|---|
| 22 | **A local project becomes unrunnable the moment a DSN is configured, and stays that way until adoption ships.** §5.2 makes the local store read-only on `Online` and `Offline`; §12 criterion 8 pins that for `mint_item` and §4.8 applies the same rule to runs. Before §10.8 was answered the cost was "your items are readable but frozen"; now it is "the graph you were halfway through will not resume". Under a later "never fund adoption", that is permanent. | **Downgraded but not removed by §10.5's funding decision.** With M7 funded, the parked state is a phase rather than a destination — but it is still real for every release between M3 and M7, and §4.5's copy must say, at first local creation, that configuring a DSN **parks** local work rather than merely leaving it readable. §9.1's M3 export path stays the floor: a `VACUUM INTO` copy is the escape while M7 is in flight. |
| 23 | **Two implementations of one safety predicate.** ANA-2 §4.7's overlap rule is `&&` over `run.repo_scope` with a GIN index in Postgres and a Rust walk over decoded JSON locally; `R-ORCH-10`'s capability check is `required_tags <@ (probed_tags \|\| declared_tags)` in Postgres and a Rust subset test locally. Both are refusals: when they disagree, one store admits a run the other would have serialised or refused, and the symptom is a corrupted tree, not an error. | Both predicates are already specified as pure functions over resolved inputs. **Implement each once in `htui-core` as a `fn` over decoded values and have both stores call it**, with the Postgres SQL used only as a pre-filter, never as the decision. §12 criterion 28 asserts the two agree. |
| 24 | **`0001_local.sql` freezes MOD-4's shape before MOD-4 has written it.** §5.3's Set A is authored at M3 against `migrations/0001_init.sql` as it stands; if MOD-4's `0003` turns out to need a *table-level* constraint or a `UNIQUE` on a Set A table, SQLite offers only the 12-step rebuild. | The split itself is the mitigation: Set A is copied from a shipped file ANA-9 §5.0 forbids editing, so its shape cannot move under MOD-4; Set B is reserved and is column-and-table only, which SQLite adds cheaply. The residual is the two table-level CHECKs, which §10.32 puts to the maintainer rather than deciding silently. |
| 25 | **The conformance suite roughly doubles, and its case count is currently a hard assertion.** `CASES` is 20 (`conformance.rs:23-43`) and two tests assert the count exactly (`tests/pg_conformance.rs:24`, `crates/htui-core/tests/mem_store.rs:25`). ANA-2 §8 requires a case per added method, so MOD-4 takes it to roughly 38 — and every one now runs against three stores. | The hard-asserted counts are exactly the right mechanism and must not be softened; they are what stops a method landing without a case. §12 criterion 1 is corrected to "unchanged **by this MOD**", and MOD-4 updates both numbers in its own commit. |
| 26 | **A local box's `agent` registry is minted from nothing, and adoption has no rule for it.** §5.6's `seed_bootstrap` writes `agent` rows locally because a local box never reaches MOD-2's first-connect seed (ANA-2 risk 14). `agent.name` is `UNIQUE` (`0001_init.sql:96`), so a local `claude` row and a server's are two UUIDs for one name, and every local `run_step.agent_id`, `phase_agent.agent_id` and `agent_box.agent_id` points at the local one. Adoption must remap all three — a *third* identity remap. | Named, not solved; it is M7's, and §10.33 asks the maintainer for the rule. The `box` remap has a working precedent in the adopt-DB-id rule (`pg/mod.rs:335-375`), and the agent remap is its natural analogue — but it is new work, and exactly the class of thing `upload_pending` sidesteps by receiving the id as an argument. |

---

## 12. Validation criteria (for the implementation MOD)

Numbered and checkable. Criteria 13 to 15 are the measurements **nobody has taken**; they are listed
as criteria rather than as open questions precisely so they get taken.

1. `LocalStore` passes all 20 `conformance::CASES` unmodified, including the four mint cases
   (`mint_consecutive_keys`, `mint_prefix_isolation`, `mint_writes_revision_v1`,
   `mint_unknown_kind_rejected`) and `update_cas_diverged`, with no Postgres running. Both
   hard-asserted case counts (`tests/pg_conformance.rs:24`, `crates/htui-core/tests/mem_store.rs:25`)
   are unchanged **by this MOD**, because this MOD adds no case. MOD-4 adds roughly eighteen and
   updates both numbers in its own commit (§11 risk 25).
2. On a box with no DSN, the top bar reads `local-only` with no age from the first frame;
   `Backend::is_writable()` is `false`; `Backend::writable()` is `None`; and with a DSN present and
   `--offline`, the label is still `offline · <age>` and
   `offline_never_dials_and_starts_at_an_age` (`tests/connect.rs:227-241`) is green unmodified.
3. `box.toml` round-trips: a file written by the previous binary parses and starts; a file written
   by the new binary parses in the previous binary's `BoxToml` shape or is proven not to reach one; a
   hostname change and an adopt-DB-id rewrite each preserve `local_notice_shown`, `adopted_into` and
   `local_store_id`.
4. The first-run notice opens exactly once across two consecutive launches on the same box, does not
   reopen a second after dismissal despite `StoreState` arriving every fourth tick, and does not
   open at all on a box with a DSN — including a box whose DSN is unreachable.
5. Bumping the embedded Postgres migration count (adding a `migrations/000N_*.sql`) deletes and
   rebuilds `cache/<fingerprint>/cache.sqlite` as today, and leaves `local/local.sqlite` byte-identical
   with every row intact. Separately, `Settings > Rebuild cache` empties every `MIRRORED_TABLES` row
   and leaves `local/local.sqlite` byte-identical.
6. A test asserts the local schema's `CHECK` vocabularies equal the Postgres ones: `item.status`'s
   eight values, `key_number >= 1`, `last_value >= 0`, and `item_kind.prefix`'s pattern (as `GLOB`
   against the Postgres regex's accepted set). It fails when either schema changes alone.
7. Minting two items in one local project produces consecutive key numbers and real
   `<PREFIX>-<N>` keys; a mint refused by the kind-belongs-to-project guard consumes no number; and
   `item_revision` version 1 exists for every minted item. A mint attempted in a project whose
   `sealed_at` is set is refused with `StoreError::ReadOnly`.
8. `mint_item` on a `Backend::Online` box, targeting a project that lives in the local store, is
   refused; targeting a mirrored project it goes to Postgres. On `Backend::Offline` both are refused,
   the mirrored one exactly as today. (This is the criterion that makes §4.3's origin rule
   non-vacuous; it cannot be written before M3 and M6 both land.)
9. `Backend::workspaces()` on an `Online` box with a non-empty local store returns both sets, with no
   duplicate id, each row carrying its origin; and every read below a workspace resolves against the
   store that holds it, with no query touching both files.
10. A chat started on a local-only box writes `run`, `run_step` and `session_event` rows into
    `local.sqlite`, creates **no** file under any `pending/` directory,
    replays through `step_events` in `seq` order identical to the Postgres path, and reports
    `writer_label = "local"` on `StoreReply::ChatAccepted`.
11. Every event reaching `Writer::Local::append_events` has passed the recorder's scrubber on the
    same path a `BufferedWriter` event does, asserted by a fixture whose payload contains a masked
    value; and a residue that would fail closed on the Postgres path fails closed here identically
    (`R-ID-7`, `R-SEC-3`).
12. *(For the adoption item.)* Adoption of a project subtree is one transaction that inserts the
    rows, fast-forwards `item_key_counter` with `GREATEST`, sets `sealed_at` and `adopted_at`, and
    records `adopted_into`; running it twice inserts nothing the second time; a row the server
    refuses leaves a **queryable, UI-visible** status rather than a log line; a pre-adoption copy of
    the file exists before the first statement; and adoption against a second server with a
    different `system_identifier` is refused.
13. **Measured, not asserted: startup cost of the second SQLite file.** Time from process start to
    first frame on the reference workstation, on a virgin box (create + migrate `local.sqlite`) and
    on a warm box (open an existing one), reported alongside the existing warm-cache figure of
    ANA-9 §11 criterion 5. If §10.4 extends `R-STO-6`, the local-only figure must be under one
    second; if not, the figure is still recorded so a regression is visible.
14. **Measured, not asserted: expected local file size.** Bytes of `local.sqlite` after a
    representative load — one workspace, three projects with the seeded kinds, graphs and templates,
    200 items with revisions and notes, and 20 chats of 500 events each — reported so §10.26's
    "no bound" decision rests on a figure rather than on an intuition.
15. **Measured, not asserted: local write latency under a concurrent refresh.** On an `Online` box
    carrying an open local store, p50 and p99 of a local write while a full refresh pass runs
    against `cache.sqlite`. The expected answer is that the two do not interact at all, because they
    are different files with different pools; the measurement exists to prove that rather than to
    assume it, and to catch the case where a shared runtime makes them interact anyway.
16. `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
    `cargo fmt --all -- --check`, `cargo doc --workspace --no-deps` with zero warnings, and
    `cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store` all pass
    with the local store in place, and the only new `[workspace.dependencies]` entry is `zeroize`,
    which `cargo tree` already showed as a transitive dependency of `keyring`, `rustls` and
    `dbus-secret-service` before this change — so no crate is added to the build (§4.9(6)).
17. A grep of `crates/` finds no path that writes a DSN to `box.toml` or to any file, and the
    keyring remains the only DSN store (`R-STO-1` sentence 2).
18. Two `htui` processes on one box: the second opens the local store read-only, says so, and
    neither stalls the UI thread; the first keeps writing.
19. A `local.sqlite` whose `local_meta.store_id` does not match `box.toml`'s `local_store_id` refuses
    writes with a named error and leaves reads working.
20. **The DSN reaches the keyring and nothing else, proven by a grep and a run.** A ripgrep of
    `crates/` finds no path that writes a DSN to a file, to `box.toml`, to a cache row, to an env var
    or to a `tracing` macro: every use of the `Dsn` newtype terminates at `secret::set_dsn`, and no
    `tracing::{trace,debug,info,warn,error}!` call site in `crates/htui/src` or
    `crates/htui-store/src` takes a `Dsn`, a `StoreRequest`, a `RequestEnvelope` or any binding named
    `dsn` as a `%`/`?` field. Separately, a run that enters a DSN whose password is a distinctive
    token (e.g. `zzq7-canary`) through the field leaves that token absent from `--log`'s output, from
    `box.toml`, from `local/local.sqlite`, from every file under `cache/`, and from the process's own
    stdout and stderr. **This criterion supersedes and widens criterion 17**, which greps only for
    `box.toml`.
21. **The masked field's behaviour on paste, backspace and resize.** (a) *Paste:* feeding the
    character sequence of a 62-character DSN as individual `KeyCode::Char` events, as an unbracketed
    terminal paste delivers them (`app/state.rs:319-327`), leaves a buffer byte-identical to typing
    it; a burst containing `\t` or a control character stores the DSN without them; a burst
    containing `\n` does **not** submit a truncated prefix. (b) *Backspace:* `KeyCode::Backspace`
    removes exactly one character including a multi-byte one, the mask length and the count decrement
    by one, and backspacing an empty buffer is a no-op rather than a panic. (c) *Resize:* rendering
    the same 62-character buffer into 20-, 40- and 100-column `Rect`s produces a mask that always
    ends at the cursor, is never wider than the pane, and shows a leading `…` in the first two cases.
22. **Nothing recoverable is ever drawn.** A snapshot of the connection section mid-entry contains
    only `•` characters and a count, with no substring of the input present. After a successful
    parse, the summary line is present, is built from `PgConnectOptions` getters, and contains no
    password — structurally guaranteed, since `PgConnectOptions` exposes no password getter at all,
    and the test asserts it rather than assuming it. No key sequence reveals the buffer.
23. **A validation failure stores nothing and says nothing derived from the input.** Submitting
    `not a dsn`, `postgres://h/db?sslmode=banana`, `postgres://h:999999/db` and an empty buffer each:
    leave the keyring entry unchanged (asserted through `keyring::mock`, as `secret.rs:117-129`
    already does); emit **no** `StoreRequest`; render a message drawn from §4.9(5)'s fixed vocabulary;
    and write nothing to the log. The `sslmode=banana` case is the load-bearing one — it is the only
    sqlx 0.9.0 parse error that quotes its input — so the test asserts `banana` is absent from the
    rendered frame.
24. **`R-NF-3`: the keyring write is off the UI thread.** `secret::set_dsn` is reached only from
    inside a `tokio::task::spawn_blocking` on the store worker, and no call site exists in
    `crates/htui/src/ui/`. Behaviourally: with a fixture whose keyring write sleeps 500 ms, the event
    loop continues to draw and the tick continues to fire. Same for `ConnectionInfo`'s
    `secret::get_dsn`.
25. **Section cycling and key capture do not interfere.** With no field open, `h`/`l`/`[`/`]`/Left/
    Right cycle the Settings strip exactly as before and the 100×30 snapshot suite
    (`crates/htui/tests/settings.rs:36-40`) is green unmodified. With the field open, `h`, `l`, `q`
    and `[` all land in the buffer as text, no section change occurs, and no global binding fires —
    which follows from `Handled::Consumed` short-circuiting before both keymap lookups
    (`app/state.rs:404-421`) and is asserted rather than assumed. `Esc` closes the field, clears the
    buffer, and restores cycling in the same frame.
26. **The `Dsn` newtype cannot be printed.** A unit test asserts
    `format!("{:?}", Dsn::from("postgres://u:secret@h/db"))` contains neither `secret` nor
    `postgres`, and that the same holds for a `RequestEnvelope` carrying `StoreRequest::SetDsn`.
    `StoreRequest::name()` returns `"set_dsn"`, so `StoreReply::Failed` and every log line carry the
    name and not the value (`store_worker.rs:131-170`).
27. *(For MOD-4.)* A `FEAT` item on a **local-only** box runs `prd → plan → implement → review` end to
    end against `FakeDriver` and `FakeIsolator`, with every row in `local.sqlite` and none in any
    `pending/` directory: four `run_step` rows at positions 0 to 3, four documents whose kinds are the
    phase names, `item.status` moving `open → queued → in_progress → done`, and `Backend::active_runs`
    counting the run while it is live. This is `docs/ANA-2.md` §12 criterion 1 run against a third
    store, and it must pass with no Postgres process on the box.
28. **The two shared predicates agree across stores.** `htui-orch::conformance` runs ANA-2 §4.7's
    overlap predicate and `R-ORCH-10`'s tag-subset test over the same `RunScope` and tag inputs
    against `MemStore`, `PgStore` and `LocalStore`, and asserts identical answers, including the edge
    cases the SQL and the Rust spell differently: an empty `touched_paths` (unknown, overlaps
    everything), a `**` prefix, an empty `required_tags`, and a tag present in `declared_tags` but not
    `probed_tags`. This is §11 risk 23 made checkable.
29. **Scrubbing covers the three sinks runs add.** Extending criterion 11: a local graph step's
    `session_event` rows, a local `command_run.output` row and a captured `verify_command` output have
    each passed the recorder's scrubber before they reach `local.sqlite`, asserted by a fixture whose
    payload contains a masked value; and a residue that fails closed on the Postgres path fails closed
    here identically (`R-ID-7`, `R-SEC-3`, §10.6).
30. **The judge round-trips locally, and the constraint that would break it is absent.** A local
    fan-out of two produces a judge `run_step` at `fanout_index = -1` that inserts, selects and
    round-trips through `runs()` and `step_events()`; and a schema test asserts that
    `run_step.fanout_index` carries **no** `CHECK` in `local_migrations/`, failing if one is ever added
    (`docs/ANA-2.md` §12 criterion 8, its risk 12 at `:2075`, and §5.3's permanent prohibition).
31. **Interrupted local run, adjudicated by artefact and not by status.** An `htui` killed mid-step on
    a local-only box leaves a `running` `run_step` and an expired lease; on restart the recovery sweep
    of `docs/ANA-2.md:1284-1299` marks the step `done` when `after_hash` is present for every repo in
    scope **and** a document of `output_kind` exists, and `failed` with `gate_note = 'interrupted'`
    otherwise; a `dirty` `local`-mode tree is **never** reset. Separately, a second `htui` process
    started while the first is running takes no lease, opens the local store read-only (criterion 18)
    and adopts nothing.


---

## 13. Sources

**Local repository** (branch `main`, HEAD `2257e87`, read 2026-09-08).

`docs/REQUIREMENTS.md` — `R-ID-2..3` `:29-32`, `R-ID-7` `:41-42`, `R-USR-2` `:47-49`, `R-BOX-1`
`:52-53`, `R-ENT-5` `:75-77`, `R-ENT-6` `:78-88`, `R-ENT-7` `:89-90`, `R-ENT-10` `:98-99`,
`R-STO-1` `:111-113`, `R-STO-3` `:115-117`, `R-STO-4` `:118-121`, `R-STO-5` `:122-123`, `R-STO-6`
`:123-124`, `R-HIS-1..2` `:194-197`, `R-SEC-2..3` `:225-230`, `R-TUI-1` `:247-248`, `R-TUI-8`
`:264-265`, `R-NF-3` `:290-291`, the ID-stability rule `:16-17`.
`CONCEPTS.md` — `:20-21`, `:25-26`, `:28-30`, `:31-32`.
`.claude/rules/workflow-docs.md` — the REQUIREMENTS.md edit rule and the ANA/MOD lifecycle.
`HANDOFF.md` — ANA-10 `:56-74`, MOD-13 `:230-240`, MOD-15 `:245-258` including `:251` and `:257-258`.
`docs/ANA-9.md` — §1 `:20-25`, §2 invariants `:48-58`, §3 conventions `:100`, §4.1 `:111-152`, §4.2
`:154-190` (step 4 at `:187`), §4.3 `:191-265`, §4.4 `:266-348` (layout `:284-290`, mirrored/not
`:306-311`, rebuild triggers `:329-332`), §5.10 `:820-824`, §6.1 `:849-853`, §6.2 `:855-871`, §6.3
`:873-878`, §7.1 `:884-905`, §7.5 `:968-975`, §10 risks `:1025-1035`, §11 criteria `:1038-1052`,
status line `:11-13`.
`docs/ANA-5.md` — invariant 2 `:128-133`, §4.1 free-standing chat `:418-422`, `{{item_key}}` `:323`,
§8's store-method list `:1984-2008`, the conformance-target note `:2016-2025`, §10-13 as the house
format for this document's §10-13.
`docs/ANA-2.md` — §3 mirror-carries-run-tables `:222-227`, §4.9 offline window `:1325-1340` and the
rejected cancel row `:1258`, §8 read placement `:1699-1705` and the method lists `:1707-1756`, the
stale-premise table shape `:65-71`.
`docs/decisions/mod/mod-6.md` — the store as built: `Backend`'s three arms and the four labels
(§7 of that file), the mirror's rebuild triggers and single writer (§5), the seed and adopt-DB-id
rule (§3), the keyring DSN and the "no environment fallback" rule (`:66-69`), and the statement that
ANA-9 §11 criteria 1-4, 6 and 7 are pinned by tests.

`crates/htui-core/src/store/traits.rs` `:27-45`, `:47-55`, `:57-152`, `:153`, `:181-191`;
`conformance.rs` `:1-7`, `:23-43`, `:47`, `:56`; `error.rs` `:12-18`, `:23-32`;
`model/ids.rs` `:4-6`, `:31-33`, `:77-112`; `model/item.rs` `:48-52`, `:145-152`;
`model/run.rs` `:178-198`, `:229-246`; `store/mem.rs` `:60-90`, `:73-74`, `:548-575`.
`crates/htui-store/src/backend.rs` `:3-4`, `:6-19`, `:38-58`, `:66-91`, `:93-106`, `:113-118`,
`:133-146`, `:180-208`, `:222-296`, `:304-360`; `connect.rs` `:34-41`, `:66-70`, `:161-232`,
`:240-243`, `:267-284`, `:292-323`; `identity.rs` `:18-34`, `:44-52`, `:63-94`, `:104-124`,
`:126-144`, `:152-154`; `writer.rs` `:44-66`, `:67-88`, `:90-110`, `:156-184`, `:186-319`,
`:322-331`; `cache/mod.rs` `:1-8`, `:28-29`, `:31-56`, `:58-72`, `:74`, `:99-131`, `:135-162`,
`:164-193`, `:226-253`, `:261-316`; `cache/read.rs` `:18-19`, `:22-26`, `:54-117`, `:205-210`,
`:323-343`, `:596-659`, `:689-704`, `:762-800`; `cache/refresh.rs` `:3-6`, `:52-55`, `:200-305`,
`:317-393`, `:401-558`, `:596-625`, `:921-934`, `:1295-1372`; `cache/pending.rs` `:1-35`, `:63-69`,
`:84-134`, `:136-254`, `:267-360`, `:374-465`, `:507-580`; `pg/mod.rs` `:44-50`, `:104-108`,
`:230-333`, `:335-375`, `:387-389`, `:425-472`; `pg/write.rs` `:12-15`, `:40-117`, `:252`,
`:301-328`, `:444-465`; `testkit.rs` `:237-380`; `migrations/0001_init.sql` `:9-11`, `:21-25`,
`:214-224`, `:282-293`, `:299-304`, `:310-331`, `:340-352`, `:358-359`, `:554-561`, `:563-580`;
`cache_migrations/0001_mirror.sql` `:5-12`, `:14-25`, `:28-31`, `:33-128`;
`cache_migrations/0002_agent_mirror.sql` `:9-14`; `lib.rs` `:9-10`, `:20-21`, `:36-44`;
`tests/connect.rs` `:165-185`, `:227-241`; `tests/pg_criteria.rs` `:125`, `:172`, `:242-292`,
`:594-699`, `:803`; `tests/cache.rs` `:965-1060`; `tests/pg_conformance.rs` `:24`.
`crates/htui/src/lib.rs` `:6-9`, `:42-44`, `:61-79`, `:87`, `:109-134`; `cli.rs` `:20-32`;
`store_worker.rs` `:127-153`, `:209-236`, `:525-528`, `:537-548`, `:560-585`;
`agent_worker.rs` `:341-351`, `:1234-1238`; `event_loop.rs` `:31-49`; `app/action.rs` `:50-62`;
`app/mod.rs` `:47-49`, `:61`; `app/state.rs` `:33-34`, `:39-124`, `:161-184`, `:218`, `:375-377`;
`app/update.rs` `:12-13`, `:74-87`, `:132-134`, `:211-245`, `:262-279`, `:533-544`;
`ui/top_bar.rs` `:19-43`; `ui/overlay/registry.rs` `:36-53`, `:80-83`, `:95-169`;
`ui/overlay/migration_prompt.rs` `:26`, `:84-106`; `ui/overlay/workspace_switcher.rs` `:8`,
`:110-118`; `ui/tabs/registry.rs` `:134`; `ui/tabs/settings/mod.rs` `:3-9`, `:26-58`, `:60-141`,
`:143-169`, `:197-213`; `ui/tabs/settings/agents.rs` `:63-65`; `ui/tabs/chat/composer.rs` `:3-4`,
`:14-25`, `:58-84`; `tests/settings.rs` `:36-40`.
`Cargo.toml` (sqlx 0.9.0 with `postgres` + `sqlite`; `[workspace.lints.rust] unsafe_code = "forbid"`),
`Cargo.lock` (`libsqlite3-sys 0.37.0` → SQLite 3.51.3), `crates/htui-store/.sqlx/` (86 descriptions,
every one `"db_name": "PostgreSQL"`).
Vendored crate sources read for behaviour: `sqlx-core-0.9.0/src/config/{mod,common}.rs`,
`sqlx-macros-core-0.9.0/src/query/data.rs`, `sqlx-sqlite-0.9.0/src/{migrate.rs,options/mod.rs,
connection/worker.rs}`.

**Local-first prior art and promotion paths.**
ElectricSQL's four write patterns and the post-pivot "Electric does not do write-path sync":
https://electric.ax/docs/guides/writes ;
https://electric.ax/blog/2023/09/20/introducing-electricsql-v0.6 ;
https://queryplane.com/blog/electricsql-postgres-sync-engine/
PowerSync's schemaless client, `ps_crud` upload queue, consistency model and backend rules:
https://powersync.com/sync-postgres ;
https://docs.powersync.com/installation/app-backend-setup/writing-client-changes ;
https://docs.powersync.com/architecture/consistency ;
https://docs.powersync.com/handling-writes/custom-conflict-resolution ;
https://docs.powersync.com/usage/lifecycle-maintenance/implementing-schema-changes ;
https://powersync.com/blog/electricsql-vs-powersync ;
https://queryplane.com/blog/write-patterns-for-powersync/
Linear's sync engine, reverse-engineered and endorsed by its CTO:
https://github.com/wzhudev/reverse-linear-sync-engine ;
https://marknotfound.com/posts/reverse-engineering-linears-sync-magic/ ;
https://linear.app/docs/editing-issues ; https://linear.app/changelog/2021-02-18-importer-preview
WatermelonDB's `_status`/`_changed` sidecar and the client-id orphaning defect:
https://watermelondb.dev/docs/Implementation/SyncImpl ; https://watermelondb.dev/docs/Sync/Backend ;
https://github.com/Nozbe/WatermelonDB/issues/216 ; https://github.com/Nozbe/WatermelonDB/issues/1309
Turso embedded replicas, write forwarding and the `offline: true` opt-in:
https://docs.turso.tech/features/embedded-replicas/introduction ;
https://docs.turso.tech/reference/data-consistency
Litestream's read-only replicas and VFS write mode: https://litestream.io/how-it-works/vfs/ ;
https://litestream.io/guides/vfs-write-mode/ ; https://tip.litestream.io/guides/read-replica/
cr-sqlite's mergeable-schema rules: https://github.com/vlcn-io/cr-sqlite ;
https://docs.sqlitecloud.io/docs/sqlite-sync-best-practices
Fossil's one-file repository, ticket identity and project-code guard:
https://fossil-scm.org/home/doc/tip/www/tickets.wiki ;
https://fossil-scm.org/home/doc/tip/www/bugtheory.wiki ;
https://fossil-scm.org/home/doc/tip/www/sync.wiki ;
https://fossil-users.fossil-scm.narkive.com/DOlyodWL/error-wrong-project ;
https://sqlite.org/appfileformat.html
Anki's local ids, full sync and its directional prompt: https://docs.ankiweb.net/syncing.html ;
https://forums.ankiweb.net/t/full-sync-everytime/70479 ;
https://forums.ankiweb.net/t/unique-identifiers-during-anki-import/38029 ;
https://github.com/ankitects/anki/issues/4341
Zotero, Syncthing and Obsidian on attaching a sync identity to populated local data:
https://www.zotero.org/support/sync ;
https://forums.zotero.org/discussion/15114/merging-zotero-libraries-throug-sync ;
https://docs.syncthing.net/users/faq.html ;
https://deepwiki.com/obsidianmd/obsidian-help/2.3-synchronization-and-conflict-resolution
jj and git-bug on "start local, add a remote later" and content-addressed ids:
https://docs.jj-vcs.dev/latest/git-compatibility/ ; https://github.com/git-bug/git-bug
Realm / Atlas Device Sync client resets and breaking schema changes:
https://www.mongodb.com/docs/atlas/app-services/sync/error-handling/client-resets/ ;
https://www.mongodb.com/docs/atlas/app-services/sync/data-model/update-schema/
CouchDB/PouchDB replication ancestry and checkpoint failure:
https://docs.couchdb.org/en/stable/replication/protocol.html ;
https://docs.couchdb.org/en/stable/replication/conflicts.html ;
https://github.com/pouchdb/pouchdb/issues/3999 ; https://github.com/apache/pouchdb/issues/6730
The SQLite-then-Postgres promotion evidence — Grafana, Gitea, Synapse:
https://github.com/grafana/grafana/issues/102459 ;
https://github.com/wbh1/grafana-sqlite-to-postgres ;
https://github.com/go-gitea/gitea/issues/31820 ; https://github.com/go-gitea/gitea/issues/5651 ;
https://element-hq.github.io/synapse/latest/postgres.html
Kleppmann's local-first essay and its 2024 revision:
https://www.inkandswitch.com/essay/local-first/ ; https://martin.kleppmann.com/papers/local-first.pdf ;
https://powersync.com/blog/local-first-software-origins-and-evolution

**Rust and sqlx dual-dialect practice; SQLite capability references.**
sqlx `Any` driver limits and the UUID decode failure:
https://github.com/launchbadge/sqlx/issues/3571 ; https://docs.rs/sqlx/latest/sqlx/any/index.html ;
https://github.com/launchbadge/sqlx/pull/3998 ;
https://docs.rs/sqlx/latest/sqlx/sqlite/struct.SqliteConnectOptions.html ;
https://github.com/transact-rs/sqlx/blob/main/CHANGELOG.md
SQLite references: https://www.sqlite.org/lang_with.html ;
https://www.sqlite.org/lang_returning.html ; https://www.sqlite.org/lang_upsert.html ;
https://www.sqlite.org/lang_altertable.html ; https://www.sqlite.org/gencol.html ;
https://www.sqlite.org/stricttables.html ; https://www.sqlite.org/quirks.html ;
https://www.dbpro.app/learn/sqlite/errors/database-locked ;
https://sqlite.org/forum/forumpost/4350638e78869137
Multi-backend Rust precedents and the portability critique:
https://deepwiki.com/dani-garcia/vaultwarden/2.4-database-layer ;
https://github.com/dani-garcia/vaultwarden/pull/493 ;
https://github.com/dani-garcia/vaultwarden/wiki/Using-the-PostgreSQL-Backend ;
https://github.com/atuinsh/atuin ; https://lib.rs/crates/atuin-server-postgres ;
https://deepwiki.com/SeaQL/sea-orm/4-database-connections-and-transactions ;
https://github.com/drizzle-team/drizzle-orm/discussions/5269 ;
https://lobste.rs/s/2etd7f/sqlite_postgresql_it_s_complicated ;
http://mako.ai/guides/migrate-sqlite-to-postgresql ;
https://dev.to/grouparoo/dialect-differences-between-sqlite-and-postgres-in-sequelize-1f83
Embedded-Postgres options, named so they are rejected on evidence:
https://lib.rs/crates/postgresql_embedded ; https://lib.rs/crates/pglite-rs ;
https://github.com/pglite/pglite

**Offline id minting and reconciliation.**
RFC 9562 (UUIDv7 layout §5.7, index locality §2.1 and §6.11, no central registry §6.4, security §8):
https://www.rfc-editor.org/rfc/rfc9562.html — *the fetch of this document passed through a text
compression step and the fetcher flagged that some strings are not verbatim RFC wording; the section
numbers and the substantive claims above are what this document relies on, and any direct quotation
should be re-verified at rfc-editor.org.*
Postgres `uuidv7()` and the absence of any monotonicity or primary-key claim in its docs:
https://www.postgresql.org/docs/18/functions-uuid.html
UUIDv7 benchmarks, with the caveats recorded in §4.3:
https://mblum.me/posts/pg-uuidv7-benchmark/ ;
https://dev.to/umangsinha12/postgresql-uuid-performance-benchmarking-random-v4-and-time-based-v7-uuids-n9b ;
https://saybackend.com/blog/uuidv7-postgres-comparison/ ;
https://www.authgear.com/post/time-sortable-identifiers-uuidv7-ulid-snowflake/
Jira's server-assigned keys: https://support.atlassian.com/jira/kb/issue-keys-not-created-sequentially/ ;
https://support.atlassian.com/jira/kb/issue-key-increments-unexpectedly-on-new-issue-creation-in-jira-server/
Todoist's `temp_id` / `temp_id_mapping` and `INVALID_TEMPID`:
https://developer.todoist.com/api/v1/ ;
https://todoist-python.readthedocs.io/en/latest/_modules/todoist/api.html ;
https://github.com/Doist/todoist-python/issues/81
GitLab's `id` vs `iid` renumbering defects: https://gitlab.com/gitlab-org/gitlab-foss/-/work_items/1756 ;
https://gitlab.com/gitlab-org/gitlab-foss/-/merge_requests/3759 ;
https://gitlab.com/gitlab-org/gitlab/-/issues/519457
Postgres sequences are not gapless: https://www.postgresql.org/docs/current/sql-createsequence.html
Hi/lo and stride allocation, and their failure modes: https://vladmihalcea.com/the-hilo-algorithm/ ;
https://www.baeldung.com/hi-lo-algorithm-hibernate ;
https://vladmihalcea.com/hibernate-hidden-gem-the-pooled-lo-optimizer/ ;
https://www.percona.com/blog/2011/01/12/conflict-avoidance-with-auto_increment_incremen-and-auto_increment_offset/ ;
https://mariadb.org/auto-increments-in-galera/ ;
https://docs.oracle.com/cd/E17952_01/mysql-shell-8.0-en/mysql-innodb-cluster-auto-increment.html
Postgres `system_identifier` as the standby guard: https://pgpedia.info/d/database-system-identifier.html
Git 2.9's "refusing to merge unrelated histories" and its stated no-config rule:
https://oneuptime.com/blog/post/2026-01-24-git-refusing-merge-unrelated-histories/view
git-annex repository UUIDs and `reinit`: https://git-annex.branchable.com/forum/uuid_mismatch/ ;
https://manpages.ubuntu.com/manpages/focal/man1/git-annex-reinit.1.html
Import/adoption machinery — external-id mapping, FK remap through the same map:
https://johal.in/csv-import-pipeline-validation-guide ;
http://banner.tbr.edu/E11882_01/server.112/e22490/dp_import.htm ;
https://www.doopartners.com/blog/integrations-and-data-6/fixing-the-external-id-errors-when-you-import-data-into-odoo-6

**Gaps in the source base, recorded rather than papered over.** No first-party ElectricSQL
retrospective on the CRDT-to-read-path pivot was found; the reasoning is reconstructed from current
docs plus third-party analysis. No benchmark compares UUIDv7 against a bigint sequence primary key
in Postgres, so the cost of client-minted UUIDs relative to `htui`'s own integer `key_number` is
unmeasured. Nothing was found on SQLite `application_id` / `user_version` conventions used to gate a
writable local store, and no project was found that upgraded a read-only local mirror **in place**
into a writable primary — the direction §4.4 contemplates appears unrepresented in the literature.
And no external UX precedent was found for distinguishing "no server configured" from "server known,
unreachable"; §4.5's answer rests on the in-tree facts and on Syncthing's and Turso's structural
argument that the two must be declared rather than inferred.









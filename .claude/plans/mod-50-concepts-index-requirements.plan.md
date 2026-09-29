# Plan: MOD-50 — requirements and resolution in the concepts index

> **Status: done** (2026-09-29). Confirmed by the maintainer with the R-STO-8 amendment (D229),
> implemented in `9c381db`, review fixes `5417e78`, closed out in
> [`docs/decisions/mod/mod-50.md`](../../docs/decisions/mod/mod-50.md). Fact-check: 16 claims, 16
> verified, 0 falsified; verdicts in "Verified claims" below. Two deviations are recorded in the
> write-up: `point_type` is derived rather than stored (D225), and the live exact-key case asserts
> membership rather than first rank (T4).

**Source**: `HANDOFF.md` MOD-50 (from MOD-34 and MOD-38; `docs/decisions/mod/mod-34.md` "Left to
other items", `docs/decisions/mod/mod-38.md`). MOD-34 built the concepts index over items and their
latest documents and reserved `type = requirement`. MOD-38 added the `requirement` table and
`item.resolution`. Until now `--decisions` has kept every point whose item status is `done` or
`closed` (`crates/htui/src/concepts.rs:33-35`). That includes withdrawn, duplicate and superseded
items, and done items that were never closed out.

**Requirements**: `R-STO-8` (the index), `R-ENT-8` (a closed item carries a resolution; resolution
is not a status), `R-ENT-14` (the requirement row).

**Complexity**: Small to medium. Two store modules, the CLI's search arm and help text, the live
suite and the README. **No migration, no store-trait change, no `.sqlx` change, no new
dependency.** Main's next migration stays 0008.

**Routing**: routed as **plan** by `/handoff-run MOD-50` and accepted by the maintainer on
2026-09-29. 0 of 4 criteria fired: no cross-repo reach; no new architecture surface, because the
point type was reserved on the existing `VectorStore` seam; no open question in the item text;
about 6 files. Ultracode: not needed. The tasks are serial through one set of types.

## Summary

Every point gains what its row says about itself. Item and document points carry
`item.resolution` next to `status`. Requirement rows become points of their own. `--decisions`
filters on resolution. The collection goes to `htui_concepts_v2`, because the points already in v1
lack the new field and the indexer's freshness check cannot tell.

## Design decisions (settled here, not in code review)

- **D222: the collection becomes `htui_concepts_v2`.** The indexer rebuilds an item only when its
  `updated_at` or `status` moved (`vector_sync.rs:67-71`). An item closed before this change has
  neither moved, so without a new collection its points would never gain `resolution`. The listing
  it compares against (`ItemSummary`) has no resolution to compare (`item.rs:162-179`).
  `vector.rs:30-31` already sets the rule: "a change to the vector layout or the payload gets a new
  name rather than a migration, since the whole index can be rebuilt from Postgres". The first
  `--index-items` after the upgrade fills v2 from scratch. v1 is left in place, never read again,
  and the write-up says how to drop it. The bump also settles MOD-34's deferred "points whose
  payload no longer parses are invisible to `indexed()`" for requirement points. No binary that
  cannot parse `type = requirement` ever reads v2.
- **D223 (maintainer, 2026-09-29): `--decisions` means resolution ∈ {`done`, `concluded`,
  `rejected`}.** `withdrawn` ("dropped without a decision"), `duplicate` and `superseded` are left
  out, and so is a `done` item that was never closed. `DECISION_STATUSES` becomes
  `DECISION_RESOLUTIONS`. `SearchQuery` gains `resolutions: Vec<Resolution>` (empty = all). It
  keeps `statuses`, which nothing in the CLI sets any more but which stays a valid filter for
  MOD-11. A requirement point has no `resolution` field, so a resolution filter excludes it.
  Qdrant's `match any` fails on a missing key (claim 9), and the in-memory fake mirrors that.
- **D224: one point per requirement row, withdrawn ones included.** Text:
  `"{key} {body}\n\n{rationale}"`, embedded dense and sparse like the rest, so `R-STO-8` finds an
  exact key through BM25. Payload: `type`, `requirement_id`, `key`, `project_id`, `area_code`,
  `priority`, `state`, `version`, `updated_at` and `snippet`. The point ID is a v8 UUID from
  SHA-256 over `"htui-concepts/requirement\0"` and the requirement UUID. It is built the way
  `document_point_id` is built, so it cannot collide with an item point's raw item UUID whatever the
  ID classes do. A withdrawn requirement stays searchable because ANA-11 keeps it as history. The
  hit line says so: `requirement (withdrawn)`.
- **D225: points name their row through a `Subject` enum.** `ConceptPoint`'s `item_id`, `kind_id`
  and `status` become
  `subject: Subject`, with `Item { id, kind_id, status, resolution }` or
  `Requirement { id, area_code, priority, state, version }`. `point_type` stays a stored field
  checked against the subject by a debug assertion, which keeps every construction site explicit.
  `IndexedPoint` and `Hit` replace `item_id` with `owner: Owner`, which is `Item(ItemId)` or
  `Requirement(RequirementId)`. `IndexedPoint` gains the requirement `version` next to `status`.
  `Hit` gains `resolution` and the requirement `state` for the result line. Outside
  `vector.rs`/`vector_sync.rs`, the only users are `crates/htui/src/concepts.rs` and
  `crates/htui-store/tests/qdrant_live.rs` (claim 1). No MOD-11 code exists yet.
- **D226: a requirement is fresh when its indexed `version` equals the row's.** Every amend and
  withdraw bumps `requirement.version` on both stores (claims 4, 5), so `version` alone decides.
  `updated_at` is carried for display and parity only. Per project the indexer makes one
  `requirements(project, &RequirementFilter::default())` call, with no per-row read. It rebuilds the
  stale requirement points, deletes those whose row is gone, and counts them in the same
  `SyncReport` fields. `items_rebuilt`/`items_unchanged` stay item counts, and the report gains
  `requirements_rebuilt`/`requirements_unchanged`. `--index-items` prints both.
- **D227: the result line names the resolution of a closed item.** `D223` puts `rejected` next to
  `done` in one list. A rejected decision printed like an adopted one misleads, so the place column
  reads `item (rejected)` / `summary document (done)`. A requirement reads `requirement` or
  `requirement (withdrawn)`. An open item prints as today.
- **D228: `resolution` gets a keyword payload index,** added to `ensure_collection`'s loop, which
  re-runs on every connect and is idempotent (`vector.rs:387-411`).

- **D229 (maintainer, 2026-09-29): `R-STO-8` is amended in place** to name requirement rows and
  the decisions filter. The maintainer typed the ok ("yes include amend to r sto 8"). The status
  header of `docs/REQUIREMENTS.md` gains the matching amendment line.

## Patterns to Mirror

- `document_point_id` (`vector.rs:130-139`): the SHA-256 → v8 UUID derivation and its golden test.
  `requirement_point_id` copies both.
- `Indexer::sync`'s per-project loop (`vector_sync.rs:47-105`): list, compare with `indexed()`,
  upsert the stale, delete what is gone. The requirement pass is the same shape after the item pass.
- `search_filter` (`vector.rs:295-325`): one `must` condition per non-empty set.
  `resolutions` is one more.

## Files to Change

| File | Change |
|---|---|
| `crates/htui-store/src/vector.rs` | `PointType::Requirement`, `Subject`/`Owner`, payload keys and parsing, `resolutions` filter, `requirement_point_id`, `COLLECTION = "htui_concepts_v2"`, `resolution` index, `MemVectorStore` parity, unit tests (D222-D226, D228) |
| `crates/htui-store/src/vector_sync.rs` | `item_point`/`document_points` fill `Subject::Item` with the resolution; new `requirement_point`; requirement pass in `sync`; `SyncReport` fields; unit tests (D224, D226) |
| `crates/htui/src/concepts.rs` | `DECISION_RESOLUTIONS` and the `resolutions` arm in `search_items` (lines 33-35, 126-137); `format_hit` (D227); the `--index-items` report line. **Not lines 52-62 (`open`), which MOD-40 moves** |
| `crates/htui/src/cli.rs` | `--decisions` help text (line 47) |
| `crates/htui-store/tests/qdrant_live.rs` | live cases: the decisions filter, a requirement found by its exact key, a withdrawn requirement re-indexed |
| `README.md` | the `--decisions` row (line 84) and the Search section: requirements are indexed |
| `docs/REQUIREMENTS.md` | `R-STO-8` amended in place, plus the status header line (D229) |

Close-out (after review): `HANDOFF.md` MOD-50 closed, `DECISIONS.md` index line,
`docs/decisions/mod/mod-50.md`.

## Tasks

Serial. T2-T4 all build on T1's types, so no two tasks run in parallel (every file set below
intersects with T1's through the types it exports).

### T1: the index types (`vector.rs`)

1. **Tests first:** payload round trip for a requirement point and for a closed item with a
   resolution; `requirement_point_id` golden and distinct from the item UUID; `search_filter` with
   `resolutions` adds one `must`; `MemVectorStore` excludes requirement points under a resolution
   filter.
2. Green: D222, D224, D225, D228, and the fake's filter.

### T2: the indexer (`vector_sync.rs`)

1. **Tests first,** on `MemStore::demo()`, whose three htui requirements and closed `FIX-1`
   (resolution `done`) the fixture already has (claims 6, 7):
   - the first sync writes one point per requirement;
   - a second sync writes nothing;
   - an amended requirement is rebuilt alone;
   - a withdrawn one keeps its point with `state = withdrawn`;
   - a closed item's points all carry its resolution.
2. Green: D226.

### T3: the CLI (`concepts.rs`, `cli.rs`, `README.md`)

1. **Tests first:** `format_hit` for a rejected item, a document of a done item, an active and a
   withdrawn requirement, and an open item unchanged; `DECISION_RESOLUTIONS` is exactly D223's
   three.
2. Green: D223, D227, and the help text and README.

### T4: the live suite (`qdrant_live.rs`)

Against a real Qdrant (`HTUI_TEST_QDRANT_URL`):
- `--decisions`' query returns no requirement point and no item outside D223's three;
- `R-ENT-1` finds its requirement point first;
- a withdrawn requirement is rebuilt with its new state and nothing else moves.

## Test plan

- `cargo test -p htui-store --features test-support --lib vector` covers T1 and T2.
- `cargo test -p htui --lib concepts` covers T3 (with `ORT_LIB_LOCATION` if `local-embed` builds).
- `HTUI_TEST_QDRANT_URL=http://localhost:6334 cargo test -p htui-store --features test-support --test qdrant_live`
  covers T4. Qdrant 1.19.0 runs locally here, and the existing three cases are green on it as the
  baseline.

## Risks

1. **The first `--index-items` after the upgrade re-embeds everything.** It is a one-off full
   rebuild, the same cost as the very first index. It is stated in the write-up and the README.
2. **v1 is left behind on the server.** Harmless, but it takes space until dropped. The write-up
   gives the one `curl -X DELETE` that drops it. Dropping it automatically was rejected, because a
   binary older than this one may still be pointed at the same Qdrant.
3. **`Hit`/`IndexedPoint`/`ConceptPoint` change shape.** They are public in `htui-store`, but their
   only users are the two named files (claim 1). MOD-11, which will call `search`, has no code yet.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
HTUI_TEST_QDRANT_URL=http://localhost:6334 cargo test -p htui-store --features test-support -- --test-threads=1
cargo test -p htui -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

No SQL, no migration, no `.sqlx` change, so `sqlx prepare --check` is not re-run.

## Acceptance

- [ ] Requirement rows are indexed as `type = requirement` points, and a withdrawn one keeps its
      point with its state.
- [ ] Item and document points carry `resolution` when the item is closed.
- [ ] `--decisions` keeps only items (and their documents) closed as done, concluded or rejected,
      and no requirement.
- [ ] A second sync with no change writes nothing, for requirements as for items.
- [ ] The result line names a closed item's resolution and a withdrawn requirement.
- [ ] The collection is `htui_concepts_v2`, and the live suite is green against a real Qdrant.
- [ ] `concepts.rs` lines 52-62 are untouched.

## Verified claims

Fact-checked 2026-09-29 against the tree at `34b9888` (+ `a345074`, HANDOFF only) and Qdrant 1.19.0.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Outside `vector.rs`/`vector_sync.rs`, only `concepts.rs` and `qdrant_live.rs` use `ConceptPoint`/`IndexedPoint`/`Hit`/`SearchQuery`/`PointType` | verified | `grep -rn "SearchQuery\|IndexedPoint\|vector::Hit\|PointType" crates` → `crates/htui-store/tests/qdrant_live.rs` only, besides `concepts.rs`; `crates/htui/src/{lib,cli}.rs` reach the module only through `concepts::{index_items,search_items}` |
| 2 | Freshness compares only `updated_at` and `status` | verified | `vector_sync.rs:67-71` |
| 3 | `ItemSummary` carries no resolution | verified | `item.rs` `Item::summary()`: `id … touched_paths`, no `resolution` |
| 4 | MemStore's amend and withdraw both bump `version` | verified | `mem.rs:6127`/`:6146` both run the shared update under `self.write`; `:5217-5220` sets `state = Withdrawn` for a withdraw, then `row.version += 1; row.updated_at = now` |
| 5 | PgStore's amend and withdraw both bump `version` | verified | `pg/write.rs:5305`/`:5338` share the `UPDATE requirement SET … state = COALESCE($5, state), version = version + 1` at `:520-531` |
| 6 | The demo fixture has requirements in project htui | verified | `fixtures.rs:11` ("two areas, three requirements and five citations"); `:327-331` `REQ_ENT_1`, `REQ_ENT_2`, `REQ_STO_1` |
| 7 | The demo's closed items carry `Resolution::Done` | verified | `fixtures.rs:1081`; `FIX-1` is `Status::Closed` at `:958` |
| 8 | A closed item's resolution never changes, so the status check covers it after the first index | verified | `Status::Closed.can_move_to(_) == false` (`item.rs:64`); `close_out` sets both at once (`chk_item_resolution_iff_closed`, `docs/decisions/mod/mod-38.md:19-21`) |
| 9 | Qdrant `match any` on a keyword field excludes a point that lacks the key | verified | probe on 1.19.0: points `{resolution: done}`, `{type: requirement}`, `{resolution: withdrawn}`; filter `resolution any [done, concluded, rejected]` returned `[1]` only |
| 10 | `Resolution`, `RequirementState`, `Priority` have `as_str`, `FromStr` and `ALL` | verified | all three are `str_enum!` (`item.rs:69-87`, `requirement.rs:12-30`); the macro emits `ALL` (`mod.rs:47`), `as_str` (`:51`), `FromStr` (`:64`) |
| 11 | `requirements()` with the default filter returns withdrawn rows too | verified | `RequirementFilter` fields are all `None` by default, and `None` "does not filter" (`requirement.rs` doc on `RequirementFilter`); `mem.rs:4873-4875` filters states only when `Some` |
| 12 | `ensure_collection` re-creates payload indexes on every connect, idempotently | verified | `vector.rs:387-411` and its comment |
| 13 | `vector.rs` sets the rename-on-payload-change rule | verified | `vector.rs:28-30`, the doc on `COLLECTION` |
| 14 | MOD-40's `concepts.rs` footprint is `open()` only, lines 52-62, which this plan does not touch | verified | `/mnt/project-files/next-items/parallel-plan-2026-09-29.md` ("MOD-40's remaining footprint"); `open` is `concepts.rs:52-69` |
| 15 | The help text and README both describe `--decisions` as "done and closed" | verified | `cli.rs:47`, `README.md:84` |
| 16 | T2-T4 each depend on T1's exported types, so there is no parallel pair | verified | T2 builds `ConceptPoint`/`Subject`, T3 matches on `Hit`/`Owner`, T4 constructs `SearchQuery` with `resolutions`; all are T1 outputs |

# MOD-50 - Requirements and resolution in the concepts index (done, 2026-09-29)

**Requirements:** `R-STO-8` (amended in place by this item), `R-ENT-8`, `R-ENT-14`.
**Origin:** MOD-34 ("Left to other items") and MOD-38. MOD-34 built the concepts index over items
and their latest documents and reserved `type = requirement`. MOD-38 added the `requirement` table
and `item.resolution`.
**Artifacts:** plan [`.claude/plans/mod-50-concepts-index-requirements.plan.md`](../../../.claude/plans/mod-50-concepts-index-requirements.plan.md)
(routed plan 2026-09-29, 0 of 4 routing criteria fired, decisions D222–D229).
**Commits:** `a345074` (MOD-64 minted), `f493ca5` (plan), `9c381db` (the change), `5417e78`
(review findings), plus the close-out commit.

## What shipped

- **Requirement points.** Each `requirement` row is one `type = requirement` point. The embedded
  text is `"{key} {body}\n\n{rationale}"`. The payload holds `requirement_id`, `area_code`,
  `priority`, `state` and `version`. Withdrawn requirements stay indexed, since ANA-11 keeps them as
  history. The point ID is a v8 UUID derived from SHA-256 over a domain separator and the
  requirement UUID, so it can never meet an item point's raw UUID (D224).
- **Resolution on item and document points.** Item and document points carry `resolution` when the
  item is closed. The key is absent, not null, otherwise. `resolution` has a keyword payload index
  (D228).
- **`--decisions` filters on resolution.** `htui --search-items --decisions` keeps items closed as
  `done`, `concluded` or `rejected`, and their documents. The maintainer chose that set on
  2026-09-29 (D223). `withdrawn`, `duplicate` and `superseded` are out, and so is a `done` item
  that was never closed. Requirements are out too, because a missing key fails Qdrant's
  `match any`. The in-memory fake mirrors that rule. `SearchQuery` gained `resolutions` and keeps
  `statuses` for MOD-11.
- **Indexer.** Per project, the indexer lists requirements once. It rebuilds a requirement when
  its indexed `version` differs from the row's, which every amend and withdraw bumps on both
  stores. It deletes points whose row is gone (D226). `SyncReport` gained
  `requirements_rebuilt`/`requirements_unchanged`, and `--index-items` prints them.
- **Result line.** It names a closed item's resolution (`item (rejected)`,
  `summary document (done)`) and a withdrawn requirement (`requirement (withdrawn)`) (D227).
- **Collection `htui_concepts_v2`** (D222). A v1 point of an item closed before this change would
  never have gained `resolution`: neither its `updated_at` nor its status moves again, and the
  listing the indexer compares against (`ItemSummary`) has no resolution. The first
  `--index-items` after upgrading rebuilds everything. v1 is left on the server and never read. The
  README gives the `curl -X DELETE` that drops it. The rename also settles MOD-34's deferred
  "unparseable payloads are invisible to `indexed()`" for requirement points, because no binary
  that cannot parse them reads v2.
- **Types.** `ConceptPoint` names its row through `Subject` (`Item { id, kind_id, status,
  resolution }` | `Requirement { id, area_code, priority, state, version }`). `IndexedPoint` and
  `Hit` carry `owner: Owner` in place of `item_id`. `Hit` gained `resolution` and `state` (D225).
- **`R-STO-8` amended in place** by maintainer decision (D229, typed ok "yes include amend to r sto
  8"). It now names requirement rows and the decisions filter, and the `docs/REQUIREMENTS.md` status
  header carries the amendment line.
- **MOD-64** (concepts search in the TUI) was minted at the maintainer's request. Until now the
  index was command-line only.

## Deviations from the plan

- **`point_type` is derived, not stored.** `ConceptPoint::point_type()` computes it from the
  subject and the document. That makes an inconsistent point unconstructible where the plan only
  had a debug assertion. The review then found the remaining gap: a `Requirement` subject with a
  `document` could still write document keys. `payload` now writes them only in the `Item` arm,
  and a test pins that.
- **The live exact-key case asserts "among the hits", not "first".** The live suite embeds with
  `HashEmbedder`, whose dense arm is noise. Under RRF, `R-ENT-2`, which shares the `ent` term,
  tied `R-ENT-1` on score. The item case already asserted membership for the same reason.

## Review

The configured reviewer (`rust-reviewer`) found nothing blocking. It confirmed the payload round
trip, the freshness rules, the delete scope, point-ID stability and filter parity between Qdrant
and the fake. Applied in `5417e78`:
- a sync test now gives the closed demo item a document and asserts that its document points carry
  the resolution. Before, nothing pinned the document half.
- the README upgrade note (the v2 rebuild, and dropping v1);
- document keys are written only for item subjects;
- the result column widened to 26, so `summary document (done)` keeps the columns aligned;
- the live decisions case asserts each hit's resolution is one of the three, not just present;
- `Subject::{status, resolution, state, version}` replace ad hoc matches in the fake;
- requirement upserts are batched by 256;
- doc comments updated (`points_deleted`, the module summary, `SearchQuery::resolutions`).

## Verification

- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-features --all-targets -- -D warnings` are clean.
- `cargo test -p htui-store --features test-support` passed with `HTUI_TEST_QDRANT_URL` set,
  against a local Qdrant 1.19.0. The live suite ran 5 cases (2 new), and the lib's
  vector/vector_sync tests passed 25. `cargo test -p htui` passed (lib 427). The Postgres suites
  were skipped (no `HTUI_TEST_DATABASE_URL`). This change touches no SQL.

## Left to other items

- **MOD-64:** the TUI search over this index.
- **MOD-11:** the agent-facing `search_concepts` tool now also returns requirement hits. `Hit.owner`
  says which kind each hit is.
- **MOD-41:** the automatic sync. It picks up requirements with no further change.

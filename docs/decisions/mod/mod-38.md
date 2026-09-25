# MOD-38 - Requirements schema, seam and close-out resolution (done, 2026-09-25)

From ANA-11 (`docs/ANA-11.md` §5, §5.1, §5.2). Requirements: `R-ENT-8`, `R-ENT-14`, `R-ENT-15`,
`R-NF-4`, `R-STO-3`, `R-STO-5`, `R-TUI-9`. The route was PRD, accepted by the maintainer:
- PRD: `.claude/prds/mod-38-requirements-schema.prd.md`
- plan: `.claude/plans/mod-38-requirements-schema.plan.md`
- blueprint: `.claude/plans/mod-38-requirements-schema.blueprint.md`

## Before it shipped

The maintainer applied ANA-11 §7 to `docs/REQUIREMENTS.md` on 2026-09-25 (`86c57c4`):
- `R-ENT-14` and `R-ENT-15` were added.
- `R-ENT-8`, `R-NF-4`, `R-STO-3` and `R-TUI-1` were amended in place.
- The optional `R-MCP-2` `requirement_cite` amendment is deferred to MOD-11.

## What shipped

**Resolution** (milestone 1):
- `item.resolution` is one of `done`, `concluded`, `rejected`, `withdrawn`, `superseded` or
  `duplicate`. It is guarded by `chk_item_resolution_iff_closed`
  (`(status = 'closed') = (resolution IS NOT NULL)`).
- The migration backfills `done` onto already-closed rows with `trg_item_updated_at` disabled, so
  their `updated_at` does not move.
- `Resolution` is a `str_enum!` in `htui-core/src/model/item.rs`.
- `close_out(item, resolution, summary, commits)` now takes the resolution on every store.
- `Command::CloseOut { item, resolution }` carries it. Until MOD-39's picker, the Runs pane sends
  `Resolution::default_for(status)`, which `closeout::Preview` also shows in the first
  confirmation as "close KEY as RESOLUTION".
- The cache mirrors the column.

**Requirements** (milestone 2):
- Tables: `requirement_spec`, `requirement_area`, `requirement_key_counter`, `requirement`
  (generated key `R-<AREA>-<N>`) and `requirement_revision`, all in migration
  `0006_requirements.sql` (written as `0005`, renumbered at the merge with MOD-7 milestone 1,
  whose `0005_box_identity.sql` landed first).
- Writes: `set_requirement_spec` (CAS), `create_requirement_area`, `mint_requirement`,
  `amend_requirement` (CAS, returning `RequirementUpdate::Diverged` with the ancestor revision) and
  `withdraw_requirement`. They are implemented on MemStore and PgStore.

**Citations and offline** (milestone 3):
- `item_requirement` holds `addresses`, `amends`, `withdraws` and `reserves` citations, each with
  a version stamp and a tombstone.
- `cite`, `uncite` and `reconfirm` are the writes. Suspect status is derived on read and never
  stored.
- `item_requirements(item)` and `requirement_coverage(requirement)` are the reads.
- The cache migration `cache_migrations/0004_requirements.sql` mirrors the spec, areas,
  requirements and live citations. `MIRRORED_TABLES` has 21 entries.
- `DeleteReach` counts six new row kinds, and `StoreReply` now boxes it.
- The demo fixture has a spec header, two areas and three requirements, with one suspect citation
  and one tombstoned citation. FIX-1 closes as `done`.

## Decisions

- **D1 (maintainer): `closed` is reachable only through `close_out`.** `Status::can_move_to` lost
  its `blocked`, `failed` and `done` → `closed` edges, so `WriteStore::transition` refuses
  `→ closed` on every store. This **amends ANA-2 §4.3**. The close-out law is
  `Resolution::closes_from`:
  - `done` and `concluded` close only from `done`;
  - `rejected`, `withdrawn`, `superseded` and `duplicate` close from `open`, `blocked`, `failed` or
    `done`;
  - a live run still refuses, before the resolution is checked.

  An item withdrawn before it ever ran can now close.
- **D2 (maintainer): the command carries the resolution now.** Until MOD-39 the default is
  `done` → `done` and `blocked`/`failed` → `withdrawn`. An `open` item stays greyed in the Runs
  pane: the store accepts it, but the TUI offers it only once MOD-39 has a picker.
- **D3 (maintainer): amending or withdrawing a requirement also records the citation.** The same
  transaction upserts the deciding item's `amends` or `withdraws` citation, stamped at the new
  version.
- Plan decisions D4–D15 are in the plan. Where they deviate from ANA-11:
  - The requirement mint follows `mint_item`'s `INSERT … ON CONFLICT DO UPDATE … RETURNING` CTE,
    not §5.1's "`UPDATE … RETURNING`".
  - The cache also mirrors the one-row `requirement_spec`, which §5.2 did not. Revisions stay
    online-only: `requirement_revisions` answers `None` offline.
  - The constraint is named `chk_item_resolution_iff_closed`, following 0001's `chk_` convention.
- `reconfirm` refuses an `addresses` or `reserves` citation of a withdrawn requirement, as `cite`
  does. This was a review finding.

## Verification

- `cargo test --workspace --all-features` against Postgres 16: 1866 passed. The only failure,
  `htui-agent/tests/excerpt.rs::every_provider_failure_leaves_a_valid_prompt`, fails on main too.
- `cargo clippy --workspace --all-targets --all-features -D warnings` and `cargo fmt --check` are
  clean.
- `cargo sqlx prepare --check` is clean.
- Pins: store conformance `CASES` 65 and `READ_CASES` 14; `htui-orch` `CASES` 70 (unchanged);
  256 `.sqlx` files. After the merge with MOD-7 milestone 1 (migration renumbered to `0006`):
  `CASES` 68 and 263 `.sqlx` files; `migrations.rs` and `connect.rs` pin six migrations.
- The `rust-reviewer` gate found no blocking or major findings. All nine minor findings and nits
  were fixed in `f5842a1`.

## Carried

- MemStore's project delete does not clear `item_link.proposed_by_step_id` on surviving links.
  This predates MOD-38 and is outside its scope.
- `idx_item_requirement_req` covers live citations only.
- Cross-project citations whose other end is not mirrored are absent offline, as `links` already
  are.
- MOD-39 builds the Requirements tab and the resolution picker. MOD-8 imports into these tables.
  MOD-11 owns `requirement_cite`. MOD-34 can add `resolution` to its payload and index
  `requirement` rows.

## Commits

- `86c57c4`: requirements amendments.
- `c6f03ca`, `957c5c6`, `7998f9e`: PRD, plan and blueprint.
- `873631e` T1, `97cfc76` T2, `3b141a6` T3, `d6f61fd` T4, `9fda0e7` T5, `b5863fc` (sqlx).
- `92e5f7e` T6, `dc6c4e1` T7, `26f417e` T8, `cc0aa9c` T9, `8b877e6` (sqlx).
- `5f4d080` T10.
- `f5842a1`: review fixes.

# MOD-39 — Requirements tab and item traceability

> Routed as **PRD** by `/handoff-run MOD-39` (criteria C2 and C4 fired, C3 a low-confidence miss;
> accepted by the maintainer 2026-09-29 as "PRD + ultracode"). Ultracode for the implement phase
> (C4). CLEAN-6 (the Runs pane module doc) is folded in because it is a comment in the same file the
> resolution picker changes. `docs/ANA-11.md` §6 is the design; MOD-38 built the seam
> (`docs/decisions/mod/mod-38.md`). Requirements: `R-TUI-1`, `R-TUI-4` (CLEAN-6), `R-TUI-9`,
> `R-ENT-8`, `R-ENT-14`, `R-ENT-15`.

## Problem

MOD-38 put requirements, revisions, suspect-aware citations and `item.resolution` in the store, but
nothing in the TUI reads or writes them. A maintainer cannot see which items address a requirement,
cannot tell that a citation was written against an older text, cannot amend or withdraw a
requirement, and cannot close an `open` item as withdrawn or rejected: the Runs pane's close-out
still uses `Resolution::default_for(status)`, so `open` is greyed out and a `done` item can only
ever close as `done`. `R-TUI-1` already lists a Requirements tab that does not exist.

## Evidence

Read at `34b9888` (main). Paths are relative to `crates/`.

- **Seam is complete.** Reads (`requirement_spec`, `requirement_areas`, `requirements`,
  `requirement`, `requirement_revisions`, `item_requirements`, `requirement_coverage`) at
  `htui-core/src/store/traits.rs:202-253`; writes (`set_requirement_spec`,
  `create_requirement_area`, `mint_requirement`, `amend_requirement`, `withdraw_requirement`,
  `cite`, `uncite`, `reconfirm`) at `:1314-1413`, all implemented on MemStore, PgStore and `Writer`;
  the cache implements the seven reads (`htui-store/src/cache/read.rs:1071-1246`) but
  `requirement_revisions` is always `Ok(None)` offline (`:1183`). No `todo!()`. No area rename,
  reorder or delete exists.
- **No role model.** `app_user` has no role (`htui-store/migrations/0001_init.sql:31-37`); the
  only ownership hook is `requirement_spec.owner_id` (`0006_requirements.sql:36`). MOD-38's PRD
  put "maintainer-only" enforcement on MOD-39 (`.claude/prds/mod-38-requirements-schema.prd.md:132`).
  Human vs agent on a citation is `proposed_by_step_id: None`.
- **No TUI wiring.** `StoreRequest` (`htui/src/store_worker.rs:98`) and `StoreReply` (`:750`) have
  no requirement variant. Domain families are served by their own module and one or-ed arm in
  `try_serve` (`:1135-1275`, e.g. `templates::serve` `:1219`, `skills::serve` `:1225`); the worker
  fills `this_user`, and a CAS miss comes back as a `*Stale` reply (`TemplatesStale`, `:902-905`).
- **Tabs.** `trait Tab` at `htui/src/ui/tabs/registry.rs:35-64`; `register_all`
  (`htui/src/app/mod.rs:47-91`) registers Backlog, Skills, Settings, Chat, so digits read 1..4.
  `R-TUI-1` orders them "Backlog, Chat, Skills, Requirements, Settings". Skills
  (`ui/tabs/skills/mod.rs`, 147 lines, list + detail + `TextField`/`TextArea` modes, one write in
  flight via `busy`) is the pattern to mirror.
- **Scope is a workspace.** `Scope { workspace_id, project_ids }` (`htui-core/src/model/scope.rs:10`);
  requirements are per project, so the tab groups by project the way the Backlog does.
- **Item detail.** Six sub-tabs (`ui/tabs/backlog/mod.rs:61-70`); `go()` (`:106-127`) sends one
  request per sub-tab on selection. The strip " Body Runs Graph Documents Notes Prompt " is 40
  columns and `the_detail_strip_fits_the_detail_pane` pins it inside a 43-column pane
  (`detail/mod.rs:247-253`), so a seventh title does not fit as is (see D3).
- **Close-out.** `CloseOutStage { Counting, Warn(Preview), Typed{..}, InFlight }`
  (`detail/runs.rs:218-232`); `Typed` sends `Command::CloseOut { item, resolution:
  preview.resolution }` (`:716-719`). `Command::CloseOut` already carries a resolution
  (`htui-orch/src/command.rs:131`). `close_out_enabled` (`command.rs:1184-1206`) returns
  `default_for(status)` or refuses, which is what greys out `open`; `Resolution::closes_from`
  (`htui-core/src/model/item.rs:94-102`) is the full legality law and the store enforces it
  (`resolution_not_closable`, `traits.rs:1827`).
- **CLEAN-6.** `runs.rs:19` reads "`a` / `x` approve / reject with a typed note"; `a` sends
  `AnswerGate(Approved)` at once (`:504-510`), only `x` opens `Mode::RejectNote` (`:511-517`).
- **Demo.** `htui-core/src/fixtures.rs:1785-1998`: project htui has a spec (owner `USER`), areas
  ENT and STO, R-ENT-1 (v2, amended by ANA-2), R-ENT-2, R-STO-1, five citations (ANA-1 → R-ENT-1
  stamped v1 is **suspect**; FEAT-2 → R-ENT-2 is tombstoned). Enough for every snapshot below.
- **Pins that move.** HANDOFF "Live coordinates" pins `StoreRequest` 75 and `StoreReply` 43
  variants and 96 snapshot files; MOD-39 changes all three.

## Users

- **The maintainer**, who decides requirements, reviews coverage and closes items. After MOD-39 the
  tab answers "which items address R-X, and which were written against an older text", and every
  close-out states how the item ended.
- **MOD-8, MOD-11, MOD-60, MOD-14**: the importer and MCP citations write into what this tab shows;
  MOD-60 (display width) and MOD-14 are sequenced after this item.

## Hypothesis

If the tab reads the seam as it stands and the only new logic is the TUI's own (a maintainer gate,
a picker, a sub-tab), MOD-39 ships with no migration, no store change and no `engine.rs` change,
and the markdown workflow can later cut over (ANA-11 §6 phase 3) with MOD-13 as the last gap.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Tab reachable | A Requirements tab in the strip, keyboard-only | Harness snapshot of the strip and the tab on the demo |
| Coverage | Selecting R-ENT-1 lists ANA-1 (suspect, done) and ANA-2 (amends, open); R-STO-1 lists FEAT-1 and FIX-1 with resolution `done` | Snapshot on MemStore demo |
| Withdrawn dimmed | A withdrawn requirement renders with the dim style and stays listed | Unit test on the row style + snapshot after a withdraw |
| Revision trail | R-ENT-1 shows v1 and v2 with "amended by ANA-2"; offline shows "revisions need the database" | Snapshot online; unit test on the `None` branch |
| Maintainer gate | Create/amend/withdraw refused for a non-owner, with a status message; allowed for the owner | Worker-level test with two users on MemStore |
| Suspect + re-confirm | ANA-1's citation shows a suspect marker; re-confirm clears it | Harness: key sequence then snapshot |
| Picker | An `open` item can close as withdrawn/rejected/superseded/duplicate; a `done` item offers all six; the default is `default_for` where it exists | Runs unit tests + `htui/tests/runs_pg.rs` case |
| No overlap | No migration, no change under `htui-store/src/pg`, `htui-core/src/store`, `htui-orch/src/engine.rs` | `git diff --stat` at close-out |

## Scope

**In scope**

- `htui/src/requirements.rs`: a store-worker domain module (snapshot, `serve`, `REQUEST_NAMES`,
  `READ_NAME`) with requests for the scope snapshot (spec, areas, requirements per project), one
  requirement's coverage and revisions, an item's citations, and the writes (area create, mint,
  amend, withdraw, cite, uncite, re-confirm), plus a `RequirementsStale` reply for a CAS miss. One
  or-ed arm in `try_serve`.
- The maintainer gate in that module (D1), enforced in the worker, mirrored in the view as greyed
  keys.
- A Requirements tab: projects, then areas, then requirements (key, priority, state, first line of
  body); a detail pane with body, rationale, version, coverage (citing item, kind, status,
  resolution, suspect) and the revision trail with its deciding item. Withdrawn rows dimmed. A text
  filter over key and body (`RequirementFilter.text`).
- Writes from the tab: new area (code, title), new requirement (area, body, rationale, priority),
  amend (body/rationale/priority plus the deciding item's key), withdraw (deciding item's key and a
  typed-key confirmation, as close-out does).
- Item detail: cited requirements with kind, stamped version, and a suspect marker; re-confirm on
  the selected citation (D3, D4).
- Runs pane close-out: a resolution picker in the `Warn` stage offering exactly
  `closes_from(status)`, starting on `default_for(status)`; `open` items become closable.
- CLEAN-6: the module doc line at `runs.rs:19`.
- Close-out bookkeeping: HANDOFF pins (variant counts, snapshots, tab strip), write-up, DECISIONS
  line, CLEAN-6 closed with MOD-39.

**Out of scope**

- Any store, schema or migration change (next migration stays `0008`). Area rename/reorder/delete
  (no seam; mint a new area instead). Editing the spec preamble.
- MCP `requirement_cite` (MOD-11), the importer (MOD-8), Qdrant indexing of requirements (MOD-50),
  the prompt `requirements` section.
- Jumping from a coverage row to the item in the Backlog tab (a cross-tab focus action; noted as a
  follow-up if wanted).
- A general role model for users.

## Constraints (fixed before planning)

- **No overlap with MOD-40.** Nothing under `htui-store/src/pg`, `writer.rs`, `connect.rs`,
  `htui-core/src/store/{traits,mem,conformance}.rs`, `htui-orch/src/{engine,recover}.rs`.
  `store_worker.rs` gets new enum variants and one `try_serve` arm only; MOD-40's ticker and status
  notice are elsewhere in that file. Whoever lands second merges main and recounts pins.
- **The picker does not touch `engine.rs`.** `close_out_enabled` keeps its signature and returns the
  picker's starting resolution: `default_for(status)`, else `withdrawn` when
  `closes_from(status)` allows it (so `open` becomes closable and starts on `withdrawn`); it still
  refuses a live run and any other status. `Engine::close_out` (`engine.rs:1553`) and
  `close_out_preview` (`:2056`) only call it, so they need no edit; the pin at
  `command.rs:2193-2214` that `open` is refused flips. The TUI computes the legal set from
  `Resolution::closes_from(item.status)`; the store stays the authority.
- **Skills tab and `TextArea` are Luigi's MOD-9 ground.** Reuse `TextArea` and `TextField` as they
  are; no edits to them or to `ui/tabs/skills`.
- **Offline**: reads from the cache; revisions show a one-line "needs the database" note; write keys
  greyed with the existing offline message. No offline write (MOD-25).
- **Deciding item** for amend and withdraw is typed as an item key, resolved in the worker against
  the workspace scope; an unknown key is refused before any write.
- `unsafe_code = "forbid"`, workspace lints unchanged, TDD per repo convention (harness + insta
  snapshots under `htui/tests`, unit tests in module); reviewer `rust-reviewer`.

## Decisions for the maintainer

> **Decided 2026-09-29 by the maintainer: all recommended** — D1 (a), D2 (a), D3 (a), D4 (a).

- **D1: who is the maintainer.** The store has no role.
  - **(a) recommended**: the project's `requirement_spec.owner_id`. If a project has no spec yet,
    the first requirement write creates one owned by `this_user`. Others see the tab read-only.
    Uses the one ownership hook that exists; no migration.
  - (b) anyone at the TUI. Agents never reach these writes (MCP is MOD-11), so "maintainer-only"
    means "human-only"; simpler, but it makes the word mean nothing on a shared database.
- **D2: tab position.**
  - **(a) recommended**: before Settings, per `R-TUI-1`'s order: 1 Backlog, 2 Skills,
    3 Requirements, 4 Settings, 5 Chat. Settings moves from `3` to `4`; tests that press `3`
    for Settings are updated, and so are README.md's tab tour and key list (PR #24 numbers the
    tabs, Chat as 4).
  - (b) after Settings, so no digit moves.
- **D3: where citations show in item detail.**
  - **(a) recommended**: a seventh sub-tab "Reqs", and "Documents" is shortened to "Docs" so the
    strip stays at 40 of 43 columns. It gets a cursor and its own keys like Runs.
  - (b) a section at the foot of Body, with a small cursor mode; no strip change, but Body stops
    being scroll-only.
- **D4: human citing from item detail.**
  - **(a) recommended**: also `c` cite (pick a requirement from the item's project, kind
    `addresses` or `reserves`) and `u` uncite, alongside re-confirm. Without it a human cannot add a
    citation until MOD-11 or MOD-8, and the seam already has both writes.
  - (b) re-confirm only, as the HANDOFF text says.

## Milestones

| # | Milestone | Proves |
|---|---|---|
| 1 | Worker module: requests, replies, snapshot, maintainer gate, deciding-item resolution | `serve` tests on MemStore, two users |
| 2 | Requirements tab, read side: tree, detail, coverage, revisions, filter, dimmed withdrawn | Harness snapshots on the demo |
| 3 | Tab writes: area, mint, amend, withdraw, stale handling | Harness key sequences + snapshots; Pg case |
| 4 | Item detail citations: suspect marker, re-confirm (+ cite/uncite per D4) | Harness snapshots; strip-width test |
| 5 | Close-out resolution picker + CLEAN-6 | Runs unit tests, `runs_pg.rs` |
| 6 | Close-out bookkeeping, pins, write-up | Validator green |

## Open risks

- `store_worker.rs` merge with MOD-40: different arms, but both touch the file and the HANDOFF
  pins. Mitigation: keep the diff to enum variants and one arm.
- Snapshot churn from D2 (every strip snapshot changes) and D3 (every detail strip snapshot
  changes). Accepted; counts recorded at close-out.
- MOD-60 (display width) runs after this item and may re-measure the new strips.

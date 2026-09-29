# Plan: MOD-39 Requirements tab and item traceability (with CLEAN-6)

> Source: `.claude/prds/mod-39-requirements-tab.prd.md` (PRD decisions D1-D4 all (a), maintainer,
> 2026-09-29). Routed PRD + ultracode for implement. Read at main `34b9888`. Paths are relative to
> `crates/` unless they start with a top-level directory.

## Summary

Six tasks. T1 is the store-worker module every view reads through; T2 is the close-out picker and
CLEAN-6, independent of T1 by file set. T3 (the Requirements tab) and T4 (the item detail "Reqs"
sub-tab) both need T1 and run in parallel on disjoint source files; neither accepts a changed
**existing** snapshot. T5 is the serial sweep that accepts the strip churn both cause, updates
README.md's tab tour, and runs the full suite. T6 is review and close-out.

No migration (main's next stays `0008`, cache `0005`). No change under `htui-store/src`,
`htui-core/src/store`, `htui-orch/src/engine.rs` or `recover.rs` (MOD-40's ground), and none to the
Skills tab or `ui::TextArea` (MOD-9's).

## Design decisions (settled here)

- **P1 One module, `htui/src/requirements.rs`**, the `templates.rs` shape: `snapshot`, `serve`,
  `REQUEST_NAMES`, `READ_NAME`, one or-ed arm in `store_worker::try_serve`. It serves both the tab
  and the Reqs sub-tab.
- **P2 Requests** (all carry only what the user typed; the worker fills `this_user`):
  - reads: `Requirements(Scope)`, `RequirementDetail(RequirementId)`, `ItemRequirements(ItemId)`;
  - tab writes: `CreateRequirementArea { scope, project, code, title }`,
    `MintRequirement { scope, area, body, rationale, priority }`,
    `AmendRequirement { scope, id, expected_version, body, rationale, priority, deciding }`,
    `WithdrawRequirement { scope, id, expected_version, deciding }` (`deciding` is the typed item
    key);
  - sub-tab writes: `Cite { item, requirement, kind }`, `Uncite { item, requirement, kind }`,
    `Reconfirm { item, requirement, kind }`.
  - Body and rationale travel in a length-only `Debug` newtype, as `TemplateBody` does.
- **P3 Replies**: `Requirements(Box<RequirementsSnapshot>)` (read and every applied tab write),
  `RequirementsStale(Box<RequirementsSnapshot>)` (amend/withdraw `Diverged`),
  `RequirementDetail(Box<RequirementDetail>)`, `ItemCitations(Box<ItemCitations>)` (read and every
  sub-tab write). A refusal is the shell's `Failed { request, message }`.
- **P4 Snapshot**: per scope project, in scope order: `project_id`, project `slug`/`name`
  (`ReadStore::project`), `spec: Option<RequirementSpec>`, `areas`, `requirements` (unfiltered),
  and `maintainer: bool`. Filtering is client-side over the snapshot (key and body,
  case-insensitive), so a keystroke never reads the store.
- **P5 Maintainer (D1)**: `maintainer = spec.map_or(true, |s| s.owner_id == this_user)`. Area
  create, mint, amend and withdraw re-check it in the worker and refuse with
  `StoreError::Constraint("only the project's requirements owner can …")`. With no spec, the first
  gated write calls `set_requirement_spec(project, None, this_user, String::new())` before its own
  write; a `Stale` answer there (someone else created it first) re-checks the owner. Cite, uncite
  and re-confirm are not gated (ANA-11 §6 gates create/amend/withdraw only).
- **P6 Deciding item**: `items(scope, ItemFilter { project_ids: Some(vec![req.project_id]), text:
  Some(key), .. })`, then an exact `key` match; none is `Constraint("no item KEY in PROJECT")`
  before any write. Keys are per project, so the requirement's project is the only one searched.
- **P7 Detail**: `requirement`, `requirement_coverage`, and `requirement_revisions` with each
  revision's `amended_by_item_id` resolved to its key by `item(id)`; `revisions: None` offline is
  kept as `None` and drawn as "revisions need the database".
- **P8 Item citations**: `item_requirements(item)` plus `candidates`: the active requirements of
  the item's project (`requirements(project, { states: [Active] })`) for the cite picker.
- **P9 Uncite** only `addresses` and `reserves`: `amends`/`withdraws` citations record a decision
  and stay. Re-confirm only offered on a suspect citation.
- **P10 Tab (D2)** registered between Skills and Settings: `1 Backlog 2 Skills 3 Requirements
  4 Settings 5 Chat`. Layout mirrors the Backlog: a left tree (project header, area header,
  requirement rows `R-ENT-1 must first-line-of-body`; withdrawn rows `theme.dim`-style), a right
  detail pane (body, rationale, `must · active · v2`, Coverage, Revisions). Keys: `j/k/g/G` move,
  `Enter` folds a header, `J/K/PgUp/PgDn` scroll the detail, `/` filter, `a` new area (project
  row), `n` new requirement (area or requirement row), `e` amend, `W` withdraw. Write keys on a
  non-maintainer project or offline answer the status line and send nothing.
- **P11 Forms**: `TextField` for area code/title and the deciding key; `TextArea` (reused as is)
  for body and rationale; priority toggles with `m`/`l`. Withdraw ends with the requirement key
  typed back, the close-out pattern. One write in flight (`busy`), lands by content, `Stale`
  keeps the form open with the existing `CHANGED_ELSEWHERE` sentence.
- **P12 Reqs sub-tab (D3, D4)**: registered after Prompt; "Documents" shortens to "Docs". Rows:
  `R-ENT-1 addresses v1 ! suspect` then the requirement's first line. `J/K` move, `r` re-confirm,
  `c` cite (a picker over `candidates`, `Enter` picks, then `a` addresses / `v` reserves), `u`
  uncite after `y`. `BacklogTab::go` sends `ItemRequirements(id)` with the other six.
- **P13 Picker (T2)**: `Resolution::default_for(Open)` becomes `Some(Withdrawn)`, so
  `close_out_enabled` (unchanged code) lets `open` through and `engine.rs` needs no edit. The Warn
  stage cycles `Resolution::ALL` filtered by `closes_from(preview.status)` with `←/→` (and
  `h/l`), starting on `preview.resolution`, and `Command::CloseOut` sends the chosen one.
  `Preview.status` already exists (`htui-orch/src/closeout.rs:24`).

## Patterns to Mirror

| Pattern | Where |
|---|---|
| Domain worker module, `REQUEST_NAMES`, stale reply | `htui/src/templates.rs:1-178`, `store_worker.rs:537-552`, `:902-905`, `:1219` |
| Worker fills the user | `templates.rs:147-150` (`backend.writer()`, `this_user()`) |
| Tab with list + detail, busy, modes | `htui/src/ui/tabs/skills/library.rs:197-259`, `:1436-1500` |
| Tree grouped by project, fold, `go` on move | `htui/src/ui/tabs/backlog/mod.rs:55-130` |
| Typed-key confirmation | `detail/runs.rs:700-733` |
| Harness + insta tests | `htui/tests/skills.rs`, `htui/tests/backlog.rs` |
| Two users on MemStore | `htui-core/src/store/mem.rs:6979` (`data.users.push`) |

## Files to Change

| File | Task | Change |
|---|---|---|
| `htui/src/requirements.rs` | T1 | new: snapshot types, `serve`, gate, tests |
| `htui/src/store_worker.rs` | T1 | variants, `name()` arms, one `try_serve` arm, `use` |
| `htui/src/lib.rs` | T1 | `pub mod requirements` |
| `htui-core/src/model/item.rs` | T2 | `default_for(Open)`, its doc and table test |
| `htui-orch/src/command.rs` | T2 | `close_out_enabled` docs; `:2186-2214` tests flip for `open` |
| `htui-orch/src/closeout.rs` | T2 | doc of `Preview.resolution` |
| `htui/src/ui/tabs/backlog/detail/runs.rs` | T2 | picker, render, module doc (CLEAN-6), tests |
| `htui/tests/runs_pg.rs` | T2 | an `open` item closes as `withdrawn` on Postgres |
| `htui/src/ui/tabs/requirements/*.rs` | T3 | new tab (mod, tree, detail, forms) |
| `htui/src/ui/tabs/mod.rs` | T3 | `pub mod requirements`, `pub use` |
| `htui/src/app/mod.rs` | T3 | `register_tab` before Settings |
| `htui/tests/requirements.rs`, new snapshots | T3 | harness tests |
| `htui/tests/requirements_pg.rs` | T3 | gate + stale over `PgStore` |
| `htui/src/ui/tabs/backlog/detail/requirements.rs` | T4 | new sub-tab |
| `htui/src/ui/tabs/backlog/detail/mod.rs` | T4 | `pub mod`, `pub use` |
| `htui/src/ui/tabs/backlog/detail/documents.rs` | T4 | title "Docs" |
| `htui/src/ui/tabs/backlog/mod.rs` | T4 | register, `go` request, strip-width test |
| `htui/tests/backlog.rs`, new snapshots | T4 | Reqs sub-tab tests |
| `htui/tests/connection.rs`, `htui/src/keymap.rs` tests | T5 | Settings digit `3` → `4` where it means Settings |
| `htui/tests/snapshots/*.snap` (existing) | T5 | accept strip churn only |
| `README.md` | T5 | tab tour and key list |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-39.md` | T6 | close-out |

## Milestone → task map

| PRD milestone | Tasks |
|---|---|
| 1 worker module | T1 |
| 2-3 tab read + writes | T3 |
| 4 item citations | T4 |
| 5 picker + CLEAN-6 | T2 |
| 6 close-out | T5, T6 |

## Tasks

TDD per repo convention: each task opens with the test that fails for the stated reason.
Independence is decided by file set.

Order: (T1 ∥ T2) → (T3 ∥ T4) → T5 → T6.

### T1: Worker module (M1; parallel with T2)
- **Tests first** (in-module, MemStore demo): the snapshot of the platform scope holds htui's two
  areas and three requirements with `maintainer: true`; a second, earlier-created user reads
  `maintainer: false` and every gated write is refused with no row written; mint on a project with
  no spec creates the spec owned by `this_user`; amend at a stale version answers
  `RequirementsStale`; amend with an unknown deciding key is refused before the write; detail of
  R-ENT-1 lists ANA-1 (suspect) and ANA-2, and revision v2 resolves "ANA-2"; `ItemRequirements`
  of ANA-1 shows the suspect citation and re-confirm clears it; uncite of an `amends` citation is
  refused; `request_names_match_the_name_arms` extended.
- **Files:** `htui/src/requirements.rs`, `htui/src/store_worker.rs`, `htui/src/lib.rs`.

### T2: Close-out picker + CLEAN-6 (M5; parallel with T1)
- **Tests first:** `default_for` table (`Open → Withdrawn`); `close_out_enabled` on `open` now
  answers `Withdrawn`, a live run still refuses; Runs unit tests: `→` on a `done` preview walks all
  six, on a `blocked` preview the four non-success ones, and `y`+key sends `CloseOut` with the
  chosen resolution; the Warn line names the chosen one and shows `←/→ resolution`; `runs_pg.rs`
  closes an `open` item as `withdrawn` and reads `resolution = withdrawn`.
- **Code:** as P13; CLEAN-6 changes `runs.rs:19` to "`a` / `x` | step | approve (`AnswerGate`) /
  reject with a typed note" and adds `←/→` to the `C` row.
- **Files:** `htui-core/src/model/item.rs`, `htui-orch/src/command.rs`, `htui-orch/src/closeout.rs`,
  `htui/src/ui/tabs/backlog/detail/runs.rs`, `htui/tests/runs_pg.rs`.

### T3: Requirements tab (M2-M3; needs T1; parallel with T4)
- **Tests first** (`htui/tests/requirements.rs`, harness on the demo): the tab shows htui's tree
  and R-ENT-1's detail (coverage with the suspect ANA-1, revisions v1/v2 "amended by ANA-2");
  `/ENT` filters; `n` then body/rationale/Enter mints `R-ENT-3`; `e` with deciding key `ANA-2`
  amends to v3 and ANA-1 stays suspect; `W` + deciding key + typed key withdraws and the row is
  dimmed; a non-maintainer store answers the status line on `n`. New snapshots only.
  `requirements_pg.rs`: mint, amend stale, gate over `PgStore`.
- **Files:** `htui/src/ui/tabs/requirements/*`, `htui/src/ui/tabs/mod.rs`, `htui/src/app/mod.rs`,
  `htui/tests/requirements.rs`, `htui/tests/requirements_pg.rs`, their new `.snap` files.

### T4: Reqs sub-tab (M4; needs T1; parallel with T3)
- **Tests first** (`htui/tests/backlog.rs`): ANA-1's Reqs sub-tab shows `R-ENT-1 addresses v1`
  with the suspect marker; `r` clears it; `c` on FEAT-1 picks R-ENT-2 as `addresses`; `u` + `y`
  removes it; `u` on ANA-2's `amends` row answers the status line. The strip-width test keeps
  passing with seven titles (40 of 43 columns). New snapshots only.
- **Files:** `htui/src/ui/tabs/backlog/detail/{requirements,mod,documents}.rs`,
  `htui/src/ui/tabs/backlog/mod.rs`, `htui/tests/backlog.rs`, its new `.snap` files.

### T5: Snapshot and README sweep (serial, after T3 and T4)
- Run the whole `htui` suite; accept existing snapshots whose only diff is the top strip (digits
  after Skills shift) or the detail strip ("Docs", "Reqs"); any other diff is a bug to fix. Fix
  tests that press `3` for Settings. Update README.md's tab tour and key list (no work-item ids).
- **Files:** `htui/tests/snapshots/*.snap`, `htui/tests/connection.rs`, `htui/src/keymap.rs` (tests
  only), `README.md`.

### T6: Review and close-out (serial, last)
- `rust-reviewer` over the full diff; findings applied or deferred with the maintainer.
- HANDOFF: MOD-39 and CLEAN-6 closed; "Live coordinates" pins (StoreRequest/StoreReply counts,
  snapshot count, tab strip); DECISIONS line; `docs/decisions/mod/mod-39.md`; validator green.

## Validation

`source /home/user/htui-env.sh`; `cargo fmt --check`; `cargo clippy --workspace --all-targets
--all-features -- -D warnings`; `cargo test -p htui --features testkit`; `cargo test -p htui-orch
-p htui-core`; `htui/tests/*_pg.rs` against the local Postgres; `bash
.claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Risks

- `store_worker.rs` merges with MOD-40: new variants and one arm only.
- `TextArea` is reused unmodified; if a form needs something it lacks, stop and ask rather than
  edit it (MOD-9 ground).
- README and HANDOFF pins are shared churn with the four sibling threads; whoever lands second
  merges main and recounts.

## Verified claims

Checked against the tree at `34b9888` on 2026-09-29.

| Claim | Verdict | Evidence |
|---|---|---|
| The seam's reads and writes exist on MemStore, PgStore, `Writer`; the cache has the reads | true | `traits.rs:202-253`, `:1287-1413`; `cache/read.rs:1071-1246` |
| No requirement `StoreRequest`/`StoreReply` variant exists | true | grep of `store_worker.rs` |
| `templates.rs` is the domain-module pattern; the worker fills the user | true | `templates.rs:118-178`, `:147-150`; `store_worker.rs:1219` |
| `ReadStore::project(id)` gives slug and name | true | `traits.rs:~141`, `model/hierarchy.rs:56-62` |
| `ItemFilter` has `project_ids` and `text` for the deciding-key lookup | true | `model/item.rs:215-229` |
| `set_requirement_spec(project, None, …)` inserts v1 when absent, else `Stale` | true | `traits.rs:1304-1314` |
| `Resolution::ALL` exists | true | `str_enum!` emits `ALL` (`model/mod.rs:47`) |
| `Preview.status` exists, so the picker needs no engine change | true | `htui-orch/src/closeout.rs:25` |
| `default_for` is used only by `close_out_enabled` and its tests | true | grep: `command.rs:1203`, `:2190` |
| `Engine::close_out` and `close_out_preview` only call `close_out_enabled` | true | `engine.rs:1553`, `:2056` |
| The Runs pane captures every key outside `Browse`, so `h/l` in the Warn stage stay in the pane | true | `runs.rs:1123-1125`; Backlog hands a capturing sub-tab every key |
| Detail strip is 40 columns now and stays 40 with "Docs" + "Reqs" | true (arithmetic: `Documents`→`Docs` −5, ` Reqs` +5); pinned by `backlog/mod.rs:413-423` | test runs in T4 |
| `theme.dim` and `CHANGED_ELSEWHERE` exist to reuse | true | `ui/theme.rs:15`; `settings/mod.rs:105` |
| Two users on a MemStore are constructible for the gate test | true | `mem.rs:6979` pushes a second `AppUser`; `this_user` is the earliest (`mem.rs:220`) |
| Only `tests/connection.rs:2055` presses `3` meaning Settings; the keymap `chord("3")` test is generic | true | grep; `keymap.rs:380-382` |
| `tab` presses in `tests/skills.rs`/`templates.rs` may cross tabs | unverified; T5 reruns and fixes | `skills.rs:300…919`, `templates.rs:509` |
| 20 existing snapshots show ` 3 Settings` and change in T5 | true | grep of `htui/tests/snapshots` (96 files) |
| T1/T2 and T3/T4 file sets are disjoint | true | Files to Change table; snapshot churn isolated in T5 |

## Acceptance

PRD success metrics, all tests green except the known pre-existing reds (if any, named in the
write-up), validator green, `git diff --stat` shows nothing under MOD-40's files.

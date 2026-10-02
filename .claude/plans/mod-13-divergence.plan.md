# Plan: MOD-13 milestone 3 — Divergence

**Source PRD**: `.claude/prds/mod-13-backlog-editing.prd.md`
**Selected Milestone**: 3 — Divergence
**Complexity**: Medium
**Status**: complete (2026-10-02) — CONFIRMed, implemented `54667c9`..`ec0f00c`, rust-reviewer approve-with-fixes, fixes applied (L3, NIT-2 deferred)

## Summary
A stale edit opens a three-way view in place of milestone 2's D6 notice. The three columns are
ancestor (the item the form opened on), theirs (the head) and mine (the form). The user picks
which side wins the fields both sides changed, then lands back in the form rebased on the head. A
free edit and Ctrl+S send one §7.2 compare-and-set at `head.version`, which writes a revision
with `reason = 'divergence_resolution'`.

**The store half is already built.** Both `update_item`s answer `Diverged { head, ancestor }`
(`mem.rs:1431-1441`, `pg/write.rs:916-917`), and both write `patch.reason` into the revision
(`mem.rs:1483-1495`, `pg/write.rs:848-850`). This milestone adds no migration and no store
method. It adds:
- one pure merge function;
- a `reason` on the edit request;
- the catalogue on the divergence reply;
- the view;
- one conformance case that pins the reason on both stores.

## Design decisions
- **D1: the ancestor is the item the form opened on, not only the reply's `ItemRevision`.**
  - `item_revision` snapshots only title, body and tags (ANA-9 §5.5; `item.rs:291-310`). The
    other four `version`-covered columns (`kind_id`, `priority`, `touched_paths`,
    `step_graph_id`) have no history.
  - The form already holds the whole `Item` it opened on, read fresh at the token's version
    (milestone 2 D3), so it serves as the ancestor for all seven columns.
  - A divergence reply is the form's only if `ancestor.version == form token`. Otherwise it is
    stale: the tab drops it, as milestone 2 drops a reply for another item. The reply's
    `ancestor` stays as the spec's (§4.2 step 3) cross-check and is never rendered.
  - **Rejected: widening `item_revision` with a migration.** That is a schema change for a view
    that already has the data, and old revisions would still lack the four columns.
- **D2: field-level three-way merge, a pure function in `htui-core`
  (`model/item_merge.rs`).**
  - `merge(ancestor: &ItemSpec, theirs: &ItemSpec, mine: &ItemSpec) -> SpecMerge` classifies
    each of the seven fields as one of:
    - `Same`: no side changed it, or both changed it to the same value;
    - `Theirs`: only the head changed it;
    - `Mine`: only the form changed it;
    - `Conflict`: both changed it, to different values.
  - `SpecMerge::resolve(Side) -> ItemSpec` takes:
    - `theirs` for `Same`/`Theirs` fields;
    - `mine` for `Mine` fields;
    - the chosen side for `Conflict` fields.
  - **Why not whole-spec "take mine".** The form's spec still holds the ancestor's value in
    every field the user did not touch. Sending it whole would silently revert the head's
    changes to those fields: R-ENT-10's overwrite, run in the other direction. Fields with no
    conflict therefore always keep both sides' changes. The pick only decides conflicts.
  - Body and paths are compared whole. Per-hunk merge is out of scope (PRD).
- **D3: the view is a mode of the item form.**
  - `ItemForm` gains `resolving: Option<Divergence>`, held in a new module
    `ui/tabs/backlog/divergence.rs` with its state, keys and render.
  - The form keeps owning the capture guards (`on_key`, `on_paste`, the reveal guard,
    `on_scope_change`), so none of the tab's guards change.
  - `item_form.rs` is already 1467 lines, and the review deferred splitting its render. The
    view's code therefore goes in the new file, not in `item_form.rs`.
- **D4: the view takes the whole tab area (list and detail panes), not the detail pane's rect.**
  - At 100x30 the detail pane's inner width is 43 columns (`item_form.rs:44`), too narrow for
    three columns or two diffs side by side. The full area gives ~98.
  - Contents:
    - A header: `KEY  v{ancestor} → theirs v{head}`.
    - One row per field that is not `Same`: the label, its state, then the ancestor, theirs and
      mine values. Short fields (kind, title, priority, tags, graph) wrap rather than clip. Kind
      and graph render through the catalogue labels.
    - When body or paths is not `Same`: two unified diffs side by side, through `ui::diff`
      (`unified`/`lines`): `ancestor → theirs` | `ancestor → mine`. `Tab` switches between body
      and paths when both differ, and `j/k/PgUp/PgDn` scroll.
    - A hint line: `t theirs wins  m mine wins  Tab body/paths  Esc back`.
- **D5: keys in the view.**
  - **`m` / `t`** resolve conflicts with mine or theirs (D2). The form is then rebuilt on the
    head:
    - the token is `head.version`;
    - `opened` holds the head's texts, so A4's "unchanged keeps the stored value" now means
      "unchanged from the head";
    - the widgets hold the resolved spec's texts;
    - the reason becomes `divergence_resolution`.

    The user may edit freely, then Ctrl+S.
  - With no conflict, `m` and `t` give the same result, and the hint says
    `no conflicts — m or t continues`.
  - **`Esc` returns to the form exactly as it was**: same token, same text, and a notice
    `still behind v{head}; Ctrl+S compares again`. A second `Esc` cancels, as today. Esc in the
    view never throws work away.
  - Every other key is swallowed, except the chord pass and Ctrl+S, which does nothing in the
    view.
- **D6: the request carries the reason.**
  - `htui_core::model::item_spec` gains `enum EditReason { Edited, DivergenceResolution }`
    with `as_str()` (`"edited"`, `"divergence_resolution"`). `DIVERGENCE_RESOLUTION` joins
    `EDITED` (`item_spec.rs:24`).
  - `SpecChanges::into_patch(self, author, box, reason: EditReason)`. It has three callers: the
    two in `item_spec.rs` tests (`:994`, `:1008`) and `item_writes.rs:255`.
  - `StoreRequest::EditItem` gains `reason: EditReason`. The form sends `Edited` until it is
    rebased (D5), and `DivergenceResolution` after that.
  - The worker passes the reason through. It is a closed enum, so there is nothing to validate.
- **D7: `ItemDiverged` carries the catalogue.**
  - `ItemDivergence` gains `context: ItemFormContext` with `item = Some(head)`. The worker
    already read the catalogue for validation (`item_writes.rs:247`), so it reuses it.
  - This matters because the head may have been re-kinded, or re-graphed onto a graph the
    form's old catalogue lacks. Without it the rebased form's picker would show `?`
    (`picker_label`, `item_form.rs:682-685`).
  - The rebased form is `ItemForm::open_resolution(context, &resolved)`.
  - `ItemDivergence`'s hand-written `Debug` (review L2) adds the catalogue as counts.
- **D8: a resolution that diverges again re-opens the view.** The ancestor is then the head the
  form was rebased on (D1 holds unchanged: the form's opened item), and the user can resolve as
  many times as needed. A resolved spec equal to the head is `NOTHING_TO_SAVE`, as today. That
  only happens with `t` when every change of mine conflicted, and the notice says so.
- **D9: one conformance case, `item_edit_reason_lands_in_revision`.**
  - The case edits with `reason = "divergence_resolution"`, then reads the revision back with
    the diverge trick of `status_cas_keeps_version` (`conformance.rs:1056-1083`). There is no
    revision reader on either trait.
  - It asserts the reason on both stores. Counts go 120 → 121 in `mem_store.rs:37` and
    `pg_conformance.rs:23-31`, with their history messages.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Capturing form state + outcome enum | `ui/tabs/backlog/item_form.rs` (`ItemForm`, `ItemFormOutcome`, `on_key` 384) | Pure state, the outcome answers the tab, Ctrl+S before the chord pass |
| Line diff | `crates/htui/src/ui/diff.rs` `unified`, `lines`, `NO_DIFFERENCES` | `similar` (workspace dep), styled gutters |
| Notice sentences | `item_form.rs:867-893` (`item_changed_elsewhere`, `mint_may_have_landed`) | `pub fn` returning a `String`, tested by text |
| Hand-written redacting `Debug` | `item_writes.rs:96-171` (`ItemDigest`, `RevisionDigest`) | Ids, versions, lengths, counts |
| Reply routing in the tab | `backlog/mod.rs:422-459` (`on_item_written`, `on_item_diverged`) | Match on `busy` and `item_id` first; a stale reply is dropped |
| Pure model fn + unit tests | `htui-core/src/model/item_spec.rs` (`SpecChanges::between` 423) | Field-by-field, no I/O |
| Conformance case | `store/conformance.rs` `CASES` (48) / `run_case` (178), `status_cas_keeps_version` | Case-named panics, count bump + message |
| Stale write in an integration test | `tests/backlog.rs:1662-1708` | A held `MemStore` clone writes between `e` and Ctrl+S |
| Pg through the worker | `tests/item_writes_pg.rs:126-180` | `store_worker::serve` over Online, `demo_db()` SKIP |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/item_merge.rs` | CREATE | D2 `merge`, `SpecMerge`, `FieldState`, `Side`, `resolve`; unit tests |
| `crates/htui-core/src/model/mod.rs` | UPDATE | `pub mod item_merge;` plus re-exports |
| `crates/htui-core/src/model/item_spec.rs` | UPDATE | D6 `EditReason`, `DIVERGENCE_RESOLUTION`, `into_patch` takes the reason; 2 test call sites |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | D9 case in `CASES` and `run_case` |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | 120 → 121 and its message |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | `EXPECTED_CASES` 120 → 121, doc and message |
| `crates/htui/src/store_worker.rs` | UPDATE | `EditItem.reason` (D6) |
| `crates/htui/src/item_writes.rs` | UPDATE | Pass the reason through; `ItemDivergence.context` and its `Debug` (D7); module doc D6 → M3; tests |
| `crates/htui/src/ui/tabs/backlog/divergence.rs` | CREATE | D3–D5 view state, keys, render, sentences; unit tests |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | UPDATE | `resolving` mode, `open_resolution`, reason state, key/paste routing into the view; D6 notice replaced |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | `mod divergence;`, `on_item_diverged` opens the view (D1 staleness check), render over the whole `area` while resolving (D4) |
| `crates/htui/tests/backlog.rs` | UPDATE | The D6 case becomes the M3 flow; new cases |
| `crates/htui/tests/snapshots/backlog__item_divergence.snap` | CREATE | The view at 100x30 |
| `crates/htui/tests/item_writes_pg.rs` | UPDATE | A resolution at `head.version` lands with the reason on Postgres |

**Every task:** each new public item gets a doc comment and each public type a `Debug`
(`missing_docs` in `lib.rs`, the `missing_debug_implementations` workspace lint, `-D warnings`).
Body and paths never reach a `Debug` (E6/E10).

## Tasks
### Task 1: core model and conformance (TDD), `htui-core` + `htui-store` test
- **Action**: Write the tests first.
  - `item_merge` tests:
    - Each `FieldState`, per field type (`title`, `body`, `kind_id`, `priority`,
      `required_tags`, `touched_paths`, `step_graph_id` including `Some`→`None`).
    - Both sides set to the same value is `Same`.
    - `resolve(Mine)`/`resolve(Theirs)` take the conflict side and keep both sides' changes
      that do not conflict.
    - **The regression for D2's reason:** mine edits the title, theirs edits the priority, and
      `resolve(Mine)` keeps theirs' priority.
  - `item_spec` tests: `EditReason::as_str`, and `into_patch` carries the reason.
  - Conformance `item_edit_reason_lands_in_revision` (D9), with both counts bumped.

  Then implement.
- **Files**: `model/item_merge.rs`, `model/mod.rs`, `model/item_spec.rs`, `store/conformance.rs`,
  `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`.
- **Validate**: `cargo test -p htui-core --all-features --lib item_merge item_spec`;
  `cargo test -p htui-core --all-features --test mem_store`;
  `cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1`

### Task 2: worker (TDD), `htui`
- **Action**: Write the tests first in `item_writes.rs`.
  - An `EditItem` with `DivergenceResolution` at the head's version lands, and its revision
    reason is `divergence_resolution` (read back with the diverge trick).
  - A stale `EditItem` answers `ItemDiverged` whose `context` holds the project's kinds and
    graphs and `item == Some(head)`.
  - Extend `form_and_divergence_debug_print_no_body_and_no_paths` (`:815`) to cover `context`.

  Then add `reason` to `EditItem`, pass it through, and add `context`. Update every
  `EditItem {..}` site so all targets still compile (F1): constructors at `item_writes.rs:238,
  401`, `item_form.rs:570` and `tests/item_writes_pg.rs:106`; test patterns without `..` at
  `item_form.rs:1091, 1145` and `backlog/mod.rs:2020`. The `ItemDivergence` destructure at
  `item_writes.rs:634` gains `..`.
- **Files**: `store_worker.rs`, `item_writes.rs`, plus mechanical compile fixes in
  `item_form.rs`, `backlog/mod.rs` (tests) and `tests/item_writes_pg.rs`
  (`reason: EditReason::Edited`; the real logic lands in T3 and T4). Gate with
  `--all-targets` so a broken test target cannot hide.
- **Validate**: `cargo test -p htui --all-features --lib item_writes store_worker`;
  `cargo check -p htui --all-features --all-targets`

### Task 3: the view and the form wiring (TDD), `htui` UI
- **Action**: Write the unit tests first, in `divergence.rs`, `item_form.rs` and
  `backlog/mod.rs`.
  - `ItemDiverged` while `Busy::Editing` on this item, with `ancestor.version` equal to the
    token, opens the view. The form's text is untouched.
  - A reply for another item, or with an `ancestor.version` other than the token, is dropped.
  - The view lists only the fields that are not `Same`, each with its state.
  - A body conflict shows both diffs, and `Tab` switches to paths when both differ.
  - `m` rebuilds the form at `head.version`: conflicting fields take mine, a field only theirs
    changed shows theirs (D2), and Ctrl+S sends `EditItem { expected_version: head.version,
    reason: DivergenceResolution, changes: <only the fields that differ from the head> }`.
  - `t`: conflicts take theirs, and my changes that do not conflict are kept.
  - `t` with every one of my changes conflicting, then Ctrl+S: `NOTHING_TO_SAVE`, nothing sent.
  - `Esc` in the view returns to the form with the same text and the old token, and shows the
    "still behind" notice. Ctrl+S compares again by sending at the old token.
  - A second divergence after a rebase re-opens the view, with the ancestor = the rebased head
    (D8).
  - The rebased form's pickers label a kind and graph that only the new catalogue has
    (D7).
  - Paste is dropped while the view is open, and the reveal guard and scope change behave as
    with the form.
  - With no view open, nothing on screen changes, and the milestone 2 form tests stay green
    except the D6-specific ones, which are rewritten.

  Then implement `divergence.rs`, the `item_form.rs` mode, and the `mod.rs` routing and render.
  Remove `item_changed_elsewhere` and its use.
- **Files**: `ui/tabs/backlog/divergence.rs`, `ui/tabs/backlog/item_form.rs`,
  `ui/tabs/backlog/mod.rs`.
- **Validate**: `cargo test -p htui --all-features --lib backlog`

### Task 4: integration, snapshot, Pg parity
- **Action**: In `tests/backlog.rs`:
  - Rewrite `a_stale_edit_keeps_the_form_and_never_overwrites_the_head` (`:1667`) into the M3
    flow: a held clone retitles between `e` and Ctrl+S, so the view shows both titles. `m`,
    then Ctrl+S, lands v3, and Body shows v3 with mine's title.
  - A held clone changes the priority while I change the title. After `m` + Ctrl+S, both
    changes are in the head (D2 end to end).
  - `Esc` from the view keeps the text, and the head is unchanged.
  - The snapshot `backlog__item_divergence` (title and body conflict). Every existing snapshot
    stays untouched.

  In `tests/item_writes_pg.rs`, extend `mint_edit_and_a_stale_edit_on_postgres`: the
  divergence's `context` is populated, a resolution at `head.version` lands, and its reason
  reads back as `divergence_resolution`.
- **Files**: `tests/backlog.rs`, the new snapshot, `tests/item_writes_pg.rs`.
- **Validate**: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`;
  `cargo test -p htui --all-features --test item_writes_pg -- --test-threads=1`

**Task order: serial, T1 → T2 → T3 → T4.**
- Each task builds on the previous one's types: T2 needs `EditReason`, T3 needs T2's request
  and reply shape, and T4 needs all three.
- T1's files and T3's are disjoint, but T3 cannot compile before T2's request change.
- **No task is marked independent.**

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui-core --all-features
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # full gate; grep SIGABRT
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| "Take mine" reverts a column only the head changed | M | D2 field merge; the T1 unit regression and the T4 end-to-end case |
| Esc in the view loses the user's text | M | D5: Esc returns to the unchanged form; T3 and T4 tests |
| A rebased picker cannot label the head's kind or graph | L | D7: the fresh catalogue rides on the reply |
| A late divergence reply lands on the wrong form | L | D1: the token is checked against `ancestor.version`, plus the busy and item check |
| The view is unreadable at 100x30 | M | D4: the whole tab area; the snapshot pins the layout |
| Mem and Pg disagree on the revision reason | L | D9 conformance case on both |
| Snapshot churn | L | The view renders only while resolving; existing snapshots untouched |
| `htui-orch` stack headroom | L | No orchestrator change; the gate runs `--no-fail-fast` and greps SIGABRT |

## Verified claims
| # | Claim | Verdict | Evidence |
|---|---|---|---|
| C1 | Both stores already answer `Diverged { head, ancestor }` and write `patch.reason` into the revision; no store change needed | ✓ | `mem.rs:1431-1441`, `1483-1495`; `pg/write.rs:848-850`, `916-917` |
| C2 | `item_revision` holds only title, body and tags, so the reply's ancestor cannot cover four of the seven columns | ✓ | `item.rs:291-310`; ANA-9 §5.5 DDL |
| C3 | The form holds the whole `Item` it opened on, read fresh at the token's version | ✓ | `ItemFormContext.item` (`item_writes.rs:81`), filled by `ItemForm` (`:212-221`); token `item.version` (`item_form.rs:570-574`) |
| C4 | `into_patch` hard-codes `EDITED`, with 3 callers | ✓ | `item_spec.rs:447-460`; callers `item_spec.rs:994, 1008`, `item_writes.rs:255` |
| C5 | Every `EditItem {..}` site is in T2's reach | ✗ → amended (F1) | Beyond `item_writes.rs`/`item_form.rs:570`: `tests/item_writes_pg.rs:106`, and test patterns without `..` at `item_form.rs:1091, 1145`, `backlog/mod.rs:2020`. All are now listed in T2 |
| C6 | `ItemDivergence` built or destructured only in `item_writes.rs` | ✓, amended | Built at `:266`, `:842`; destructured exhaustively at `:634` (gains `..`). `item_writes_pg.rs:165-171` reads fields only |
| C7 | The worker already holds the catalogue when `Diverged` comes back | ✓ | `catalogue(&writer, project)` at `item_writes.rs:246-247`, before `update_item` at `:254-256` |
| C8 | Without D7 a re-kinded head shows `?` in the rebased picker | ✓ | `picker_label` falls back to `"?"` (`item_form.rs:682-685`) |
| C9 | No revision reader exists, so tests read the reason through a deliberate divergence | ✓ | No `ItemRevision` method on `ReadStore`/`WriteStore` (`traits.rs` uses it only in `UpdateOutcome`); pg `revision` is private (`pg/write.rs:6239`); trick at `conformance.rs:1056-1083` |
| C10 | Conformance count is 120 on both suites | ✓ | `mem_store.rs:37`; `pg_conformance.rs:23-31` |
| C11 | A line-diff renderer exists to reuse | ✓ | `ui/diff.rs` `unified`/`lines`, `pub mod diff` (`ui/mod.rs:5`); `similar` in `htui/Cargo.toml:63` |
| C12 | The whole tab area is ~98 inner columns at 100x30; the detail pane is 43 | ✓ | `testkit.rs:36` (100x30); `chrome` body spans the width (`ui/layout.rs:21-28`); `panes` 55/45 (`backlog/mod.rs:47-50, 760`); 43 at `item_form.rs:44` |
| C13 | The view's keys reach it while the form is open | ✓ by precedent | `on_key` routes every key to `on_item_form_key` while `item_form` is set (`backlog/mod.rs:568-573`), ahead of the list's `m` (`:586`); milestone 2's form already takes `Tab` and letters this way |
| C14 | `item_form.rs` is large and its render split was deferred, so the view goes in a new file | ✓ | 1467 lines; HANDOFF MOD-13 phase 2 "Deferred" |
| C15 | T1–T4 are serial (no independence claimed) | ✓ | T2 needs `EditReason` (T1), T3 the request/reply shape (T2), T4 all three; T1 ∩ T3 file sets are disjoint, but T3 cannot compile before T2 |

## Acceptance
- [x] All tasks complete
- [x] Validation passes
- [x] Patterns mirrored, not reinvented
- [x] No existing snapshot changed
- [x] A stale edit opens the three-way view; resolution lands as one revision with
      `reason = 'divergence_resolution'` on both stores
- [x] No resolution path reverts a column only the head changed, and Esc never discards text

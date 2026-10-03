# Plan: MOD-13 milestone 5 — Notes and documents

**Source PRD**: `.claude/prds/mod-13-backlog-editing.prd.md`
**Selected Milestone**: 5 — Notes and documents
**Complexity**: Medium
**Status**: complete (2026-10-03) — CONFIRMed, implemented `6242e7a3`..`c2ae9699`, rust-reviewer approve-with-fixes, review fixes `a0bca934`..`598b69d9` (M1, L1–L3, NIT1–3; NIT4 deferred)

## Summary
Two Backlog detail sub-tabs become writable.
- **Notes**: `a` opens a compose area. Ctrl+S appends the note to the thread (`R-ENT-11`).
- **Docs**: `a` writes a new hand-written document of any kind, and `v` writes a new version of
  the kind under the cursor, prefilled from that kind's latest version (`R-ENT-12`). A version
  of `summary` is the editable summary MOD-4 R-43 handed to this item.

**Both stores already implement the write half.**
- `WriteStore::add_note` (`traits.rs:1566`) is one insert.
- `WriteStore::write_document` (`traits.rs:1445`) allocates `max(version) + 1` under the item's
  row lock (`pg/write.rs:5219`).
- The cache mirrors both tables, so an offline box still reads them (`cache/mod.rs:55-56`).

**The rest of the chain is missing.** That is a worker module, a small shared validator, a compose
area in each of the two sub-tabs, and `$EDITOR` routing down to a sub-tab, which today stops at the
item form (`backlog/mod.rs:688-694`). PRD open question 4 is now validated against the store: a
save is a new version, with no compare-and-set (D5).

## Design decisions
- **D1: one worker module, `crates/htui/src/hand_written.rs`, mirroring `item_writes.rs`.**
  - Four `StoreRequest`s, `REQUEST_NAMES = ["note_form", "add_note", "document_form",
    "write_document"]`:
    - `NoteForm { item }`, the read that opens the note compose;
    - `AddNote { item, body: HandText }`;
    - `DocumentForm { item, kind: Option<String> }`, the read that opens the document form, with
      `kind` set for `v`;
    - `WriteDocument { item, kind: String, title: String, body: HandText }`.
  - Four `StoreReply`s, self-naming (MOD-59):
    - `NoteForm { item }`;
    - `NoteAdded { item }`;
    - `DocumentForm(Box<DocumentFormContext { item, base: Option<Document> }>)`;
    - `DocumentWritten { item, kind, version }`.
  - The worker fills in identity, as `item_writes.rs` does (`:234-235`):
    - `created_by` from `backend.this_user()` (`backend.rs:170`);
    - the note's `box_id` from `backend.box_info()?.map(|b| b.box_id)` (`:252`);
    - ids from `NoteId::new()`/`DocumentId::new()`;
    - `created_at = Utc::now()`. Both stores take `created_at` from the argument (`mem.rs:5117,
      5413`; `pg/write.rs:265-295, 5798-5823`).
  - `via_step_id` and `produced_by_step_id` are always `None`, meaning written by hand.
  - **Redaction (`store_worker.rs:116-122`):** a body travels in `HandText`, whose hand-written
    `Debug` prints its length, as `ItemSpec` and `RequirementText` do. `DocumentFormContext`
    carries a `Document` with its body, so its `Debug` is hand-written too (milestone 2 review L2).
    A title and a kind stay plain `String`, as an item title does.
  - **Routing.** Its `Failed`s are **not** `item_writes::is_item_request`, so they skip the Backlog's
    early return (`backlog/mod.rs:646`) and reach every sub-tab through `detail.on_reply`. Each
    sub-tab acts only on its own requests and only for its own `item`, which is `ReqsTab`'s rule
    (`requirements.rs:496-530`).
- **D2: offline is refused by the worker before anything is read, so an offline box never opens
  a compose area.**
  - All four requests take `backend.writer()` first and answer
    `Unreachable(DATABASE_UNREACHABLE)` without it (`item_writes.rs:295-301`).
  - `a`/`v` send the form read. The compose area opens only on that read's reply, as `N`/`e` do
    (milestone 2 D2/D3).
  - `App::on_reply` already puts the `Failed` on the status line (`app/update.rs:285-287`). That is
    the visible read-only notice, and the sub-tab adds no second `Action::Error` (milestone 2 A8).
- **D3: both writes check the item first.**
  - They read `writer.item(id)` and answer `NotFound { entity: "item" }`, because the two Mem
    paths disagree on a missing item: `add_note` says `Constraint` (`mem.rs:5383`) and
    `write_document` says `NotFound`.
  - The two form reads check it the same way. `DocumentForm` with a `kind` reads
    `documents_of_kinds(item, [kind])` (`writer.rs:182`), which is the latest version by number,
    hand-written included (`mem.rs:1198-1203`, `pg/read.rs:507`), and returns it as `base`.
- **D4: one shared validator, `htui_core::model::hand_written`, mirroring `item_spec`.** The worker
  is the authority, and the compose area calls it for early feedback.
  - **Note body**: `trim_end`, then not blank. Leading indentation is kept, because it is
    markdown.
  - **Document kind**: trimmed, not blank, no control characters (A2: stricter than
    `name_is_valid`, which refuses only `\n`, `\r` and `\0`). Neither the schema nor
    Settings › Kinds has a kind rule (`0001_init.sql:392`, `kinds.rs:783`). A phase name, which is
    an `output_kind`, may hold a space, so a space is allowed. The shape is
    `PromptTemplate::name_is_valid`'s after the trim (`kind.rs:393-395`). `judge`/`handoff` are
    reserved *phase* names (`template.rs:52-58`), not document kinds, so they are not refused.
  - **Document title**: trimmed, not blank, single line.
  - **Document body**: `trim_end`, then not blank.
  - Each refusal is a `StoreError::Constraint` with the sentence naming the field.
  - There is no length cap. Neither the stores nor the orchestrator's `add_note` callers have one.
- **D5: a document save is a new version, never a compare-and-set.**
  - This is the PRD default, validated against the store: append-only, `max + 1` under the row
    lock, and the `(item, kind, version)` unique key (`0001_init.sql:399`).
  - Nothing is ever overwritten. But a version written by someone else while the form was open
    would be superseded silently. So the reply carries the landed `version`, and the Docs pane says
    so: "saved as plan v4", plus "— v3 was written after you opened v2; both are kept" when the
    version is not the expected one. Expected is `base.version + 1` for `v`, and for `a` the highest
    version of that kind in the pane's list plus one.
- **D6: sub-tabs own the compose areas.** `DetailRegistry`'s contract is that "approve/reject …
  are `DetailTab::on_key` bodies inside their own file" (`detail/mod.rs:4-7`). `ReqsTab` already
  writes this way, through `ctx.request`.
  - `captures_input()` is true while composing, so the Backlog hands it every key
    (`backlog/mod.rs:588-589`).
  - The reveal guard already names "a half-typed note" (`:764-768`).
  - A scope change drops the compose area through `on_item_change(None)` (`:545`), as it closes
    the item form.
  - **Shared state, `detail/compose.rs`:**
    - a body `TextArea`, plus an optional kind `TextField` and title `TextField`;
    - focus (Tab/BackTab cycle the fields);
    - `busy: Option<&'static str>`, the request in flight;
    - `notice: Option<String>`;
    - `external`, the field handed to `$EDITOR`.
  - **Keys:**
    - **Ctrl+S is checked before chords are passed on.** `TextArea` returns `Submit` for it
      (`text_area.rs:192-199`); a `TextField` passes chords (`text_field.rs:146-153`).
    - Ctrl+E is handled on the body only.
    - Esc cancels and drops the text.
    - While busy, keys are swallowed.
    - Every other CONTROL chord passes, so Ctrl+C still quits.
- **D7: `$EDITOR` reaches the composing sub-tab.**
  - A new `DetailTab::on_external_edit(&mut self, outcome, ctx)` defaults to a no-op.
  - `DetailRegistry::on_external_edit` hands the outcome to the active sub-tab.
  - `BacklogTab::on_external_edit` gives it to the item form when one is open, else to the
    registry. Both can never be open together: the item form opens only when no sub-tab captures
    (`backlog/mod.rs:582-589`).
  - The sub-tab emits `Action::EditExternally` through `ctx.emit`. `App::drain` stamps the Backlog
    as the asking tab (`app/state.rs:355-361`).
  - The milestone 4 return rules apply unchanged. `strip_added_newline` (`item_form.rs:310`) and
    `without_controls` (`:325`) move to `crate::editor` as `pub(crate)`, and the item form calls
    them from there. `EDITED`/`NO_CHANGES`/`WAIT_FLAG` are already there (`editor.rs:20-28`).
  - The temp-file stem is `{key}-note` or `{key}-{kind}`, which `editor::run` sanitises.
- **D8: Notes sub-tab.**
  - `a` sends `NoteForm { item }`; with no item selected it does nothing.
  - The reply opens a body-only compose area.
  - Ctrl+S runs the validator, then sends `AddNote`.
  - `NoteAdded` closes the area and re-sends `StoreRequest::Notes(item)`.
  - **Multi-line bodies render line by line.** `notes.rs:49` draws a body as one `Line`, which a
    hand-written note with `\n` breaks. Each body line becomes its own `Line` in the same
    `Paragraph`/`Wrap { trim: false }`. A one-line body therefore draws byte-identically: both
    demo notes are one line (`fixtures.rs:1211`). The scroll clamp counts real rows instead of
    `len * 3` (`:71`).
- **D9: Docs sub-tab.**
  - `J`/`K` move a row cursor, drawn by style only (`theme.selected`), so the text snapshot is
    unchanged. PgUp/PgDn scroll as before. This is `ReqsTab`'s cursor (`requirements.rs:183-187,
    561-564`).
  - `a` sends `DocumentForm { kind: None }` and opens an empty form: kind, title, body.
  - `v` sends `DocumentForm { kind: Some(row.kind) }` and opens the form with the kind fixed and the
    title and body prefilled from `base`, the latest version, not the row under the cursor.
  - Ctrl+S sends `WriteDocument`.
  - `DocumentWritten` closes the form, re-sends `StoreRequest::Documents(item)`, and shows the D5
    notice under the table until the next key.
  - A hint line names the item's existing kinds plus `summary`, read from the rows the pane
    already holds, so no extra read is needed.
- **D10: a write whose answer was lost is hedged, as a mint is (milestone 2 D11, MOD-59 M1).**
  - A retried note or document would land twice.
  - `hand_written::write_refused(message)` is true for:
    - the offline refusal;
    - any `Constraint`, which is the validator's;
    - `NotFound { entity: "item" }`, which D3 checks before the write.

    It mirrors `item_writes::mint_refused` (`:199`) plus `NotFound`.
  - On a refusal, the area keeps its text and shows the sentence.
  - On anything else, it keeps the text and says "it may have been written; the thread/list is
    being re-read — look for it before Ctrl+S", and re-reads `Notes`/`Documents`.
  - A `Failed` for a form read opens nothing.
- **D11: hint lines and snapshots.**
  - Plain `a`/`v` cannot become Backlog help rows: `a` already means a Runs action
    (`runs.rs:15-29`), and a `KeyScope::Tab(Backlog)` row would be shown for every sub-tab. So each
    sub-tab draws its own one-line dim hint, as `ReqsTab` and Runs do:
    - Notes: `a add note`;
    - Docs: `J/K move · a new · v new version`.
  - Neither pane binds `w`, so the global workspace switcher stays reachable (`app/mod.rs:78-83`).
  - The hint changes four existing snapshots, **deliberately and in one commit**, which is the PRD
    risk row's second mitigation: `backlog__detail_notes`, `backlog__detail_documents`,
    `backlog__empty_notes` and `backlog__empty_documents`. Every other `backlog__*.snap` stays
    untouched.
- **D12: one new conformance parity case, `hand_written_rows_round_trip`.**
  - It writes a note with `via_step_id: None` and `box_id: Some(BOX)`, and a document with
    `produced_by_step_id: None` after a step-produced version of the same kind. It reads all three
    back field-equal through `notes`, `documents` and `document`.
  - No existing case pins this. `gate_answers_write_their_outcome`'s successful note carries a
    step, and `write_document_allocates_its_version` asserts only versions and ranking
    (`conformance.rs:10457, 11162`).
  - The count goes 130 → 131, with its history message (`mem_store.rs:37-60`,
    `pg_conformance.rs:17-34`).

## Out of scope
- **A document body viewer in the Docs pane.** `v`'s prefilled form shows the latest body. A
  read-only `o` like Runs' (`runs.rs:689-740`) would need those helpers promoted. Deferred unless
  you want it.
- Suggesting the item's graph phase output kinds in the kind field. The hint uses the kinds the
  item already has, plus `summary`.
- **The step-input ranking of hand-written documents. This is an open question for you, below.**

## Open question for the maintainer (not blocking milestone 5)
**A hand-written new version is not what the next step reads.**
- `resolve_inputs` ranks this run's output first, then another run's, then a hand-written one,
  whatever the version (`mem.rs:4230-4239`, `pg/read.rs:805`). The conformance case
  `write_document_allocates_its_version` pins this.
- ANA-2 §4.8 says "an edit is a new document version followed by `approved`" (`docs/ANA-2.md:459`).
  With today's ranking, that edit is shown everywhere but is not fed to the next phase while a
  step-produced version of the kind exists.
- `documents_of_kinds` (prompt assembly, the latest by version) does pick it.
- Milestone 5 ships the writer either way. **Recommendation:** file a follow-up item to reconcile
  §4.2's ranking with §4.8, rather than change a pinned engine rule inside an editor milestone.

## Maintainer answers (2026-10-03, after the blueprint)
- **Blueprint §6 Q1: an unchanged `v` save is refused.** Ctrl+S on a `v` form whose title and body
  equal `base`'s says `NOTHING_TO_SAVE` (`item_spec`'s sentence) in the form and sends nothing.
  The check is in the pane only; the worker cannot know the base. One test in T4.
- **Blueprint §6 Q2: `v` opens with the cursor on the body** (the blueprint default).
- **Open question (ranking): file a follow-up item at close-out** (lifecycle P0, minted through
  `scripts/hr-mint`), to reconcile ANA-2 §4.2's input ranking with §4.8. Milestone 5 ships
  unchanged.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Worker write module | `crates/htui/src/item_writes.rs` (`serve` 216, `write_access` 295, `offline` 301, `REQUEST_NAMES` 50) | Writer first, then the item read, then the validator, then the write; identity from `Backend` |
| Redacting text newtype | `item_writes.rs` `ItemFormContext`'s hand-written `Debug`; rule at `store_worker.rs:116-122` | A body prints as its length |
| Self-naming write reply | `StoreReply::ItemWritten` (`store_worker.rs:1327`) | The applied write answers its own variant |
| Worker routing | `store_worker.rs` or-arm `:1709-1711`, `name()` `:1045-1048` | A new or-arm plus 4 `name()` arms, with a comment naming `hand_written::REQUEST_NAMES` |
| Shared validator | `htui-core/src/model/item_spec.rs` | `fn` per input, refusal names the field, unit tests in-file |
| Writing sub-tab | `ui/tabs/backlog/detail/requirements.rs` (`send` 199, `on_reply` 496, `captures_input` 488, `footer` 384) | `busy`, an item-guarded `on_reply`, a dim hint footer cut by `cells::clip` |
| Compose and `$EDITOR` | `ui/tabs/backlog/item_form.rs` (`on_external_edit` 530, `hand_off` 594, Ctrl+S before chords 623) | Ctrl+S first, Ctrl+E on the area, outcome back into the same field |
| Lost-answer hedge | `item_writes::mint_refused` (199); `backlog/mod.rs` `on_item_failed` (487) | Refusal plain; anything else hedged plus a re-read |
| Width | `ui/cells.rs` (`clip` 123, `wrap` 185); `settings::wrapped` (`settings/mod.rs:86`) for notices | Cells, never `.len()` (MOD-60) |
| Sub-tab unit tests | `detail/requirements.rs` tests (`Shell` 592, `drained` 629, `key` 643) | Drive the pane directly, assert on requests and errors |
| Worker tests | `item_writes.rs` tests (`demo()`, the offline test) | Memory and `Backend::Offline { cache: CacheStore::open(..) }` |
| Integration | `tests/backlog.rs` (`sub_tab` 80, `backlog_over` 1330, `offline_backlog` 1846) | `Harness::over(store.clone())`; Docs = `sub_tab(3)`, Notes = `sub_tab(4)` |
| Pg via worker | `tests/item_writes_pg.rs` (`#![cfg(feature = "testkit")]`, `Stack::new` + `demo_db()` SKIP) | `store_worker::serve` over Online |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/hand_written.rs` | CREATE | D4 validator and its unit tests |
| `crates/htui-core/src/model/mod.rs` | UPDATE | `pub mod hand_written;` plus re-exports |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | D12 case in `CASES` (52) and `run_case` (194) |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | Count 130 → 131 and its history message |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | `EXPECTED_CASES` 130 → 131, doc and message |
| `crates/htui/src/hand_written.rs` | CREATE | D1–D3, D10: serve, `HandText`, `DocumentFormContext`, `write_refused`, worker unit tests |
| `crates/htui/src/lib.rs` | UPDATE | `pub mod hand_written;` between `event_loop` and `hierarchy` (A1) |
| `crates/htui/src/store_worker.rs` | UPDATE | 4 requests, 4 `name()` arms, 4 replies, 1 or-arm |
| `crates/htui/src/editor.rs` | UPDATE | `strip_added_newline` and `without_controls` moved here, `pub(crate)` (D7) |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | UPDATE | Call the moved helpers; behaviour unchanged |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | `pub mod compose;`, `DetailTab::on_external_edit`, `DetailRegistry::on_external_edit` (D7) |
| `crates/htui/src/ui/tabs/backlog/detail/compose.rs` | CREATE | D6 shared compose state, keys and render |
| `crates/htui/src/ui/tabs/backlog/detail/notes.rs` | UPDATE | D8: `a`, compose, replies, hedge, multi-line render, hint, unit tests |
| `crates/htui/src/ui/tabs/backlog/detail/documents.rs` | UPDATE | D9: cursor, `a`/`v`, form, D5 notice, hedge, hint, unit tests |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | `on_external_edit` falls through to the registry (D7); a unit test |
| `crates/htui/tests/backlog.rs` | UPDATE | Integration cases (Task 5) |
| `crates/htui/tests/snapshots/backlog__{detail,empty}_{notes,documents}.snap` | UPDATE | D11 hint line, one commit |
| `crates/htui/tests/snapshots/backlog__note_compose.snap`, `backlog__document_form.snap` | CREATE | The two compose areas |
| `crates/htui/tests/hand_written_pg.rs` | CREATE | Pg parity through `store_worker::serve` |

**Every task:** each new public item gets a doc comment and each public type a `Debug`, because of
`#![warn(missing_docs)]`, the `missing_debug_implementations` workspace lint and `-D warnings`.

## Tasks
### Task 1: validator and conformance case (TDD), `htui-core`
- **Action**: Write the tests first.
  - Note: `""`, `"  \n"` refused; `"  - item\n\n"` becomes `"  - item"`.
  - Kind: `""`/`"  "` refused; `" plan "` becomes `"plan"`; `"code review"` kept; `"pl\nan"`/`"a\0"`
    refused; `"summary"`, `"judge"` kept.
  - Title: blank refused; `"a\nb"` refused; trimmed.
  - Body: as the note.
  - Each refusal's sentence names its field.

  Then add the D12 case and bump both counts with their messages.
- **Files**: `model/hand_written.rs`, `model/mod.rs`, `store/conformance.rs`, `tests/mem_store.rs`,
  `htui-store/tests/pg_conformance.rs`.
- **Validate**: `cargo test -p htui-core --all-features --lib hand_written`;
  `cargo test -p htui-core --all-features --test mem_store`;
  `cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1`

### Task 2: worker requests (TDD), `htui`
- **Action**: Write the tests first in `hand_written.rs`, over `Backend::memory(MemStore::demo())`.
  - `NoteForm` answers `NoteForm { item }`.
  - `AddNote` lands a note by `ids::USER` with this box, `via_step_id: None`, and answers
    `NoteAdded`.
  - `DocumentForm { kind: None }` has `base: None`. `DocumentForm { kind: Some("plan") }` on
    `HTUI_FEAT_1` has base `plan` v2.
  - `WriteDocument` lands `plan` v3, hand-written, by `ids::USER`, and answers
    `DocumentWritten { version: 3 }`. A new kind lands v1, and so does `summary` on an open item.
  - An unknown item answers `NotFound { entity: "item" }` for all four.
  - Each D4 refusal answers `Failed` naming the field, through `store_worker::serve`, and writes
    nothing.
  - Offline, all four answer `DATABASE_UNREACHABLE` before any read.
  - `write_refused` is true for offline, `Constraint` and `NotFound(item)`, and false for
    `Backend`/other `Unreachable`.
  - The `Debug` of `AddNote`, `WriteDocument` and the `DocumentForm` reply holds no body.

  Then add the variants, the `name()` arms and the or-arm.
- **Files**: `hand_written.rs`, `lib.rs`, `store_worker.rs`.
- **Validate**: `cargo test -p htui --all-features --lib hand_written store_worker`

### Task 3: shared plumbing and the Notes sub-tab (TDD), `htui` UI
- **Action**: Write the tests first.
  - `editor`: the moved helpers' cases (the item form's existing tests stay green).
  - The Backlog: an outcome with no item form open reaches the active sub-tab.
  - Compose: Ctrl+S on a blank body refuses in the area and sends nothing; Esc drops the text;
    Ctrl+C passes; busy swallows keys; Ctrl+E on the body emits `EditExternally` with the
    `{key}-note` stem; an `Edited` outcome lands in the body with the cursor at the end.
  - Notes:
    - `a` sends `NoteForm`, or nothing with no item.
    - The reply opens the area, and it captures.
    - Ctrl+S sends `AddNote` with the text.
    - `NoteAdded` closes the area and sends `Notes(item)`.
    - A reply for another item is ignored.
    - A refusal keeps the text and the sentence.
    - A store `Failed` hedges and re-reads.
    - A `Failed` for `note_form` opens nothing and adds no error.
    - A two-line note renders as two rows.
    - A one-line note renders exactly as before.

  Then implement `compose.rs`, the hook, the helper move and `notes.rs`.
- **Files**: `editor.rs`, `item_form.rs`, `detail/mod.rs`, `detail/compose.rs`, `detail/notes.rs`,
  `backlog/mod.rs`.
- **Validate**: `cargo test -p htui --all-features --lib editor item_form backlog`

### Task 4: Docs sub-tab (TDD), `htui` UI
- **Action**: Write the tests first.
  - `J`/`K` move the cursor and clamp.
  - `a` sends `DocumentForm { kind: None }`. `v` sends `DocumentForm { kind: Some(row kind) }`.
    Neither sends anything with no rows or no item, as applicable.
  - A `v` reply opens the form with the kind fixed and the base title and body.
  - Ctrl+S sends `WriteDocument`.
  - `DocumentWritten` at the expected version closes the form, re-reads `Documents` and says
    "saved as plan v3". At an unexpected version it adds the "written after you opened" clause.
  - The kind/title/body refusals show in the form.
  - Hedge and re-read on a store `Failed`.
  - The hint names the item's kinds plus `summary`.

  Then implement `documents.rs`.
- **Files**: `detail/documents.rs`.
- **Validate**: `cargo test -p htui --all-features --lib documents`

### Task 5: integration, snapshots, Pg parity
- **Action**: Add these cases to `tests/backlog.rs`:
  - Notes: `a`, typed text, Ctrl+S, and the thread shows the note.
  - Docs: `v` on `plan`, an edited body, Ctrl+S, and the table shows `plan v3 hand`.
  - Docs: `a` with kind `summary` lands `summary v1`.
  - Docs: a version written through a held `MemStore` clone between open and save produces the
    D5 notice.
  - Offline (`offline_backlog`), for `a` in both panes: the status line ends with
    `DATABASE_UNREACHABLE`, no area opens, and nothing is written.
  - Ctrl+E through the loop's fake editor (`tests/backlog.rs`' milestone 4 case) lands a note
    body.

  Re-accept the four D11 snapshots in one commit, and add `backlog__note_compose` and
  `backlog__document_form`.

  In `tests/hand_written_pg.rs`, run note, document, version allocation, a refusal and
  `NotFound` through `store_worker::serve` over Postgres, with the Memory outcomes.
- **Files**: `tests/backlog.rs`, the snapshots, `tests/hand_written_pg.rs`.
- **Validate**: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`;
  `cargo test -p htui --all-features --test hand_written_pg -- --test-threads=1`

**Task order: serial, T1 → T2 → T3 → T4 → T5.**
- T1's files are disjoint from the rest, but T2 imports its validator, and both T1's gate and every
  later gate build `htui-core`.
- T3 and T4 share `detail/compose.rs` and the `detail/mod.rs` hook. T4 only consumes them, but on
  one tree T4's gate compiles T3's half-written module.
- A worktree split is not worth ~10G of `target/` per worktree.
- Ultracode runs this as a serial implement → adversarial-verify pipeline, one stage per task.

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui-core --all-features
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # full gate; grep SIGABRT
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A lost answer leads to a duplicate note or version on retry | L | D10 hedge and re-read |
| A concurrent version is silently superseded | L | D5 notice; append-only, so nothing is lost |
| A hand-written edit is not fed to the next step | M | Outside milestone 5; open question above, a follow-up item recommended |
| `$EDITOR` outcome reaches the wrong owner | L | D7: item form first, else the active sub-tab; a sub-tab ignores an outcome it did not ask for |
| Snapshot churn | M | D11: four re-accepted in one commit, the rest untouched; the cursor is style-only |
| A body leaks through `Debug` | L | `HandText` and a hand-written `DocumentFormContext` `Debug`, each pinned by a test |
| `w` is shadowed in a sub-tab | L | Neither pane binds `w`; a test presses it in each |
| Offline write reachable | L | D2 worker gate, plus an integration test per pane |

## Verified claims
Fact-check 2026-10-03, against `hr/MOD-13` at `f4e577a4`. Amendments: A1 the `lib.rs` slot; A2 the
kind rule is stricter than `name_is_valid`; plus three line offsets corrected in place (`:646`,
`:545`, `serve` 216).

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | Both stores implement `add_note`/`write_document`; `write_document` allocates under the row lock | ✓ | `traits.rs:1445, 1566`; `pg/write.rs:5219` `SELECT 1 FROM item WHERE id = $1 FOR UPDATE` |
| V2 | Notes and documents are mirrored offline | ✓ | `cache/mod.rs:55-56` |
| V3 | `$EDITOR` outcomes stop at the item form | ✓ | `backlog/mod.rs:688-694` |
| V4 | Worker identity, writer-first and offline refusal shapes | ✓ | `item_writes.rs:234-235, 295-302`; `backend.rs:170, 252` |
| V5 | Both stores take `created_at` from the argument | ✓ | `mem.rs:5117, 5413`; Pg binds it (`pg/write.rs:265-295, 5798-5823`) |
| V6 | Redaction rule | ✓, line corrected | `store_worker.rs:116-122` (not 99-105) |
| V7 | Or-arm, `name()` arms, `ItemWritten` | ✓ | `store_worker.rs:1709-1711, 1045-1048, 1327` |
| V8 | Non-item `Failed`s reach the sub-tabs | ✓, line corrected | The early return is only for `is_item_request` (`backlog/mod.rs:646`); the rest go to `detail.on_reply` (`:682`) |
| V9 | `App::on_reply` puts every `Failed` on the status line | ✓ | `app/update.rs:285-286` |
| V10 | Mem disagrees on a missing item | ✓ | `add_note` → `Constraint` (`mem.rs:5380-5385`); `write_document` → `NotFound` (`require_item`) |
| V11 | `documents_of_kinds` = latest by version, hand-written included | ✓ | `writer.rs:182`; `mem.rs:1198-1203`; `pg/read.rs:505-507` |
| V12 | No kind rule exists; `name_is_valid` shape; reserved names are phase names | ✓, amended (A2) | `0001_init.sql:392`; `kinds.rs:783`; `kind.rs:393-395`; `template.rs:52-58` |
| V13 | Capture, reveal guard and scope change already cover a typed sub-tab | ✓, line corrected | `backlog/mod.rs:585-589, 764-767`; `on_item_change(None)` at `:545` |
| V14 | `TextArea` gives `Submit` on Ctrl+S and `Pass` on other chords; `TextField` passes all chords | ✓ | `text_area.rs:192-199`; `text_field.rs:146-153` |
| V15 | The editor helpers are private to the item form; the notices are already in `editor` | ✓ | `item_form.rs:310, 325`; `editor.rs:20-28` |
| V16 | A sub-tab's `EditExternally` is stamped with the Backlog tab | ✓ | `app/state.rs:355-361`; a sub-tab's `Ctx` has origin `Tab(BacklogTab::ID)` |
| V17 | Demo notes are one line, so the multi-line render keeps `detail_notes` byte-identical before D11's hint | ✓ | `fixtures.rs:1211-1220` |
| V18 | `a` is a Runs key; `w` is global | ✓ | `runs.rs:15-29`; `app/mod.rs:78-83` |
| V19 | Conformance anchors and counts | ✓ | `CASES` `conformance.rs:52`, `run_case` `:194`; `mem_store.rs:37` = 130; `pg_conformance.rs:26` = 130 |
| V20 | No case round-trips a hand-written note or document | ✓ | `gate_answers_write_their_outcome` (`:10457-10479`) notes carry a step or are refusals; `write_document_allocates_its_version` (`:11162`) asserts versions and ranking only |
| V21 | Hand-written documents rank last as step inputs | ✓ | `mem.rs:4230-4239`; `pg/read.rs:805` |
| V22 | Integration anchors | ✓ | `tests/backlog.rs` `sub_tab` 80, `backlog_over` 1330, `offline_backlog` 1846, milestone 4 `$EDITOR` case 2176+; strip order Body Runs Graph Docs Notes, so Docs = 3, Notes = 4 |
| V23 | Text snapshots carry no style, so a style-only cursor changes no `.snap` | ✓ | `backlog__detail_documents.snap` is plain text |
| V24 | `FEAT-1` has `plan` v1, v2 and no `summary` | ✓ | `backlog__detail_documents.snap`; `fixtures.rs` `documents()` |
| V25 | Width helpers | ✓ | `cells.rs:123, 185`; `settings/mod.rs:86` |
| V26 | `NoteId::new()`/`DocumentId::new()` exist | ✓ | `ids.rs` macro (`:93, 95`) |
| V27 | Tasks marked independent | none | All five are serial (shared crate builds; T3/T4 share `compose.rs`), so there is no file-set intersection to check |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] Offline: `a`/`v` refused with the read-only notice and no write
- [ ] A note and a hand-written document of any kind, `summary` included, land through the TUI
- [ ] Only the four D11 snapshots change

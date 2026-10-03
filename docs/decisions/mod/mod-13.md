# MOD-13 - Backlog filters and item editing (done, 2026-10-03)

**Requirements:** `R-TUI-2`, `R-ENT-5`, `R-ENT-10..12`.
**Origin:** MOD-1 (`docs/decisions/mod/mod-1.md`). Design inputs: ANA-9 §4.2, §7.1, §7.2 and §7.4,
ANA-2 §4.7, ANA-5 §4.5. MOD-25's online-only rule (`docs/decisions/mod/mod-25.md`) bounds every
write. MOD-4 R-43 (`docs/decisions/mod/mod-4.md`) handed this item the editable `summary`.

**Artifacts:**
- PRD `.claude/prds/mod-13-backlog-editing.prd.md`, 5 milestones, all in the MVP. Its four open
  questions were resolved on 2026-10-01 by taking the defaults.
- One plan and one blueprint per milestone, under `.claude/plans/`:

  | # | Milestone | Plan | Blueprint |
  |---|---|---|---|
  | 1 | Filters | `.claude/plans/mod-13-filters.plan.md` | `.claude/plans/mod-13-filters.blueprint.md` |
  | 2 | New and edit | `.claude/plans/mod-13-new-edit.plan.md` | `.claude/plans/mod-13-new-edit.blueprint.md` |
  | 3 | Divergence | `.claude/plans/mod-13-divergence.plan.md` | `.claude/plans/mod-13-divergence.blueprint.md` |
  | 4 | `$EDITOR` | `.claude/plans/mod-13-editor.plan.md` | `.claude/plans/mod-13-editor.blueprint.md` |
  | 5 | Notes and documents | `.claude/plans/mod-13-notes-docs.plan.md` | `.claude/plans/mod-13-notes-docs.blueprint.md` |

Decision numbers (D*n*, E*n*, A*n*) are local to each milestone's plan or blueprint.

Each milestone was routed separately (PRD, then plan) and run in a TOOL-7 sandbox (`hr/MOD-13`).
Ultracode was used for the implement phase only. The configured `rust-reviewer` was the final
review authority every time.

---

## What was built

### Milestone 1: filters (`c9c01dc`..`025ef49`, 2026-10-01)
- `f` opens a capturing filter form in the list pane: status, project, required tags and "ready
  here". `F` clears it.
- "Ready here" is ANA-9 §7.4 for this box. It is composed in the store worker, so the offline
  mirror answers it too (`Items { ready_here }`).
- With no filter set, the screen is byte-identical to before.

### Milestone 2: new and edit (`7b32f86`..`ac7c989`, 2026-10-02)
- `N` opens a new-item form and `e` an edit form, both in the detail pane.
- A new item is minted through §7.1. An edit is one §7.2 compare-and-set carrying only the changed
  fields, and an edit that changes nothing is refused.
- **Offline.** The worker refuses `ItemForm`, `MintItem` and `EditItem` with
  `DATABASE_UNREACHABLE` before any read, so an offline box never opens a form.
- **Validation.** There is one shared validator, `htui_core::model::item_spec`. It checks title,
  body, tags, kind and the step graph, including the graph's project. It also checks
  `touched_paths` using `PathPrefix::parse`'s own `repo:glob` split. An unknown repo, a bare glob
  with no primary repo, a bad glob, and `/`, `./` or `..` entries are each refused by name.
- **Failed mint (D11).** A failed mint after a possible COMMIT is hedged ("may have been written")
  and the list is re-read unfiltered. A refusal is reported plainly.

### Milestone 3: divergence (`54667c9`..`ec0f00c`, 2026-10-02)
- A stale edit opens a three-way view over the whole Backlog area: ancestor, theirs and mine for
  each field that differs, plus side-by-side body and paths diffs.
- `m` or `t` rebases the form on the head. Ctrl+S then saves one §7.2 compare-and-set at
  `head.version` with `reason = 'divergence_resolution'`.
- **Ancestor.** The ancestor is the item the form opened on. That covers all seven
  `version`-covered columns, whereas `item_revision` snapshots only three. No migration was added.
- **Merge.** `htui_core::model::item_merge` keeps both sides' non-conflicting changes, so no path
  reverts a column that only the head changed.

### Milestone 4: `$EDITOR` round-trip (`1b96f9a`..`726633f`, 2026-10-02)
- In the item form, Ctrl+E on the body or the paths hands that field to `$VISUAL`/`$EDITOR`
  through MOD-9's handoff.
- The text comes back into the same field and saves with the same compare-and-set, so a stale save
  still opens the three-way view.
- No new event-loop code, request, conformance case or migration was needed.

### Milestone 5: notes and documents (`6242e7a3`..`598b69d9`, 2026-10-03)

**Notes sub-tab.**
- `a` opens a compose area. Ctrl+S appends the note to the thread (`R-ENT-11`).
- Multi-line bodies render line by line, wrapped in cells. A tab is drawn as spaces.
- The scroll limit counts wrapped rows. After the user's own note lands, the thread follows to the
  bottom.

**Docs sub-tab.**
- `J`/`K` move a row cursor.
- `a` writes a new hand-written document of any kind, `summary` included (`R-ENT-12`, MOD-4 R-43).
- `v` writes a new version of the kind under the cursor. The form is prefilled from that kind's
  latest version, with the cursor on the body.
- A save is a new version, never a compare-and-set (D5). The pane says which version landed, and
  names any versions written by others while the form was open.

**Worker.** `crate::hand_written` serves four requests: `NoteForm`, `AddNote`, `DocumentForm` and
`WriteDocument`.
- All four take the writer before any read, so offline they answer `DATABASE_UNREACHABLE` and no
  compose area ever opens (D2).
- Both writes check that the item exists first, which gives Mem and Pg the same `NotFound` (D3).
- Bodies travel in `HandText`, whose `Debug` prints only lengths.

**Validator.** `htui_core::model::hand_written` checks note and document text (D4, E3):
- a blank note or document body is refused;
- a document kind must be trimmed and contain no control characters; spaces are allowed;
- a document title must be one line;
- NUL is refused everywhere, because Postgres cannot store it.

**Plumbing.**
- `detail/compose.rs` is the shared compose area: Ctrl+S checked before chords, Ctrl+E on the body,
  Esc, busy and a notice line.
- A new `DetailTab::on_external_edit` routes an `$EDITOR` outcome to the active sub-tab when no
  item form is open (D7). The milestone 4 return helpers moved to `crate::editor`.
- Form replies (`NoteForm`/`DocumentForm`) go only to the active sub-tab. The Backlog also holds
  one back while another form is open (E4).

**Lost answers (D10, review M1).** A write whose answer may have been lost after a COMMIT is hedged.
The area stays busy and settles itself on a re-read.
- **Notes** compare the re-read thread against the note ids that existed when the write was sent.
  That avoids box clocks entirely.
- **Docs** read the kind's latest version, body included, through the writer.
- The outcome is one of three:
  - "it was written": the area closes, and Docs gives the version;
  - "not written, Ctrl+S tries again";
  - "could not check" or "cannot tell", which keeps the text. This happens offline, after an
    `Unreachable` failure, or when someone else's version has landed past the expected one.

  A read served by the offline mirror never claims "not written".

**Conformance.** Case 131, `hand_written_rows_round_trip`, round-trips a hand-written note and
document on both stores.

**Snapshots.** Two new: `backlog__note_compose` and `backlog__document_form`. Four were
re-accepted in one commit, each gaining only its hint line: `backlog__detail_notes`,
`backlog__detail_documents`, `backlog__empty_notes` and `backlog__empty_documents`.

## Decisions worth keeping
- **The worker is the offline gate** (M2 D2, M5 D2). No view carries a `writable` flag that could
  drift from the backend. Every write path, the form-opening read included, takes
  `Backend::writer()` first.
- **A stale edit is never written over the head** (M2 D6, M3). The token never moves silently. The
  three-way view and `divergence_resolution` are the only way past a divergence.
- **Documents are append-only, with no compare-and-set** (PRD OQ4, validated in M5).
  `write_document` allocates `max + 1` under the item's row lock. Nothing is overwritten. The pane
  surfaces concurrent versions instead of refusing them.
- **An unchanged `v` save is refused** with `NOTHING_TO_SAVE` (maintainer, 2026-10-03). The check
  is in the pane only, against a canonical base: the close-out summary's trailing `\n` must still
  read as unchanged (review round 7db4e32c).
- **Hedge, then settle; never invite a duplicate** (M2 D11, M5 D10 + M1). A refusal (offline, a
  `Constraint`, `NotFound(item)`) is plain. Anything else may follow a COMMIT. Each settle rule
  avoids a specific trap:
  - an offline-mirror read cannot prove absence (round 1 F1);
  - box clocks are not ordered (round 1 F2);
  - a copied title does not identify a version (round 2 F1).
- **Sub-tabs own their writes** (M5 D6). This follows `DetailRegistry`'s contract and `ReqsTab`'s
  precedent. The Backlog only routes. Sub-tab keys therefore get their own hint lines rather than
  Backlog help rows: `a` is already a Runs action.

## Gate
The full workspace gate ran serially (`--no-fail-fast -- --test-threads=1`, grepping for
`SIGABRT`) against Postgres at `localhost:5439`.
- Milestone 5 before the review fixes: 4012 passed, 0 failed, 30 ignored. `fmt` and `clippy -D
  warnings` were clean.
- After the review fixes (`598b69d9`): 4033 passed, 0 failed, 30 ignored. `fmt` and `clippy -D
  warnings` were clean. A closing adversarial verify of the last fix (`598b69d9`) returned "sound".
- `rust-reviewer`: approve-with-fixes (M1, L1–L3, NIT1–4). M1, L1–L3 and NIT1–3 were applied (NIT4
  only where it overlapped L1). The fix round's verifiers found four further defects, all fixed.

## Carried
- **MOD-73** (new, from this milestone): a hand-written new version of a step's output kind is
  *not* what the next step reads. `resolve_inputs` ranks this run's output first, then another
  run's, then a hand-written one, whatever the version, while ANA-2 §4.8 says "an edit is a new
  document version followed by `approved`". The maintainer chose to file it rather than change a
  pinned engine rule inside an editor milestone.
- **Deferred by the maintainer at earlier reviews:**
  - M2: splitting `item_form::render` and `item_writes::serve`; the paths/body indent in the form;
    the wording after a lost edit answer.
  - M3: the by-value `merge` (NIT-2).
  - M4: notices are drawn in the error colour, so "edited in $EDITOR" is red (L5). The
    control-character filter covers only `$EDITOR` returns.
  - M5 review NIT4: per-frame allocations in the Docs hint and `Compose::kind()`.
- **From the closing adversarial verify of `598b69d9`** (verdict sound; non-blocking):
  - Two Docs hedge cases have no test: "nothing landed, latest below expected", and the `a`-form
    path, whose expected version comes from the list and may cover a kind with no rows.
  - When the check read fails, the message still says "the list could not be re-read", although
    it is the latest-version check that failed (`documents.rs`, the `Failed` arm's
    `could_not_check(.., "list")`).
- **Out of scope in M5:**
  - a read-only document viewer in Docs (an `o` like Runs');
  - suggesting the item's graph phase output kinds in the kind field.
- **Pre-existing, not from MOD-13:** `cargo doc` with `-D warnings` fails on four broken
  intra-doc links:
  - `htui-core` `store/traits.rs:1427` (`MIRRORED_TABLES`);
  - `htui` `agent_worker.rs:825`, `ui/text_area.rs:19` and `ui/text_field.rs:5`.

  No gate runs `cargo doc`.

## Commits (milestone 5)
- **Plan:** `ab6e21de` (plan and PRD row), `a65fc8ec` (blueprint), `8b6020b9` (maintainer answers).
- **T1, validator and conformance case:** `6242e7a3`, `55687fb0`.
- **T2, worker:** `fc074cde`.
- **T3, plumbing and Notes:** `b2733b19`, `324ae872`, `011d6bb0`, `73d92833`, plus verify fixes
  `7db4e32c` and `cc90110c`.
- **T4, Docs:** `e872f145`.
- **T5, hints with snapshots, integration, Postgres:** `2b92841b`, `c949c3fc`, `c2ae9699`.
- **Review fixes:** `a0bca934` (L3, NIT1–3), `719b8fa7` (L1–L2), `cee4c7a1` (M1).
- **Verify-round fixes:** `f33b6c68`, `171a2cea`, `5c6db8b4`, `598b69d9`.

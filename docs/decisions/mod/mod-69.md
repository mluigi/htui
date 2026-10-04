# MOD-69 - Waiting-on-you list across items (done, 2026-10-04)

**Requirements:** `R-TUI-11` (added for this item), `R-TUI-1` (top-bar line amended), `R-TUI-4`,
`R-ORCH-2`, `R-ORCH-4`, `R-NF-3`.
**Origin:** ANA-27 §5.1 T8 (`docs/ANA-27.md`, `docs/decisions/ana/ana-27.md`).
**Artifacts:**
- PRD [`.claude/prds/mod-69-waiting-on-you.prd.md`](../../../.claude/prds/mod-69-waiting-on-you.prd.md):
  problem, evidence, two milestones, resolved open questions;
- plan [`.claude/plans/mod-69-waiting-on-you.plan.md`](../../../.claude/plans/mod-69-waiting-on-you.plan.md):
  D1-D9, plus a verified-claims table (27 claims, 17 amended before CONFIRM);
- blueprint `.claude/plans/mod-69-waiting-on-you.blueprint.md`: amendments A-1-A-12, hazards
  H-1-H-24, maintainer decisions M-1-M-3.

Decision numbers are local to MOD-69 (the MOD-31 convention).

Routing was **PRD** (C2, C3 and C4 fired), with ultracode on the implement phase. The run was in a
TOOL-7 sandbox (`hr/MOD-69`). Both milestones, M1 "See what waits" and M2 "Jump to it", were planned
and built together.

**Decisions (maintainer, 2026-10-03 and 2026-10-04):**
- route accepted, with ultracode for implement;
- agent questions go to a new item, **MOD-75** (an MCP tool that parks the step), which is blocked
  on MOD-11. Its rows join this list once it lands;
- the list covers only the active workspace;
- a judge failure gets its own row;
- the top bar shows working and waiting runs separately;
- `R-TUI-11` added and `R-TUI-1` amended as proposed (`docs/REQUIREMENTS.md`);
- offline, open permission requests are left out and the overlay says so;
- M-1: `Ctrl+w waiting` stays clipped from the 100-column help line; the `?` box shows it;
- M-2: a promoted step is listed as a gate row (`promoted to chat`);
- M-3: the three Unblock texts read as written in the blueprint (§3.3);
- review: every finding applied except M3's body trim, which is deferred (see Carried).

**Commits:**
- docs: `69682b6c` (PRD, MOD-75 filed), `375a2cc9` (requirements), `c81ffbbd` (plan), `ba41b6f8`
  (blueprint);
- T0 model rows: `cfcafcb3`, `1755f634`, `f019b5cb`, `a31ada18`;
- T1 store reads: `9e8cd4bd` (red), `8a017cfc` (MemStore), `e299b89e` (Postgres, mirror, `.sqlx`),
  `550af45d`, `ea421851`; merged `8abf19aa`;
- T2 classifier: `3aff8d8c` (`unblock_case` extracted), `6b5fa0ec` (red), `34869dd1`, `1afc322d`,
  `f717c86b`, `9809d97d`; merged `dbd7688a`;
- T5 reveal to step: `e41f9449`, `ec611fd4`, `e535b087`; merged `0d9400a2`;
- T3 TUI: `cf35654e` (request, top bar), `593d4178` (overlay, `Ctrl+W`), `b2749e0e` (end to end),
  `a40b7367` (109 snapshots re-accepted);
- review fixes: `7baa1b88` (H1), `cdda1a87` (M3), `17bca328` (L5), `0f27dd31`, `4d193bec` (L1),
  `6537d704` (M1), `d783211f` (L4), `37373b44` (M2), `fe02b100` (L2, L3, L6), `48ab4406` (T1, T2),
  `b26747eb`, `c2f86183`, `79ca858e`, `a5119cdb`, `3f788e46`, `1c78a4dd`.

---

## What was built

**Top bar.** `… · N working · M waiting`. Waiting is the number of rows in the list. Working is the
number of active runs that own no row, so no run is counted twice. The waiting count is accented
when it is not zero. Before the first reply both counts read zero.

**Overlay (`Ctrl+W`, from every screen).** It lists every run in the active workspace that waits on
a person, one row per reason. Each row shows the item key, the step label, the reason and its text.
The reasons, in sort order:

| Reason | When |
|---|---|
| gate | a step at `awaiting_approval`; text is its `gate_note`, else `promoted to chat`, else `gate` |
| selection | a parked run with no parked step, and a fan-out slot whose candidate `select` the Runs pane would enable |
| judge failed | the same, when the slot's judge (`judge_at`) is `failed` with a note; text is `<note> - pick a candidate` |
| unblock | `unblock_case` answers Reopen, FollowRun or Resume; the text says what `u` does |
| interrupted | a parked run that none of the rules above covers (review H1: `park_interrupted`), on the run's rest step |
| permission | each open permission request (pending, under the run's live lease, not a chat run); text is the tool summary |

`Enter` closes the overlay and opens the item's Runs sub-tab with that step under the cursor. This
works when the item is already selected, and when it is not yet loaded. `j`/`k` and the arrow keys
move the cursor, which stays on its row when the list re-sorts. Offline, the list keeps its gate,
selection, unblock and interrupted rows, marked read-only, and says permissions are unavailable. A
permission read that fails while online says so rather than claiming to be offline.

**How it is read (D1, D6).** The shell's refresh tick (4 × 250 ms) and a scope change send
`StoreRequest::Waiting { scope }`, which replaces `ActiveRuns`. The serve arm makes these reads:
- `active_runs`;
- `ReadStore::waiting_candidates(scope)`: items that are `blocked` or `awaiting_approval`, or that
  own a parked run, each with its active runs and their steps;
- online only, `WriteStore::open_permissions(scope)`.

It then calls the pure `htui_worker::waiting`. The classifier reuses the Runs pane's own guards
(`select_of`, shared with `verdicts`; `unblock_case`, pulled out of `verdicts`), so the list and the
pane cannot disagree. The reply carries its workspace and project set, and a reply for a scope that
has been left is dropped. The view lives in `TopBarState.waiting`; the overlay draws from it and
keeps nothing but its cursor. The list has no table and no persisted state of its own.

**Store.** Both new methods are in Mem, Postgres and the mirror (the permission read is on
`WriteStore`, because the relay tables are not mirrored). Grouping and order come from one place,
`WaitingCandidate::assemble`/`sort_canonical`. There are two new conformance cases (`CASES` 135 →
137), a mirror-versus-Postgres test, and four new `.sqlx` entries (322 → 326). There is no migration
and no new crate.

**Reveal.** `RevealTarget::Step { item, key, run, step }` routes as an item reveal.
`DetailTab::focus` (a no-op by default) arms the Runs pane's pending focus. The pane applies it to
loaded rows at once, or on the next `Runs` reply. An item change or the user's own cursor move
disarms it.

**Keys.** `Ctrl+W` is a global chord. Settings › Boxes used to take Ctrl+W as its `w`; it now ignores
modified keys.

## Verification

- Every task ran as implement → conformance and adversarial verifiers → repair, with a verifier
  after every repair. T1, T2 and T5 ran in parallel worktrees. The verifiers found test gaps a green
  gate missed:
  - sort tests that a sort by id alone would pass;
  - a selection check that was never reached;
  - a reveal focus that survived a cursor move.
- The `rust-reviewer` gate asked for changes. H1 was a real gap: a run parked by an interrupted step
  was counted as working. The fix round's own verifiers then caught two problems in its fixes: an
  "offline" message shown online, and a cursor that was only anchored after the first key press.
- Final gate on `hr/MOD-69` after the review fixes: fmt and workspace clippy clean; `cargo test
  --workspace --all-features --no-fail-fast -- --test-threads=1` 4277 passed, 0 failed, no SIGABRT.
  (An earlier merged-tree run hit the known `qdrant_live`/`qdrant_worker` payload-index timeouts
  under load; they passed re-run alone.)

## Carried

- **M3 body trim (deferred, maintainer).** The per-tick read ships every candidate's `item.body`,
  which the classifier never reads. Trimming it would hand back `Item` rows with an empty body, a
  trap for MOD-12's planned reuse. It would also touch three backends and `.sqlx`. The reviewer
  rates the cost as fine at today's sizes.
- **MOD-75** adds a "question" reason once the MCP server (MOD-11) exists.
- **MOD-67** registers `Ctrl+W` as a named action when it lands.
- **MOD-12** may reuse `waiting_candidates` + `htui_worker::waiting` for its escalation list.

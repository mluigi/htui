# MOD-59 - A write's reply names itself, so a form never stays "in flight" (done, 2026-09-30)

**Requirements:** `R-TUI-7`, `R-NF-3`.
**Origin:** MOD-9 milestone 3 review, finding 3 (`docs/decisions/mod/mod-9.md`), found 2026-09-26.
MOD-39 (`docs/decisions/mod/mod-39.md`, "Carried") added the Requirements tab to the item.
**Artifacts:** plan with its verified-claims table:
[`.claude/plans/mod-59-self-naming-write-replies.plan.md`](../../../.claude/plans/mod-59-self-naming-write-replies.plan.md);
blueprint:
[`.claude/plans/mod-59-self-naming-write-replies.blueprint.md`](../../../.claude/plans/mod-59-self-naming-write-replies.blueprint.md).
There is no PRD. The item was routed as a plan on 2026-09-30 with 1 of C1-C4 fired (C3: the item named
two fixes without choosing one).
**Commits:**
- Plan and blueprint: `172db01` plan, `5894118` confirmed, `53d85ce` blueprint.
- Skills: `4b36851`/`2b60daa` (T1, red then green).
- Templates: `35c7ed9`/`51eda84` (T2).
- Requirements: `11d78b0`/`1a0dc1e` (T3).
- Review fixes: `cd22987`, `5cf60ff`, `5e1db9b`, `cef5132`, `d898df9`, `58aee62`, `151590b`, `0c099e6`, `7eff2d5`.
- Re-review fixes: `c86dcc4`, `483de51`, `b851a22`.
- Outside the item: `5c53050`, a gate fix (see "Also in this branch").
- Then this write-up.

## The defect

Three views decided that their own write had landed by searching the re-read snapshot for what they
sent:
- the Skills library, in `LibraryView::land`;
- the Templates view, in `land_save` (milestone 1's D27);
- the Requirements tab, in `RequirementsTab::land`.

The worker answered an applied write with the same variant it used for a plain read (`Skills`,
`Templates`, `Requirements`). The view therefore could not tell its own write's reply from a read
served ahead of it. When the content match failed, `busy` stayed set, and the form refused `Esc` and
`Ctrl+S` until the workspace changed. The match failed in these cases:
- another session renamed or re-described a skill, or attached or detached one, between the write
  and the re-read;
- `MemStore` stamped two writes with the same instant (a rename's `updated_at != token`, an
  attach's token);
- a read that already showed the write landed it before the write's own reply arrived.

The Requirements mint also re-read in order to check itself (`verifying`, `CHECKING_MINT`), because
an applied mint whose re-read failed answered `Failed`.

Version saves and template saves were in practice immune to the race. Both tables are append-only,
so another session's later append leaves our version's body where the predicate looks (blueprint
DV-2). Their failure modes were the early landing and the failed re-read.

## What shipped

**Self-naming replies (D1, D2).** The same shape as MOD-23's `AgentWritten`:
- `StoreReply::SkillWritten { snapshot, outcome: SkillWrite }`, where `SkillWrite` is one of
  `Created`, `Versioned`, `Edited`, `Attached { key, updated_at }` or `Detached`;
- `StoreReply::TemplateSaved { snapshot, project, name, version }`;
- `StoreReply::RequirementWritten { snapshot, outcome: RequirementWrite }`, where `RequirementWrite`
  is one of `Area`, `Minted`, `Amended` or `Withdrawn`.

The outcome is what the store returned: ids, names, keys and versions, never a body. `Skills`,
`Templates` and `Requirements` now answer reads only. The `*Stale` replies are unchanged (D3): only
a write produces them.

**Landing on the write in flight (D4).**
- A view lands a `*Written` reply only while `busy` names that outcome's write
  (`request_name()`; `SAVE_NAME` for templates). A plain read never lands and never frees `busy`.
- The notices, the cursor move and the "later edits kept" rule come from the outcome and the body
  the view sent. Nothing searches the snapshot.
- The Requirements tab's `Sent`, `verifying` and `CHECKING_MINT` are gone (DV-3).
- The library's `Sent` keeps only the bodies, and the attach change and target, that landing reads
  (review L2).

**A write that applied but could not be re-read (D5).**
- `snapshot` is `Result<Box<_>, String>`. The view lands the write, keeps drawing the tree it held,
  and says `… — the re-read failed, r reloads: <why>`. It sets `unavailable` only when it held no
  tree.
- The Requirements tab skips the detail read in that case (review L4).
- A refused write is still `Failed`.
- The rule lives in one place: `WriteOutcome<T, S>::answer` beside `StoreReply` (review L3).

**The attach token (DV-1).** `Attached` carries the landed row's `updated_at`. A kept attach edit
therefore saves over this write's own row, not over a row another session wrote in between. That
racing row answers `SkillsStale` instead of being overwritten.

## Decisions worth keeping

- **A self-naming variant, not "release on any reply to the request name".** Only `Failed` carries
  a request name, and the envelope's `seq` stops at `App::is_fresh`. The freshness key
  `(Origin, Discriminant<StoreRequest>)` means a read never supersedes a write, and a newer write of
  the same kind drops an older reply. Every view resets `busy` on a scope change. An `Err` snapshot
  cannot be scope-checked, so it acts only while its write is still in flight.
- **DV-4 (not applied):** `snapshot` carries a rendered `String`, not a `StoreError`. As a result,
  an unreachable re-read after an applied write no longer reaches the worker loop's `go_offline` or
  the status line. The refresher's health pass (`lost_the_server`) takes the backend offline
  instead, a little later. Code notes at `written`/`saved` (review L5).
- **A mint's `Failed` is hedged (review M1, the maintainer's call).** A `Failed` mint can still hide
  a committed write whose acknowledgement was lost. It is the one tab write whose retry duplicates
  (a requirement can be withdrawn, never deleted).
  - The form keeps its text. The tree, which stays visible beside the form, is re-read, and the
    notice says the mint may have been written: look for it before `Ctrl+S`.
  - The four refusals `serve` gives before the insert (offline, blank body, area not the project's,
    not the maintainer) read as plain refusals. They are recognised by `requirements::mint_refused`,
    built from the same error values `serve` uses (re-review L1, L2).
  - Recorded as a residue in the `requirements.rs` module doc.

## Tests

The first commit of each task was red and compiled. Its tests failed at runtime for the stated
reasons: the worker still answered the read variant, or the view ignored the new one.
- **The HANDOFF scenarios:**
  - another session re-describes a skill before the re-read;
  - a rename or attach at `MemStore`'s frozen `TestClock` instant;
  - an amend landing although another session amended again;
  - a read showing the save or mint does not land it.
- **The D5 paths**, in all three views.
- **The DV-1 attach token:** the next save goes stale against a racing row.
- **Guards:** a reply for another write lands nothing (Skills, Templates, Requirements with a
  pending reveal), and a late `Failed` for another skills write does not free the one in flight
  (review L8).
- **Worker tests:** every applied write answers its variant with the stored ids; each outcome's
  `request_name()` equals its request's `name()`.
- **Postgres:** `tests/requirements_pg.rs`'s `applied()` follows the variant.

Gate at close-out: `cargo fmt --check`, `cargo clippy --workspace --all-features --all-targets
-D warnings` and `cargo test --workspace --all-features -- --test-threads=1` (2980 passed after the
review fixes; 1273 in `htui` after the re-review fixes), with the `_pg` suites run against the
sandbox Postgres.

## Review

`rust-reviewer` approved with fixes (2 MEDIUM, 9 LOW), then re-checked the fixes and approved.
- **Fixed:**
  - **M1:** the mint hedge (above).
  - **M2:** a Requirements test for another write's reply.
  - **L1:** `land` matches before it frees the draft.
  - **L2:** the library's `Sent` trimmed.
  - **L3:** one `WriteOutcome` for the three writers, and the Requirements `land` checks `busy`
    itself.
  - **L4:** no detail read after a failed re-read.
  - **L5:** the offline note.
  - **L6:** a test renamed, and a cursor check that always passed made to bite.
  - **L8:** the library frees only the write in flight.
- **Re-review fixes:** L1 (the hedge no longer sends the user to `Esc`, which discards the draft),
  L2 (no hedge on definite refusals), and wrapping lines over 100 columns.
- **Accepted:** L7, two red tests (the race, the attach token) failed in their helper and not on
  the wedge. The variant cannot exist on the base, so "fails on base" holds only indirectly.
- **Declined:** L9, narrowing `loopback.rs`'s file-level `#![expect]` (MOD-65 D6 sets the
  file-level form for fake-agent test files).

## Also in this branch

`5c53050`: `crates/htui-agent/tests/loopback.rs` (MOD-22) gains MOD-65's file-level
`#![expect(clippy::disallowed_methods, …)]`. MOD-22 was merged after MOD-65's crate lint, so
workspace clippy was red on `main` at `08e0880`.

## Not done

- **The Backlog detail's Reqs pane (`ItemCitations`) is out of scope (D6).** It frees `busy` on any
  `ItemCitations` for its item, so it cannot wedge. Its three writes still answer `Failed` when only
  their re-read failed (residue in the `requirements.rs` module doc).
- **Consistency notes, unreachable because the worker answers in order:**
  - `SkillsStale` and the Requirements `stale()` still `take()` `busy` whatever write is in flight.
  - The Templates `Failed` arm has no `busy` guard.
- **A lost-ack mint while offline.** The hedge's re-read is answered from the offline mirror, which
  may not show the mint yet. Recorded in the residue.

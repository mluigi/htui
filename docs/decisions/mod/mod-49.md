# MOD-49 - Interactive path picker for repo and workspace roots (done, 2026-10-02)

**Requirements:** `R-BOX-4` (a refusal or a display never names a link's target), `R-TUI-8`
(Settings), `R-NF-3` (off the UI task). F-102 held as a constraint: the chosen path goes through the
unchanged `htui_core::root_path::canonical_root`.
**Origin:** MOD-7, raised by the maintainer at its PRD gate (2026-09-25); MOD-7 D5 left the picker
here and milestone 4 (D115, OQ-28) made Settings > Hierarchy's `b` the typed fallback.
**Artifacts:**
- plan [`.claude/plans/mod-49-path-picker.plan.md`](../../../.claude/plans/mod-49-path-picker.plan.md): P1-P11, with its verified-claims table;
- blueprint `.claude/plans/mod-49-path-picker.blueprint.md`: deviations B-1-B-9, hazards H-1-H-6, decisions D1-D24.

Decision numbers are local to MOD-49 (the MOD-31 convention), because parallel sandbox runs share the global sequence.

Routed as **plan** (C2 fired; C3 and C4 borderline, low confidence). Run in a TOOL-7 sandbox (`hr/MOD-49`).

**Decisions (maintainer, 2026-10-02):**
- route accepted;
- plan confirmed as fact-checked (P1-P11 as proposed);
- review: every finding applied.

**Commits:**
- plan and blueprint: `9a60cf96`, `c64dfa4b`;
- T1 `list_dirs`: `75f31437`, `a780ef1e`;
- T2 `ListDir` through the store worker: `b150e3c4`, `99eb2bd8`;
- T3 `PathPicker`: `ba93bf54`, `09f10766`;
- T4 Hierarchy `b`: `59d35ed2`, `fd82d015`;
- review fixes: `1dc1fdd1`, `8a781d89` (M-1), `3186c9d5`, `a1621673` (L-1), `b4069a58`, `b9363bdb`
  (L-2, N-2), `2b7197ff`, `9652ec2c` (N-1), `239092a4` (N-3).

---

## What shipped

`b` on the workspace row or a repo row of Settings > Hierarchy no longer opens a one-field text
editor. It opens a popup over the section that lists directories **on this box** and chooses one.
The choice is sent as the same `SetWorkspaceRoot` / `SetRepoPath` the editor sent, so the store
worker's `canonical` → `canonical_root` guard still decides what is stored and what is refused.

- **Listing (T1, `htui_core::root_path::list_dirs`).** Sync `std::fs`, beside `canonical_root`.
  Directories, and links that resolve to a directory (marked, target never carried: `DirEntry` is
  `{ name, is_link }`). Files, dangling links, links to files, non-UTF-8 names and `Err` entries
  are dropped. Hidden entries are filtered **before** the cap unless asked for. Byte order, cap
  1000, the rest reported as `more` (`+N more`), never silently cut. A directory that cannot be
  read is the new `RootRefusal::Unreadable` (D3), checked after the four `canonical_root` refusals.
- **Request (T2, review M-1).** `StoreRequest::ListDir { path, show_hidden }` →
  `StoreReply::DirListing(DirListing)`, or `Failed { request: "list_dir" }` naming the path as
  typed. Served by `store_worker` itself, not `hierarchy::serve` (which demands a writer and a box
  row), and outside `hierarchy::REQUEST_NAMES`. Since M-1 the worker loop never awaits a listing:
  it runs on its own task under `spawn_blocking`, bounded by `LIST_TIMEOUT` (10 s), and replies to
  the request's `seq` and `origin`. A hung mount costs one blocking thread, not the store worker.
  The no-runtime path (harness, `--demo`) answers inline with the same bound.
- **Component (T3, `crate::ui::path_picker::PathPicker`).** Owned by a section as a mode, not a
  registered `Overlay`: overlay factories take no arguments and have no way back to a section
  (P2). It holds no channel; keys return a `PickerOutcome` (`Request`, `Chosen`, `Cancelled`).
  Keys: `j`/`k` move, `Enter`/`l` open, `h`/`Backspace` up (highlighting the directory left, D13),
  `s` choose the highlighted entry, `S` choose the listed directory, `/` go to (pastes accepted,
  D12), `.` hidden, `Esc` cancel. Every unbound key is swallowed, so `q` cannot quit mid-pick;
  `ctrl` chords still pass (D15). Navigation is lexical: `join` and `parent`, never `canonicalize`
  (P5); since L-1 a typed go-to path has `.` and `..` resolved on its text before it is asked for,
  so the kernel never resolves `..` after a link. A reply for any path but the newest asked is
  ignored (P9, D10). The popup is fixed-size with a scroll window (D14); since L-2 a refusal wraps
  and the hint splits on a narrow popup.
- **Wiring (T4).** `Mode::Picking { target: PathTarget, picker, chosen }`; `PathTarget
  { WorkspaceRoot, RepoPath }` replaced the editor's two `b` kinds (D7). The start directory is the
  stored path, else (for a repo) the workspace root on this box, else `$HOME` read once in `new()`
  (`with_home` for tests), else `/` (P7, D9). The popup stays open while its write is in flight and
  after a refusal, and a `Hierarchy` reply closes it (D8, D21). A root write is still followed by
  one `InferRepoPaths` (MOD-7 D136, D20). A `list_dir` refusal shows in the popup and on the status
  line (D16).

## Deviations from the plan

- **B-1:** `ListDir` carries `show_hidden`; the plan's `{ path }` could not serve `.`.
- **B-2, I-1:** three existing tests typed into the `b` editor (the plan's fact-check said none);
  all three were moved onto the picker with their intent kept. The inference-report one goes
  through go-to so its trailing-slash `stored as` case survives.
- **B-3:** `list_dir` is its own constant outside `REQUEST_NAMES`, with its own `Failed` arm ahead
  of the busy-clearing one; adding it to the list would have broken the offline-refusal and
  name-stability tests.
- **B-6:** the "link to nothing" refusal test uses a directory removed after it was listed, since
  the picker never lists a dangling link.
- **D23, amended by L-1:** the blueprint left `.`/`..` un-normalised; a go-to through a link then
  `..` listed the link target's parent. Typed paths are now normalised lexically.
- **Boxes section not wired (P10).** It has no path field; the component is reusable when one
  appears. The HANDOFF text's "repo paths in the box section" predated MOD-7 D115.
- **I-2:** the hint sits under the entries, as in the workspace switcher.

## Review

`rust-reviewer`: approve with fixes; one medium, two low, three nits, all applied.

- M-1 (`8a781d89`): a listing on a hung mount blocked the whole store worker loop; now off the
  loop with a 10 s bound.
- L-1 (`a1621673`): `<link>/..` typed in go-to listed the link target's parent (`R-BOX-4`).
- L-2 (`b9363bdb`): a long refusal and the key hint were cut on an 80-column popup.
- N-1 (`9652ec2c`): a stale reselect could move the cursor in an unrelated directory.
- N-2, N-3 (`b9363bdb`, `239092a4`): stale doc comments.

## Pins moved

`StoreRequest` 95 → 96, `StoreReply` 54 → 55 (HANDOFF had 91 / 52, already behind: B-9),
`crates/htui/tests/snapshots` 128 → 129 (the new `hierarchy__picker`), `hierarchy::REQUEST_NAMES`
stays 13. No migration, no `.sqlx` change. No existing snapshot moved.

## Carried

| What | Owner | Detail |
|---|---|---|
| The blocking thread of a timed-out listing | accepted | `std::fs` cannot be cancelled; after `LIST_TIMEOUT` the thread is no longer waited for but stays parked on the dead mount until the kernel answers. |
| A path field in Settings > Boxes | none | None exists; `PathPicker` is the component to reuse when one does. |
| Sandbox Qdrant hung on create-collection | environment | `tests/qdrant_worker.rs::the_worker_job_indexes_a_new_item` failed in the `hr/MOD-49` sandbox on one run (a plain REST `PUT /collections/…` also hung past 30 s) and passed on a later one; MOD-49 touches no Qdrant code. The host gate decides. |

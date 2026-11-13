# MOD-122 M2 — review fix round R1, TUI (Logs tab, Settings › Boxes)

Source: the configured reviewer (`rust-reviewer`, TUI instance) over `a05deeac..2499049b`.
Confirm each finding against the code before acting; if one is wrong, say so in `rejected_findings`
and leave it. Tests first for every behaviour change. Snapshot changes: run `cargo insta test` on
the affected tests, review every pending `.snap.new`, accept only intended diffs.

Context (plan decisions that bind here): D10 — worker sources are listed after the local ones, one
per box (or per stored name of a deleted box, marked `(removed)`); `all` stays this process only; a
box with no lines is not listed. D11 — the Settings › Boxes level row; `v`
(`settings.boxes.log_level`) cycles `off · error · warn · info · debug` and saves at once. Every key
goes through keys.toml (no hardcoded keys).

## Files owned

`crates/htui/src/ui/tabs/logs/mod.rs`, `crates/htui/src/ui/tabs/logs/workers.rs`,
`crates/htui/src/ui/tabs/logs/snapshots/*`, `crates/htui/src/ui/tabs/settings/boxes.rs`
(and its snapshots), `crates/htui/tests/box_settings.rs`,
`crates/htui/tests/snapshots/box_settings__*.snap`, `README.md`.

## Findings to apply

**M1 — worker labels are cut so `(removed)` disappears and similar hostnames collide**
(`logs/mod.rs:605`, `worker_line` cuts `worker · <name>[ (removed)]` into ~17 cells;
`logs_worker_removed.snap` already shows `worker · OLD-RIG (… 1 · ⚠ 1`; `DESKTOP-ABC1234` and
`DESKTOP-XYZ9876` both show `worker · DESKTOP…`). Fix, in the sources **list rows only** (the lines
pane title keeps the full `worker · <name>` label):
- never cut the `(removed)` marker (a short form such as `✕`/`(rm)` is acceptable if the full word
  cannot fit — keep it legible and say which in the report);
- cut the hostname so its **distinct end** survives (cut from the left: `…ABC1234`), not its start;
- drop the `worker · ` prefix in list rows if the rows are visibly grouped as workers otherwise
  (e.g. they already sit under a separator/heading, or gain a one-line `workers` heading in the same
  style as anything existing in the list); keep the prefix if dropping it would make worker rows
  indistinguishable from local ones;
- if the result gives worker rows the same name-column layout as local rows, the counts line up
  (closes the reviewer's L4).
The test `worker_lines_arrive_from_the_store_and_on_refresh` (`:1757`) currently builds its expected
row with the same `cells::fit` call — assert literal text instead. Add a snapshot with two
`DESKTOP-…` boxes and a removed box.

**M2 — README misses the Logs tab's worker sources** (plan file table, F12). `README.md:191` still
says "htui's own log as it runs"; the `### Logs` section (`:424-436`) does not mention worker rows.
Add one sentence each: worker rows (`worker · <box>`) show each connected worker's shipped log read
from the database about once a second, at the box's level set in Settings › Boxes; `all` stays this
process only.

**L2 — row label vs. the Logs empty text.** The Logs empty state (`logs/mod.rs:92`) points at
"Settings › Boxes › Worker log level", but the row is labelled `log level`. Rename the row to
`worker level` (12 cells, fits the 14-cell label column) and make every user-facing reference say
`worker level` (Logs text, row help, README). Update snapshots.

**L5 — the empty-worker-source view is dead UI.** A worker with no lines is never listed
(`Workers::get`, `keep_selection_listed` after every pull/reply/restore), so the branch at
`logs/mod.rs:511` is reachable only by setting the field in a test. Delete the branch, its constant
and `logs_worker_empty.snap`. If the "set the box's level" hint text has another home, keep it there;
otherwise it goes.

**L6 — `v cycles` shows while an editor or the executor confirm is open** (`executor_confirm.snap`,
`quirks_editor.snap`; `level_help`, `boxes.rs:239`). There `v` does not cycle. Show the key clause
only in Browse mode.

Not to change: L1 (`v log level` cut at 80 cols in `HINT_BROWSE` — accepted), L3 (load.rs edit is
fine), L7 (a box deleted mid-session keeps its lines under its id until restart — accepted, noted in
the plan), L8 (pre-existing `on_boxes` race).

## Check

`cargo test -p htui --all-features --lib -- ui::tabs::logs ui::tabs::settings::boxes store_worker --test-threads=1`;
`cargo test -p htui --all-features --test box_settings --test keys_file -- --test-threads=1`;
`cargo clippy -p htui --all-targets --all-features -- -D warnings`; `cargo clippy -p htui -- -D warnings`;
`cargo fmt -p htui`.

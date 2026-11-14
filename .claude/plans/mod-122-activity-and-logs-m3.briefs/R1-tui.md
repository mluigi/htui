# Brief R1-tui — MOD-122 M3 review fix round, TUI area

The review gate (`rust-reviewer`, TUI instance) over `e99acd97..HEAD` found these; all are applied in
this one fix round (the maintainer took the recommended answer on L4 and L7). **Confirm each against
the code before acting**; fix what you confirm, test-first where a test can pin it; list what you
reject, with the reason. The data-area fix round (R1-data, already landed before you) moved the
offline activity read to a `Failed` reply (M1); do not redo it. Plan context:
`.claude/plans/mod-122-activity-and-logs-m3.plan.md` D6, D8–D11. Every key from `keys.toml`.

## H1 — the duplicate-item warning loses its opening words on the default 80x24 Backlog

`crates/htui/src/ui/tabs/backlog/item_form.rs:~1091`. Under the new default `IdeActivity` at 80x24
the detail's inner area is 13 rows; the form's fixed rows take 11, leaving 2 for the warning, which
wraps to 3 rows at ~32 cells and the code drops the **first** row ("— it may have been written; the
list is being re-read, look for it first" loses "it may have been"). That warning is what stops a
second Ctrl+S from creating a duplicate. T6 pinned the two hedge tests
(`the_hedge_shows_whole_on_an_80_by_24_terminal`, `a_failure_too_long_for_the_pane_keeps_the_hedge`)
to the `ide` preset (`flowed_at`), hiding it.
Fix (local to the form): every row of the "— it may have been written…" clause always stays; only
the store's own message before it may be cut. The warning takes rows from the fixed rows, not only
the body's (the form already scrolls the focused field into view, `laid.offset(above.height)`):
`room = inner.height − (rows the focused field needs + 1)`; the body keeps at least 1 row when it has
focus. Then remove the `ide` pin so both tests run under the default layout.
Same pattern: `crates/htui/src/ui/tabs/backlog/detail/compose.rs:~668` (`may_have_landed` /
`could_not_check`) cuts its notice the same way: apply the same rule and pin it at 80x24 under the
default layout.

## M2 — the first retention sweep goes to a store still connecting

`crates/htui/src/app/update.rs:~107`. "connecting" is `Backend::Offline { since: None }`; the first
refresh tick (~1 s) often precedes the connection, so the sweep gets `RetentionSwept(0)` (or now a
failure) and the next is an hour later; with no worker and short sessions events are never pruned by
age. Fix: send the sweep and advance `sweep_due` only when `top_bar.store` is online or memory; the
first sweep goes on the first refresh after the store is up. Test it.

## M3 — keep `paint_panes`' `modal && zoomed` early return and test it

`crates/htui/src/app/state.rs:~827`. Reachable: `e` in the list, then `tab`, `tab`, `z` before the
item form's data returns (`BacklogTab::on_item_form`, `backlog/writes.rs:~134`); the reply opens the
form with a modal stack while the activity pane is zoomed and focused. Correct the T5b doc comments
in `tests/it/backlog.rs`/`requirements.rs` that call it unreachable. Add a test: a `SplitTab` with
stack `views::CAPTURE`, `IdeActivity` with `zoom: true`, focus on Activity → the panel's title is not
drawn (in `shell_tests`), or a harness test pressing `e` `tab` `tab` `z` without settling, then
`drive_to_end`.

## Lows

- **L1** `crates/htui/src/ui/shell/activity.rs:~247`: `render` builds every held row (~700) per frame.
  Compute the scroll window from the selection and divider positions first; build only visible lines.
- **L2** `crates/htui/src/app/update.rs:~719`: `merge(page.clone(), workspaces.clone())` copies the
  reply each second; take the reply by value.
- **L3** `update.rs:~584`: a sweep that always fails leaves no trace. `tracing::warn!` in
  `on_app_reply`'s `Failed { request: "sweep_retention" }` arm (keep `absorbed` side-effect free).
- **L4** (maintainer: yes) `activity.rs:~425`: rows from before today show `MM-DD HH:MM`, as the Runs
  table does; today's keep `HH:MM`. Update the panel's tests/snapshots.
- **L5** `tests/it/activity.rs` (`frame`) and `tests/it/waiting.rs` (`clockless`) blank every `HH:MM`
  in a frame, including unrelated times (the Runs table's `14:00`), weakening `platform_mixed`. Make
  the panel's UTC test zone available under `testkit` and have the harness set it; drop both helpers.
- **L6** Add shell-level tests for two D9 paths: Enter on another workspace's row while an editor is
  open (refused, status line says why, nothing moves) and the `<KEY> was deleted` status line.
- **L7** (maintainer: fix now) Close-out dialogs overlap the run rows on the shorter default panes and
  the row beneath shows through to the right (`╮00│` in `runs_closeout_typed`; `09`, `e/`, `ef` in
  `closeout_typed_80`). Clear the full rows under the dialog (or anchor it below the table).

## Snapshots that lost their subject (pin the `ide` preset in the test)

- Must: `backlog::the_flow_draws_the_plan_step_s_tool_call_as_a_chip` (`runs_flow_tool_chips`: the
  `implement` node is no longer drawn).
- Recommended, do it: `backlog::the_six_sub_tabs_render_the_selected_item` (`detail_body`,
  `detail_prompt`).
- Close-out snapshots (`runs_closeout_typed`, `closeout_naming`, `closeout_typed_80`): re-check after
  L7; pin only if the dialog still covers the rows its text refers to.

Run `cargo insta test -p htui --all-features` (full) and review every `.snap.new`; list each accepted
snapshot with one line on why it moved.

Files: `crates/htui/src/ui/tabs/backlog/item_form.rs`, `…/backlog/detail/compose.rs`,
`crates/htui/src/app/{state,update}.rs`, `crates/htui/src/ui/shell/activity.rs`, the close-out dialog's
render file, `crates/htui/src/testkit.rs` (L5's zone only), `crates/htui/tests/it/{activity,waiting,backlog,requirements}.rs`
and the snapshots that move.
Check: `env -u HTUI_TEST_QDRANT_URL cargo test -p htui --all-features --lib`; `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it)'`
(background, check at intervals under 4 minutes); `cargo insta test -p htui --all-features`; clippy both forms and fmt on `htui`;
`RUSTDOCFLAGS='-D warnings' cargo doc -p htui --no-deps --all-features`.

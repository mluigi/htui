# Brief R1-backlog — MOD-147 M3 review fix round, Backlog area

Source: rust-reviewer over `0dc19385..4cb58dbd` (plan `.claude/plans/mod-147-m3-tui-surfaces.plan.md`, D1–D6).
**Confirm each finding against the code before acting**; if one is wrong, say so in `rejected_findings` and skip it.
The Settings-area fix task ran before you on this branch; do not touch its files.

## Low 1 — a test that passes without its feature
`crates/htui/src/ui/tabs/backlog/tests/workspace.rs:~189` `a_scope_change_forgets_the_workspace_group` would pass
without the reset in `on_scope_change`: `items_reply(..)` carries `scope_row: None` and `clone_from` overwrites the
field anyway. Assert on `rows()` (or the rendered frame) right after `on_scope_change`, **before** any reply; check it
fails with the reset removed (then restore it).

## Low 4 — end-to-end WS create
No test submits the `N` form on the workspace header and sees `WS-1` listed under the workspace group. Add one in
`crates/htui/tests/it/backlog_forms.rs` (or `backlog.rs`), driving the real store worker over the MemStore demo:
`N` on `▾ Platform (0)`, fill the title, submit, then the Items reply lists `WS-1` under `▾ Platform (1)`.

## Low 5 — one lookup failure blanks the Backlog
`crates/htui/src/store_worker.rs:~2978`: if the scope-row lookup (`scope_project` / `project`) fails, the whole `Items`
read fails. Fall back to the members-only read with `scope_row: None` and a `tracing::warn!` naming the workspace and
the error. Test it with a backend whose scope-row lookup fails, if the test seams allow it cheaply. Otherwise
explain why not in the report.

## Low 7b — clones per key and frame
`group_projects` (`crates/htui/src/ui/tabs/backlog/mod.rs:~301`) and `workspace_projects`
(`crates/htui/src/ui/tabs/backlog/detail/graph.rs:~194`) clone every `ProjectRef` on each call. Return an iterator or
`Cow`/borrowed chain (e.g. `impl Iterator<Item = &ProjectRef>`, or build the `Vec` once when the reply or scope
changes) where the callers allow it without contortions; keep behaviour identical.

## Files owned
`crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/backlog/{mod.rs,navigation.rs,list.rs,detail/graph.rs}`,
`crates/htui/src/ui/tabs/backlog/tests/workspace.rs`, `crates/htui/tests/it/{backlog.rs,backlog_forms.rs}`.

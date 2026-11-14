# Brief T5b — MOD-122 M3: the five MOD-115 tests that open a form from the focused activity pane

T5 (`aa34317f`, `7a8980c0`) wired the activity panel as D10/F8 of the M3 plan specify: while
`App::activity_has_keys()` is true (no overlay, the activity pane focused and shown, no modal global
layer on the tab's stack), the shell's `ACTIVITY_BROWSE` step in `App::dispatch_key` consumes every
key, so the tab never sees it. That decision stands (accepted plan; do not change the routing).

Five integration tests in `crates/htui/tests/it/` now fail, because they press a tab key (`f`, `a`,
`/`) with the activity pane focused, a path that no longer opens the tab's form:

- `backlog::the_filter_opened_with_the_activity_zoomed_is_on_screen`
- `backlog::the_list_is_drawn_above_the_filter_opened_with_the_activity_zoomed`
- `backlog_docs::the_compose_rule_follows_the_detail_pane_while_the_activity_pane_has_the_focus`
- `requirements::a_form_opened_with_the_activity_zoomed_is_on_screen`
- `requirements::the_tree_is_drawn_above_the_filter_opened_with_the_activity_zoomed`

They pin a MOD-115 invariant (the T7 repair in `App::paint_panes`: a form opened while the activity
pane is zoomed or focused is drawn on screen, never hidden under it; the compose rule follows the
detail pane). Keep that invariant tested:

1. For each test, read it and the code path it pins (`paint_panes`' early return, `claimed_region`,
   `hides`). Reach the same screen state through a path that still exists with the new routing:
   e.g. open the form first from a tab pane, then move the focus to (or zoom) the activity pane by
   whatever key or mouse path the bindings allow (`global.next_pane`/`prev_pane`, a click, the layout
   zoom key; every key from the key catalogue / `keys.toml`, none hardcoded). If the zoomed state is
   now unreachable with a form open, pin the nearest reachable state that still exercises the same
   draw branch, and say so in the test's doc comment.
2. Add one test that pins the new rule itself where the old tests stood: with the activity pane
   focused, the tab's key (`f` in the Backlog) is the panel's (`activity.filter` cycles the filter)
   and the tab's form does not open.
3. Rename a test only when its name no longer says what it checks; update snapshots only where the
   screen genuinely differs, review each `.snap.new` and list them.

Files: the three test files above (`crates/htui/tests/it/backlog.rs`, `backlog_docs.rs`,
`requirements.rs`) and their snapshots under `crates/htui/tests/it/snapshots/`. Product code only
if a test reveals a real defect in T5's wiring; then say so as a deviation.

Check: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^backlog::/) or test(/^backlog_docs::/) or test(/^requirements::/) or test(/^activity::/))'`,
then clippy (`--all-targets --all-features` and featureless) and fmt on `htui`.

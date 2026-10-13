# MOD-67 M5 blueprint — Lane L-B: the Runs pane

Scope: `crates/htui/src/ui/tabs/backlog/detail/runs.rs` (`runs/execution_graph.rs` builds mouse
events and matches no key: untouched). Owned tests: `tests/backlog_keys/l_b.rs`, the in-file tests
of `runs.rs`; `tests/runs_pg.rs` unchanged. Snapshots: `backlog__detail_runs`,
`backlog__empty_runs`, `backlog__runs_*` (10) and, flagged (PA-6), `replay__runs_step_selected`.
Branch `mod-67-m5-l-b`, worktree `target/wt/l-b`.

Lane rules (plan, M3/M4): never edit `keys/`, `app/`, the registries, `testkit.rs`,
`tests/keys*.rs`, `backlog/{mod,filter,item_form,divergence,list}.rs`, `backlog/detail/mod.rs`, the
other panes, `chat/**` (`chat/permission.rs`'s `PermissionStrip` is used, not edited). A missing
act or stack goes back to the main thread. Format only own paths. Gate with
`env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit incrementally; stage own paths only.

Complete: sections 0-9.

## 0. Facts (read at `3af293a6`)

- `RunsTab::on_key` (`runs.rs:1515-1594`): a non-`Browse` mode → `modal_key` (`:1516-1518`);
  CONTROL/ALT → `Pass` (`:1519-1524`); a digit `1`-`9` with a pending request under the cursor →
  `AnswerPermission`, every digit consumed (`:1528-1544`, MOD-42 D14); then `J`/`K`, `Enter`, `v`,
  `+`/`-`/`=` (flow only), `PgUp`/`PgDn` (flow: no-op), the thirteen action letters → `action(char)`
  (`:1580-1592`), `_ → scroll.on_key` (`:1593`, i.e. `PgUp`/`PgDn` in the list).
- `modal_key` (`:631-680`): CONTROL → `Pass` (`:632`, **ALT is not passed**: `alt-y` confirms a
  cancel, D7); `ConfirmCancel` `y`/`n`/`Esc` (`:639-646`); `CloseOut` → `close_out_key`
  (`:647-650`); `Artifact` maps `j`/`k` to `J`/`K`, `Esc` closes, `Scroll::on_key` scrolls
  (`:651-678`). Every key is consumed (`:679`).
- `reject_key` (`:900-928`), `follow_up_key` (`:930-949`): a `TextField`; `Submit`/`Cancel` are
  the field's; `Pass` and `Consumed` keep the mode (so ALT and `F1` are swallowed today).
- `close_out_key` (`:980-1023`): `Counting` leaves on `n`/`Esc` (`:982-985`); `Warn` `y` →
  `Typed`, `n`/`Esc` leave, `Right`/`l` and `Left`/`h` cycle the resolution (`:986-1003`); `Typed`
  is a field (`:1004-1019`); `InFlight` swallows (`:1020`).
- The browse view has **no hint row** today (`footer`, `:1917-1919`: `Browse | Artifact =>
  Vec::new()`); `render_artifact` draws `read-only · j/k scroll · Esc close` (`:883-893`).
- With the Backlog tab unconverted on this branch, the pane receives every key the tab does not
  intercept (`j k g G h l [ ] arrows Home End m f F N e Q P Enter` are the tab's), every chord and
  `Enter` on an item row; the legacy `Enter` miss row still answers a `Pass`.
- T1 provides `RUNS_BROWSE`, `RUNS_CONFIRM`, `RUNS_COUNTING`, `RUNS_ARTIFACT`, `CAPTURE`, the
  nineteen `Runs*` acts, the `BacklogRuns` `PaneScrollDown`/`Up` view defaults (`j`/`k`), and
  `DetailTab::key_stack`.

## 1. Modes and stacks

| Mode (`enum Mode`, `:283`) | Handler today | captures | Stack |
|---|---|---|---|
| `Browse` (list and flow) | `on_key` `:1515` | no | `RUNS_BROWSE` |
| `RejectNote` | `reject_key` `:900` | yes | `CAPTURE` |
| `FollowUp` | `follow_up_key` `:930` | yes | `CAPTURE` |
| `ConfirmCancel` | `modal_key` `:639` | yes | `RUNS_CONFIRM` |
| `CloseOut(Counting)` | `close_out_key` `:982` | yes | `RUNS_COUNTING` |
| `CloseOut(Warn)` | `:986` | yes | `RUNS_CONFIRM` (+ the fixed resolution keys) |
| `CloseOut(Typed)` | `:1004` | yes | `CAPTURE` |
| `CloseOut(InFlight)` | `:1020` | yes | `CAPTURE` |
| `Artifact` | `modal_key` `:651` | yes | `RUNS_ARTIFACT` |

```rust
/// The current mode's stack (MOD-67 M5 D4): the only place a mode maps to its keys; `on_key`,
/// `footer` and the Backlog tab read it.
fn stack(&self) -> Stack<'static> {
    match &self.mode {
        Mode::Browse => views::RUNS_BROWSE,
        Mode::RejectNote { .. }
        | Mode::FollowUp { .. }
        | Mode::CloseOut(CloseOutStage::Typed { .. } | CloseOutStage::InFlight) => views::CAPTURE,
        Mode::ConfirmCancel { .. } | Mode::CloseOut(CloseOutStage::Warn(_)) => views::RUNS_CONFIRM,
        Mode::CloseOut(CloseOutStage::Counting) => views::RUNS_COUNTING,
        Mode::Artifact { .. } => views::RUNS_ARTIFACT,
    }
}
// impl DetailTab for RunsTab
fn key_stack(&self) -> Stack<'static> { self.stack() }
```

## 2. Arm-by-arm conversion

### 2.1 Browse (`on_key`, main blueprint §6.2)

| file:line | today | → act (`RUNS_BROWSE`) | notes |
|---|---|---|---|
| `:1516-1518` | non-browse → `modal_key` | unchanged | §2.2 |
| `:1519-1524` | CONTROL/ALT → `Pass` | **removed** (chord equality) | D7 |
| `:1528-1544` | digits with a pending request | **fixed, first**, plain keys only (no CONTROL/ALT, as the guard made it) | D5 |
| `:1546-1549` | `J` → cursor +1, `sync_graph` | `RunsNextStep` | ANA §6.2 |
| `:1550-1553` | `K` | `RunsPrevStep` | |
| `:1554-1559` | `Enter` → `Action::Replay`, no step → `Pass` | `BacklogReplay`: a step → emit `Replay`, `Consumed`; none → `continue` (declines: the tab answers with the miss) | D6; the pane is the one that accepts a `backlog` act |
| `:1560-1574` | `v` → toggle list/flow | `RunsToggleView` | body unchanged |
| `:1575` | `+` (flow) → zoom in | `RunsZoomIn` if `View::Flow`; in the list `continue` (today it fell to `scroll.on_key` → `Pass`) | |
| `:1576` | `-` (flow) | `RunsZoomOut`, same rule | |
| `:1577` | `=` (flow) | `RunsZoomFit`, same rule | |
| `:1579` | `PgUp`/`PgDn` in the flow → no-op | `PanePageUp`/`PanePageDown` in the flow → `Consumed` | MOD-28 D8 |
| `:1593` | `_ → scroll.on_key` (list `PgUp`/`PgDn`) | `PanePageUp`/`PanePageDown` in the list → `return self.scroll.apply(act, self.runs.len())` | `Scroll::on_key` loses this caller |
| `:1580-1592` | `a x r p c o s u i A R C T` → `action(char)` (+ `end_gesture` when it captures) | `RunsApprove`, `RunsReject`, `RunsRetry`, `RunsPromote`, `RunsCancelRun`, `RunsOpen`, `RunsSelect`, `RunsUnblock`, `RunsFollowUp`, `RunsAccept`, `RunsStartRun`, `RunsCloseOut`, `RunsCleanup` → `self.action(act, ctx)` (+ the same `end_gesture`) | |
| — | (anything else) | `_ => continue`; loop end `Handled::Pass` | `list.*`, `pane.*_subtab`, `backlog.*`, `common.edit` are the tab's |

`action(&mut self, act: Act, ctx)` (`:702-826`) keeps its body: `key == 'i'` → `act ==
Act::RunsFollowUp`; `'u'`, `'R'`, `'C'` → `RunsUnblock`, `RunsStartRun`, `RunsCloseOut`; `'c' |
'T'` → `RunsCancelRun | RunsCleanup` (`if key == 'c'` → `if act == Act::RunsCancelRun`); the step
arm `'a' | 'x' | 'r' | 'p' | 'o' | 's' | 'A'` → `RunsApprove | RunsReject | RunsRetry |
RunsPromote | RunsOpen | RunsSelect | RunsAccept`, its inner `match key` on the acts; `_ =>
return Handled::Pass`. Every refusal and request unchanged. Its doc names the acts.

### 2.2 Modal modes (`modal_key`, main blueprint §6.3/§6.4)

`modal_key` keeps "the mode it leaves the pane in" but also answers `Handled`: the free functions
`reject_key`, `follow_up_key`, `close_out_key` return `(Mode, Handled)` (or take `&mut Mode` and
return `Handled`; the implementer's call), with `Handled::Pass` exactly when the mode did not use
the key and `stack.passes(chord)` (`modal_rest`).

| file:line | today | → | notes |
|---|---|---|---|
| `:632-634` | CONTROL → `Pass` (ALT swallowed or answered) | **removed**; each mode ends in `modal_rest(self.stack(), chord)` | D7: `alt-y` no longer cancels; ALT and `F1` pass |
| `:639-643` | `ConfirmCancel` `y` → `CancelRun`, browse | `ConfirmYes` (`RUNS_CONFIRM`) | |
| `:644` | `n`/`Esc` → browse | `ConfirmNo` | |
| `:645` | `_` → keep asking | keep; `modal_rest` | |
| `:982-985` | `Counting` `n`/`Esc` → browse, else wait | `ConfirmNo` (`RUNS_COUNTING`) → browse; else keep, `modal_rest` | `y` is no act here (as today) |
| `:987-990` | `Warn` `y` → `Typed` | `ConfirmYes` (`RUNS_CONFIRM`) | |
| `:991` | `Warn` `n`/`Esc` → browse | `ConfirmNo` | |
| `:992-1001` | `Warn` `Right`/`l`, `Left`/`h` → cycle the resolution | **fixed** value keys, plain only, before the resolver | main §1 (borderline keys) |
| `:1002` | `Warn` `_` → keep | keep; `modal_rest` | |
| `:1004-1019` | `Typed`: field; `Submit` matches the key → `CloseOut`, else `NOT_THE_KEY`; `Cancel` → browse | widget-owned, unchanged; field `Pass` → keep, `modal_rest(CAPTURE)` | |
| `:1020` | `InFlight` swallows | keep; `modal_rest(CAPTURE)` (CONTROL/ALT/F pass) | |
| `:900-928` | `RejectNote` field | widget-owned, unchanged; `Pass` → keep, `modal_rest(CAPTURE)` | `Tab`, digits, `q` stay swallowed (CAPTURE passes none of them) |
| `:930-949` | `FollowUp` field | same | |
| `:657-668` | `Artifact` `j`/`k` → `J`/`K`; `Scroll::on_key` | `PaneScrollDown`/`Up` (`J`/`j`, `K`/`k` by `VIEW_DEFAULTS`), `PanePageDown`/`Up` → `scroll.apply(act, len)` (`len` as today, `artifact_rows(doc, width).len()`) | `Down`/`Up` stay inert, as today |
| `:662-663` | `Artifact` `Esc` → browse | `Back` (`RUNS_ARTIFACT`) | |
| `:679` | everything consumed | each mode's `modal_rest` | |
| `:87` | `use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, …}` | `KeyCode` stays (digits, resolution keys); `KeyModifiers` stays for `plain` and the mouse code | |

## 3. Hints (the new browse row and 8 literals → `HintSpec`s)

`footer(&self, width, keys: &Keys, theme)` (`:1917`) and `render_artifact(…, keys: &Keys, theme)`
(`:864`); `render` passes `ctx.keys()`. Each spec renders through `self.stack()`.

| file:line | today's text | `HintSpec` | rendered with defaults | differs? |
|---|---|---|---|---|
| `:1919` (new) browse, list view, runs present | — | `RUNS_LIST_HINT`: `Pair(RunsNextStep, RunsPrevStep, "step")`, `One(BacklogReplay, "replay")`, `One(RunsToggleView, "flow")`, `One(Help, "keys")` | `J/K step · Enter replay · v flow · ? keys` (41 in 43) | **new row** (ANA §8) |
| `:1919` (new) browse, flow view | — | `RUNS_FLOW_HINT`: `Pair(RunsNextStep, RunsPrevStep, "step")`, `Pair(RunsZoomIn, RunsZoomOut, "zoom")`, `One(RunsToggleView, "list")`, `One(Help, "keys")` | `J/K step · +/- zoom · v list · ? keys` (37) | **new row** |
| `:1919` (new) browse, no runs | — | `RUNS_EMPTY_HINT`: `One(RunsStartRun, "start run")`, `One(Help, "keys")` | `R start run · ? keys` | **new row** |
| `:1924` | `Enter reject · Esc cancel` | `Text("Enter reject")`, `Text("Esc cancel")` | identical | no |
| `:1929` | `Enter send · Esc cancel` | `Text("Enter send")`, `Text("Esc cancel")` | identical | no |
| `:1933` | `y cancel it · n keep it` | `One(ConfirmYes, "cancel it")`, `One(ConfirmNo, "keep it")` | identical | no |
| `:1936` | `counting what the close-out writes…` | `Text(…)` | identical | no |
| `:1950` | `←/→ resolution · y continue · n cancel` | `Text("←/→ resolution")`, `One(ConfirmYes, "continue")`, `One(ConfirmNo, "cancel")` | identical | no |
| `:1958` | `Enter close · Esc cancel` | `Text`, `Text` | identical | no |
| `:1960` | `closing…` | `Text` | identical | no |
| `:891` | `read-only · j/k scroll · Esc close` | `Text("read-only")`, `Pair(PaneScrollDown, PaneScrollUp, "scroll")`, `One(Back, "close")` | `read-only · J/K scroll · Esc close` | **yes**: `j/k` → `J/K` (main §1 item 5) |

The browse row is the footer's last line in `Mode::Browse` with an item selected (the "No item
selected." branch draws no footer, as today); `render` already sizes the footer from its lines, so
the list (or the flow canvas) loses exactly one row. The help label `? keys` is `global.help`'s
first chord through `RUNS_BROWSE`.

Every user-visible difference: (1) the new browse row (three variants); (2) the document view's
`J/K scroll`. Nothing else. Prose that names keys stays (M6): the module doc's key table
(`:15-31`), `:686`'s doc, `NOTE_NEEDED`, `NOT_THE_KEY`.

## 4. D7 in L-B

`modal_key`'s CONTROL-only pass (`:632`) goes: every modal mode ends in `modal_rest`, so `alt-y`
in the cancel question and the close-out warning resolves to nothing and passes to the shell (no
act), and the question stays. The browse CONTROL/ALT guard (`:1519-1524`) goes too: `ctrl-J` is
no act. Pinned (§6).

## 5. In-file tests that change (`runs.rs`)

- `footer` helper (`:3439-3441`): `pane.footer(43, Keys::compiled(), &Theme::default())`. Tests
  that read `footer(&pane)` in browse (expecting an empty footer, if any) now see the browse row;
  the modal-mode assertions (`:3468`, `:3504`, `:3568`, `:3587`, `:3595`, `:3638`, `:3813-3819`,
  `:3840`, `:3853`, `:4262`, `:5873`) keep their text (all literals above are identical).
- `assert_refused` (`:3095`) and `allowed_emit` (`:3109`) send `KeyEvent`s: unchanged (default
  chords). A direct `action('a', …)` call, if any, becomes `action(Act::RunsApprove, …)`.
- Any test asserting that an ALT chord is swallowed in a modal mode now expects `Pass` (D7).
- `the_step_cursor_moves_and_clamps_at_both_ends`, `enter_emits_a_replay_of_the_step_under_the_
  cursor`, `a_pane_with_no_step_passes_enter_on` (`:2054-2118`): green unchanged.

## 6. Tests (L-B; write first, red, then code)

`tests/backlog_keys/l_b.rs` (App `Harness`, demo, Backlog → Runs on `FEAT-1` or the parked step
the existing Runs tests use):

| Test | Asserts |
|---|---|
| `alt_y_does_not_cancel_a_run` (D7, D12) | `c` opens `cancel this run?`; `alt-y` sends no `CancelRun` and the question stays; `y` sends one |
| `alt_y_does_not_continue_a_close_out` (D7) | `C`, counts in; `alt-y` stays on the warning; `y` moves to the typed stage |
| `the_resolution_keys_stay_fixed_and_plain` | at the warning `l`/`Right` cycle the resolution, `alt-l` does not |
| `f1_opens_help_from_the_reject_note_and_the_note_stays` (§1 item 3) | `x` on a parked step, type `no`, `f1`: `help_visible`; `f1` closes it; the note still reads `no`; `tab` and `2` are swallowed (still on Backlog, note unchanged). The box's content follows the pane's stack only on the merged tree (main §7 step 2) |
| `the_runs_browse_hint_row` (ANA §8) | list: the pane's last row is `J/K step · Enter replay · v flow · ? keys`; after `v`: `J/K step · +/- zoom · v list · ? keys`; an item with no runs: `R start run · ? keys` |
| `a_rebound_next_step_moves_the_cursor_and_j_capital_is_inert` (the lane's rebinding test) | `[backlog.runs]\nnext_step = "n"\n`: `n` moves the step cursor (the `▸` row moves), `J` leaves it; the browse row reads `n/K step · Enter replay · v flow · ? keys`. `n` passes the unconverted Backlog tab (it intercepts no `n`) and the converted one (no `backlog`/`list`/`pane` act binds it) |
| `enter_replays_the_step_and_misses_without_one` (D6) | a step under the cursor: `Enter` emits `Replay` (the Chat tab opens); no step: the miss text (legacy row here, the tab's act after L-A: same text) |

`tests/replay.rs`, `tests/backlog.rs`: no assertion changes expected (the Runs tests drive keys,
not hint rows). If one breaks, edit it flagged.

## 7. Snapshots (L-B's 12 and one app-level)

On this branch the Backlog tab reports no stack, so the status line stays `Stack::BASE`'s: the
capturing modes' status rows move only at the T-close merge (main §7 step 1, X2).

| Snapshot | Rows that move on this branch | Expected |
|---|---|---|
| `backlog__detail_runs` | the pane's last row | `J/K step · Enter replay · v flow · ? keys` |
| `backlog__empty_runs` | the pane's last row | `R start run · ? keys` |
| `backlog__runs_flow_fanout`, `backlog__runs_flow_tool_chips` | last row; the canvas is one row shorter (rataflow may lay the run out differently: review cell by cell) | `J/K step · +/- zoom · v list · ? keys` |
| `backlog__runs_follow_up_queued`, `backlog__runs_pending_permission` | last row (list or flow variant, whichever the frame is in); a list that filled the pane scrolls up one row | the browse row |
| `backlog__runs_artifact` | the document view's hint | `read-only · J/K scroll · Esc close` |
| `backlog__runs_reject_note`, `backlog__runs_flow_reject_note`, `backlog__runs_follow_up_input`, `backlog__runs_closeout_typed`, `backlog__runs_closeout_warn` | none (status row moves at X2 to `Ctrl+c quit · F1 help`) | byte-identical here |
| `replay__runs_step_selected` (app-level, PA-6, flagged) | the Runs pane's last row | `J/K step · Enter replay · v flow · ? keys` |

At X2 (T-close) the six capturing snapshots (the five above plus `runs_artifact`) take
`Ctrl+c quit · F1 help` on their status row and nothing else.

## 8. Cross-lane couplings

- The miss is the legacy row on this branch, `BacklogReplay` after L-A; same text (L-A pins it).
- The browse row's `Enter replay` is `backlog.replay`'s label through `RUNS_BROWSE`; a
  `[backlog] replay` or `[backlog.runs] replay` override renames it (T1's stack, no L-A code).
- `tests/backlog.rs`/`tests/replay.rs` shared: flagged edits only.
- `Scroll::on_key` loses its Runs callers (browse, artifact); T-close deletes it.

## 9. Work order, commits and gate (serial inside the lane; modal modes first)

1. **C1** `feat(mod-67): Runs questions on their stacks; alt-y no longer answers (D7)` —
   `stack`/`key_stack`, `ConfirmCancel`, `CloseOut(Counting|Warn)`, the resolution keys fixed,
   their footers; `alt_y_does_not_cancel_a_run`, `alt_y_does_not_continue_a_close_out`,
   `the_resolution_keys_stay_fixed_and_plain`.
2. **C2** `feat(mod-67): Runs text fields and the document view on their stacks` — `RejectNote`,
   `FollowUp`, `Typed`, `InFlight`, `Artifact`, their footers, `render_artifact`'s spec;
   `f1_opens_help_from_the_reject_note…`; accept `runs_artifact`.
3. **C3** `feat(mod-67): Runs browse resolves its acts; the browse hint row (D6, ANA §8)` — §2.1,
   `action(act)`, the three browse specs; `the_runs_browse_hint_row`,
   `enter_replays_the_step_and_misses_without_one`; accept the six browse snapshots and
   `replay__runs_step_selected` (flagged in the body).
4. **C4** `test(mod-67): L-B rebinding test` — `a_rebound_next_step_…`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/backlog/detail/runs.rs \
  crates/htui/tests/backlog_keys/l_b.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::backlog::detail::runs
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features \
  --test backlog --test backlog_keys --test replay --test chat --test waiting -- --test-threads=1
cargo insta test -p htui --all-features --check
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
```
(`tests/chat.rs` drives the Runs pane's `p`/`a` through promotion; `tests/waiting.rs` reveals a
step into the Runs pane. Memory: grep the full run for `SIGABRT`.)

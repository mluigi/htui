# MOD-67 M6 blueprint — Lane L-A: the queue overlay

Scope: `crates/htui/src/ui/overlay/queue.rs` (production and its in-file tests). Owned tests:
`tests/queue_overlay.rs`, the in-file tests of `queue.rs`. Snapshots: `queue_overlay__opened`
(`tests/snapshots`) and the eight `htui__ui__overlay__queue__tests__*` (`src/ui/overlay/snapshots`)
— **none may move** (the hint renders byte-identical). Branch `mod-67-m6-l-a`, worktree
`target/wt/l-a`.

Lane rules (plan, M3-M5): never edit `keys/`, `app/`, the registries, `testkit.rs`,
`tests/keys*.rs`, `ui/tabs/backlog/**` (`queue_sentence` is T-close X-S, main blueprint §7), any
other overlay. A missing act or stack goes back to the main thread. Format only own paths. Gate
with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit incrementally; stage own
paths only.

Complete: sections 0-7.

## 0. Facts (read at `59ff7d02`, after T0/T1 as the blueprint specifies them)

- `QueueOverlay::on_key` (`queue.rs:437-480`) matches `key.code` alone, no modifier check:
  `j`/`Down` cursor +1, `k`/`Up` cursor −1 (both clamp, both a no-op on an empty or unread list but
  still `Consumed`), `J`/`K` `request_move(Down/Up)`, `P` pause/resume by the shown batch
  (`:455-466`, M3 review R1 M1), `Q` dequeue the cursor row, `Enter` `self.enter(ctx)` (close, then
  reveal); everything else `Pass` (`Esc` → the shell's `overlay.close`, `q` and the rest → the
  modal swallow). So `ctrl-j` moves, `alt-P` pauses, `ctrl-Q` dequeues today.
- No `key_stack`: the shell resolves the overlay's passed keys through `Stack::OVERLAY` and the
  `?` box shows the legacy overlay box (`app/state.rs` `render_help`, `None` arm).
- `HINT` (`:44`) is a literal: `j/k move · J/K reorder · P pause/resume · Q dequeue · Enter open ·
  Esc close`, drawn by `lines` (`:288`) clipped to the box.
- T0 gives the test `Bench` a `keys: Keys` field in place of `keymap` (main blueprint §3).
- T1 provides `Context::Queue`, `Act::{QueueMoveDown, QueueMoveUp, QueuePause, QueueDequeue,
  QueueOpen}`, `views::QUEUE_OVERLAY` = `view(Queue, [those five])`, `LIST`, `OVERLAY`, `HELP`, and
  `views.rs` tests with `QUEUE_OVERLAY` and the five acts on `PENDING`/`PENDING_ACTS` (T-close
  empties them; L-A never edits `views.rs`).

## 1. Stack and dispatch

```rust
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};

/// The line under the list (MOD-67 M6 D2), through [`views::QUEUE_OVERLAY`]: `list.*`, the
/// `queue` verbs and `overlay.close` (the shell's). `j/k move · J/K reorder · P pause/resume ·
/// Q dequeue · Enter open · Esc close` under the defaults.
const HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::Pair(Act::QueueMoveDown, Act::QueueMoveUp, "reorder"),
    Hint::One(Act::QueuePause, "pause/resume"),
    Hint::One(Act::QueueDequeue, "dequeue"),
    Hint::One(Act::QueueOpen, "open"),
    Hint::One(Act::OverlayClose, "close"),
];

impl Overlay for QueueOverlay {
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::QUEUE_OVERLAY)
    }

    /// Each candidate of the chord through `views::QUEUE_OVERLAY` (MOD-67 M6 D2), the first this
    /// overlay takes winning; chord equality includes the modifiers, so `ctrl-j` is not `j`.
    /// `overlay.close` and `global.help` are the shell's (`Pass`); the rest is swallowed by
    /// `is_modal`, so nothing quits or reaches the tab below.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let len = self.rows().len();
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::QUEUE_OVERLAY, chord) {
            match act {
                Act::ListDown => { /* today's `j` body, :441-444 */ }
                Act::ListUp => { /* today's `k` body, :446-449 */ }
                Act::QueueMoveDown => self.request_move(QueueMove::Down, ctx),
                Act::QueueMoveUp => self.request_move(QueueMove::Up, ctx),
                Act::QueuePause => { /* today's `P` body, :455-466, comment kept */ }
                Act::QueueDequeue => { /* today's `Q` body, :467-473 */ }
                Act::QueueOpen => self.enter(ctx),
                _ => continue, // `overlay.close`, `global.help`: the shell's
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }
}
```
`use crossterm::event::{KeyCode, KeyEvent}` → `KeyEvent` only (production); tests keep
`KeyCode`. `lines` takes the rendered hint: `fn lines(&self, room, text_width, visible, hint:
&str, theme)`; `render` computes `ctx.keys().hint(views::QUEUE_OVERLAY, HINT)` once and passes it
to both `lines` calls (the natural size and the clipped one, as `waiting_list.rs:322-323` does).
Module doc (`:1-8`) gains: "Its keys resolve through `views::QUEUE_OVERLAY` (MOD-67 M6)." The doc
of the old `HINT` (`:43`, "the rest are handled here") is replaced by the one above; the comment at
`:475-476` goes with the arm.

## 2. Arm-by-arm

| file:line | today | → act (`QUEUE_OVERLAY`) | notes |
|---|---|---|---|
| `:440` | `j`/`Down` | `ListDown` | `ctrl-j`, `alt-j` now pass (then swallowed: modal) |
| `:445` | `k`/`Up` | `ListUp` | |
| `:450` | `J` | `QueueMoveDown` | |
| `:451` | `K` | `QueueMoveUp` | |
| `:455-466` | `P` | `QueuePause` | the `expect`/`expect_paused` rule unchanged |
| `:467-473` | `Q` | `QueueDequeue` | |
| `:474` | `Enter` | `QueueOpen` | |
| `:475-477` | `_ → Pass` | loop end `Pass` | `Esc` → `overlay.close` (shell), `q` swallowed (modal) |

## 3. What the user sees

- Byte-identical hint and frames with the defaults; the nine snapshots do not move.
- Modifier-blindness fixed (main blueprint §1 item 1).
- The status line is unchanged (`QUEUE_OVERLAY`'s global layer is `HELP`, exactly
  `Stack::OVERLAY`'s): `Ctrl+c quit · ? help` as today.
- The `?` box (now from the stack): `Queue: J move down · K move up · P pause / resume · Q dequeue
  · Enter open`, `List: j/Down down · k/Up up`, `Overlay: Esc close`, `Global: Ctrl+c quit · ?/F1
  help`, then the closer (exact text from the run).

## 4. Tests (write red first)

In-file (`queue.rs`):

| Test | Asserts |
|---|---|
| `the_overlay_reports_the_queue_stack` (new) | `QueueOverlay::new().key_stack() == Some(views::QUEUE_OVERLAY)` |
| `a_chord_is_no_queue_key` (new) | fed a three-row overview: `ctrl-j`, `alt-j`, `ctrl-J`, `alt-P`, `ctrl-q` (CONTROL `q`), `alt-enter` each return `Pass` and emit nothing; the cursor stays on row 0 (fails today: `ctrl-j` moves, `alt-P` pauses) |
| `a_rebound_key_moves_the_entry_and_the_old_one_is_inert` (new) | bench with `load_str("[queue]\nmove_down = \"D\"\n")`: `D` emits `MoveQueueEntry { to: Down }`, `J` returns `Pass` and emits nothing; the rendered box shows `D/K reorder` |
| `the_hint_is_the_rendered_spec` (new) | `Keys::compiled().hint(views::QUEUE_OVERLAY, HINT)` == the old literal, byte for byte |
| existing (stay green unchanged): `capital_j_and_k_request_a_move_of_the_cursor_row`, `p_pauses_a_running_queue_and_resumes_a_paused_one`, `capital_q_dequeues_the_cursor_row`, `keys_on_an_empty_or_unread_list_do_nothing`, `enter_closes_then_reveals_the_run_or_the_item`, `the_cursor_follows_its_item_across_a_reorder`, `the_row_window_keeps_the_cursor_drawn`, `esc_and_lower_q_pass`, every `header_*`/`states`/`narrow`/`reading`/`offline` snapshot | `Bench::code` feeds `KeyEvent::from(code)` (no modifiers), so every existing arm resolves to its act |

The `Bench` gains `fn with_keys(mut self, keys: Keys) -> Self` (T0 already gave it `keys: Keys`).

Integration (`tests/queue_overlay.rs`, `#![cfg(feature = "testkit")]`):

| Test | Asserts |
|---|---|
| `a_rebound_reorder_key_round_trips_through_the_store` (new) | `Harness::over(store).with_keys(load_str("[queue]\nmove_down = \"D\"\nmove_up = \"U\"\n"))`, overlay open on two queued items: `D` moves the first down (store order changes, as `a_reorder_round_trips_through_the_store` reads it), `J` changes nothing, the frame shows `D/U reorder` |
| `the_help_box_lists_the_queue_keys` (new) | open, `?`: the box has a `Queue:` line starting `J move down`, a `List:` line and `Overlay: Esc close`; `?` again closes it |
| `a_ctrl_chord_does_not_steer_the_queue` (new) | `ctrl-j` then `ctrl-P` (CONTROL + `P`): the cursor and the store's paused flag are unchanged |
| existing six (`ctrl_q_opens_…`, `a_reorder_round_trips_…`, `p_resumes_and_pauses_…`, `capital_q_dequeues_…`, `the_overlay_re_reads_…`, `p_over_a_batch_the_runner_closed_…`) | unchanged, green (the last one asserts `before P reached it`, which X-S keeps byte-identical) |

## 5. Snapshots

None move. The lane's `cargo insta test --check` must report zero pending; a moved queue
snapshot is a defect in this lane (the spec is byte-identical with the defaults).

## 6. Commits

1. `test(mod-67): M6 L-A queue overlay pins` — the new tests above, red (they compile: T1's acts
   and stack exist; they fail on behaviour).
2. `feat(mod-67): M6 the queue overlay dispatches through its stack` — §1-§2, green.

## 7. Gate

```bash
rustfmt --edition 2024 --check crates/htui/src/ui/overlay/queue.rs crates/htui/tests/queue_overlay.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::overlay::queue
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test queue_overlay -- --test-threads=1
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui --all-features --check
```
`views.rs`' `every_view_stack_is_named_by_a_view` and the D7 test stay green on this branch with
`PENDING`/`PENDING_ACTS` (T1's); they name the stack and acts after the merge, when T-close empties
the lists.

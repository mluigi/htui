# MOD-67 M5 blueprint — Lane L-D: Chat

Scope: `crates/htui/src/ui/tabs/chat/{mod,transcript,composer}.rs` (`chat/permission.rs` matches
no key: untouched). Owned tests: `crates/htui/tests/chat.rs`, the in-file tests of those files;
`tests/chat_offline.rs`, `chat_live*.rs` unchanged. Snapshots: `chat__*` (15). Branch
`mod-67-m5-l-d`, worktree `target/wt/l-d`.

Lane rules (plan, M3/M4): never edit `keys/`, `app/`, the registries, `testkit.rs`,
`tests/keys*.rs`, `ui/tabs/backlog/**`. A missing act or stack goes back to the main thread.
Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit
incrementally; stage own paths only.

Complete: sections 0-9.

## 0. Facts (read at `3af293a6`)

- `ChatTab::on_key` (`chat/mod.rs:455-529`), in order: the refusal is cleared (`:458`);
  CONTROL/ALT → `Pass` (`:460-466`); an open replay → `on_key_replay` (`:471-473`); the composer
  (`:476-485`: every key while active, `Pass` while closed); a digit with a parked request →
  `answer`, every digit consumed (`:489-496`); `i`/`Enter` open the composer (`:498-501`); `a`
  cycles the agent with no session and agents known (`:502-505`, else falls to the transcript);
  `Esc` arms, then cancels (`:506-521`); `_ → transcript.on_key` (`:522`); any key but `Esc` that
  got this far disarms (`:524-526`; a chord returned at `:466` and never disarmed).
- `on_key_replay` (`:232-249`): `Esc` closes the replay (`:233-239`); `1`-`9`, `i`, `a`, `Enter`
  are swallowed (`:242`); the rest goes to the transcript (`j`/`Down`, `k`/`Up`, `g`, `G`, `t`),
  else `Pass` (so `q`, `Tab`, `w`, `?` reach the shell).
- `Composer::on_key` (`composer.rs:81-107`): inactive → `Pass`; active → `Char` typed,
  `Backspace`, `Enter` submits (empty: consumed), `Esc` leaves, `_ → Consumed` (so `F1`, `Tab`,
  arrows are swallowed; chords never reach it, `:460-466`).
- `Transcript::on_key` (`transcript.rs:460-500`): `j`/`Down`, `k`/`Up`, `g`, `G`, `t`; else
  `Pass`. No `Home`/`End`.
- Hints: `REPLAY_HINT` (`:64`), `hint()` (`:408-432`, five literals and a refusal), the replay
  header's ` · read-only · Esc leave` (`:259`).
- T1 provides `CHAT_LIVE`, `CHAT_REPLAY`, `CAPTURE`, the six `Chat*` acts. `ChatTab::key_stack`
  is this lane's (no other lane reads it).

## 1. Modes and stacks (PA-4)

| Mode | Handler today | captures | Stack |
|---|---|---|---|
| replay open (over anything) | `on_key_replay` `:232` | no | `CHAT_REPLAY` |
| composer active | `composer.on_key` `:476` | yes | `CAPTURE` |
| idle, live, parked, ended | `:489-526` | no | `CHAT_LIVE` |

```rust
// impl Tab for ChatTab
/// The stack of the Chat's current mode (MOD-67 M5 D4, PA-4): the replay's while one is open,
/// `CAPTURE` while the composer is, else the live view's (idle, live, parked or ended: one key
/// set, the agent cycle declined once a session exists, the digits fixed ahead of it).
fn key_stack(&self) -> Option<Stack<'static>> {
    Some(if self.replay.is_some() {
        views::CHAT_REPLAY
    } else if self.composer.is_active() {
        views::CAPTURE
    } else {
        views::CHAT_LIVE
    })
}
```

## 2. Arm-by-arm conversion (main blueprint §6.5)

### 2.1 `chat/mod.rs`

| file:line | today | → act (stack) | notes |
|---|---|---|---|
| `:458` | refusal cleared on every key | unchanged | |
| `:460-466` | CONTROL/ALT → `Pass` | **removed** as a guard (D7); its two effects are kept where they matter: the composer never sees what `CAPTURE` passes (below), and a chord does not disarm (below) | |
| `:471-473` | replay first | unchanged order | |
| `:233-239` | replay `Esc` → close | `Back` (`CHAT_REPLAY`) | the catalogue's `back` row already cites `chat/mod.rs:233` |
| `:242` | replay swallows `1`-`9`, `i`, `a`, `Enter` | **fixed, first**, plain keys only | D5; `alt-2` passes (as the guard made it) |
| `:243-246` | replay → `transcript.on_key` | `ListDown`, `ListUp`, `ChatTop`, `ChatBottom`, `ChatThoughts` → `transcript.on_act(act)`; else `Pass` | `q`, `Tab`, `w`, `?` still reach the shell |
| `:476-485` | composer takes every key while active | `if views::CAPTURE.passes(chord) { return Pass }`, then `composer.on_key` (arms unchanged) | ANA §6.1: the composer stays fixed; `F1` now opens help (was swallowed), ALT passes (was passed) |
| `:489-496` | parked: every digit answers / is consumed, disarms | **fixed**, plain keys only, ahead of the resolver | D5 |
| `:498-501` | `i`/`Enter` → composer | `ChatCompose` (`CHAT_LIVE`) | |
| `:502-505` | `a` (no session, agents) → next agent | `ChatNextAgent`; with a session or no agent → declined: falls to the transcript arm as today (`a` is no transcript act) → `Pass` | |
| `:506-521` | `Esc` → arm, then `ChatCancel` | `ChatEnd` (body unchanged; no session: consumed, as today) | |
| `:522` | `_ → transcript.on_key` | `ListDown`, `ListUp`, `ChatTop`, `ChatBottom`, `ChatThoughts` → `transcript.on_act(act)`; else `Pass` | |
| `:524-526` | disarm on any key but `Esc` that got here | disarm unless the first act is `ChatEnd`, **and only for a key without CONTROL/ALT** (today's chords returned before the disarm) | a rebound `[chat] end = "ctrl-e"` arms on `ctrl-e` |
| `:45` | `use crossterm::event::{KeyCode, KeyEvent, KeyModifiers}` | stays (fixed keys, `plain`) | |

### 2.2 `transcript.rs`

`Transcript::on_key(&mut self, key: KeyEvent) -> Handled` (`:460`) becomes `on_act(&mut self, act:
Act) -> Handled`: `ListDown` (`:462`, `j`/`Down`), `ListUp` (`:469`), `ChatTop` (`:483`, `g`),
`ChatBottom` (`:487`, `G`), `ChatThoughts` (`:491`, `t`), else `Pass`. Bodies unchanged. Its
`KeyCode`/`KeyEvent` import goes. `Home`/`End` stay unbound (no default change).

### 2.3 `composer.rs`

No code change. Doc (`:1-4`): "… while it is active every printable key is text; the tab offers
it only what `CAPTURE` does not pass, so CONTROL, ALT and function keys reach the shell (MOD-67
M5)."

## 3. Hints (1 constant + 6 literals → specs or label-built text)

`hint(&self, keys: &Keys) -> String` (`:408`); `replay_header(replay, ctx)` uses `ctx.keys()`;
`render` passes `ctx.keys()` (`:749-752`). Each renders through `self.key_stack()`'s stack.

| file:line | today's text | `HintSpec` / construction | rendered with defaults | differs? |
|---|---|---|---|---|
| `:64` `REPLAY_HINT` | `Esc leave replay · t thoughts · j/k scroll` | `One(Back, "leave replay")`, `One(ChatThoughts, "thoughts")`, `Pair(ListDown, ListUp, "scroll")` (`CHAT_REPLAY`) | identical | no |
| `:413-417` | the refusal | unchanged | — | no |
| `:419` | `Enter send · Esc leave` | `Text("Enter send")`, `Text("Esc leave")` (the composer's own keys) | identical | no |
| `:422` | `Esc again ends this chat` | stays a literal: key prose, M6 (D11) | identical | no |
| `:425` | `1-9 answer · i compose · t thoughts` | `Text("1-9 answer")` (fixed digits), `One(ChatCompose, "compose")`, `One(ChatThoughts, "thoughts")` | identical | no |
| `:428` | `this chat has ended · i compose` | `Text("this chat has ended")`, `One(ChatCompose, "compose")` | identical | no |
| `:429` | `i compose · Esc Esc end · t thoughts · j/k scroll` | `One(ChatCompose, "compose")`; then `keys.label(CHAT_LIVE, ChatEnd).map(\|l\| format!("{l} {l} end"))` (a doubled key has no `Hint` element); then `One(ChatThoughts, "thoughts")`, `Pair(ListDown, ListUp, "scroll")`; the non-empty parts joined `" · "` | identical | no |
| `:430` | `i compose the first prompt · a next agent` | `One(ChatCompose, "compose the first prompt")`, `One(ChatNextAgent, "next agent")` | identical | no |
| `:259` | ` · read-only · Esc leave` | `format!(" · read-only · {} leave", label(CHAT_REPLAY, Back))`; unbound: ` · read-only` | identical | no |

Every user-visible difference in L-D's hints: **none** with the defaults. Behaviour: `F1` opens
help from the composer (main §1 item 3); the `?` box gains `Chat:`/`List:` lines.

## 4. D7/D5 in L-D

The CONTROL/ALT guard (`:460-466`) goes; the replay swallow, the parked digits and the composer
stay fixed ahead of the resolver (D5), each on plain keys only, so a chord still passes exactly
as the guard made it.

## 5. In-file tests that change

- `chat/mod.rs:1565` `assert_eq!(tab.hint(), sentence)` → `tab.hint(Keys::compiled())`.
- `transcript.rs` tests calling `on_key(KeyEvent)` → `on_act(Act::…)`.
- Any test asserting a function key is swallowed by the composer now expects `Pass`.

## 6. Tests (L-D; write first, red, then code; `tests/chat.rs`, App `Harness`)

| Test | Asserts |
|---|---|
| `the_composer_status_line_and_box` (D12) | `i`: status `Ctrl+c quit · F1 help` (bare; with `register_all`: `Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue`); `?` is typed into the composer (its text ends with `?`, no box); `f1` opens the box with one `Global:` line and `F1 closes this box`; `f1` closes it; the text is intact |
| `the_live_chat_box_lists_chat_list_and_global` (D12) | a live session, `?`: `Chat: i/Enter compose · a next agent · Esc end chat (twice) · t thoughts · g top · G bottom`, `List: j/Down down · k/Up up`, `Global:`, `?/F1 closes this box` |
| `a_rebound_thoughts_key_folds_and_t_is_inert` (the lane's rebinding test) | `[chat]\nthoughts = "T"\n`, a turn with a thought: `t` leaves it shown, `T` folds it; the hint reads `i compose · Esc Esc end · T thoughts · j/k scroll` |
| `a_rebound_end_key_arms_and_ends_the_chat` | `[chat]\nend = "ctrl-e"\n`: the hint reads `… · Ctrl+e Ctrl+e end · …`; `ctrl-e`, `ctrl-e` sends `ChatCancel`; `Esc` does neither |
| `replay_swallows_plain_digits_and_passes_alt_digits` (D5) | replay open: `2` stays on Chat with the replay open; `alt-2` neither switches tabs nor closes the replay |
| `a_chord_does_not_disarm_esc_esc` | live: `Esc`, `alt-x`, `Esc` → one `ChatCancel` (a chord never disarmed, `:460-466`); `Esc`, `x`, `Esc` → none (a plain key disarms) |

Existing `tests/chat.rs` and `tests/replay.rs` assertions: the replay header text (`chat.rs:655`)
and the `Esc Esc` paths stay green unchanged.

## 7. Snapshots (`chat__*`, 15)

None moves: every hint and the replay header render byte-identical, and the status line of
`CHAT_LIVE`/`CHAT_REPLAY` (unfiltered global) equals `Stack::BASE`'s. No snapshot shows an open
composer. Any drift is a defect (D9).

## 8. Cross-lane couplings

None at build time. `tests/replay.rs` drives the Runs pane (L-B) into the Chat replay: the replay
keys are this lane's, the Runs `Enter` L-B's; edit `tests/replay.rs` only for a flagged break.

## 9. Work order, commits and gate

1. **C1** `feat(mod-67): Chat dispatches through its stacks (D4, D5, PA-4)` — `key_stack`, §2.1,
   §2.2, §2.3 doc; `the_composer_status_line_and_box`, `the_live_chat_box_…`,
   `replay_swallows_plain_digits…`, `a_chord_does_not_disarm_esc_esc`.
2. **C2** `feat(mod-67): Chat hints from its stacks` — §3; `a_rebound_end_key_…`.
3. **C3** `test(mod-67): L-D rebinding test` — `a_rebound_thoughts_key_…`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/chat/{mod,transcript,composer}.rs \
  crates/htui/tests/chat.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::chat
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features \
  --test chat --test chat_offline --test replay -- --test-threads=1
cargo insta test -p htui --all-features --check     # nothing pending
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```

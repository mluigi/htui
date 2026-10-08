# MOD-67 M5 blueprint — Lane L-C: Body, Graph, Prompt and Reqs

Scope: `crates/htui/src/ui/tabs/backlog/detail/{body,graph,prompt,requirements}.rs`. Owned tests:
`tests/backlog_keys/l_c.rs`, the in-file tests of those files; `tests/prompt_preview.rs` unchanged.
Snapshots: `backlog__detail_{body,graph,prompt,reqs,reqs_cite_picker,reqs_reconfirmed}`,
`backlog__empty_{body,graph,prompt,reqs}`, `backlog__graph_*` (14). Branch `mod-67-m5-l-c`,
worktree `target/wt/l-c`.

Lane rules (plan, M3/M4): never edit `keys/`, `app/`, the registries, `testkit.rs`,
`tests/keys*.rs`, `backlog/{mod,filter,item_form,divergence,list}.rs`, `backlog/detail/mod.rs`
(`Scroll` included), the other panes, `chat/**`. A missing act or stack goes back to the main
thread. Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`.
Commit incrementally; stage own paths only.

Complete: sections 0-9.

## 0. Facts (read at `3af293a6`)

- `BodyTab::on_key` (`body.rs:77-79`) and `PromptTab::on_key` (`prompt.rs:327-342`) have **no
  modifier guard**: `Scroll::on_key` matches the code alone, so `alt-J`/`ctrl-PgDn` scroll, and
  Prompt's `n`/`p` match the code alone, so `ctrl-n`/`alt-p` cycle the template (the Backlog tab
  hands every CONTROL/ALT chord to the pane, `backlog/mod.rs:746-752`). PA-1.
- Prompt draws `n / p · N template(s)` as a header row (`prompt.rs:141-144`), built once per reply
  in `rebuild` (`:124-130`) → `render_lines` (`:134`), not per frame.
- `GraphTab::on_key` (`graph.rs:687-707`): CONTROL/ALT → `Pass`; `J`/`K` move the cursor,
  `PgDn`/`PgUp` move it by a page, `+`/`-` set the depth, `Enter` re-roots (always consumed: "the
  Backlog's `replay step` miss would be the wrong answer from this pane", `:680-686`); `_ → Pass`.
  Its footer is `depth d/3 · +/- depth · Enter re-root` (`:766-775`).
- `ReqsTab::on_key` (`detail/requirements.rs:464-486`): a non-browse mode → `modal_key`;
  CONTROL/ALT → `Pass`; no item → `Pass`; `J`/`K`, `r`, `c`, `u`; `_ → scroll.on_key` (`PgUp`/
  `PgDn`). `modal_key` (`:267-322`) passes CONTROL only (`:268`): `alt-y` uncites (D7). The picker
  takes `j`/`J`/`Down`, `k`/`K`/`Up`, `Enter`, `Esc` (`:325-345`); the kind question `a`, `v`, `Esc`
  (back to the picker); the uncite question `y`, `n`/`Esc`; everything else is swallowed.
- With the Backlog tab unconverted on this branch, a pane receives every key the tab does not
  intercept, every chord, and `Enter` on an item row; the legacy miss row answers a `Pass`.
- T1 provides `BACKLOG_BODY` (Body's stack, the `DetailTab::key_stack` default),
  `BACKLOG_GRAPH`, `BACKLOG_PROMPT`, `BACKLOG_REQS`, `REQS_PICK`, `REQS_KIND`, `REQS_CONFIRM`,
  the `Graph*`, `Prompt*`, `Reqs*` acts, and `Scroll::apply`.

## 1. Modes and stacks

| Pane / mode | Handler today | captures | Stack |
|---|---|---|---|
| Body | `body.rs:77` | no | `BACKLOG_BODY` (the trait default: no override) |
| Prompt | `prompt.rs:327` | no | `BACKLOG_PROMPT` |
| Graph | `graph.rs:687` | no | `BACKLOG_GRAPH` |
| Reqs `Browse` | `requirements.rs:464` | no | `BACKLOG_REQS` |
| Reqs `Pick` | `modal_key` → `pick_key` `:325` | yes | `REQS_PICK` |
| Reqs `Kind` | `modal_key` `:278` | yes | `REQS_KIND` |
| Reqs `ConfirmUncite` | `modal_key` `:303` | yes | `REQS_CONFIRM` |

```rust
// impl DetailTab for PromptTab / GraphTab
fn key_stack(&self) -> Stack<'static> { views::BACKLOG_PROMPT } // / views::BACKLOG_GRAPH

// impl ReqsTab
/// The current mode's stack (MOD-67 M5 D4): the only place a mode maps to its keys.
fn stack(&self) -> Stack<'static> {
    match self.mode {
        Mode::Browse => views::BACKLOG_REQS,
        Mode::Pick { .. } => views::REQS_PICK,
        Mode::Kind { .. } => views::REQS_KIND,
        Mode::ConfirmUncite { .. } => views::REQS_CONFIRM,
    }
}
// impl DetailTab for ReqsTab
fn key_stack(&self) -> Stack<'static> { self.stack() }
```

## 2. Arm-by-arm conversion (main blueprint §6.2-§6.4)

### 2.1 `body.rs`

| file:line | today | → act (`BACKLOG_BODY`) | notes |
|---|---|---|---|
| `:77-79` | `self.scroll.on_key(key, self.len())` | `PaneScrollDown`/`Up`, `PanePageDown`/`Up` → `return self.scroll.apply(act, self.len())`; else `continue`; loop end `Pass` | `alt-J` no longer scrolls (§1 item 2 of the main blueprint) |

### 2.2 `prompt.rs`

| file:line | today | → act (`BACKLOG_PROMPT`) | notes |
|---|---|---|---|
| `:329` | `n` → cycle forward | `PromptNextTemplate` | `ctrl-n` is no act any more (PA-1) |
| `:330` | `p` → cycle back | `PromptPrevTemplate` | |
| `:331-336` | no item or nothing to cycle → `Pass`; else `PromptPreview` request | unchanged under the act (`Pass` → `continue`) | |
| `:331` | `_ → scroll.on_key` | the four pane-scroll acts → `scroll.apply(act, self.lines.len())` | |
| `:141-144` | `n / p · N template(s)` | `render_lines(preview, picker: &str)`; `rebuild(&mut self, keys: &Keys)` builds `picker` from `keys.label(views::BACKLOG_PROMPT, PromptNextTemplate)` and `…PrevTemplate`, joined `" / "` (both unbound: empty, and the row reads `N template(s)`); `on_reply` passes `ctx.keys()` | byte-identical with the defaults; a rebound `n` shows its label |

### 2.3 `graph.rs`

| file:line | today | → act (`BACKLOG_GRAPH`) | notes |
|---|---|---|---|
| `:688-693` | CONTROL/ALT → `Pass` | removed | |
| `:695` | `J` → cursor +1 | `GraphNextRow` | |
| `:696` | `K` | `GraphPrevRow` | |
| `:697` | `PgDn` → cursor +page | `PanePageDown` | the catalogue's `page_down` row cites `graph.rs:697` |
| `:698` | `PgUp` | `PanePageUp` | |
| `:699` | `+` → deeper | `GraphDeeper` | |
| `:700` | `-` → shallower | `GraphShallower` | |
| `:701` | `Enter` → `re_root` (always consumed) | `GraphReroot` → `re_root`, `Consumed` | PA-2: shadows `backlog.replay`; on a header the tab folds first |
| `:702` | `_ → Pass` | loop end `Pass` | |
| `:766-775` | `depth d/3 · +/- depth · Enter re-root` | `format!("depth {}/{} · {}", …, keys.hint(views::BACKLOG_GRAPH, GRAPH_HINT))`, the `" · {}"` dropped when the hint is empty | identical with the defaults |
| `:680-686` doc | "Matched on the code alone …" | "Resolved through `BACKLOG_GRAPH` (MOD-67 M5); `Enter` is `backlog.graph.reroot`, which shadows the Backlog's replay miss" | |

### 2.4 `detail/requirements.rs`

Browse (`on_key`, `BACKLOG_REQS`):

| file:line | today | → act | notes |
|---|---|---|---|
| `:468-473` | CONTROL/ALT → `Pass` | removed | |
| `:474-476` | no item → `Pass` | unchanged (before resolving) | |
| `:478` | `J` → cursor +1 | `ReqsNextRow` | |
| `:479` | `K` | `ReqsPrevRow` | |
| `:480` | `r` → reconfirm | `ReqsReconfirm` | |
| `:481` | `c` → picker | `ReqsCite` | |
| `:482` | `u` → uncite question | `ReqsUncite` | |
| `:483` | `_ → scroll.on_key(key, rows * 2)` | `PanePageDown`/`Up` → `return self.scroll.apply(act, self.rows().len() * 2)` | `PgUp`/`PgDn` only, as today |

Modal (`modal_key`, `self.stack()`; every mode ends in `modal_rest`):

| file:line | today | → | notes |
|---|---|---|---|
| `:268-270` | CONTROL → `Pass` (ALT reaches the mode) | **removed** | D7: `alt-y` no longer uncites |
| `:271-274` | no item → browse, `Consumed` | unchanged, after a `stack.passes(chord)` check returns `Pass` (as the CONTROL pass did) | |
| `:328-330` | picker `j`/`J`/`Down` → +1 | `ReqsNextRow` (`J`) or `ListDown` (`j`, `down`) | two acts, one move: `J`/`K` are the pane's cursor verbs, `j`/`k` the list's |
| `:331-333` | `k`/`K`/`Up` → −1 | `ReqsPrevRow` or `ListUp` | |
| `:334-340` | `Enter` → kind question (no candidate: stay) | `ReqsPick` | |
| `:341` | `Esc` → browse | `Back` (`REQS_PICK`) | |
| `:342` | `_` → stay | stay; `modal_rest` | |
| `:280` | kind `a` → cite addresses | `ReqsAddresses` | |
| `:281` | kind `v` → cite reserves | `ReqsReserves` | |
| `:294-300` | kind `Esc` → back to the picker on that requirement | `Back` (`REQS_KIND`) | |
| `:301` | kind `_` → stay | stay; `modal_rest` | |
| `:306-316` | uncite `y` → uncite | `ConfirmYes` (`REQS_CONFIRM`) | |
| `:317` | uncite `n`/`Esc` → browse | `ConfirmNo` | |
| `:318` | `_` → stay | stay; `modal_rest` | |

`pick_key(&self, cursor, code: KeyCode)` becomes `pick_act(&self, cursor, act: Act) -> Option<Mode>`
(the mode on an accepted act, `None` to fall to `modal_rest`).

## 3. Hints (3 constants + 2 inline literals + the two label-built rows)

`footer(&self, width, room, keys: &Keys, theme)` (`:383`); `render` passes `ctx.keys()`.

| file:line | today's text | stack | `HintSpec` | rendered with defaults | differs? |
|---|---|---|---|---|---|
| `requirements.rs:77` + `:80` | `HINT_MOVE` `J/K move · ` + `HINT_WRITES` `r reconfirm · c cite · u uncite` (writes dim offline) | `BACKLOG_REQS` | `MOVES`: `Pair(ReqsNextRow, ReqsPrevRow, "move")`; `WRITES`: `One(ReqsReconfirm, "reconfirm")`, `One(ReqsCite, "cite")`, `One(ReqsUncite, "uncite")`. Spans: moves (base), `" · "` (base, only when both are non-empty), writes (base or dim) | `J/K move · r reconfirm · c cite · u uncite` (42 in 43) | no |
| `requirements.rs:83` | `PICK_HINT` `j/k move · Enter pick · Esc back` | `REQS_PICK` | `Pair(ListDown, ListUp, "move")`, `One(ReqsPick, "pick")`, `One(Back, "back")` | identical | no |
| `requirements.rs:425` | `a addresses · v reserves · Esc back` | `REQS_KIND` | `One(ReqsAddresses, "addresses")`, `One(ReqsReserves, "reserves")`, `One(Back, "back")` | identical | no |
| `requirements.rs:429` | `y uncite · n keep` | `REQS_CONFIRM` | `One(ConfirmYes, "uncite")`, `One(ConfirmNo, "keep")` | identical | no |
| `graph.rs:766-775` | `depth d/3 · +/- depth · Enter re-root` | `BACKLOG_GRAPH` | `GRAPH_HINT`: `Pair(GraphDeeper, GraphShallower, "depth")`, `One(GraphReroot, "re-root")` | identical | no |
| `prompt.rs:141-144` | `n / p · N template(s)` | `BACKLOG_PROMPT` | labels joined `" / "` (§2.2) | identical | no |

Every user-visible difference in L-C's hints: **none** with the defaults. The behaviour
differences are the modifier fixes (main §1 item 2), the Reqs `alt-y` (D7) and `F1` in the Reqs
modes (main §1 item 3). Prose naming keys stays (M6): `requirements.rs`'s module doc table
(`:7-13`), `NOT_SUSPECT`, `NO_CITATION`.

## 4. D7 in L-C

The Reqs modal CONTROL-only pass (`:268`) and the Graph/Reqs browse guards go; Body and Prompt
gain the modifier exactness they never had. Pinned (§6).

## 5. In-file tests that change

- `requirements.rs:997-1022`: `BROWSE_HINT_TEXT` unchanged; the span search for `HINT_WRITES`
  looks for `Keys::compiled().hint(views::BACKLOG_REQS, WRITES)`; `footer(…, Keys::compiled(),
  …)`.
- `requirements.rs` modal tests sending `KeyEvent`s with CONTROL stay (passes); any that sends
  `alt-y` expecting an uncite flips (D7).
- `graph.rs` tests that call `on_key` keep their keys; a footer-text test, if any, is identical.
- `prompt.rs` tests: `rebuild(Keys::compiled())`, `render_lines(preview, "n / p")`; the expected
  rows (`:484-496`) unchanged.
- `body.rs`: none (its scroll test sends `J`).

## 6. Tests (L-C; write first, red, then code)

`tests/backlog_keys/l_c.rs` (App `Harness`, demo; `TO_REQS`-style sub-tab steps as
`tests/backlog.rs` does):

| Test | Asserts |
|---|---|
| `alt_y_does_not_uncite` (D7, D12) | Reqs on a cited item, `u`: `alt-y` sends no `UnciteRequirement` and the question stays; `y` sends one |
| `ctrl_n_does_not_cycle_the_prompt_template` (PA-1) | Prompt: `ctrl-n` and `alt-p` send no `PromptPreview`; `n` sends one |
| `a_chord_does_not_scroll_the_body` (main §1 item 2) | Body on a long item: `alt-J` and `ctrl-pgdn` leave the first row; `J` scrolls |
| `enter_re_roots_the_graph_and_never_says_the_miss` (PA-2) | Graph on an item with links: `Enter` re-roots (the border reads the new root), the status line has no miss |
| `f1_opens_help_from_the_reqs_picker_and_it_stays` (main §1 item 3) | `c`, `f1`: `help_visible`; `f1` closes it; the picker is still open on the same candidate. (The box's `Reqs:` line needs L-A's `BacklogTab::key_stack`: T-close's merged-tree test checks it, main §7 step 2.) |
| `the_picker_moves_on_j_and_j_capital` | `J` and `j` and `Down` each move the picker cursor; `K`/`k`/`Up` back |
| `a_rebound_cite_key_opens_the_picker_and_c_is_inert` (the lane's rebinding test) | `[backlog.reqs]\ncite = "C"\n`: `c` opens nothing; `C` opens the picker; the browse hint reads `J/K move · r reconfirm · C cite · u uncite`. `C` passes the unconverted Backlog tab and the converted one (no `backlog`/`list`/`pane` act binds it) |

`tests/backlog.rs` (the Reqs and Graph tests at `:280-1300`) and `tests/prompt_preview.rs`: no
assertion changes expected. If one breaks, edit it flagged.

## 7. Snapshots (L-C's 14)

On this branch every L-C hint renders byte-identical and the Backlog tab reports no stack, so
**no snapshot moves**. At X3 (T-close) `backlog__detail_reqs_cite_picker`'s status row takes
`Ctrl+c quit · F1 help` and nothing else. Any other drift here is a defect (D9). `prompt_preview__`
(app-level) is identical too (the `n / p` row is label-built).

## 8. Cross-lane couplings

- `BacklogTab::key_stack` (L-A) reads `GraphTab`/`PromptTab`/`ReqsTab::key_stack`: only on the
  merged tree do their `?` boxes and the picker's status line show these stacks.
- On a project header the tab folds before Graph sees `Enter` (L-A); on this branch the
  unconverted tab already does.
- `Scroll::on_key` loses Body's, Prompt's and Reqs' callers; T-close deletes it (`detail/mod.rs`
  is not this lane's).
- `tests/backlog.rs` shared with L-A/L-B: flagged edits only.

## 9. Work order, commits and gate (serial inside the lane)

1. **C1** `feat(mod-67): Body and Prompt resolve their keys; Prompt names its template keys
   (PA-1)` — §2.1, §2.2; `ctrl_n_does_not_cycle…`, `a_chord_does_not_scroll_the_body`.
2. **C2** `feat(mod-67): the Graph pane on its stack; Enter re-roots by name (PA-2)` — §2.3;
   `enter_re_roots_the_graph…`.
3. **C3** `feat(mod-67): the Reqs pane and its picker on their stacks (D7)` — §2.4, hints;
   `alt_y_does_not_uncite`, `f1_opens_help_from_the_reqs_picker…`,
   `the_picker_moves_on_j_and_j_capital`.
4. **C4** `test(mod-67): L-C rebinding test` — `a_rebound_cite_key_…`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/backlog/detail/{body,graph,prompt,requirements}.rs \
  crates/htui/tests/backlog_keys/l_c.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::backlog::detail
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features \
  --test backlog --test backlog_keys --test prompt_preview --test reveal -- --test-threads=1
cargo insta test -p htui --all-features --check     # nothing pending on this branch
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```

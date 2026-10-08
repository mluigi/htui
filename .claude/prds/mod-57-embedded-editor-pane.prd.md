# MOD-57 — Run the external editor inside the TUI pane

> Routed as **PRD** by `/handoff-run MOD-57` (criteria C2 and C4 fired, C3 partly; ultracode
> recommended for implement and review; the maintainer accepted, 2026-10-07). Spawned by MOD-9
> (`docs/decisions/mod/mod-9.md`) at the merge of MOD-7 milestone 2, maintainer-decided 2026-09-26.
> The contract is `R-TUI-1`, `R-TUI-7`, `R-TUI-10` and `R-NF-1`, with ANA-26 §6.4 and C3
> (`docs/ANA-26.md`) on keys.

## Problem

The maintainer edits templates, skill bodies, backlog item bodies, documents and notes in a real
editor (nvim, vim, emacs) through `E`/`Ctrl+E`. Today that suspends the whole TUI and gives the
terminal to `$VISUAL`/`$EDITOR` until it exits. For the whole edit the maintainer cannot see the
item, the template's placeholders, run progress, chat activity or a permission ask, because htui
is not drawn at all. Live state goes unseen and cannot be acted on until the editor is closed.

## Evidence

- Assumption — needs validation by prototype: use the in-pane mode for the next 10 external edits
  and compare with the suspend mode.
- The need was raised by the maintainer at the merge of MOD-7 milestone 2 and recorded in MOD-9's
  spawn list (`docs/decisions/mod/mod-9.md`, 2026-09-26). No incident (a missed permission ask
  during an edit) is recorded.

## Users

- **Primary**: the htui maintainer, editing a body with a terminal editor from any of the five
  views that hand off to `$EDITOR` today (Skills Templates and Library, the Backlog item form, Backlog
  Documents and Notes), while runs and chats continue in other tabs.
- **Not for**: GUI editors (`code --wait` keeps working, but in the pane there is nothing to draw
  while it runs); the two overlays (Asker, Popup), which refuse external edits today and keep
  refusing; agents.

## Hypothesis

We believe **an opt-in mode that runs the editor in the requesting pane while the rest of htui
stays drawn and usable** will **remove the blackout during an external edit** for **the
maintainer**.
We'll know we're right when, **over the next 10 external edits in the in-pane mode, no edit is
lost or differs from what the editor saved, the maintainer can leave the editor, act elsewhere in
htui and return to it, and no edit needed the suspend mode as a workaround**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Edits whose read-back differs from the file the editor saved | 0 | Tests over the read-back gate with a real child process; maintainer self-report over the 10 uses |
| Edits that needed a fallback to suspend mode | 0 of 10 | Maintainer self-report in the close-out write-up |
| Terminal editors that work end to end in the pane (draw, keys, resize, `ctrl-c` forwarded, exit) | nvim, vim, nano at least; emacs `-nw` and helix TBD — needs validation by manual run | Automated e2e with whichever editor the test box has; manual run for the rest |
| Child processes left running after htui exits | 0 | Test: quit with a live editor leaves no child |

## Scope

**MVP** — A per-machine preference, defaulting to today's suspend, chooses what `E`/`Ctrl+E` do in
every view that hands off to `$EDITOR`. In the in-pane mode the editor runs on a pseudo-terminal
inside the requesting view's editing area, falling back to the whole tab body when that area is
below a minimum size. The rest of htui keeps drawing. Every key goes to the editor, `ctrl-c`
included, except one reserved **leave** action (a named action in MOD-67's catalogue, never
hard-coded, which must stay bound). Leave returns focus to htui with the editor still alive and
drawn; the same action refocuses it. With focus in htui the maintainer can switch tabs and act
anywhere. The editor is drawn only in its own view, the status line shows that an edit is open,
and a second `E`/`Ctrl+E` is refused while one editor is alive. htui offers an **abort** for a
stuck editor. Pane resizes reach the editor. When the editor exits, the file is read back through
the same outcome path and `parse` gate as today, so the view sees the same result either way.
The `quit` action with a live editor asks for confirmation, then kills the editor; `ctrl-c` with
focus in htui still quits at once (ANA-26 C3). Linux and
macOS are built and tested here; Windows (ConPTY) compiles and passes the cross-check.

**Out of scope**
- nvim's `--embed` RPC UI (`nvim-rs`): nvim-only, not preferred at MOD-9.
- More than one live editor at a time: one is enough to test the hypothesis.
- External edits from the Asker and Popup overlays: they refuse today and keep refusing.
- Mouse forwarding, scrollback and htui-side copy/paste in the pane: the editor's own handling
  is enough.
- The kitty keyboard protocol: ANA-26 left it out of scope, and key encoding follows the same
  legacy terminal encoding htui reads.
- Verifying ConPTY on real Windows: it goes on MOD-16's Windows verification list, as the
  existing `$EDITOR` Windows path did.
- Changing today's suspend mode beyond sharing its read-back path.

## Decisions taken at the PRD gate

| # | Question | Decision |
|---|---|---|
| D1 | Where the editor draws | **The requesting pane.** The view that asked provides its editing area; below a minimum size the editor uses the whole tab body. |
| D2 | How in-pane is chosen over suspend | **One per-machine preference** (default: suspend) that `E`/`Ctrl+E` follow everywhere. No new edit key. |
| D3 | What the reserved leave action does | **Unfocus, editor stays alive.** The same action refocuses it. The edit ends when the editor exits. htui has its own abort action for a stuck editor. |
| D4 | What is allowed while unfocused | **Anything, one editor at a time.** Tabs can be switched; the editor is drawn only in its own view; the status line shows the open edit; a second edit request is refused. |
| D5 | Quit with a live editor | **The `quit` action confirms, then kills the editor and quits. `ctrl-c` keeps ANA-26 C3** (with focus in htui it quits at once and kills the editor; only the focused pane forwards it). Settled at the gate as Q5. |
| D6 | Windows coverage before done | **Cross-check now, real verification under MOD-16.** |

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Editor in the pane | With the preference on, `E`/`Ctrl+E` open the editor in the requesting pane with htui drawn around it: keys and `ctrl-c` forwarded, resizes forwarded, leave and refocus within the view, abort, and the file read back through the same gate. Dependencies decided and in `Cargo.lock`. | complete | `.claude/plans/mod-57-m1-editor-in-pane.plan.md` |
| 2 | Live editor across htui, close-out | With focus in htui, tabs switch and work while the editor stays alive; status-line notice; a second edit refused; quit confirms and kills; no child left behind; help, docs and the MOD-16 Windows entry; decision record. | pending | — |

## Open Questions

- [ ] **Q1 Dependencies.** `portable-pty`, `vt100` and `tui-term` are not in `Cargo.lock`. The
      plan decides by probe: versions against the workspace's ratatui and crossterm, licences,
      maintenance, `vt100`'s fidelity with nvim (alternate screen, true colour, cursor shape), and
      whether `tui-term` can draw through `ui::cells` (MOD-54/MOD-60) or a small own widget is needed.
- [ ] **Q2 Where the preference lives.** Per machine (like `$EDITOR` itself and `keys.toml` under
      the config root) or in the store's settings. Per machine is the PRD's intent; the plan picks
      the mechanism and the Settings surface.
- [ ] **Q3 Minimum pane size** for the D1 fallback to the tab body: TBD — needs validation by
      manual run with nvim in the smallest editing area (Notes compose).
- [ ] **Q4 Key encoding.** How htui's crossterm key events are encoded back to bytes for the child
      (Alt, function keys, `ctrl-` chords), and which `TERM` the child sees.
- [x] **Q5 `ctrl-c` with focus in htui while an editor is alive.** Settled (D5): keep C3, only
      `quit` confirms.
- [ ] **Q6 MOD-67 coupling.** The leave and abort actions, a forwarding context in which nothing but
      leave resolves, and a validator rule that keeps leave bound all land in `keys/` while MOD-67
      M2-M5 are still in flight. The plan names the merge order and the shared files.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Merge conflicts with MOD-67 M2-M5 in the key catalogue, resolver and hint snapshots | High | Medium | Keep the key additions small and isolated; plan names the files; rebase after MOD-67 M2 merges |
| The VT parser draws some editors wrongly (colours, cursor, alternate screen) | Medium | Medium | The Q1 probe with nvim before committing to the crate; suspend mode stays as the fallback |
| A PTY reader blocking or a child left running stalls the event loop or leaks a process | Medium | High | Reader off the UI thread; kill on drop, on abort and on quit; test that quit leaves no child |
| Key encoding gaps (Alt, function keys) make some editor bindings unusable | Medium | Low | Q4; tests over the encoder; the suspend mode is still available |
| ConPTY behaves differently on real Windows | Medium | Medium | D6: cross-check here, verification under MOD-16 |
| Paused-time tests misfire beside a real child process | Medium | Low | Split unit tests (paused time, no child) from e2e tests (real time, real child) |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*

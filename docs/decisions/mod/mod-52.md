# MOD-52 - `ctrl-c` does not quit `htui` (done, 2026-09-29)

**Requirements:** `R-TUI-1`. Found 2026-09-26 in the MOD-7 milestone 2 plan fact-check.
**Design authority:** no ANA precedes this item. The maintainer chose "bind it" over "correct the
comments" on 2026-09-29. Routed plan (1 of 4 routing criteria fired, C3, answered by that choice;
no ultracode). No plan file, because the change is one binding and its tests.
**Commits:** `62450bc` (the binding and its tests), plus the close-out commit.

## What shipped

Crossterm's raw mode clears `ISIG`, so `ctrl-c` raises no `SIGINT` and arrives as an ordinary key.
Nothing was bound to it. Meanwhile about a dozen capturing sections and `TextField` passed `CONTROL`
chords on "so `ctrl-c` still quits", and their comments said it did.

`Keymap::default_global` now binds `ctrl-c` to `Action::Quit` in two scopes:

- **`KeyScope::Global`**, reached from any tab once the tab and its section pass the chord on. Every
  capturing section and `TextField` already does, so `ctrl-c` quits from inside a half-typed field,
  where `q` is a letter.
- **`KeyScope::Overlay(OverlayId::ANY)`**, the wildcard every overlay inherits, as `Esc` does. The
  two overlays that exist are modal and swallow what they do not bind. Without this binding,
  `ctrl-c` would do nothing over the startup workspace switcher and the migration prompt, the first
  screens a user sees. `q` is still not an overlay key.

The global help line is unchanged, because `ctrl-c`'s help text is "quit", the same as `q`, and
`help_line` collapses rows that share help text. The comment in `boxes.rs` that deferred to this
item and the `box_settings.rs` test comment now say `ctrl-c` quits.

## Tests

- `keymap::tests::ctrl_c_quits_globally_and_from_any_overlay` checks the two bindings and that `q`
  stays global only.
- `ctrl_c_quits_from_the_modal_switcher` (`tests/shell.rs`): the switcher swallows `q`, and `ctrl-c`
  quits.
- `ctrl_c_quits_from_a_half_typed_field` (`tests/hierarchy.rs`): in the repo editor `q` is a letter,
  and `ctrl-c` quits. It fails without the binding.

The whole `htui` suite (`--features testkit`) and `cargo clippy -p htui --all-targets -D warnings`
are green.

## Follow-up

The maintainer asked for the hotkeys to be configurable, so **ANA-26** was opened to analyze scope,
storage, validation (including whether `ctrl-c` quit may be unbound) and how hints follow a
rebinding.

# ANA-26 - Configurable hotkeys (concluded, 2026-09-30)

Opened 2026-09-29 during MOD-52 (`docs/decisions/mod/mod-52.md`) when the maintainer asked for
htui's hotkeys to be configurable. Only the global and overlay-wildcard table
(`Keymap::default_global`) was data. Every tab, section and overlay key was a hard-coded `KeyCode`
match, and hints were string constants. Addresses `R-TUI-1`, `R-TUI-8`, `R-STO-1`, `R-STO-4`,
`R-USR-1`, `R-NF-1`, `R-NF-3`. Analysis: `docs/ANA-26.md`.

**Verdict (`docs/ANA-26.md` §6).**

1. **Scope.** Every key outside text entry becomes a named action with one resolver. Printable
   characters, in-widget editing, a widget's own Enter and Esc, the numbered permission answers
   and a choice field's value keys stay fixed.
2. **Naming.** One name per meaning, not per letter. Shared verbs live in `list`, `pane`,
   `confirm`, `form` and `common`. Where one letter means two things, the names split (agents'
   `r` is a probe; boxes has two edit targets; Runs `J`/`K` move the step cursor). A narrower
   context may override a shared verb for one view.
3. **Ownership.** A local `<config_root>/keys.toml`, per OS user and machine, listing only
   changes. The store is never read or written for keys: they are needed before it connects, and
   store settings are neither mirrored nor typed for this.
4. **Validation.** An invalid file refuses to start. Every error prints as `path:line`, the exit
   status is 2, and the error stays out of error capture. `--default-keys` bypasses the file,
   `--keys PATH` reads another one, and `--print-keys` prints the result and doubles as the check.
   The file is checked for:
   - unknown names;
   - chords the legacy terminal encoding cannot deliver;
   - duplicate chords on a composed stack (reviewed default pairs are allowed as shadowing or
     state-guarded). Amended by MOD-12 M3 R1 H1 (2026-10-08): an entry that takes a chord from an
     action of its own context left at its default wins; that action loses the chord (unbound
     if it has no other), with a stderr notice and a `--print-keys` mark. Two entries on one
     chord, and a chord shared with another context's default across a stack, stay errors;
   - printable chords on actions offered while a field captures;
   - `overlay.close` left with no chord.

   `ctrl-c` is checked first in `App::on_key` and always quits. The one declared exception is
   MOD-57's pseudo-terminal pane, which forwards it to its child and must keep a leave action
   bound.
5. **Hints and help.** Hints are generated from per-mode hint specs over the resolved keys. `?`
   and the new `f1` default open the help box from every screen, overlays included. The box
   re-renders from the live stack. The status line follows the active stack.

**Found in passing.** `ctrl-c` opens the clear confirm in Settings > Connection and Qdrant browse,
because the browse arms ignore modifiers. The Qdrant editor passes `Tab` to the tab bar while its
key field is open. ALT pass-through is inconsistent across capturing views. MOD-67 fixes all three
by construction.

**Survey.** gitui, lazygit, helix, yazi and zellij were compared, along with the ratatui
component template and two keybinding crates. htui takes partial override (gitui) and shared verbs
with narrow-wins contexts (lazygit). It takes TOML merged per key onto compiled defaults, with
prompt keys left out of scope (helix). It takes the text-input contract of claiming modifier-free
printable keys (yazi) and refusal on a bad file (zellij). None of the surveyed tools reserves
`ctrl-c` or checks cross-context conflicts; htui does both.

**Fact-check.** A citation check (112 claims) and a completeness critic ran over the draft. They
corrected ten citations and counts and six design gaps before conclusion:
- the capture rule became "no printable chord", not "CONTROL chords only";
- form choice fields became fixed keys;
- the MOD-57 `ctrl-c` exception was added;
- help was made reachable over overlays and in capture;
- the resolver returns ordered candidates so state-dependent keys fall through;
- M1 keeps the `Ctx` constructor, so M3-M5 stay independent, and M2 carries the `main.rs` exit
  code.

**Requirements (maintainer-approved 2026-09-30).** `R-TUI-10` is added. `R-TUI-1` gains
"keys are configurable (R-TUI-10)". `R-STO-1` states that local files holding no secret and no
domain data (box identity, cache, key bindings) are not a store.

**Spawned.** **MOD-67**, configurable hotkeys, in six milestones: catalogue and resolver, the
file, Settings and overlays, Skills and Requirements, Backlog and Chat, close-out. MOD-57 gained a
note that its leave chord is a catalogue action.

**Open (`docs/ANA-26.md` §10).**
- Harmonising the probe key (`r` in agents, `p` in boxes).
- Whether `ctrl-h` and `ctrl-j` join the rejected legacy aliases.
- The `toml` span API.
- A read-only Keys section in Settings (not planned).
- A future store layer under the file.
- The kitty keyboard protocol.

Commits: analysis, requirement amendments and close-out are in the commit that added this file.

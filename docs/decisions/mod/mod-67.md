# MOD-67 - Configurable hotkeys: named actions, `keys.toml`, generated hints (done, 2026-10-14)

**Requirements:** `R-TUI-10`, `R-TUI-1`, `R-STO-1`, `R-NF-1`.
**Origin:** ANA-26 (`docs/ANA-26.md`, `docs/decisions/ana/ana-26.md`); TUI design review of 2026-10-04
(https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH findings 1 and 3).
**Artifacts:** one plan and blueprint per milestone in `.claude/plans/`:
`mod-67-m1-catalogue-resolver`, `mod-67-m2-keys-file`, `mod-67-m3-settings-overlays`,
`mod-67-m4-skills-requirements`, `mod-67-m5-backlog-chat`, `mod-67-m6-close-out` (each a plan file
with its verified-claims table and review gate, a blueprint file, and lane files under
`mod-67-m<N>-blueprint/`).
Every milestone was routed **plan** (C4 only, low confidence), with ultracode for the implement phase
only; `code-architect` and `rust-reviewer` ran as plain agents.
**User guide:** README "Changing keys" and the per-tab key tables; `docs/htui-keys.md` (every action
and its default, generated).

## The problem

Every key in htui was matched by hand in the view that used it: about 100 meanings spread over
`KeyCode` arms, a legacy `Keymap` for the global and a few Backlog keys, and hint rows typed as text.
Nothing could be rebound, hints and messages named keys that a view did not actually answer, and
ANA-26 found routing defects the scattering hid (`ctrl-c` opened the clear confirm in Settings >
Connection and Qdrant browse; the Qdrant editor passed `Tab` to the tab bar; modifier-blind arms such as
`ctrl-d` deleting in Hierarchy).

## What was built

- **One compiled-in catalogue of named actions** (`crates/htui/src/keys/catalogue.rs`, 170 actions in
  42 contexts): named once per meaning, with shared contexts (`list`, `pane`, `confirm`, `form`,
  `common`) and a narrower view context overriding a shared verb for one view.
- **Context stacks and an ordered resolver** (`keys/stack.rs`, `keys/views.rs`): every mode of every
  view declares its stack (80 in `DECLARED`); the resolver returns ordered candidates, so a view can
  decline one and fall through. `ctrl-c` is checked first and always quits (MOD-57's pane excepted).
  Keys a view keeps ahead of the resolver (a choice's value keys, a form's own `Enter`/`Esc`, the
  permission digits) are listed in `FIXED`, and the validator refuses a rebind onto them.
- **`<config_root>/keys.toml`** (`keys/load.rs`, `keys/validate.rs`), per OS user and machine, never
  the store: a user lists only what they change. An invalid file refuses to start with every error as
  `keys.toml:LINE:`, exit status 2, kept out of Sentry. `--keys PATH`, `--default-keys`,
  `--print-keys` (a complete commented `keys.toml` that reads back to the same keys).
- **Everything that names a key is generated:** the status line, the `?` box (every layer of the
  focused pane), every hint, and (M6) every message that names a rebindable key, through
  `Keys::label` on the stack the key acts in; an unbound action drops the clause.
- **One fixed hint row above the status line** for every tab (M6, design review finding 3), blank under
  an overlay or the in-pane editor. Overlays keep the hint inside their own box.

## Milestones

- **M1 catalogue and resolver** (`c3b3f7bd`..`add7fb39`, 2026-10-06): the `keys/` module, `KeyChord`
  with a strict parser, the global and overlay actions, generated status line and `?` box; `ctrl-c`
  quits first everywhere.
- **M2 the file** (`d4be4861`..`97cad2aa`, 2026-10-07): `keys.toml` with line spans, per-context and
  per-stack validation, the three flags, the Unix legacy-ctrl rejects. MOD-12 M3 later let a user entry
  take a chord from an action left at its default (stderr notice, `(unbound by quit)` mark).
- **M3 Settings and overlays** (`55064b73`..`4ae61bf9`, 2026-10-08): ten Settings sections and four
  overlays on declared stacks, 65 hints generated, view overrides of shared verbs; fixed the
  modifier-blind arms and the Qdrant editor's `Tab`.
- **M4 Skills and Requirements** (`ba082c94`..`4802a157`, 2026-10-08): `form.save` is the only save
  chord outside Backlog; the MOD-57 `[editor]` rules closed.
- **M5 Backlog and Chat** (`850d4bee`..`c65527db`, 2026-10-13): the Backlog tab, its forms, seven
  sub-tabs and compose areas, and Chat; `FIXED` and the fourth validator pass; the last legacy
  `Keymap` rows gone.
- **M6 close-out** (`fcf996b1`..`37326035`, 2026-10-14):
  - T0 `fcf996b1`: `keymap.rs` deleted; `Ctx::new` takes `&Keys` (42 call sites in 17 files);
    `App::new` takes no key table; `OverlayId::ANY` gone.
  - T1 `202c9ca9`..`011f1aa7`: `[queue]` and `[picker]` contexts (11 actions, stacks `QUEUE_OVERLAY`
    and `HIERARCHY_PICKER`); `FIXED` for the Requirements priority keys and the attachment form's
    activation keys; the fixed pass reports a line's chord once (D6); a view row pinned to its
    derivation keeps its line, so `--print-keys` marks it and collisions land on its line (one line
    in `Keys::derive`); reachability tests; `docs/htui-keys.md` generated and pinned.
  - Lanes (merged `85482818`, `23ab2afa`, `6b1b794e`): the queue overlay (no longer modifier-blind)
    and the directory picker on their stacks; 61 key-naming sentences generated (`Ctrl+s` is the one
    spelling, PA-7; `y / n` question tails built from `confirm.*`, PA-6).
  - X-S `7cc8c9c4`, `67783120`: the queue's stale-pause sentences name the pause key; the
    reachability tests cover every stack and act.
  - T2 `7847f9f4`..`0fd2054c`: the shell's hint row (`Tab::key_hint`, `SettingsSection::key_hint`,
    `DetailTab::key_hint`, `cells::clip_line`); the Backlog row shows the focused pane's keys, then
    the list's when they fit; 161 snapshots re-baselined, reviewed against the blueprint's accept list.
  - R1 `1996ab0f`..`37326035`: the review gate's findings (below).

## Decisions taken with the maintainer (M6)

- Plan Q1 (a): the fixed hint row is built in M6, not handed to MOD-82. Q2 (a): the reference page is
  the literal `htui --default-keys --print-keys` output in one `toml` block, pinned by a test.
- Blueprint PA-1..PA-13 accepted whole: D4 covers all 61 sentences (not ~25); both carried print lows
  fixed in `Keys::derive`; label-built `y / n` tails; `Ctrl+s` spelling; agent help names its leaving
  key per state; D6 dedupes the fixed pass only (a collision and a capture error on one chord stay two
  errors).

## Changed for users

- Any key can be rebound in `keys.toml`; `--print-keys` shows what is in force, `docs/htui-keys.md`
  the defaults.
- Refused now (each error names the line and the screen): a `[global]` key on a key a view uses; a
  shared rebind that collides in some stack (`[list] down = ["G"]`, `[common] back = ["esc",
  "backspace"]` since M6's picker); a rebind onto a key a screen keeps for itself
  (`[backlog.filter_form] clear = ["space"]`, the Requirements priority keys, the attachment form's
  `Space`).
- Modifier-blind keys fixed: `ctrl-d`/`ctrl-l`/`ctrl-t`/`alt-y` in Settings and the overlays (M3),
  Skills `ctrl-l` and `alt-y` (M4), Backlog/Chat `alt-y` and Body/Prompt keys (M5), the queue overlay's
  `ctrl-j`/`alt-P` (M6).
- Hints moved from inside each view to one row above the status line (M6); `Ctrl+S` in messages reads
  `Ctrl+s`, like the hints.

## Review gate (M6)

`rust-reviewer` over `d8171e44..0fd2054c`: approve with changes; 0 CRITICAL/HIGH/MEDIUM, 5 LOW, 7 NIT.
All fixed in R1 (`1996ab0f`..`37326035`, one implement/verify workflow, both lenses clean): rustdoc
private-item links written as code spans (11 fewer errors, 4 of them pre-existing in `keys/`); docs
that still described per-view hint rows; a stale test name; the reachability test now checks each
stack's acts in that stack's own files (with a small per-stack delegate table, no exception list); one
`line_of` helper for every validator pass; `PickerOutcome::Stay`; one `settings::key_line` helper.
Every lane and task verified clean in round 1; no repair round was needed before the review.

## Carried

- **MOD-82**: the Prompt pane header's `n / p · N templates` and the Skills/Templates pane titles'
  ` J/K scroll ` are pane chrome that names keys, not hint lines; they stay literal until MOD-82's
  shared pane chrome.
- Five pre-existing rustdoc private-item links outside `keys/` (`claude_emit`, `concepts`,
  `secrets_settings`, `state_cmd`) stay with CLEAN-10.
- Owed on the host: the Postgres gate on the merged tree, and a manual run on Windows (MOD-16).

# Blueprint: MOD-67 milestone 1, "key catalogue and resolver"

**Status**: proposed (2026-10-06). The findings in §0 (F-1 to F-10), the plan deviations in §9
(PD-1 to PD-4) and the blueprint decisions in §10 (B1 to B12) are proposed here. A finding marked
**Blocker** means the plan, read literally, leaves a red tree between tasks, fails its own named
test, or cannot be validated as written. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-67-m1-catalogue-resolver.plan.md` (approved, CONFIRM 2026-10-06). Its
D1-D12, T0-T4, file sets and Verified-claims table are authoritative except where §0 and §9 amend
them. **Spec**: `docs/ANA-26.md` §6, §7.1-§7.3, §7.6, §8 (M1 row).

**Verified at**: HEAD `c3b3f7bd` ("M1 T0 - keys module scaffold"), branch `hr/MOD-67`. Every
`file:line` below was read at that HEAD through Gortex (`read`, `search` text/symbols); the `.snap`
counts were grepped. **Line numbers are pre-edit.** A citation into a file a task edits moves after
that task's first commit.

**T0 is already done** (`c3b3f7bd`): `lib.rs:27` has `pub mod keys;` and
`crates/htui/src/keys/{mod,chord,catalogue,stack,hint}.rs` exist, each with a one-line `//!`
header (`mod.rs` declares the four submodules). Every task below keeps those headers and appends
below them.

**Scope**:
- **Order**: T1 ∥ T2 (disjoint files), then T3, then T4, serial on the merged tree. After each
  merge, re-run that task's gate on the real tree with `--test-threads=1`.
- **New public surface**: `htui::keys::{KeyChord, ChordError, CTRL_C, Act, Context, ActionSpec,
  CATALOGUE, STATE_GUARDED, Keys, Layer, Stack, Hint, HintSpec, HelpLine}`, `App::offer`,
  `App.keys`, `Ctx::keys`, `Ctx::with_keys`.
- **Counts**: `Act` has **41** variants and `Context` has **7** (§3). `CATALOGUE` has 41 rows.
- **No file under `crates/htui/src/ui/` changes. No snapshot changes.** `testkit.rs` and `lib.rs`
  do not change after T0.
- **No new dependency.** `crates/htui` has no `thiserror` (`crates/htui/Cargo.toml`), so
  `ChordError` gets a hand-written `Display`. `unicode-width` is already a dependency.

**House style (carried)**:
- Lints (`Cargo.toml` `[workspace.lints]`): `unsafe_code = "forbid"`,
  `missing_debug_implementations` and `unused_qualifications` warn, `clippy::all` warns.
  **`clippy::pedantic` is deliberately not enabled.** `lib.rs:10` adds `#![warn(missing_docs)]`.
  rustdoc denies `broken_intra_doc_links`, `private_intra_doc_links` and
  `redundant_explicit_links`. The gate runs clippy with `-D warnings`, with and without features.
- So every `pub` item, `pub` field and enum variant gets a doc comment, and every `pub` type
  derives `Debug`. No `pub` doc may link a private item (`Row`, §4.1).
- **No intra-doc link from a T1 item to a T2 item, or back.** Each task builds and runs
  `cargo doc` alone; name the other task's items in plain backticks. T3 and T4 may link both.
- Inline format arguments (`format!("{x}")`, never `format!("{}", x)`).
- `rustfmt.toml`: edition 2024, `max_width = 100`. Toolchain `1.98.1`.
- Implementers commit incrementally and stage their own paths only (never `-A`, never `stash`).
  Every commit compiles.
- The Gortex PreToolUse hook blocks shell reads of indexed source. Read with Gortex `read`. If
  `Read`/`Edit` is blocked, use an anchored scripted replace (each anchor asserted to match once),
  then `cargo fmt`.

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree at `c3b3f7bd` | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (a red tree for two tasks) | T1: "Rework `keymap.rs` per D2/D3", i.e. `default_global()` returns an empty table. | `App::on_key` resolves global and overlay keys only through `self.keymap` (`state.rs:657`, `:706`), and the status line is `self.keymap.help_line(&KeyScope::Global)` (`state.rs:754`). If T1 empties the table, every commit from T1 to T4 has no `q`, `Tab`, digits, `?`, `Esc` or `ctrl-c`, and a blank status line. That breaks about 106 snapshots, `testkit.rs` unit tests (`the_tab_strip_follows_the_tab_bindings`, `an_overlay_survives_startup_sees_keys_first_and_closes_on_esc`) and every `Harness` suite. | **PD-2**: T1 moves `KeyChord` and adds the strict parser **only**. `default_global` keeps its body and its six table tests until **T4**, which empties it in the same commit that wires the resolver. The end state is identical to D2/D3. |
| **F-2** | Major (fails a T3 test as worded) | T3: "no two actions in one context share a default chord". | `common.back` and `common.dismiss` are both `Esc` today. `back` is at `divergence.rs:245`, `library.rs:895`, `personas.rs:841` and `chat/mod.rs:233`. `dismiss` (clear the notice) is at `connection.rs:828`, `boxes.rs:739`, `hierarchy.rs:1294`, `kinds.rs:1511` and `prompt.rs:875`. | **PD-1**: `catalogue.rs` gets `STATE_GUARDED: &[(Act, Act)] = &[(Act::Back, Act::Dismiss)]`. This is the first entry of ANA §7.4 step 7's reviewed allow-list (the "state-guarded" kind). The T3 and T2 uniqueness tests exempt the pairs on it. |
| **F-3** | Major (T1 ∥ T2 contract) | D2: `keymap.rs` does `pub use crate::keys::KeyChord;`. Independence table: "`keys/mod.rs` is … edited only by T3". | The path `crate::keys::KeyChord` exists only once `keys/mod.rs` re-exports it, and that is T3's file. | **PD-3**: T1 writes `pub use crate::keys::chord::KeyChord;` in `keymap.rs`. That path stays valid after T3 adds `pub use chord::KeyChord` to `mod.rs`, so nobody edits the line again. |
| **F-4** | Major (two plan tests contradict) | T2: `global.help == ["?", "f1"]`. T3: "no `in_capture` action has a printable chord". | `?` is printable. Both tests hold only if `global.help.in_capture == false`. If it is false, the plan must say how `f1` reaches help while a field captures. | **B3**: `in_capture` marks actions that live *inside* a capturing mode (the four `form.*` and `overlay.close`). The global layer under a capturing mode is filtered **per chord** (printable chords drop out), not per action. That filter is M3's (D7 defers capture). So `global.*` is `in_capture: false`, `f1` survives the filter, and `?` does not. M1 has no capture signal, and `f1` reaches help wherever the view passes it (plan Risks row 5). |
| **F-5** | Major (T4 test fails on screen) | D8 / T4: "the help box shows `Ctrl+w waiting` only with `register_all`". | `render_help` sizes the box as `lines.len() + 2` but renders with `Wrap { trim: true }` (`state.rs:782-788`), so a wrapped line is **clipped** at the bottom. The generated global line with `register_all` is 136 cells. The box's inside is 88 cells at the Harness's 100 columns. With the Backlog legacy line (96 cells) above it, `Ctrl+w waiting` lands in a clipped row. Today's box already clips. | **B9**: `HelpLine::rows(width)` packs a logical line into rows, breaking only between ` · ` entries. Continuation rows indent by two spaces. `render_help` sizes the box from the packed row count and drops `Wrap` (and its import, `state.rs:11`). |
| **F-6** | Minor (gate command invalid) | T1 validate: `cargo test -p htui --lib keys::chord keymap`. | `cargo test` takes one `TESTNAME`, so a second positional filter is rejected. | T1's gate is `cargo test -p htui --lib -- keys::chord keymap`, using libtest's multiple filters. |
| **F-7** | Minor (record for M4) | D12: defaults unchanged. | Every `ctrl-s`/`ctrl-e` arm also accepts the capital: `item_form.rs:275`, `:281`; `requirements/mod.rs:233`; `attach.rs:477`; `library.rs:939`, `:1021`; `templates.rs:689` (`Char('s' \| 'S')`). The strict parser refuses `"ctrl-S"` (D11), and `KeyChord` keeps `'S'`, so a resolver-dispatched `form.save` drops the caps-lock alias. | Not M1 (no view consumes `form`). M4 decides between case-folding CONTROL letters in `KeyChord::new` and accepting the loss. Recorded in §7 H-9. |
| **F-8** | Minor (record for M3) | D4: shared defaults from today's arms. | `kinds.rs:1476` binds `g` to "open graph", which collides with `list.top = g` once the kinds stack includes `list`. | M3 adds a shadowing allow-list entry, or leaves `list.top` out of the kinds stack. Recorded in §7 H-10. |
| **F-9** | Minor (stale docs, out of scope) | "No file under `ui/` changes." | After T4, these comments describe a wildcard keymap row that no longer exists: `ui/overlay/registry.rs:18-24` (`OverlayId::ANY`), "`Esc` falls through to the wildcard overlay binding" (`workspace_switcher.rs:173`, `migration_prompt.rs:103`, `waiting_list.rs:288`, `concepts_search.rs:334`), and `tabs/registry.rs:21`. The behaviour they describe still holds (`overlay.close`). | Leave them. M3 converts the overlays and rewrites the comments. Listed in the T4 close-out so the reviewer does not flag it. |
| **F-10** | Minor | Plan Files table: `app/update.rs` "`Ctx::new(..)` sites gain `.with_keys(keys)`". | There are exactly **4** sites (`update.rs:160`, `:262`, `:307`, `:336`) plus **7** in `state.rs` (`:411`, `:467`, `:501`, `:594`, `:639`, `:682`, `:714`). The `update.rs` test module builds no `Ctx` itself (its three `App::new` calls, `:581`, `:1460`, `:1488`, stay as they are). | §5.3 lists all 11. |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| How is "only help from global" expressed in the overlay stack? | A `Layer` is a context plus an optional `only: &'static [Act]` filter. `Stack::OVERLAY = [Layer::all(Overlay), Layer::only(Global, &[Act::Help])]`. | §4.2 |
| Multi-chord action with one printable chord vs `in_capture`? | `in_capture` is per action and means "every chord must be non-printable". `global.help` is `false`. Capture filtering of the global layer is per chord (M3). | B3, §3.3 |
| Do `?`/`f1` toggle help over a modal overlay? | Yes, at D6 step 2, but only for a key the overlay returned `Pass` for. The switcher, migration prompt and waiting list pass `?`. The concepts field consumes `?` and passes `f1` (`text_field.rs` passes non-`Char` keys it does not edit). | §5.2, T4 tests 4-7 |
| Does `ctrl-c` interact with `Event::Paste`? | No. The check lives in `on_key` only. `on_paste` has no key table (MOD-22 M-1), so a pasted `U+0003` is text or dropped, never a quit. | §7 H-3 |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (min; each compiles) | Gate (always on the real tree after merge, `--test-threads=1`) |
|---|---|---|---|
| T0 | done (`c3b3f7bd`) | - | - |
| T1 chord | `keys/chord.rs`, `keymap.rs` | 2 (§2.6) | `cargo test -p htui --lib -- keys::chord keymap`; clippy both ways |
| T2 catalogue | `keys/catalogue.rs` | 1-2 (§3.5) | `cargo test -p htui --lib -- keys::catalogue`; clippy both ways |
| merge T1, T2 | - | - | `cargo test -p htui --lib`; `cargo doc -p htui --no-deps` |
| T3 resolver | `keys/mod.rs`, `keys/stack.rs`, `keys/hint.rs` | 2 (§4.7) | `cargo test -p htui --lib -- keys`; clippy both ways; `cargo doc -p htui --no-deps` |
| T4 shell | `app/state.rs`, `app/update.rs`, `app/mod.rs`, `keymap.rs`, `tests/keys.rs` | 2-3 (§5.8) | the plan's full Validation block; zero `.snap.new` |

"Clippy both ways" means `cargo clippy -p htui --all-targets --all-features -- -D warnings` and
`cargo clippy -p htui -- -D warnings` (featureless: catches code dead without `testkit`).

---

## 2. T1: chord and strict parser (D2, D3, D11; PD-2, PD-3)

**First failing test**: `keys::chord::tests::a_shifted_letter_is_refused_with_its_capital`.

### 2.1 `crates/htui/src/keys/chord.rs`: what moves

Move these **verbatim** from `keymap.rs` (bodies and docs unchanged, apart from intra-doc links,
which must resolve from the new module):
- `pub struct KeyChord { pub code: KeyCode, pub mods: KeyModifiers }` with its derive
  (`Debug, Clone, Copy, PartialEq, Eq, Hash`) (`keymap.rs:14-24`);
- `KeyChord::{new, from_event, to_event, parse, label}` (`keymap.rs:26-122`);
- the private `fn key_code(name: &str) -> Option<KeyCode>` (`keymap.rs:125-155`).

Keep T0's `//!` line. Add a second paragraph: "`KeyChord` moved here from `keymap.rs` (MOD-67 M1,
plan D2); `crate::keymap::KeyChord` re-exports it until M6. `parse` is the harness's lenient reader
and is unchanged. `parse_strict` is the key file's reader (ANA-26 §7.1)."

Imports: `use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};`.

### 2.2 `chord.rs`: what is new

```rust
/// `ctrl-c`: always quits (ANA-26 §6.4). `App::on_key` checks it before any overlay or view, and
/// no catalogue action may bind it.
pub const CTRL_C: KeyChord = KeyChord { code: KeyCode::Char('c'), mods: KeyModifiers::CONTROL };

impl KeyChord {
    /// Parses a spec from a key file (ANA-26 §7.1, MOD-67 D11): `parse`'s grammar, made strict.
    /// See the rules below.
    pub fn parse_strict(spec: &str) -> Result<Self, ChordError>;

    /// The canonical spelling `parse_strict` reads back: modifiers in the order `ctrl-`, `alt-`,
    /// `shift-` (`shift-` only on a named key), then the key: the character itself, `space`, or a
    /// lower-case name (`enter`, `esc`, `tab`, `backtab`, `backspace`, `delete`, `insert`, `home`,
    /// `end`, `pgup`, `pgdn`, `up`, `down`, `left`, `right`, `f1`..`f12`).
    /// Error suggestions use it, and M2's `--print-keys` will too.
    #[must_use]
    pub fn spec(&self) -> String;

    /// Whether a text field would take this chord as typed text: a `Char` with neither CONTROL
    /// nor ALT (ANA-26 §6.4). `Space` is printable; `F1`, `Tab` and `ctrl-f` are not.
    #[must_use]
    pub fn is_printable(&self) -> bool;
}

/// Why a spec is not a chord a terminal can deliver (ANA-26 §7.1, §7.5). Every message is the
/// tail of a `keys.toml:LINE: [context] name = "spec": …` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    /// Nothing but whitespace.
    Empty,
    /// A modifier other than `ctrl`, `alt` or `shift`; carries the part as written.
    UnknownModifier(String),
    /// The same modifier twice; carries its lower-case name.
    DuplicateModifier(String),
    /// Not one character and not a key name; carries the key part as written.
    UnknownKey(String),
    /// `shift-` with a letter (and no `ctrl-`): a terminal sends the capital.
    ShiftedLetter {
        /// The spec to write instead, e.g. `A` or `alt-X`.
        suggestion: String,
    },
    /// `shift-` with a non-letter character (`shift-1`): only the character it types arrives,
    /// and that depends on the keyboard layout.
    ShiftedCharacter {
        /// The spec as written (trimmed).
        written: String,
    },
    /// `ctrl-` with a capital, or with `shift-` and a letter: a terminal sends ctrl chords in
    /// lower case.
    CtrlCapital {
        /// The spec to write instead, e.g. `ctrl-c`.
        suggestion: String,
    },
    /// A ctrl chord the legacy encoding delivers as another key (crossterm 0.29,
    /// `event/sys/unix/parse.rs:92-116`).
    Indistinguishable {
        /// The spec as written (trimmed).
        written: String,
        /// What actually arrives.
        arrives_as: KeyChord,
    },
}
impl std::fmt::Display for ChordError { /* exact strings below */ }
impl std::error::Error for ChordError {}
```

**`Display`, exact strings** (in ANA §7.5 style: lower case, no trailing period; `{label}` is
`KeyChord::label`):

| Variant | Message |
|---|---|
| `Empty` | `an empty chord` |
| `UnknownModifier(m)` | `"{m}" is not a modifier: write ctrl, alt or shift` |
| `DuplicateModifier(m)` | `"{m}" is written twice` |
| `UnknownKey(k)` | `"{k}" is not a key: write one character or a key name such as "enter" or "f5"` |
| `ShiftedLetter { suggestion }` | `write a shifted letter as "{suggestion}"` |
| `ShiftedCharacter { written }` | `"{written}": write the character the shifted key types instead` |
| `CtrlCapital { suggestion }` | `write "{suggestion}": a terminal sends ctrl with the lower-case letter` |
| `Indistinguishable { written, arrives_as }` | `"{written}" arrives as {arrives_as.label()}: a terminal cannot tell the two apart` |

So `"shift-a"` gives `write a shifted letter as "A"`, `"ctrl-C"` gives
`write "ctrl-c": a terminal sends ctrl with the lower-case letter`, and `"ctrl-i"` gives
`"ctrl-i" arrives as Tab: a terminal cannot tell the two apart`.

### 2.3 `parse_strict`: the algorithm (normative)

1. `let written = spec.trim();`. If it is empty, return `Empty`.
2. **Split at the last separator that is not the final character.** If `written` is one char, the
   key part is `written` and there are no modifiers. Otherwise let `li` be the byte index of the
   last char. If `written[..li].rfind(['-', '+'])` is `Some(i)`, the modifier part is
   `&written[..i]` and the key part is `written[i + 1..].trim()`. If it is `None`, the whole of
   `written` is the key part. This is what makes `ctrl--` (`ctrl` + `-`), `ctrl - c` and
   `ctrl++` work.
3. **Modifiers.** Split the modifier part on `['-', '+']`, trim each piece and compare it ASCII
   case-insensitively. `ctrl` gives CONTROL, `alt` gives ALT, `shift` gives SHIFT. Anything else,
   including an empty piece and the lenient aliases `control`/`meta`, is
   `UnknownModifier(piece)`. A repeat is `DuplicateModifier(lower-case name)`.
4. **Key.** Call `key_code(key_part)`, the moved lenient table: names are case-insensitive, aliases
   (`return`, `escape`, `del`, `ins`, `pageup`, `pagedown`) are accepted, and a single char is
   itself. `None` gives `UnknownKey(key_part)`.
5. **Refusals, in this order, only when the code is `Char(c)`:**
   1. If SHIFT is set: with CONTROL and `c.is_ascii_alphabetic()`, return
      `CtrlCapital { suggestion: spec(mods − SHIFT, c.to_ascii_lowercase()) }`. Otherwise, if
      `c.is_ascii_alphabetic()`, return
      `ShiftedLetter { suggestion: spec(mods − SHIFT, c.to_ascii_uppercase()) }`. Otherwise return
      `ShiftedCharacter { written }`.
   2. If CONTROL is set and `c.is_ascii_uppercase()`, return
      `CtrlCapital { suggestion: spec(mods, c.to_ascii_lowercase()) }`.
   3. If CONTROL is set and `c` is one of `i m [ \ ] ^ _`, return `Indistinguishable`.
      `arrives_as` is:
      - `i` → `Tab`, `m` → `Enter`, `[` → `Esc`, each with `mods − CONTROL`;
      - `\` → `Char('4')`, `]` → `Char('5')`, `^` → `Char('6')`, `_` → `Char('7')`, each with
        `mods` unchanged.

      `ctrl-h` and `ctrl-j` are **accepted** (D11: in raw mode `0x08`/`0x0A` arrive as
      `Char + CONTROL`).
6. `Ok(KeyChord::new(code, mods))`. `new` folds `shift-tab` to `BackTab` and drops SHIFT from a
   `Char`. Neither applies after step 5, except that a named key keeps SHIFT: `shift-up` is
   accepted and its label is `Shift+Up`.

The parser does **not** refuse `ctrl-c`. That is M2's loader step 6 ("ctrl-c always quits and
cannot be bound"). T2's test enforces it for the defaults.

### 2.4 `crates/htui/src/keymap.rs` after T1 (PD-2: the table stays)

- Delete the moved items (`keymap.rs:14-155`). Add the re-export right after the `use` block, with
  a doc line:
  ```rust
  /// Moved to `crate::keys::chord` (MOD-67 D2); re-exported so every `crate::keymap::KeyChord`
  /// and `htui::keymap::KeyChord` path keeps compiling until M6.
  pub use crate::keys::chord::KeyChord;
  ```
- Imports: `KeyEvent` is no longer used outside tests, so drop it from
  `use crossterm::event::{…}` (`keymap.rs:8`), or `unused_imports` fails clippy. `KeyCode` and
  `KeyModifiers` stay, because `default_global` still uses them in T1.
- `default_global`, `KeyScope`, `Binding`, `Keymap` and its six table and mechanics tests are
  **untouched in T1** (F-1). T4 empties the table (§5.6).
- Module doc (`keymap.rs:1-6`): append one sentence. "`KeyChord` lives in `crate::keys::chord`
  since MOD-67 M1 and is re-exported here."
- Tests: delete the three chord tests here (they move, §2.5). The `chord` helper (`keymap.rs:325`)
  stays for the six remaining tests.

### 2.5 T1 tests (`#[cfg(test)] mod tests` in `chord.rs`), TDD order

Use a `fn strict(spec: &str) -> Result<KeyChord, ChordError>` helper and the moved
`fn chord(spec) -> KeyChord` (`parse`).

| # | Test | Asserts |
|---|---|---|
| 1 | `parse_reads_plain_keys_named_keys_and_modifiers` | moved verbatim from `keymap.rs:329-358` |
| 2 | `shift_tab_normalises_to_backtab_however_it_is_written` | moved verbatim (`keymap.rs:361-371`) |
| 3 | `a_shifted_character_keeps_the_character_and_drops_the_flag` | moved verbatim (`keymap.rs:373-377`) |
| 4 | `strict_reads_every_canonical_spec_as_parse_does` | for each of `q ? enter esc tab backtab shift-tab space G f1 f5 pgdn pgup home end down up left right ctrl-f ctrl-w ctrl-s ctrl-e ctrl-c -`: `strict(s) == Ok(chord(s))` |
| 5 | `parts_are_trimmed` | `" ctrl - c "`, `"ctrl -c"` and `"ctrl- c"` all equal `chord("ctrl-c")` |
| 6 | `ctrl_minus_and_ctrl_plus_are_writable` | `"ctrl--"` → `Char('-')` + CONTROL; `"ctrl++"` and `"ctrl-+"` → `Char('+')` + CONTROL; `"-"` → `Char('-')` |
| 7 | `shift_tab_is_backtab` | `strict("shift-tab")` has code `BackTab` and no SHIFT, and equals `strict("backtab")` |
| 8 | `a_shifted_letter_is_refused_with_its_capital` | `"shift-a"` → `ShiftedLetter { suggestion: "A" }` and displays `write a shifted letter as "A"`; `"shift-A"` → same; `"alt-shift-x"` → suggestion `"alt-X"` |
| 9 | `ctrl_with_a_capital_is_refused_with_the_lower_case` | `"ctrl-C"` → `CtrlCapital { suggestion: "ctrl-c" }` with the exact message; `"ctrl-shift-c"` → same suggestion |
| 10 | `chords_the_terminal_cannot_tell_apart_are_refused_by_what_arrives` | table: `ctrl-i`→`Tab`, `ctrl-m`→`Enter`, `ctrl-[`→`Esc`, `ctrl-\`→`Ctrl+4`, `ctrl-]`→`Ctrl+5`, `ctrl-^`→`Ctrl+6`, `ctrl-_`→`Ctrl+7` (assert `arrives_as.label()`); `ctrl-i` displays `"ctrl-i" arrives as Tab: a terminal cannot tell the two apart` |
| 11 | `ctrl_h_and_ctrl_j_are_accepted` | both `Ok`, as `Char('h'/'j')` + CONTROL |
| 12 | `unknown_empty_and_repeated_parts_are_refused` | `""`/`"   "` → `Empty`; `"hyper-x"` → `UnknownModifier("hyper")`; `"control-x"` → `UnknownModifier("control")`; `"ctrl-wat"` → `UnknownKey("wat")`; `"ctrl-ctrl-x"` → `DuplicateModifier("ctrl")`; `"-x"` → `UnknownModifier("")` |
| 13 | `shift_on_a_non_letter_is_refused` | `"shift-1"` → `ShiftedCharacter { written: "shift-1" }` |
| 14 | `a_shifted_named_key_keeps_its_shift` | `strict("shift-up")` has mods SHIFT and label `Shift+Up` |
| 15 | `the_ctrl_c_constant_is_the_parsed_chord` | `CTRL_C == chord("ctrl-c")` and `== strict("ctrl-c").unwrap()` |
| 16 | `spec_round_trips_through_the_strict_parser` | for each chord of test 4 plus `alt-x`, `shift-up` and `f12`: `strict(&c.spec()) == Ok(c)` |
| 17 | `printable_means_a_character_without_ctrl_or_alt` | `q ? space G` are printable; `ctrl-q alt-q f1 tab esc` are not |

### 2.6 T1 commits and gate

1. `test(mod-67): T1 strict chord parser cases` + `refactor(mod-67): move KeyChord to keys::chord`
   (tests 1-3 move with the code; the tree stays green).
2. `feat(mod-67): KeyChord::parse_strict, ChordError, CTRL_C, spec, is_printable` (tests 4-17).

Gate: `cargo test -p htui --lib -- keys::chord keymap` (F-6), clippy both ways, and
`cargo doc -p htui --no-deps`.

---

## 3. T2: the catalogue (D4, D12; PD-1; B1-B4)

**First failing test**: `keys::catalogue::tests::global_help_is_question_mark_and_f1`.

### 3.1 Types (`crates/htui/src/keys/catalogue.rs`)

```rust
/// A key context: a TOML table of `keys.toml` and a layer of a context stack (ANA-26 §7.2-§7.3).
/// M1 has the global, overlay and shared contexts; M3-M5 append view contexts (`SettingsAgents`,
/// `BacklogRuns`, ...), each in its own block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// `[global]`: reachable from every screen, checked last.
    Global,
    /// `[overlay]`: every overlay's own keys, ahead of the modal swallow.
    Overlay,
    /// `[list]`: moving a cursor through rows.
    List,
    /// `[pane]`: scrolling a read-only pane and cycling its sub-tabs.
    Pane,
    /// `[confirm]`: answering a yes/no question.
    Confirm,
    /// `[form]`: moving between a form's fields and saving it; reachable while a field captures.
    Form,
    /// `[common]`: verbs every view that offers them shares (edit, new, delete, ...).
    Common,
}
impl Context {
    /// The TOML table name: `global`, `overlay`, `list`, `pane`, `confirm`, `form`, `common`.
    #[must_use] pub const fn table(self) -> &'static str;
    /// The `?` box heading: `Global`, `Overlay`, `List`, `Pane`, `Confirm`, `Form`, `Common`.
    #[must_use] pub const fn heading(self) -> &'static str;
}

/// A named action: one per meaning, not per letter (ANA-26 §6.2). Views (M3-M5) match on it
/// instead of `KeyCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act { /* 41 variants, §3.2, each with a one-line doc */ }
impl Act {
    /// The tab index of `select_tab_1`..`_9` (0..=8); `None` for every other action.
    #[must_use] pub const fn tab_index(self) -> Option<usize>;
    /// This action's catalogue row. `None` only if a variant was added without a row, which
    /// `every_act_has_exactly_one_row` forbids.
    #[must_use] pub fn spec(self) -> Option<&'static ActionSpec>;
}

/// One catalogue row: the single source of truth for an action's name, defaults and help
/// (ANA-26 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    /// The action.
    pub act: Act,
    /// Its context (TOML table).
    pub context: Context,
    /// Its key in that table, e.g. `"next_tab"`.
    pub name: &'static str,
    /// Default chords as strict spec strings, parsed by `keys::Keys::defaults`; `[]` is unbound.
    pub defaults: &'static [&'static str],
    /// The `?` box and status-line label, e.g. `"next tab"`.
    pub help: &'static str,
    /// Offered while a text field captures, so every chord must be non-printable (ANA-26 §6.4).
    pub in_capture: bool,
}

/// Every action, grouped by context, in display order (the status line and the `?` box follow
/// it). M3-M5 append their contexts' blocks at the end.
pub static CATALOGUE: &[ActionSpec] = &[ /* §3.2 */ ];

/// Default pairs that share a chord in one context because a view accepts at most one of them
/// in any state and declines the other (ANA-26 §7.4 step 7, "state-guarded"). It starts M2's
/// reviewed allow-list.
pub static STATE_GUARDED: &[(Act, Act)] = &[(Act::Back, Act::Dismiss)];
```

Write `static`, not `const`, for both tables (one address, and no `declare_interior_mutable_const`
questions later). The rows are plain struct literals: `ActionSpec { act: Act::Quit, context:
Context::Global, name: "quit", defaults: &["q"], help: "quit", in_capture: false }`. A small
`const fn row(...)` helper is allowed if it keeps rustfmt from spreading every row over 8 lines.

### 3.2 The M1 catalogue, complete (41 rows, in this order)

Mirror citations are the match arm each default copies. "Disagrees" lists sampled arms that differ.
M3-M5 settle those when they convert the view. Each shared-context row carries its citation as a
`//` comment in `catalogue.rs`.

**`[global]`**. The status line and `?` box order. Labels reproduce today's
`keymap.rs:205-253` / `app/mod.rs:82-109`.

| `Act` | name | defaults | help | in_capture | mirrors |
|---|---|---|---|---|---|
| `Quit` | `quit` | `["q"]` | `quit` | false | `keymap.rs:209` (`ctrl-c` is fixed: `CTRL_C`, never listed) |
| `NextTab` | `next_tab` | `["tab"]` | `next tab` | false | `keymap.rs:215` |
| `PrevTab` | `prev_tab` | `["backtab"]` | `previous tab` | false | `keymap.rs:221` |
| `SelectTab1`..`SelectTab9` | `select_tab_1`..`select_tab_9` | `["1"]`..`["9"]` | `select tab` (all nine) | false | `keymap.rs:226-231` |
| `Help` | `help` | `["?", "f1"]` | `help` | **false** (B3) | `keymap.rs:235`; `f1` added (ANA §6.5, D12) |
| `Workspaces` | `workspaces` | `["w"]` | `workspaces` | false | `app/mod.rs:84` (offered, D5) |
| `Find` | `find` | `["ctrl-f"]` | `find` | false | `app/mod.rs:95` (offered) |
| `Waiting` | `waiting` | `["ctrl-w"]` | `waiting` | false | `app/mod.rs:106` (offered; MOD-69 D7) |

**`[overlay]`**

| `Act` | name | defaults | help | in_capture | mirrors |
|---|---|---|---|---|---|
| `OverlayClose` | `close` | `["esc"]` | `close` | **true** | `keymap.rs:241`. In capture because the concepts field returns `Pass` on `Esc` and the overlay layer closes it (`concepts_search.rs:334-335`). |

**`[list]`**

| `Act` | name | defaults | help | in_capture | mirrors | disagrees |
|---|---|---|---|---|---|---|
| `ListDown` | `down` | `["j", "down"]` | `down` | false | `backlog/mod.rs:655`, `requirements/mod.rs:430`, `connection.rs:811`, `kinds.rs:1492`, `qdrant.rs:404`, `workspace_switcher.rs:163`, `waiting_list.rs:274` | `agents.rs:2420` and `hierarchy.rs:1211` take `j` only (ANA §6.6 adds `down`); `filter.rs:250` adds `Tab` (a form); `divergence.rs:259` scrolls with it |
| `ListUp` | `up` | `["k", "up"]` | `up` | false | `backlog/mod.rs:656`, `requirements/mod.rs:431`, `connection.rs:815`, `kinds.rs:1496`, `boxes.rs:699`, `library.rs:689` | as `down` |
| `ListTop` | `top` | `["g", "home"]` | `top` | false | `backlog/mod.rs:657`, `requirements/mod.rs:432` | `transcript.rs:483` has `g` only; `kinds.rs:1476` `g` = open graph (F-8) |
| `ListBottom` | `bottom` | `["G", "end"]` | `bottom` | false | `backlog/mod.rs:658`, `requirements/mod.rs:433` | `transcript.rs:487` has `G` only |
| `ListFold` | `fold` | `["enter"]` | `fold` | false | `backlog/mod.rs:684-689` (fold, else the detail pane: ANA §7.4's state-guarded `list.fold`/`backlog.runs.replay`), `requirements/mod.rs:434` | everywhere else `Enter` is a view verb (`connection.rs:807`, `workspace_switcher.rs:171`, `waiting_list.rs:284`) |

**`[pane]`**

| `Act` | name | defaults | help | in_capture | mirrors | disagrees |
|---|---|---|---|---|---|---|
| `PaneScrollDown` | `scroll_down` | `["J"]` | `scroll down` | false | `backlog/detail/mod.rs:447` (`Scroll::on_key`), `requirements/mod.rs:435`, `library.rs:708`, `templates.rs:528` | `runs.rs:1434` `J` = next step (`backlog.runs.next_step`, ANA §6.2); `documents.rs:461`, `graph.rs:695` and `detail/requirements.rs:478` move a pane cursor (M5) |
| `PaneScrollUp` | `scroll_up` | `["K"]` | `scroll up` | false | `backlog/detail/mod.rs:448` | as above |
| `PanePageDown` | `page_down` | `["pgdn"]` | `page down` | false | `backlog/detail/mod.rs:449`, `graph.rs:697`, `divergence.rs:261` | `runs.rs:1467` is a no-op in flow view |
| `PanePageUp` | `page_up` | `["pgup"]` | `page up` | false | `backlog/detail/mod.rs:450` | as above |
| `PaneNextSubtab` | `next_subtab` | `["l", "]", "right"]` | `next sub-tab` | false | `backlog/mod.rs:659` | `settings/mod.rs:340` binds the same chords to sections (`settings.next_section`, M3); `filter.rs:253` `l`/`Right` are choice-field value keys (fixed) |
| `PanePrevSubtab` | `prev_subtab` | `["h", "[", "left"]` | `previous sub-tab` | false | `backlog/mod.rs:660` | `settings/mod.rs:344` (sections) |

**`[confirm]`**

| `Act` | name | defaults | help | in_capture | mirrors | disagrees |
|---|---|---|---|---|---|---|
| `ConfirmYes` | `yes` | `["y"]` | `yes` | false | `connection.rs:483`, `qdrant.rs:214`, `kinds.rs:715`, `hierarchy.rs:769`, `personas.rs:785`, `boxes.rs:545`, `runs.rs:583`, `detail/requirements.rs:306` | `migration_prompt.rs:92` also takes `Y`; `attach.rs:608` cancels on any other key |
| `ConfirmNo` | `no` | `["n", "esc"]` | `no` | false | `connection.rs:487`, `qdrant.rs:218`, `kinds.rs:726`, `agents.rs:1161`, `boxes.rs:546`, `personas.rs:791`, `runs.rs:587`, `detail/requirements.rs:317`, `hierarchy.rs:761` | `migration_prompt.rs:97` also takes `N`, and its `Esc` is `overlay.close` |

**`[form]`** (all `in_capture: true`; every chord is named or CONTROL)

| `Act` | name | defaults | help | mirrors | disagrees |
|---|---|---|---|---|---|
| `FormNextField` | `next_field` | `["tab"]` | `next field` | `item_form.rs:613`, `requirements/forms.rs:306`, `compose.rs:340` | the Settings and attach forms also take `Down` (`agents.rs:2170`, `hierarchy.rs:870`, `kinds.rs:777`, `personas.rs:1580`, `attach.rs:488`); ANA §6.4 allows `Up`/`Down` there, added per view in M3/M4 |
| `FormPrevField` | `prev_field` | `["backtab"]` | `previous field` | `item_form.rs:617`, `requirements/forms.rs:310`, `compose.rs:344` | same `Up` alias |
| `FormSave` | `save` | `["ctrl-s"]` | `save` | `text_area.rs:194`, `item_form.rs:275`, `requirements/mod.rs:233`, `attach.rs:477`, `library.rs:939` | every site also accepts `ctrl-S` (F-7) |
| `FormExternalEditor` | `external_editor` | `["ctrl-e"]` | `$EDITOR` (`item_form.rs:63`'s label) | `item_form.rs:281`, `library.rs:1021`, `templates.rs:689` | same `E` alias (F-7) |

**`[common]`**

| `Act` | name | defaults | help | in_capture | mirrors | disagrees |
|---|---|---|---|---|---|---|
| `Edit` | `edit` | `["e"]` | `edit` | false | `connection.rs:779`, `qdrant.rs:382`, `kinds.rs:1468`, `hierarchy.rs:1241`, `prompt.rs:849`, `agents.rs:2401`, `backlog/mod.rs:680` | `boxes.rs:707` `e` = edit quirks (`settings.boxes.edit_quirks`, ANA §6.2); `requirements/mod.rs:448` `e` amends via `write_key` |
| `New` | `new` | `["n"]` | `new` | false | `kinds.rs:1452`, `hierarchy.rs:1233`, `agents.rs:2395`, `library.rs:694`, `templates.rs:527` | `backlog/mod.rs:679` uses `N`; `kinds.rs:1460` `N` = new graph; `requirements/mod.rs:448` `a`/`n` |
| `Delete` | `delete` | `["d"]` | `delete` | false | `hierarchy.rs:1278`, `kinds.rs:1484` | - |
| `Clear` | `clear` | `["c"]` | `clear` | false | `connection.rs:785`, `qdrant.rs:388` | `detail/requirements.rs:481` `c` = cite picker (view verb) |
| `Reload` | `reload` | `["r"]` | `reload` | false | `connection.rs:822`, `qdrant.rs:414`, `kinds.rs:1505`, `hierarchy.rs:1288`, `prompt.rs:869`, `boxes.rs:733`, `requirements/mod.rs:449`, `library.rs:690`, `templates.rs:523`, `attach.rs:398` | `agents.rs:2495` `r` = probe (`settings.agents.probe`); `detail/requirements.rs:480` `r` = reconfirm |
| `Back` | `back` | `["esc"]` | `back` | false | `divergence.rs:245`, `library.rs:895`, `chat/mod.rs:233` | `personas.rs:841` also takes `Enter` |
| `Dismiss` | `dismiss` | `["esc"]` | `dismiss` | false | `connection.rs:828`, `boxes.rs:739`, `hierarchy.rs:1294`, `kinds.rs:1511`, `prompt.rs:875` | `requirements/mod.rs:444` `Esc` clears the filter (view verb). Shares `Esc` with `Back`: `STATE_GUARDED` (F-2) |

`tab_index`: `SelectTab1` → `Some(0)` … `SelectTab9` → `Some(8)`, everything else `None`.

### 3.3 `in_capture` semantics (B3), in the module doc

"`in_capture` is a property of an action. It means 'offered while a text field captures', so every
default and every user chord of the action must be non-printable (`KeyChord::is_printable`; M2's
validator). The global layer is **not** marked. Under a capturing mode (M3) the global layer is
filtered chord by chord, and printable chords drop out. So `global.help` (`?`, `f1`) reaches help
through `f1` while a field types `?`, and `global.quit` (`q`) is unreachable there. `ctrl-c` quits
regardless."

### 3.4 T2 tests (no parser: T2 does not depend on T1)

| # | Test | Asserts |
|---|---|---|
| 1 | `global_help_is_question_mark_and_f1` | `Act::Help.spec().unwrap().defaults == ["?", "f1"]` |
| 2 | `every_act_has_exactly_one_row` | `CATALOGUE.len() == 41`; the acts are pairwise distinct; every row's `act.spec()` is that row |
| 3 | `names_are_unique_per_context` | no two rows share `(context, name)` |
| 4 | `help_is_never_empty` | every `help` is non-empty |
| 5 | `overlay_close_keeps_a_chord` | `OverlayClose`'s defaults are non-empty (ANA §6.4) |
| 6 | `no_default_spells_ctrl_c` | no default, lower-cased with whitespace removed, is `ctrl-c`, `ctrl+c` or `control-c` |
| 7 | `no_spec_is_shared_in_a_context_unless_state_guarded` | string-level: no two rows of one context share an identical default spec, except the pairs in `STATE_GUARDED` (T3 repeats this at chord level) |
| 8 | `the_global_block_is_in_status_line_order` | global names in order: `quit, next_tab, prev_tab, select_tab_1..9, help, workspaces, find, waiting` |
| 9 | `context_tables_and_headings` | the seven `table()`/`heading()` pairs |
| 10 | `tab_index_maps_the_nine_digits` | 1→0 … 9→8; `Quit` → `None` |
| 11 | `only_the_form_and_overlay_close_are_in_capture` | the `in_capture` set is exactly `{FormNextField, FormPrevField, FormSave, FormExternalEditor, OverlayClose}` |

### 3.5 T2 commits and gate

One or two commits, `feat(mod-67): T2 action catalogue (global, overlay, shared contexts)`. Gate:
`cargo test -p htui --lib -- keys::catalogue`, clippy both ways.

---

## 4. T3: `Keys`, stacks, hints (D6-D8; B5-B9)

**First failing test**: `keys::tests::every_catalogue_default_parses_strictly`.

### 4.1 `crates/htui/src/keys/mod.rs`

```rust
pub mod catalogue;
pub mod chord;
pub mod hint;
pub mod stack;

pub use catalogue::{Act, ActionSpec, CATALOGUE, Context, STATE_GUARDED};
pub use chord::{CTRL_C, ChordError, KeyChord};
pub use hint::{HelpLine, Hint, HintSpec};
pub use stack::{Layer, Stack};

/// The keys in force: every catalogue action's chords per context (MOD-67 D9, D10). Built once
/// from the compiled defaults (`Keys::compiled`); M2 builds one from `keys.toml` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys {
    /// One row per `(context, act)`, in catalogue order. A row with no chords is an unbound
    /// action. It still shadows the same act in wider layers (M2's narrower overrides).
    rows: Vec<Row>,
}

/// One binding row (private: no `pub` doc may link it).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    context: Context,
    act: Act,
    help: &'static str,
    chords: Vec<KeyChord>,
}

static COMPILED: std::sync::LazyLock<Keys> = std::sync::LazyLock::new(Keys::defaults);

impl Keys {
    /// The compiled-in defaults, parsed once per process. `Ctx::new` hands these to every view
    /// that is not given `App`'s own (D9).
    #[must_use]
    pub fn compiled() -> &'static Self { &COMPILED }

    /// A fresh table of the catalogue defaults, each parsed with `KeyChord::parse_strict`.
    ///
    /// # Panics
    /// If a default does not parse. `every_catalogue_default_parses_strictly` pins that none fails.
    #[must_use]
    pub fn defaults() -> Self;   // panic!("catalogue default {spec:?} of [{table}] {name}: {err}")

    /// The chords of `act` in exactly `context`, or `&[]` if there is no row or it is unbound.
    #[must_use]
    pub fn chords(&self, context: Context, act: Act) -> &[KeyChord];

    /// Test-only override, the M2 merge in miniature: replaces the `(context, act)` row's chords,
    /// or appends a row if none exists (a narrower override). `specs` parse strictly.
    #[cfg(test)]
    pub(crate) fn with_chords(mut self, context: Context, act: Act, specs: &[&str]) -> Self;

    /// The row of `act` as the stack sees it: in the first layer that admits `act` and has a
    /// row for it.
    fn resolve_row(&self, stack: Stack<'_>, act: Act) -> Option<&Row>;  // private helper
}
```

`keys/mod.rs`'s `//!` doc keeps T0's line. Add: "D6's dispatch order lives in `App::on_key`.
This module only answers 'which actions does this chord name in this stack' and 'how is this action
labelled'." The `in_capture` paragraph (§3.3) goes in `catalogue.rs`'s module doc.

### 4.2 `crates/htui/src/keys/stack.rs`

```rust
/// One layer of a stack: a context, optionally narrowed to some of its actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    context: Context,
    only: Option<&'static [Act]>,
}
impl Layer {
    /// Every action of `context`.
    #[must_use] pub const fn all(context: Context) -> Self;
    /// Only `acts` of `context`: how the overlay stack lets `global.help` through and nothing
    /// else of the global layer (D6 step 2).
    #[must_use] pub const fn only(context: Context, acts: &'static [Act]) -> Self;
    /// The context.
    #[must_use] pub const fn context(self) -> Context;
    /// Whether this layer offers `act`.
    #[must_use] pub fn admits(self, act: Act) -> bool;
}

/// A context stack, narrowest layer first (ANA-26 §7.3). M1 needs only the two constants below;
/// M3-M5 compose view stacks as slices of `Layer`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stack<'a>(&'a [Layer]);
impl<'a> Stack<'a> {
    /// A stack over `layers`, narrowest first.
    #[must_use] pub const fn new(layers: &'a [Layer]) -> Self;
    /// The layers, narrowest first.
    #[must_use] pub const fn layers(self) -> &'a [Layer];
}
impl Stack<'static> {
    /// `[global]`: D6 step 6, the status line and the `?` box's global line.
    pub const BASE: Self = Self(&[Layer::all(Context::Global)]);
    /// `[overlay, global ∩ {help}]`: D6 step 2. `Esc` closes and `?`/`F1` toggle help over a
    /// modal overlay, for keys the overlay passed.
    pub const OVERLAY: Self =
        Self(&[Layer::all(Context::Overlay), Layer::only(Context::Global, &[Act::Help])]);
}

impl Keys {
    /// The **ordered candidates** for `chord` in `stack` (ANA-26 §7.3): every action whose row in
    /// the first layer that has one binds `chord`, narrowest layer first, catalogue order within
    /// a layer. A row in a narrower layer **shadows** the same act in every wider layer, even
    /// when it is unbound or binds other chords. The caller handles the first candidate it
    /// accepts. `CTRL_C` is never a candidate.
    #[must_use]
    pub fn actions(&self, stack: Stack<'_>, chord: KeyChord) -> Vec<Act>;
}
```

The `actions` body (normative):

```rust
let mut seen: Vec<Act> = Vec::new();
let mut out = Vec::new();
for layer in stack.layers() {
    let mut here = Vec::new();
    for row in self.rows.iter().filter(|r| r.context == layer.context && layer.admits(r.act)) {
        if seen.contains(&row.act) {
            continue; // shadowed by a narrower layer
        }
        here.push(row.act);
        if row.chords.contains(&chord) {
            out.push(row.act);
        }
    }
    seen.extend(here);
}
out
```

### 4.3 `crates/htui/src/keys/hint.rs`

```rust
/// One element of a hint spec (ANA-26 §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// `"{label} {text}"`, e.g. `e edit DSN`.
    One(Act, &'static str),
    /// `"{label}/{label} {text}"`, e.g. `j/k rows`; one side unbound renders the other alone.
    Pair(Act, Act, &'static str),
}

/// A view's hint row, as data: `const BROWSE: HintSpec = &[...]`.
pub type HintSpec = &'static [Hint];

/// One logical line of the `?` box: a heading and its entries (D8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpLine {
    /// `Global`, `Overlay`, or a tab's title for its legacy rows.
    pub heading: String,
    /// `"{chords} {help}"` entries, e.g. `q/Ctrl+c quit`.
    pub entries: Vec<String>,
}
impl HelpLine {
    /// A line from its parts (the shell builds the legacy tab line with it).
    #[must_use] pub fn new(heading: impl Into<String>, entries: Vec<String>) -> Self;
    /// `"{heading}: {entries joined by " · "}"`.
    #[must_use] pub fn text(&self) -> String;
    /// The line packed into rows at most `width` cells wide (`unicode-width`), breaking only
    /// between entries. Continuation rows start with two spaces. A single entry wider than
    /// `width` gets a row of its own (and is clipped when drawn).
    #[must_use] pub fn rows(&self, width: usize) -> Vec<String>;
}

impl Keys {
    /// The first chord of `act` through `stack`, as `KeyChord::label` writes it; `None` when it
    /// is unbound. Prose such as `format!("press {}", …)` uses it from M3 on.
    #[must_use] pub fn label(&self, stack: Stack<'_>, act: Act) -> Option<String>;

    /// A view's hint row: each element through `stack`, first chord only, unbound dropped,
    /// joined by ` · ` (ANA-26 §7.6).
    #[must_use] pub fn hint(&self, stack: Stack<'_>, spec: HintSpec) -> String;

    /// The status line (D7): the `Global` rows that are `offered` and bound, in catalogue order,
    /// the first chord of each, rows sharing a help label collapsed to the first (the digits),
    /// joined by ` · `.
    #[must_use] pub fn status_line(&self, offered: impl Fn(Act) -> bool) -> String;

    /// One `?` box line for `context` (D8): every offered and bound action, all its chords
    /// joined by `/`, rows sharing a help label merged into one entry. `Quit` always appears and
    /// ends with the fixed `Ctrl+c`. `None` if the context offers nothing.
    #[must_use] pub fn help_line(&self, context: Context, offered: impl Fn(Act) -> bool)
        -> Option<HelpLine>;

    /// The box's last line, from `global.help`'s chords: `"?/F1 closes this box"`; `None` if
    /// help is unbound.
    #[must_use] pub fn help_closer(&self) -> Option<String>;
}
```

**Chord-list rendering** for one merged entry (private `fn chord_list(chords: &[KeyChord]) ->
String`): if there are **3 or more** chords, all plain `Char` with no modifiers, and consecutive
ascending (`c[i+1] as u32 == c[i] as u32 + 1`), write `"{first}-{last}"`. Otherwise join the labels
with `/`. With the defaults, the digit group is `1-9` and everything else is `/`-joined.

**Exact default strings** (T3 asserts each one verbatim):

| Call | Result |
|---|---|
| `status_line(bare)`, where `bare` = every act except `Workspaces`, `Find`, `Waiting` | `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help` |
| `status_line(all)` | `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help · w workspaces · Ctrl+f find · Ctrl+w waiting` |
| `help_line(Global, bare).text()` | `Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab · ?/F1 help` |
| `help_line(Global, all).text()` | `Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab · ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting` |
| `help_line(Overlay, all).text()` | `Overlay: Esc close` |
| `help_closer()` | `Some("?/F1 closes this box")` |
| `help_line(Global, all).rows(88)` | `["Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab", "  ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting"]` |
| `hint(Stack::new(&[Layer::all(List)]), &[Hint::Pair(ListDown, ListUp, "rows")])` | `j/k rows` |

The status line's middle dot is `U+00B7` with a space on each side, as `Keymap::help_line` writes
it (`keymap.rs:317`). With `register_all` the 116-cell status line is cut at the 100-column
terminal exactly as today (Verified claim 2).

### 4.4 T3 tests, TDD order

`keys/mod.rs` (`mod tests`):

| # | Test | Asserts |
|---|---|---|
| 1 | `every_catalogue_default_parses_strictly` | every `CATALOGUE` default is `Ok` under `parse_strict` (message names `[table] name = "spec"`) |
| 2 | `no_two_actions_in_one_context_share_a_default_chord` | at chord level, per context, except `STATE_GUARDED` pairs (PD-1) |
| 3 | `no_in_capture_action_has_a_printable_chord` | for `in_capture` rows, no parsed default `is_printable()` |
| 4 | `the_compiled_keys_are_the_catalogue_and_never_ctrl_c` | `Keys::compiled().chords(ctx, act)` equals the parsed defaults for every row; no row contains `CTRL_C` |
| 5 | `compiled_is_built_once` | `std::ptr::eq(Keys::compiled(), Keys::compiled())`; `*Keys::compiled() == Keys::defaults()` |

`keys/stack.rs`:

| # | Test | Asserts |
|---|---|---|
| 6 | `the_base_stack_resolves_quit_tabs_digits_help_and_the_offered_globals` | (replaces `keymap.rs` `the_global_table_binds_quit_tabs_digits_and_help`) `q`→`[Quit]`, `tab`→`[NextTab]`, `shift-tab`→`[PrevTab]`, `3`→`[SelectTab3]`, `?`→`[Help]`, `f1`→`[Help]`, `w`→`[Workspaces]`, `ctrl-f`→`[Find]`, `ctrl-w`→`[Waiting]`, `z`/`esc`→`[]` |
| 7 | `ctrl_c_is_no_action_in_either_stack` | (replaces `ctrl_c_quits_globally_and_from_any_overlay`, unit half) `actions(BASE, CTRL_C)` and `actions(OVERLAY, CTRL_C)` are empty |
| 8 | `the_overlay_stack_lets_only_help_through_from_global` | `esc`→`[OverlayClose]`, `?`/`f1`→`[Help]`, and `q`, `tab`, `1`, `w` → `[]` |
| 9 | `candidates_come_narrowest_first` | `Stack::new(&[Layer::all(Confirm), Layer::all(Common)])` with `esc` → `[ConfirmNo, Back, Dismiss]` |
| 10 | `a_narrower_row_shadows_the_shared_one` | `Keys::defaults().with_chords(Context::Form, Act::Reload, &["f5"])`; stack `[Form, Common]`: `f5`→`[Reload]`, `r`→`[]` |
| 11 | `an_unbound_action_is_never_a_candidate` | `with_chords(Global, Quit, &[])`: `actions(BASE, q) == []` |

`keys/hint.rs`:

| # | Test | Asserts |
|---|---|---|
| 12 | `the_status_line_without_register_all_is_todays` | (replaces `the_help_line_collapses_the_digit_rows`) bare string, §4.3 |
| 13 | `the_status_line_with_every_global_offered` | full string, §4.3 |
| 14 | `an_unbound_action_drops_out_of_the_status_line_and_a_hint` | `with_chords(Global, Quit, &[])`: the status line starts `Tab next tab`; `hint(BASE, &[One(Quit,"quit"), One(Help,"help")]) == "? help"` |
| 15 | `a_pair_renders_both_labels_or_the_bound_one` | `j/k rows`; with `ListUp` unbound, `j rows`; both unbound, `""` |
| 16 | `label_takes_the_first_chord_through_the_stack` | `label(BASE, Help) == Some("?")`; with help = `["f1"]`, `Some("F1")`; unbound, `None`; `label(BASE, OverlayClose) == None` (not admitted) |
| 17 | `the_help_lines_follow_d8` | the four `help_line`/`help_closer` strings of §4.3 |
| 18 | `quit_lists_ctrl_c_even_when_q_is_unbound` | `with_chords(Global, Quit, &[])` → global line starts `Global: Ctrl+c quit · ` |
| 19 | `a_broken_digit_run_is_listed_chord_by_chord` | `with_chords(Global, SelectTab3, &["x"])` → entry `1/2/x/4/5/6/7/8/9 select tab` |
| 20 | `rows_break_between_entries_only` | `HelpLine::new("Global", ["aaa","bbb","ccc"]).rows(16) == ["Global: aaa", "  bbb · ccc"]`; the 88-column case of §4.3 |

### 4.5 Visibility and dead code

Every item above is `pub` in the `pub mod keys` tree, so none of it is dead in a featureless lib
build, even though no view calls `hint`/`label` in M1. `with_chords` is `#[cfg(test)]`. `Row` and
`resolve_row` are private and used by `stack.rs`/`hint.rs`, which are descendant modules and can
see them. Do not make anything `pub(crate)` outside `cfg(test)`.

### 4.6 Clippy watch-list for T3

`clippy::all` only. Relevant: `redundant_closure` (write `.filter_map(Act::spec)` style),
`manual_find`, `needless_lifetimes` on `impl<'a> Stack<'a>`, `uninlined_format_args` (inline
anyway), `wrong_self_convention` (`is_*`/`spec` take `&self`, as `label` does).
`missing_docs` applies to all `pub` items and fields. `HelpLine`'s fields are `pub` and documented.

### 4.7 T3 commits and gate

1. `test(mod-67): T3 resolver, stack and hint cases` (red, behind `todo!()` bodies).
2. `feat(mod-67): Keys, Stack, ordered candidates, status line and help lines`.

Gate: `cargo test -p htui --lib -- keys`, clippy both ways, `cargo doc -p htui --no-deps`.

---

## 5. T4: shell integration (D5, D6, D8-D10; PD-2; B9-B12)

**First failing test**: `tests/keys.rs::ctrl_c_quits_from_connection_browse_with_a_stored_dsn`.

### 5.1 `App` (`app/state.rs`)

Fields (after `keymap`, `state.rs:~150`):
```rust
/// The legacy tab rows: the six Backlog rows `register_all` binds (MOD-67 D2). Global and
/// overlay keys are `keys`' since MOD-67.
pub keymap: Keymap,                // doc changes only
/// The named-action keys in force (MOD-67 D10): the compiled defaults; M2 builds them from
/// `keys.toml`.
pub keys: Keys,
/// Global actions that open a named view, offered by `register_all` (D5). An action not offered
/// is neither dispatched nor shown.
pub(super) offered: Vec<(Act, Action)>,
```
`App::new` (`state.rs:226`) keeps its signature. Add `keys: Keys::compiled().clone(), offered:
Vec::new()`.

New methods (after `push_overlay`, `state.rs:290`):
```rust
/// Offers a global action that names a view (MOD-67 D5), replacing an earlier offer of `act`.
/// Only `register_all` calls it: the shell itself never names a concrete overlay.
pub fn offer(&mut self, act: Act, action: Action) {
    debug_assert!(act.spec().is_some_and(|s| s.context == Context::Global));
    self.offered.retain(|(known, _)| *known != act);
    self.offered.push((act, action));
}

/// What the shell does for `act`: the fixed mapping, then the offered table. `None` for
/// everything a view must handle (every shared context) and for an offerable act nobody offered.
fn action_for(&self, act: Act) -> Option<Action> {
    match act {
        Act::Quit => Some(Action::Quit),
        Act::NextTab => Some(Action::Tab(TabAction::Next)),
        Act::PrevTab => Some(Action::Tab(TabAction::Prev)),
        Act::Help => Some(Action::ToggleHelp),
        Act::OverlayClose => Some(Action::Overlay(OverlayAction::Close)),
        other => other
            .tab_index()
            .map(|index| Action::Tab(TabAction::Select(index)))
            .or_else(|| {
                self.offered
                    .iter()
                    .find(|(known, _)| *known == other)
                    .map(|(_, action)| action.clone())
            }),
    }
}

/// D6 steps 2 and 6: the first candidate the shell maps to an action wins. `true` if one did.
fn apply_keys(&mut self, stack: Stack<'_>, chord: KeyChord) -> bool {
    let action = self
        .keys
        .actions(stack, chord)
        .into_iter()
        .find_map(|act| self.action_for(act));
    match action {
        Some(action) => {
            self.update(action);
            true
        }
        None => false,
    }
}
```

### 5.2 `App::on_key` (`state.rs:616-711`), new body

Doc: "The propagation chain (blueprint C.4, MOD-67 D6), stopping at the first consumer: `ctrl-c`,
the top overlay, the overlay stack, the modal swallow, the active tab, its legacy rows, the base
stack."

```rust
pub fn on_key(&mut self, key: KeyEvent) {
    self.dirty = true;
    self.status = None;                         // unchanged
    let chord = KeyChord::from_event(key);

    // 0. `ctrl-c` quits before any overlay or view sees it (ANA-26 §6.4, MOD-67 D6). MOD-57 adds
    //    the one exception (a focused child-process pane) and its leave action.
    if chord == CTRL_C {
        self.update(Action::Quit);
        return;
    }

    if let Some(id) = self.overlays.top().map(Overlay::id) {
        let origin = Origin::Overlay(id);
        let handled = {
            let Self { scope, projects, top_bar, keymap, keys, theme, emit, overlays, .. } = self;
            match overlays.top_mut() {
                Some(top) => {
                    let mut ctx = Ctx::new(scope, projects, top_bar, keymap, theme,
                                           origin.clone(), emit).with_keys(keys);
                    top.on_key(key, &mut ctx)            // 1.
                }
                None => Handled::Pass,
            }
        };
        self.drain(&origin);
        if handled == Handled::Consumed {
            return;
        }
        // 2. `Esc` closes; `?`/`F1` toggle help over any overlay, for a key the overlay passed.
        if self.apply_keys(Stack::OVERLAY, chord) {
            return;
        }
        // 3. A modal overlay swallows what it did not handle.     (unchanged)
        if self.overlays.top().is_some_and(Overlay::is_modal) {
            return;
        }
    }

    if let Some(id) = self.tabs.active_id() {
        // 4. unchanged, except the destructure gains `keys` and the ctx `.with_keys(keys)`
        // 5. unchanged: `self.keymap.resolve(&KeyScope::Tab(id), chord)` (state.rs:700)
    }

    // 6. The base stack.
    self.apply_keys(Stack::BASE, chord);
}
```

The old overlay-scope lookup (`state.rs:657`) and global lookup (`state.rs:706`) are deleted.
`let Self { keys, .. } = self` binds `&mut Keys`. Passing it to `with_keys(&'a Keys)` reborrows
it shared, which is the same borrow-split as today's `keymap`.

### 5.3 `Ctx` (`state.rs:75-131`) and its 11 construction sites

```rust
pub struct Ctx<'a> {
    // existing fields unchanged
    /// The keys in force (MOD-67 D9): `App`'s own at the shell's sites, else the compiled
    /// defaults. No view reads it in M1.
    keys: &'a Keys,
    origin: Origin,
    emit: &'a Emit,
}
// Ctx::new: signature unchanged; body adds `keys: Keys::compiled(),`
/// Hands the view these keys instead of the compiled defaults: how `App` passes its own.
#[must_use]
pub fn with_keys(mut self, keys: &'a Keys) -> Self { self.keys = keys; self }
/// The keys in force, for hint rows and labels (M3-M5).
#[must_use]
pub fn keys(&self) -> &Keys { self.keys }
```

Every site gains `keys` in its `let Self { … }` destructure and `.with_keys(keys)` on the
`Ctx::new(..)` expression. `App::ctx` uses `.with_keys(&self.keys)`.

| File:line (`Ctx::new(`) | Function |
|---|---|
| `app/state.rs:411` | `finish_external_edit` |
| `app/state.rs:467` | `on_paste` (overlay) |
| `app/state.rs:501` | `on_paste` (tab) |
| `app/state.rs:594` | `on_mouse` |
| `app/state.rs:639` | `on_key` (overlay) |
| `app/state.rs:682` | `on_key` (tab) |
| `app/state.rs:714` | `App::ctx` (render's `Ctx`) |
| `app/update.rs:160` | `refresh_active_tab` |
| `app/update.rs:262` | `reveal` |
| `app/update.rs:307` | `on_reply` (tab) |
| `app/update.rs:336` | `on_reply` (overlay) |

Not touched: the `Ctx::new` calls in test benches under `ui/` and `testkit.rs:687` (`SectionBench`).
They get the compiled defaults through `Ctx::new` (D9).

### 5.4 `render` and `render_help` (D7, D8, B9)

`render` (`state.rs:752-756`):
```rust
let (status, style) = match &self.status {
    Some(message) => (message.clone(), self.theme.error),
    None => (self.keys.status_line(|act| self.action_for(act).is_some()), self.theme.dim),
};
```

`render_help` (`state.rs:768-793`), new body:
```rust
/// The `?` box (MOD-67 D8), rebuilt from the live state every frame: the overlay context (if
/// one is up), the active tab's legacy rows, the global context, then the closing line.
fn render_help(&self, frame: &mut Frame<'_>, area: Rect) {
    let box_width = area.width.saturating_sub(10).max(20);
    let inner = usize::from(box_width.saturating_sub(2));
    let offered = |act| self.action_for(act).is_some();
    let mut lines: Vec<HelpLine> = Vec::new();
    if !self.overlays.is_empty() {
        lines.extend(self.keys.help_line(Context::Overlay, offered));
    }
    if let Some(tab) = self.tabs.active() {
        let legacy = self.keymap.help_line(&KeyScope::Tab(tab.id()));
        if !legacy.is_empty() {
            lines.push(HelpLine::new(tab.title(), legacy.split(" · ").map(str::to_owned).collect()));
        }
    }
    lines.extend(self.keys.help_line(Context::Global, offered));
    let mut rows: Vec<Line<'_>> = lines
        .iter()
        .flat_map(|line| line.rows(inner))
        .map(|row| Line::styled(row, self.theme.base))
        .collect();
    if let Some(closer) = self.keys.help_closer() {
        rows.push(Line::styled(closer, self.theme.dim));
    }
    let height = u16::try_from(rows.len()).unwrap_or(u16::MAX).saturating_add(2).min(area.height);
    let box_area = layout::centered(area, box_width, height);
    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(Text::from(rows)).block(Block::new().borders(Borders::ALL).title(" Keys ")),
        box_area,
    );
}
```
`offered` captures only `&self`, so the closure is `Copy` and can be passed twice. Remove `Wrap`
from the `ratatui::widgets` import (`state.rs:11`): it has no other user (F-5).

At 100×30 with `register_all` on the Backlog, the box shows these rows (inside width 88):

```
Backlog: Enter replay step · m open graph · f filter · F clear filter · N new item
  e edit item
Global: q/Ctrl+c quit · Tab next tab · Shift+Tab previous tab · 1-9 select tab
  ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting
?/F1 closes this box
```

### 5.5 `register_all` diff (`app/mod.rs:57-170`)

Replace the three `app.keymap.bind(Binding { scope: KeyScope::Global, … })` blocks with:

| Today | After |
|---|---|
| `app/mod.rs:82-87` (`w`) | `app.offer(Act::Workspaces, Action::Overlay(OverlayAction::Open(WorkspaceSwitcher::ID)));` |
| `app/mod.rs:93-98` (`ctrl-f`) | `app.offer(Act::Find, Action::Overlay(OverlayAction::Open(ConceptsSearch::ID)));` |
| `app/mod.rs:104-109` (`ctrl-w`) | `app.offer(Act::Waiting, Action::Overlay(OverlayAction::Open(WaitingList::ID)));` |

Each `overlay_factories.register` line stays immediately above its `offer`. The four Backlog
`KeyScope::Tab` rows (`:129-164`) are unchanged. Imports: add `use crate::keys::Act;`. `Binding`,
`KeyChord`, `KeyScope`, `KeyCode` and `KeyModifiers` are still used by the Backlog rows.

Doc comment updates:
- item 2 (`:30-34`): "global `w` is **offered** (`App::offer(Act::Workspaces, …)`, MOD-67 D5)
  rather than mapped by the shell, because it names an overlay, and the shell must not know which
  views exist". Drop the `Keymap::default_global` link.
- item 7 (`:44-45`): `Ctrl+F` is offered as `Act::Find`.
- item 11 (`:52-53`): `Ctrl+W` is offered as `Act::Waiting`. Drop "MOD-67 makes it a named
  action", which is now done.

### 5.6 `keymap.rs` in T4 (D2, D3, PD-2)

`default_global` body becomes `Self::new()`, with this doc:

```rust
/// An empty table (MOD-67 M1, plan D2).
///
/// It held the global and overlay rows until MOD-67. `q`, `Tab`, `Shift+Tab`, `1`..`9`, `?`,
/// `Esc` and `ctrl-c` are now catalogue actions (`crate::keys::catalogue`), dispatched by
/// `App::on_key` through the resolver, and `ctrl-c` is checked before anything else. The name
/// stays because the test benches of M3-M5's files call it; M6 deletes it with `Keymap`.
#[must_use]
pub fn default_global() -> Self { Self::new() }
```

- Module doc (`keymap.rs:1-6`): "Legacy key rows (MOD-1 plan D5), shrinking. Since MOD-67 M1 the
  global and overlay keys are named actions in `crate::keys`. What remains are the six Backlog tab
  rows `register_all` binds: the help box's half of five arms, and the `Enter` miss. M5 moves them
  to the `backlog` context; M6 deletes this module."
- Non-test imports become `Action`, `OverlayId`, `TabId` and the `KeyChord` re-export. Drop
  `KeyCode`, `KeyModifiers`, `OverlayAction` and `TabAction` (`unused_imports`).
- Tests. **Delete** `the_global_table_binds_quit_tabs_digits_and_help`,
  `ctrl_c_quits_globally_and_from_any_overlay` and `the_help_line_collapses_the_digit_rows`.
  Their replacements are T3 tests 6, 7 and 12 plus `tests/keys.rs` 3. **Rewrite** the three
  mechanics tests to start from `Keymap::new()` and bind their own rows:
  - `an_unknown_chord_and_a_foreign_scope_resolve_to_none`: bind Global `q` → `Quit`; `z` and
    `esc` resolve to nothing; `Tab(backlog)` with `q` resolves to nothing.
  - `every_overlay_inherits_esc_and_can_shadow_it`: bind `Overlay(ANY)` `esc` → `Close` first,
    then the existing asserts.
  - `the_newest_binding_of_a_scope_wins`: bind Global `q` → `Quit`, then `q` → `ToggleHelp`.

  **Add** `the_default_table_is_empty_since_the_catalogue_owns_those_keys`:
  `Keymap::default_global().help_line(&KeyScope::Global).is_empty()` and the same for
  `Overlay(ANY)`. Final count: 4 tests in `keymap.rs` (9 before; 3 moved to `chord.rs`, 3
  replaced in `keys/`, 3 rewritten, 1 new).

### 5.7 `crates/htui/tests/keys.rs` (new, `#![cfg(feature = "testkit")]`), written first

Common helper: `async fn shell(register: bool) -> Harness`. It builds `Harness::demo()` with
`.with_agent_runtime(AgentRuntime::new(DriverFactory::new()))`. Without `register_all` it adds
`.with_tab(Box::new(BacklogTab::new()))`. It calls `register_all` when asked, then
`drive_to_end().await`. This mirrors `tests/backlog.rs:58-64` (the runtime keeps prompt previews
off the status line) and `tests/shell.rs:30-47`.

| # | Test | Setup it mirrors | Asserts |
|---|---|---|---|
| 1 | `ctrl_c_quits_from_connection_browse_with_a_stored_dsn` | `_keyring = common::mock_keyring().await` and `secret::set_dsn(DEAD_DSN)` (`tests/connection.rs:489-492`); tempdir + `CacheStore::open` + `Backend::Offline` (`connection.rs:1953-1967`); a Backlog + `SettingsTab::with_sections(vec![ConnectionSection])` shell (`connection.rs:1187-1191`); `FocusSection(SettingsTab::ID, ConnectionSection::ID)` + `settle` (`connection.rs:1927-1931`) | the frame shows the section in browse mode; `key("ctrl-c")` sets `should_quit`; the frame does **not** contain `Remove the DSN from the keyring?` (the confirm today's `c` arm, `connection.rs:785-790`, would open) |
| 2 | `ctrl_c_quits_from_qdrant_browse` | `shell(true)`, then `FocusSection(SettingsTab::ID, QdrantSection::ID)` + `drive_to_end` | the frame shows Qdrant's rows with no field open (browse: `qdrant.rs:357-359`); `key("ctrl-c")` → `should_quit`. Today `qdrant.rs:388` consumes it. |
| 3 | `ctrl_c_quits_over_every_modal_overlay` | four fresh harnesses: `w` (switcher), `ctrl-f` (concepts, then type `ab`), `ctrl-w` (waiting list), and `Harness::demo().with_store_state("online", Some(1))` + `register_all` (migration prompt, `tests/shell.rs:30`) | each has the expected `overlays.top().id()`; `key("ctrl-c")` → `should_quit` |
| 4 | `f1_opens_and_closes_help_on_the_backlog` | `shell(true)` | `f1` → `help_visible`; `f1` → `!help_visible` |
| 5 | `question_mark_and_f1_toggle_help_over_the_workspace_switcher` | `shell(true)` + `key("w")` | `?` → `help_visible` with the switcher still on top; `?` → closed; `f1` → open, and the frame contains `Overlay: Esc close` and `?/F1 closes this box`; `esc` closes the switcher (`overlays.is_empty()`) |
| 6 | `question_mark_still_toggles_help_without_register_all` | `shell(false)`; the pattern of `tests/backlog.rs:1185-1192` | `?` opens, `?` closes |
| 7 | `a_question_mark_typed_into_the_concepts_search_is_text_and_f1_is_help` | `shell(true)` + `ctrl-f`; the pattern of `tests/concepts_search.rs:181-192` | `?` → `!help_visible`, frame contains `query ?`; `f1` → `help_visible`, search still on top |
| 8 | `w_is_inert_without_register_all_and_opens_the_switcher_with_it` | `shell(false)` / `shell(true)` | bare: `w` → `overlays.is_empty()`; full: top id is `WorkspaceSwitcher::ID` |
| 9 | `the_help_box_lists_ctrl_w_waiting_only_with_register_all` | `shell(false)` / `shell(true)`, `?`, `render()` | both frames contain `?/F1 closes this box` and `1-9 select tab`; only the full one contains `Ctrl+w waiting` (the status line is cut before it at 100 columns, so a match is the box) |
| 10 | `the_status_line_is_todays_in_both_harnesses` | `shell(false)` / `shell(true)`, `render()` | the last non-empty frame line is the bare string, or the full string cut at 99 cells (`… · Ctrl+f find`) |

Imports for test 1: `htui_store::testkit as common`, `htui_store::{Backend, CacheStore, PgStore,
secret}`, `chrono::Utc` and `tempfile` (a normal dependency of `htui`). Copy `DEAD_DSN` from
`connection.rs:66`. Every integration test needs `--features testkit` (or `--all-features`):
without it this file compiles to 0 tests and reports `ok`.

### 5.8 T4 commits and gate

1. `test(mod-67): T4 shell key behaviour (ctrl-c first, f1, offered globals)`: `tests/keys.rs`.
   Tests 1, 2, 4, 5 and 7 fail, so this commit is red; say so in the message.
2. `feat(mod-67): App dispatches global and overlay keys through the resolver`: §5.1-§5.6 in one
   commit (`keymap.rs`'s empty table must land with the resolver, F-1).
3. Optional `docs(mod-67): …` for doc-only follow-ups.

Gate: the plan's Validation block verbatim, plus `cargo test -p htui --all-features --test keys
-- --test-threads=1`. Then `find crates -name '*.snap.new' | wc -l` must print `0`.

---

## 6. Data flow and cross-task contracts

**Data flow (one key press).** The terminal delivers a `KeyEvent`, and `App::on_terminal_event`
passes Press/Repeat to `App::on_key`, which builds a `KeyChord`. Then:
- `CTRL_C` → `Action::Quit`.
- The top overlay's `on_key`.
- `Keys::actions(Stack::OVERLAY, chord)` → `App::action_for` → `update`.
- The modal swallow.
- The tab's `on_key`, then the `Keymap` Tab rows.
- `Keys::actions(Stack::BASE, chord)` → `action_for` → `update`.

**Render.** The status line is `Keys::status_line(offered)`. The `?` box is
`Keys::help_line(Overlay|Global, offered)` plus the `Keymap::help_line(Tab)` rows, packed by
`HelpLine::rows`. Views get `ctx.keys()`, which is unused in M1.

| Contract | Producer | Consumer | Pinned by |
|---|---|---|---|
| `KeyChord` at `crate::keys::chord::KeyChord`; `parse_strict`, `CTRL_C`, `is_printable`, `spec` | T1 | T3, T4 | T1 tests 4-17 |
| `Act` (41), `Context` (7), `ActionSpec`, `CATALOGUE`, `STATE_GUARDED`, `Act::tab_index`, `Act::spec` | T2 | T3, T4 | T2 tests 1-11 |
| `crate::keymap::KeyChord` keeps working | T1 | `testkit.rs:27`, `app/*`, `htui::keymap::KeyScope` users | build |
| `Keys::{compiled, defaults, chords, actions, label, hint, status_line, help_line, help_closer}`, `Stack::{BASE, OVERLAY}`, `HelpLine::{new, text, rows}` | T3 | T4 | T3 tests |
| `Ctx::new` signature unchanged | T4 | 22 call sites in 11 files (ANA §8) | build |
| `App::new` signature unchanged | T4 | 5 sites (Verified claim 4) | build |

---

## 7. Hazards

| # | Hazard | Mitigation |
|---|---|---|
| H-1 | **A status-line byte drifts**, failing ~106 snapshots: label spelling (`Shift+Tab`, `Ctrl+f`), separator (` · ` with U+00B7), order, or the digit collapse. | T3 tests 12-13 assert both strings verbatim before T4 wires them; T4 test 10 asserts them on screen; the gate requires zero `.snap.new` (snapshots: `tests/snapshots` 72 bare + 34 full). |
| H-2 | **The `?` box toggling while visible.** `?` closes only where it reaches D6 step 2 or 6. Over a capturing field `?` is text. `f1` closes it there unless the field swallows `f1`, which Settings editors do (`connection.rs:427-428`: non-CONTROL `Pass` → `Consumed`). The user leaves the field (`Esc`) first. `Esc` never closes the box (unchanged); over an overlay, `Esc` closes the overlay and leaves the box up (T4 test 5 pins it). | Accepted (plan Risks row 5); M3 gives sections stacks. |
| H-3 | **`ctrl-c` vs `Event::Paste`.** The ctrl-c check is in `on_key` only. `on_paste` (`state.rs:449`) has no key table, so a paste containing U+0003 never quits, and nothing about MOD-22 M-1 changes. In one event batch, a paste followed by `ctrl-c` applies the paste, then quits. | No code; documented in the `on_key` doc. |
| H-4 | **`ctrl-c` first removes every view's chance to see it.** The sweep found no consumer on purpose (Verified claim 9). View tests that call `on_key` directly with `ctrl-c` (`backlog/mod.rs:2017`, `runs.rs:3779`, `text_field.rs:606`, …) bypass `App` and keep passing. | T4 tests 1-3; existing `tests/shell.rs:289`, `tests/hierarchy.rs:1412`, `tests/concepts_search.rs:390` stay green. |
| H-5 | **F-1 sequencing.** If any commit empties `default_global` without the resolver in `on_key`, the whole suite goes red. | PD-2: one T4 commit does both. |
| H-6 | **Featureless dead code** (`cargo clippy --workspace -- -D warnings`). | Everything in `keys/` is `pub`; the only `pub(crate)` item is `#[cfg(test)]`; `action_for`/`apply_keys` are used by non-test code. |
| H-7 | **Lints that bite this change.** `unused_imports` (`keymap.rs`: `KeyEvent` in T1; `KeyCode`/`KeyModifiers`/`OverlayAction`/`TabAction` in T4; `state.rs`: `Wrap` in T4). `missing_docs` on every new `pub` item and field. `missing_debug_implementations`. `private_intra_doc_links` (no `pub` doc may link `Row`). `broken_intra_doc_links` (T1 must not link T2 items and the reverse). `clippy::pedantic` is **off**, so no `must_use_candidate`/`missing_panics_doc` churn, though `# Panics` is documented on `Keys::defaults` anyway. | Gate runs clippy with and without features and `cargo doc`. |
| H-8 | **`LazyLock` panic at first key** if a default stops parsing. | T3 test 1 and test 4 run on every `cargo test --lib`. |
| H-9 | `ctrl-S`/`ctrl-E` caps-lock aliases (F-7). | M4. |
| H-10 | `kinds.rs:1476` `g` vs `list.top` (F-8). | M3. |
| H-11 | Stale `ui/` comments about the "wildcard overlay binding" (F-9). | M3; the reviewer is told. |
| H-12 | **Suite flakes** from the process-wide keyring fake (test 1 takes `mock_keyring`). | Gate with `--test-threads=1`; test 1 takes the guard as its first statement (`connection.rs` header rule). |

---

## 8. Count pins

| What | Before | After |
|---|---|---|
| `Act` variants / `CATALOGUE` rows | - | 41 (global 16, overlay 1, list 5, pane 6, confirm 2, form 4, common 7) |
| `Context` variants | - | 7 |
| `STATE_GUARDED` pairs | - | 1 |
| `keymap.rs` unit tests | 9 | 4 |
| `keys/chord.rs` / `catalogue.rs` / `mod.rs`+`stack.rs`+`hint.rs` unit tests | 0 | 17 / 11 / 20 |
| `tests/keys.rs` | - | 10 |
| `Ctx::new` sites given `.with_keys` | - | 11 (7 `state.rs`, 4 `update.rs`) |
| Snapshots changed | - | 0 |

---

## 9. Plan deviations

| # | Plan text | Deviation | Evidence | End state |
|---|---|---|---|---|
| PD-1 | T3: "no two actions in one context share a default chord" | Exempt the pairs on a new `catalogue::STATE_GUARDED` (`[(Back, Dismiss)]`) | F-2: both are `Esc` in 4-5 views each | Same catalogue; the allow-list is ANA §7.4 step 7's, started early |
| PD-2 | T1: rework `keymap.rs` per D2/D3 (empty `default_global`, rewrite its tests) | T1 only moves `KeyChord` and adds the strict parser; **T4** empties `default_global` and swaps its tests | F-1: `state.rs:657`, `:706`, `:754` read the table until T4 | Identical to D2/D3 after T4 |
| PD-3 | D2: `pub use crate::keys::KeyChord;` | `pub use crate::keys::chord::KeyChord;` | F-3: `keys/mod.rs` is T3's only | Same type, same public paths |
| PD-4 | T1 Validate: `cargo test -p htui --lib keys::chord keymap` | `cargo test -p htui --lib -- keys::chord keymap` | F-6: one `TESTNAME` per `cargo test` | Same tests run |

No decision D1-D12 is reopened. D8's "layer heading" is specified as `Context::heading()` for
catalogue layers and the tab's `title()` for the legacy rows (B8). D8's rendering gains row
packing (B9), which no snapshot pins.

---

## 10. Blueprint decisions

- **B1 `Act` naming.** Shared verbs of `list`/`pane`/`confirm`/`form` carry their context prefix
  (`ListDown`, `PaneScrollDown`, `ConfirmNo`, `FormSave`). `common` verbs are bare (`Edit`,
  `Clear`, `Reload`, as in ANA §7.6's example). Overlay close is `OverlayClose`. Global acts are
  bare. M3-M5 prefix view acts with the view (`ConnectionRebuild`, `RunsApprove`).
- **B2 `Context` in `catalogue.rs`**, not `Ctx` (ANA §7.2's sketch name collides with
  `app::Ctx`). `table()` is the TOML name, `heading()` the `?` box heading.
- **B3 `in_capture`** is per action with all chords non-printable; capture filtering of the
  global layer is per chord (M3). `global.*` is `false`; `form.*` and `overlay.close` are `true`.
- **B4 Spec spellings** in the catalogue are lower-case names (`tab`, `backtab`, `esc`, `enter`,
  `pgdn`, `pgup`, `home`, `end`, `down`, `up`, `left`, `right`, `f1`), the character for
  characters (`G`, `J`, `]`), and `ctrl-x` for chords.
- **B5 `Keys` storage** is a `Vec<Row>` in catalogue order with a linear scan. There are 41 rows in
  M1 and about 100 later, scanned once per key press. Order is what the status line and the box
  need, and a map would need a second index for it.
- **B6 Shadowing.** A row for an act in a narrower layer hides that act's rows in every wider
  layer, bound or not. That is how M2's `[settings.boxes] reload = "F5"` and `reject = []` work.
  M1 tests it through the `#[cfg(test)]` `with_chords`.
- **B7 `Stack`** is a `Copy` newtype over `&[Layer]`, with `Layer = (Context, Option<&'static
  [Act]>)`. M1 ships only `Stack::BASE` and `Stack::OVERLAY`. M3's capture filter adds a third
  admission kind to `Layer` then, not now (it would be dead code in M1).
- **B8 `?` box headings.** `Overlay`/`Global` from `Context::heading()`. The legacy line uses the
  active tab's `title()` (`Backlog`). The overlay line appears only while an overlay is open, and
  the legacy line whenever the active tab has rows, as today.
- **B9 Row packing** (`HelpLine::rows`) replaces `Wrap`, so the box height is exact (F-5).
- **B10 `offer` replaces** an earlier offer of the same act, so calling `register_all` twice
  doesn't duplicate rows (it still stacks a second switcher, as documented at `app/mod.rs:55-56`).
- **B11 `action_for` is private** and is the single fixed `Act` → `Action` map. Shared-context acts
  return `None` there, so the shell can never dispatch a view's verb.
- **B12 `App.keys` is owned** (`Keys::compiled().clone()`), as D10 says. `Ctx` borrows App's at
  the shell's sites and the `'static` compiled table everywhere else. The two are equal in M1; M2
  makes `App`'s the loaded one.

## Files

### Files to create
| File | Purpose | Task |
|---|---|---|
| `crates/htui/tests/keys.rs` | App-level key behaviour (10 cases, §5.7) | T4 |

(`keys/{mod,chord,catalogue,stack,hint}.rs` were created by T0 and are filled by T1-T3.)

### Files to modify
| File | Changes | Task |
|---|---|---|
| `crates/htui/src/keys/chord.rs` | `KeyChord` moved; `parse_strict`, `ChordError`, `CTRL_C`, `spec`, `is_printable`; 17 tests | T1 |
| `crates/htui/src/keymap.rs` | T1: drop moved code, re-export, prune `KeyEvent`, drop 3 chord tests. T4: empty `default_global`, doc, prune imports, test swap (§5.6) | T1, T4 |
| `crates/htui/src/keys/catalogue.rs` | `Context`, `Act`, `ActionSpec`, `CATALOGUE`, `STATE_GUARDED`; 12 tests (11 + review M3) | T2 |
| `crates/htui/src/keys/mod.rs` | re-exports, `Keys`, `Row`, `COMPILED`; 5 tests | T3 |
| `crates/htui/src/keys/stack.rs` | `Layer`, `Stack`, `Keys::actions`; 6 tests | T3 |
| `crates/htui/src/keys/hint.rs` | `Hint`, `HintSpec`, `HelpLine`, `label`, `hint`, `status_line`, `help_line`, `help_closer`; 9 tests | T3 |
| `crates/htui/src/app/state.rs` | `App.keys`, `App.offered`, `offer`, `action_for`, `apply_keys`, `on_key`, `render`, `render_help`, `Ctx.keys`/`with_keys`/`keys`, 7 sites, imports | T4 |
| `crates/htui/src/app/update.rs` | 4 `Ctx::new` sites | T4 |
| `crates/htui/src/app/mod.rs` | 3 `offer` calls, `use crate::keys::Act`, doc items 2/7/11 | T4 |

# ANA-26 - Configurable hotkeys

> **Scope note:** "The maintainer wants htui's hotkeys configurable. Today only a thin table is
> data: `Keymap::default_global` (`crates/htui/src/keymap.rs`) holds the global and
> overlay-wildcard bindings (`q`, `ctrl-c`, `Tab`, `1`..`9`, `?`, `Esc`, plus `w` from
> `register_all`), and `KeyChord::parse` already reads spec strings such as `"ctrl-c"`. Every tab,
> section and overlay key is a hard-coded `KeyCode` match in its own `on_key` [...], and the hint
> lines spell the keys as string constants (`HINT_*` [...]), so rebinding the table alone would
> leave those keys fixed and the hints wrong." (`HANDOFF.md:164-184`, ANA-26; opened
> 2026-09-29 during MOD-52, `docs/decisions/mod/mod-52.md:41-45`.)
>
> **Requirements addressed:** `R-TUI-1`, `R-TUI-8`, `R-STO-1`, `R-STO-4`, `R-USR-1`, `R-NF-1`,
> `R-NF-3`.
>
> **Status (2026-09-30): concluded.**
> Verdict: **every key outside text entry becomes a named action**, not only the global table.
> Defaults stay in code, in one action catalogue that is the single source of truth. A user
> overrides only what they change, in a **local TOML file, `<config_root>/keys.toml`**: per OS
> user and per machine, read before the first frame, never read from or written to the store.
> Actions are named **once per meaning, not once per letter**: shared verbs (`list.down`, `edit`,
> `reload`, `confirm.yes`) live in shared contexts that every view offering them uses, and a
> narrower context may override one for a single view. **An invalid file refuses to start htui**:
> every error is printed as `keys.toml:LINE: ...`, and `--default-keys` ignores the file for one run.
> Validation covers unknown names, chords a terminal cannot deliver, duplicate chords on any
> reachable key path, and printable chords bound where a text field captures them.
> **`ctrl-c` is fixed.** It always quits, cannot be unbound or reused, and is checked before
> any htui view sees the key; the one declared exception is MOD-57's embedded editor pane, which
> forwards it to its child and always keeps a leave key. This also fixes a defect found here:
> today `ctrl-c` opens the clear-DSN confirm in Settings > Connection. **Every hint line,
> key-naming message and the `?` box is generated from the resolved keymap**, so no hint can
> drift from its binding. One implementing
> item, MOD-67, delivers this in six milestones.

Code citations are against HEAD `4b1c323`.

---

## 1. Context and problem statement

`htui` is keyboard driven (`R-TUI-1`, `docs/REQUIREMENTS.md:312-315`), and the maintainer wants
its keys configurable. The ask has five parts (`HANDOFF.md:172-183`):

1. **Scope.** Is only the global and overlay table configurable, or is every tab and section key?
2. **Ownership.** Where do bindings live: a local file, the store's settings, or both? Are they
   per user, per box or per workspace?
3. **Validation.** Load must handle duplicate chords, printable keys bound where a text field
   captures them, and `ctrl-c`. MOD-52 made `ctrl-c` quit on the assumption that every capturing
   section passes `CONTROL` chords on (`docs/decisions/mod/mod-52.md:15-23`).
4. **Hints.** How do hints and the `?` help follow a rebinding?
5. **Prior art.** What do gitui, helix, yazi and lazygit do?

The problem is mostly a structural one, not a file-format one. Almost every key `htui` answers
is matched by a view before the table is consulted. The table that exists is too thin to carry
the feature.

## 2. Current state in htui

### 2.1 The table

- **`KeyChord`** (`crates/htui/src/keymap.rs:19`) is a crossterm `KeyCode` plus `KeyModifiers`.
  `KeyChord::new` folds `Shift+Tab` to `BackTab` and drops SHIFT from characters. `parse`
  (`keymap.rs:61`) reads spec strings (`"ctrl-c"`, `"pgdn"`, `"f5"`) and never panics. `label`
  (`keymap.rs:86`) renders `Ctrl+c`, `Esc`, `PgUp`.
- The parser has four gaps that matter once a user writes the specs:
  - `"shift-a"` parses to plain `a`, which never matches a typed `A`.
  - `"ctrl-C"` parses to a chord no terminal sends.
  - `"ctrl--"` cannot be written at all.
  - Parts are not trimmed, so `"ctrl - c"` fails.

  All four follow from `keymap.rs:61-82` and the SHIFT folding in `KeyChord::new`.
- **`KeyScope`** (`keymap.rs:158`) is `Global | Tab(TabId) | Overlay(OverlayId)`. There is no
  scope for a section, a pane or a mode.
- **`Binding`** (`keymap.rs:169`) has a `help: &'static str` (`keymap.rs:177`), so help text
  cannot come from a file as the type stands.
- **`Keymap::bind`** (`keymap.rs:257`) only pushes: the newest row shadows, and nothing checks
  for duplicates. `resolve` (`keymap.rs:266`) falls back from an overlay's own scope to
  `Overlay(ANY)`.
- **Contents.** `default_global` (`keymap.rs:205-253`) binds `q` quit, `Tab`/`BackTab`, `1`..`9`
  select tab, `?` help, `Esc` close on `Overlay(ANY)` (`keymap.rs:241`), and `ctrl-c` quit on
  both Global and `Overlay(ANY)` (`keymap.rs:245-252`). `register_all` adds three rows:
  - global `w` workspaces (`crates/htui/src/app/mod.rs:72-77`);
  - global `Ctrl+f` find (`app/mod.rs:83-88`);
  - a `Tab(Backlog)` `Enter` row (`app/mod.rs:108-113`), whose only job is to say "select a step
    in the Runs pane (J/K)" when no pane took the key.

  That Enter row is the only tab-scoped binding. No view registers one.
- **Actions** are a plain enum with payloads (`crates/htui/src/app/action.rs:16`). There is no
  string name for any action.

### 2.2 Dispatch

`App::on_key` (`crates/htui/src/app/state.rs:422-514`) builds one `KeyChord`
(`state.rs:427`). It then stops at the first consumer, in this order:

1. The top overlay's `on_key` (`state.rs:453`).
2. The overlay table: the overlay's own scope, then `ANY` (`state.rs:462`).
3. The modal swallow (`state.rs:466-469`). All three overlays are modal
   (`ui/overlay/workspace_switcher.rs:150`, `migration_prompt.rs:80`, `concepts_search.rs:282`),
   so over an overlay only `Esc` and `ctrl-c` from the table work.
4. The active tab's `on_key` (`state.rs:496`), which routes inside itself by hand. Settings
   hands everything to a section whose `captures_input()` is true (`ui/tabs/settings/mod.rs:323-331`).
   Otherwise it takes `h`/`l`/`[`/`]`/arrows itself (`settings/mod.rs:333-340`), then delegates
   (`settings/mod.rs:289-294`).
5. The tab table (`state.rs:505`).
6. The global table (`state.rs:511`).

A global binding therefore works only where every view on the path returns `Pass` for its chord.
Which letters are free is tracked by hand in comments, and those comments are already stale:
- `settings/connection.rs:761-763` and `settings/hierarchy.rs:1076-1077` say the global table
  binds `-`. No `-` row exists.
- `settings/agents.rs:1804-1807` lists the global keys but omits `ctrl-f` (`app/mod.rs:83-88`).

Tabs receive the raw `KeyEvent`, not the chord. No non-test code under `ui/tabs` imports `keymap`.
`Ctx.keymap` ("for help lines", `state.rs:80-81`) has no reader; the only test that inspects the
table reads `App.keymap` (`tests/replay.rs:222`).

### 2.3 Inventory

These are hand counts from the key sweep (not tool output). They cover non-test code only.

| Area | Files | Key-match sites | Distinct chords | Candidate actions |
|---|---|---|---|---|
| Backlog (tab, detail pane, 7 sub-tab files) | 9 | 77 | | |
| Chat (tab, composer, transcript) | 3 | 23 | | |
| Requirements (tab, forms) | 2 | 29 | | |
| *Backlog + Chat + Requirements* | *14* | *129* | *55 (47 with `1`-`9` as one)* | *62* |
| Settings (tab + 7 sections) | 8 | 102 | | |
| Skills (tab, library, templates, attach) | 4 | 57 | | |
| Overlays | 3 | 16 | | |
| Text widgets (`text_field.rs`, `text_area.rs`) | 2 | 27 | | |
| *Settings + Skills + overlays + widgets* | *17* | *about 202* | *50, plus `ctrl-c`* | *about 73 per section, about 48 unified* |

- 27 files under `crates/htui/src/ui` match `KeyCode::`. That is the HANDOFF's "about 23" plus
  the two text widgets, `skills/mod.rs` and `settings/mod.rs`.
- After unifying the shared names across the two slices, the catalogue in §7.2 comes to **about
  100 actions**.
- Only one key in the whole UI goes through `Keymap`: the Backlog Enter row. Every other site
  matches `key.code`.

**Shared letters.** Settings is where the question bites:
- `e` means "edit" everywhere. It opens an editor on the selected row in agents, hierarchy,
  kinds, prompt and qdrant (`agents.rs:1823`, `hierarchy.rs:1109`, `kinds.rs:1413`,
  `prompt.rs:839`, `qdrant.rs:342`). In connection it always edits the DSN
  (`connection.rs:764`). In boxes it edits quirks, while `t` edits tags (`boxes.rs:456-460`).
- `c` means "clear a stored value behind a confirm" in both places it exists
  (`connection.rs:770`, `qdrant.rs:348`).
- `r` is a re-read in six sections and in Skills (`boxes.rs:470`, `connection.rs:807`,
  `hierarchy.rs:1156`, `kinds.rs:1450`, `prompt.rs:859`, `qdrant.rs:374`). In agents it is a
  probe that spawns processes (`agents.rs:1888-1904`; hint "r probe", `agents.rs:100`). Boxes
  spells its probe `p` (`boxes.rs:464`).
- `j`/`k` always mean list down/up. Agents and hierarchy lack the `Down`/`Up` aliases the other
  sections have (`agents.rs:1835-1839`, `hierarchy.rs:1079-1083`).
- `J`/`K` scroll the pane in body, notes and the other Backlog sub-tabs, but in the Runs pane they
  move the step cursor, and scrolling there is `PgDn`/`PgUp`. The Backlog miss message names
  "(J/K)" for step selection (`app/mod.rs:111`).

**Keys that act inside a capturing form without being text.** The Requirement form's Priority
field sets `must` on `m` and `later` on `l` and toggles on Space/Left/Right; the attach form's
Activation field cycles on Space and swallows every other key. They are the field's own value
keys, like a text field's characters.

Across tabs, the same letter often means unrelated things. `a` is approve step
(`backlog/detail/runs.rs:509`), cite-as-addresses (`backlog/detail/requirements.rs:282`), next
agent (`chat/mod.rs:507`) and new area (`requirements/mod.rs:438`). `n` is "new" in browse modes
and "no" in every confirm.

**Text entry.**
- `TextField::on_key` (`crates/htui/src/ui/text_field.rs:119-174`) inserts printable characters,
  edits and moves the cursor, maps `Enter` to Submit and `Esc` to Cancel, and passes every chord
  with CONTROL, ALT, SUPER, META or HYPER (`text_field.rs:120-128`).
- `TextArea::on_key` (`crates/htui/src/ui/text_area.rs:190-246`) is the same, except `Enter` is a
  newline and it claims `ctrl-s` for Submit (`text_area.rs:191-199`).
- The chat composer consumes every key while active (`ui/tabs/chat/composer.rs:59-86`).
- There is no readline-style editing chord (`ctrl-a`/`ctrl-e`/`ctrl-w`) anywhere.

### 2.4 Hints and help

- **Generated from the table:** only the status line, which shows `help_line(Global)` when there
  is no error (`state.rs:557-560`), and the `?` box (`render_help`, `state.rs:574-597`). The box
  shows the global line, the active tab's line if any, and a literal `"? closes this box"`
  (`state.rs:585`). Overlay scopes, section keys and pane keys never appear in it.
- **Everything else is a string constant or an inline literal.** There are 67 `*HINT*` string
  constants under `crates/htui/src`, for example:
  - `settings/connection.rs:51-52` "e edit DSN · c clear DSN · R rebuild cache · r reload · j/k rows";
  - `skills/library.rs:109-110`;
  - `overlay/concepts_search.rs:85-86`.

  Inline hint literals include `chat/mod.rs:424-434`, `backlog/detail/runs.rs:665`, `:1296` and
  `:1324`, and `backlog/detail/requirements.rs:427`. Prose messages also name keys, for example
  `connection.rs:126` ("press Enter or R"), `hierarchy.rs:83`, `settings/mod.rs:105` and
  `app/mod.rs:111`.
- **Three chord spellings** (`ctrl-s` in `boxes.rs:58`, `Ctrl+S` in `library.rs:119`, and
  `label()`'s `Ctrl+c`) and **two separators** (the middle dot, written both as ` · ` and
  `\u{b7}`, and two spaces) are in use.
- **Drift already exists.** The library browse hint dropped "h/l view" to fit
  (`skills/library.rs:107-110`), while templates keeps it (`skills/templates.rs:80`). The Runs
  pane shows no hint in Browse (`backlog/detail/runs.rs:1291`), so its twelve action letters
  appear nowhere on screen.

### 2.5 Tests that pin keys

- **116 insta snapshots**: 115 in `crates/htui/tests/snapshots/`, plus
  `src/snapshots/htui__testkit__tests__shell_empty.snap`. 114 of them contain hint text, and 89
  pin the global status line verbatim.
- **25 integration test files** drive keys by spec string through `Harness::key(&str)`
  (`crates/htui/src/testkit.rs:561`, via `KeyChord::parse`). There are about 987 `.key(` calls.
  They keep working as long as the defaults do not change.
- **About 28 hint-substring assertions**, for example `tests/replay.rs:215-226`.
- **9 `keymap.rs` unit tests**, including `ctrl_c_quits_globally_and_from_any_overlay`
  (`keymap.rs:406`) and `the_help_line_collapses_the_digit_rows` (`keymap.rs:487`).
- **Pass-through tests for `ctrl-c`**: `text_field.rs:528-530`, `backlog/detail/runs.rs:2911-2931`
  and `backlog/detail/requirements.rs:765-771`. The MOD-52 tests are `tests/shell.rs:288` and
  `tests/hierarchy.rs:1259` (`mod-52.md:29-36`).

### 2.6 `ctrl-c` today, and a defect

- Raw mode clears `ISIG`, so `ctrl-c` is an ordinary key (`mod-52.md:11`;
  `crates/htui/src/terminal.rs:45`). It quits only by reaching a table row.
- Everything else is a per-view convention: each capturing view checks CONTROL and returns
  `Pass`. The sites are `settings/boxes.rs:360`, `connection.rs:427` and `:477`,
  `hierarchy.rs:687` and `:812`, `kinds.rs:683`, `:758` and `:820`, `prompt.rs:496`,
  `agents.rs:1219`, `qdrant.rs:208`, `skills/attach.rs:237` and `:573`, `library.rs:483` and
  `:954`, `templates.rs:642`, `requirements/mod.rs:1092-1108` (which passes ALT as well),
  `backlog/mod.rs:231`,
  `backlog/detail/runs.rs:377` and `:1144`, `backlog/detail/requirements.rs:270` and `:472`,
  `chat/mod.rs:468`, and `concepts_search.rs:311`.

**Found in passing (confirmed by reading the code, pinned by no test):**

1. **`ctrl-c` does not quit from Settings > Connection or Qdrant in browse mode.** The Settings
   tab and the sections' browse arms match `key.code` without looking at modifiers
   (`settings/mod.rs:332-341`, sections from `connection.rs:763`). With Connection active, `ctrl-c` hits the `'c'` arm
   (`connection.rs:770`) and is consumed. With a stored DSN it opens the clear-DSN confirm, and a
   second `ctrl-c` then quits through `connection.rs:477`. `qdrant.rs:348` behaves the same way.
   The same blindness sends `ctrl-d` to delete in hierarchy and kinds and `ctrl-l` to cycle
   sections. `skills/mod.rs:97-101` has the same blindness for `h`/`l`. Backlog and Chat guard
   against this (`backlog/mod.rs:231`, `chat/mod.rs:468`).
2. **The Qdrant editor passes every key its field does not use** (`qdrant.rs:203`). `Tab`
   therefore switches tabs while the masked API-key buffer is still open. Every other Settings
   editor swallows keys that are not CONTROL chords.
3. **ALT is inconsistent.** The two Backlog modal handlers pass only CONTROL
   (`backlog/detail/requirements.rs:270`, `runs.rs:377`), so `Alt+y` confirms an uncite. The
   Settings forms also pass only CONTROL, while `TextField` passes ALT too.

### 2.7 Where a file could live, and when keys are needed

- **Config root.** `identity::config_root()` returns `dirs::config_dir()/htui` and creates it
  (`crates/htui-store/src/identity.rs:50-56`). That is `~/.config/htui`,
  `~/Library/Application Support/htui` or `%APPDATA%\htui` (`identity.rs:42-44`).
- **What is there today.** Its one TOML file is the machine-written `box.toml`, written by
  temp-and-rename (`identity.rs:120-129`). htui has no user-editable config file today.
- **Dependencies.** The workspace pins `serde`, `dirs` and `toml = "1.1"` (`Cargo.toml:30,52,65`;
  `toml 1.1.5` in `Cargo.lock:6723-6724`). `crates/htui` depends on none of them directly; it has
  only `serde_json` (`crates/htui/Cargo.toml:33`).
- **Startup order.** `crates/htui/src/lib.rs:102-110` starts the backend. `connect::start` does
  only local work, then returns `Offline` and spawns the dial (`crates/htui-store/src/connect.rs:300-311`).
  `lib.rs:124` builds `App::new(request_tx, Keymap::default_global())` and `lib.rs:126` calls
  `register_all`. **Keys are therefore needed before any server data can exist.** The startup
  overlay is the workspace switcher (`app/mod.rs:90`), so no workspace is known either.
- **The store has no home for this.** The settings registry's `SettingKind` is `Integer`,
  `Fraction` or `Boolean` only (`crates/htui-core/src/prompt/settings.rs:297-305`). `Rungs` is
  App/Project/Phase "for three bits that never grow" (`settings.rs:239-250`). `app_setting` is
  global, one row per key (`crates/htui-store/migrations/0001_init.sql:557-561`), and it is **not
  mirrored**: `cache_migrations/0001_mirror.sql:24-25` says so, and `Backend::app_settings()`
  errors offline (`crates/htui-store/src/backend.rs:393-398`). `app_user` has no settings column
  (`0001_init.sql:31-37`). `box.settings` exists, but nothing edits it
  (`crates/htui-core/src/model/box_.rs:157-162`).
- **A local writable settings store was designed once and withdrawn.** ANA-10's `local_setting`
  went with MOD-25 (`docs/decisions/mod/mod-25.md:17-28`).
- **Tests inject the config root** so that "a test never writes into the user's configuration"
  (`connect.rs:205-207`). Tests build `Keymap::default_global()` directly (`testkit.rs:128`,
  `:647`).
- **The terminal protocol.** `crates/htui/src/terminal.rs:45` enables raw mode. No
  `PushKeyboardEnhancementFlags` call exists under `crates/htui/src` (checked by search at HEAD).
  htui therefore reads the legacy encoding, where `ctrl-i` is `Tab`, `ctrl-m` is `Enter`,
  `ctrl-[` is `Esc`, and `Shift+a` arrives as `A`.

## 3. Constraints

- **Usable offline.** htui must start usable with no store (`R-STO-4`,
  `docs/REQUIREMENTS.md:149-151`), and must never block on network I/O (`R-NF-3`,
  `REQUIREMENTS.md:359`). Keys are needed before the first frame (§2.7).
- **One developer, several machines** (`R-USR-1`, `REQUIREMENTS.md:67`). Team use is later (`R-USR-3`,
  `REQUIREMENTS.md:71-72`).
- **`R-STO-1`** (`REQUIREMENTS.md:140-143`): "Postgres is the only writable store", and secrets
  never go in a file. A keys file holds no secret, but it would be the first user-edited file, so
  the requirement text should say it is allowed (§9).
- **Three platforms** (`R-NF-1`, `REQUIREMENTS.md:357`), so the path must come from
  `dirs::config_dir()`.
- **`ctrl-c` must quit from everywhere**, including half-typed fields and modal overlays
  (MOD-52).
- **Printable keys belong to text fields while they capture.** The pass-through rule is CONTROL
  chords (`text_field.rs:120-128`; the Settings forms, §2.6).
- **Test isolation.** Snapshot and integration tests must never read the user's file (the
  `connect.rs:205-207` precedent).
- **No behaviour change in the defaults**, except the `ctrl-c` fix. The roughly 987 spec-string
  key presses in tests are the contract for that.

## 4. Prior art

Surveyed 2026-09-30. gitui (master `2fa693cb`) and lazygit (master `ff375b12`) were read from
source, with issue history from the GitHub API. Yazi was read from source. Helix was read from
its docs and `helix-term` sources. Zellij, the ratatui component template and the two keybinding
crates were read from their docs. Claims marked *unverified* were not confirmed in source or docs.

| | gitui | lazygit | helix | yazi | zellij |
|---|---|---|---|---|---|
| **File** | `key_bindings.ron` + `key_symbols.ron` in the config dir; a CLI flag picks others ([KEY_CONFIG.md](https://github.com/gitui-org/gitui/blob/master/KEY_CONFIG.md)) | `keybinding:` in `config.yml`; per-repo `.git/lazygit.yml` overrides; reloaded on focus ([Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)) | `[keys.*]` in `config.toml`; workspace `.helix/config.toml` overrides ([remapping](https://docs.helix-editor.com/remapping.html)) | dedicated `keymap.toml` ([keymap docs](https://yazi-rs.github.io/docs/configuration/keymap)) | `keybinds {}` in `config.kdl`, live-reloaded ([keybindings](https://zellij.dev/documentation/keybindings.html)) |
| **Shape** | action → one key | action → key or list (list since v0.62.0, [#5634](https://github.com/jesseduffield/lazygit/pull/5634)) | key → command, per mode | rule `{on, run, desc}` per layer | `bind "k1" "k2" { Action; }` per mode |
| **Scopes** | none: one flat struct of about 87 actions ([key_list.rs](https://github.com/gitui-org/gitui/blob/master/src/keys/key_list.rs)) | groups: `universal` plus per-context (`files`, `commits`, ...) | normal / insert / select, plus nested minor modes | eight layers, some falling through | modes, plus `shared_among` / `shared_except` ([shared](https://zellij.dev/documentation/keybindings-shared.html)) |
| **Shared verbs** | shared navigation names; other verbs split per view | `universal.edit`/`remove`/`new`, whose meaning the view decides | one flat command namespace | the same verb (`close`, `arrow`) per layer | one bind written for several modes |
| **Several keys per action** | no ([#1455](https://github.com/gitui-org/gitui/issues/1455)) | yes, as a list; the legacy `-alt` defaults still merge in ([#5763](https://github.com/jesseduffield/lazygit/issues/5763)) | repeat the command per key | one rule per key | yes |
| **Unbind** | none | `<disabled>` ([#3060](https://github.com/jesseduffield/lazygit/pull/3060)) | `no_op` | `run = "noop"` | `unbind`, `clear-defaults` |
| **Merge** | partial override ([#946](https://github.com/gitui-org/gitui/issues/946)) | partial override | recursive overlay | prepend / append / replace layer | per key, or clear-defaults |
| **Load errors** | logged; **all defaults** silently ([#1491](https://github.com/gitui-org/gitui/issues/1491)) | fatal at startup, names the field | prints and waits for Enter, then all defaults ([main.rs](https://github.com/helix-editor/helix/blob/master/helix-term/src/main.rs)) | prints and waits for a key, then presets | **exits** on a config error ([#4375](https://github.com/zellij-org/zellij/issues/4375), a user complaint that this blocks a safe fallback); `setup --check` and file:line:col reporting *unverified* |
| **Duplicate or conflict check** | none; [#2704](https://github.com/gitui-org/gitui/issues/2704) lost a user's work | none ([#3982](https://github.com/jesseduffield/lazygit/issues/3982), #5763) | TOML rejects the same key twice in one table; no semantic check | none; the first rule wins | *unverified* |
| **Hints and help** | built from live keys; only one key shown per action | bar and `?` menu built from live bindings; stale prose fixed in [#2777](https://github.com/jesseduffield/lazygit/pull/2777) | infobox from the live trie, grouped by doc string | `help` layer renders the live rules | status bar reads the merged table; custom binds get no label ([#3106](https://github.com/zellij-org/zellij/issues/3106)) |
| **Text input** | hard-coded textarea; bound chords swallowed ([#2438](https://github.com/gitui-org/gitui/issues/2438)) | printable keys never reach bindings while typing; Enter hard-coded in prompts ([#4860](https://github.com/jesseduffield/lazygit/issues/4860)) | prompt and picker keys hard-coded, not remappable ([keymap](https://docs.helix-editor.com/keymap.html), [#5505](https://github.com/helix-editor/helix/issues/5505)) | input claims modifier-free printable keys; Ctrl/Alt reach `[input]` | dedicated text modes |
| **Reserved keys** | none; the exit key is checked before every component | none; `ctrl-c` survives only through a legacy alt field | none | none | none *(unverified)* |

Also surveyed:
- **ratatui component template** ([config.rs](https://github.com/ratatui/templates/blob/main/component/template/src/config.rs)).
  Mode → key sequence → action, with defaults added only where the user bound nothing. It has no
  unbind, no conflict check and no generated help.
- **crossterm-keybind** ([repo](https://github.com/yanganto/crossterm-keybind)). A derive over an
  action enum. The config is action → key list, and a user entry replaces one action's list.
  `key_bindings_display()` renders keys for hints. It has no scope model. Whether it detects
  conflicts is *unverified*.
- **keybinds** ([repo](https://github.com/rhysd/keybinds-rs)). Key → action with sequences. It
  has no scopes and no descriptions.

**What htui takes from each:**

- **gitui:** partial override is the only sane default. Silent fallback is the failure mode to
  avoid: #1491 disabled a whole file over a comment typo. #2704 shows why duplicate detection is
  worth doing. Its exit key, checked before any component, is the model for a fixed `ctrl-c`.
- **lazygit:**
  - A shared verb set, with each view deciding what the verb means, plus per-context groups. This
    is the shape that answers htui's shared Settings letters.
  - A list value from day one, so no hidden `-alt` fields.
  - An explicit unbind (`<disabled>`, #3060). htui spells it `[]`, its own choice: an empty list
    is the natural "no chords" in a list-valued format.
  - A fatal validation error that names the field.
  - Hints and the `?` menu built from live bindings, including prose. #2592 was a hard-coded "Press
    tab".
  - A narrower context overriding a universal one, which is what #3982 asks for.
- **helix:** TOML, in the platform config dir, merged per key onto compiled defaults. Help is
  built from each command's doc string. Prompt keys are hard-coded, which is precedent for leaving
  text-entry keys out of scope.
- **yazi:** the text input claims modifier-free printable keys and hands CONTROL/ALT and named
  keys to the keymap. That is MOD-52's contract, already written down.
- **zellij:** refuse to run on a bad file rather than fall back. #4375 is the downside htui
  accepts (a lockout when htui is launched from a script), mitigated by `--default-keys`. A check
  command (zellij's `setup --check`, *unverified*) becomes htui's `--print-keys` doubling as one.
- **crossterm-keybind:** action → key list, with defaults declared beside the action and one
  `display` for hints.

## 5. Options

### 5.1 Scope

| Option | For | Against |
|---|---|---|
| S1 Global and overlay table only | About 15 rows; the table, parser and `help_line` exist | Covers about 5% of the keys a user presses. Rebinding `q` to `x` is shadowed by Agents' `x` without warning (§2.2). Every section hint stays hard-coded. Does not meet the ask. |
| S2 S1 plus tab- and section-level `KeyScope`s filled by each view's `register` | Reuses `Keymap` | Modes (browse, confirm, form, picker) are the real unit; a section scope still needs a mode dimension. `Binding.help: &'static str` and the untyped `Action` enum carry no names. |
| **S3 Every key outside text entry becomes a named action**, dispatched through one resolver; text-entry keys stay fixed | One mechanism; hints can be generated; conflicts become checkable | About 30 view files to convert and a one-time snapshot re-baseline |

Inside S3, the question is which keys count as **text entry** and stay fixed:
- **T1: only printable characters and in-buffer editing stay fixed.** These are Char, Backspace,
  Delete, and Left/Right/Home/End/Up/Down/PgUp/PgDn inside a widget. Enter-to-submit and
  Esc-to-cancel become bindable. The cost is that every widget reads the resolver, and a user who
  binds submit to `y` cannot type `y`. lazygit hard-coded Enter in prompts for exactly this
  reason (#4860).
- **T2: T1 plus the widget's own Enter (submit or newline) and Esc (cancel).** Chords a form
  claims beside its widget (`ctrl-s` save, `ctrl-e` external editor, Tab field focus) are
  bindable.
- **T3: T2 plus numbered choices.** The chat permission digits answer a list drawn as `[n]`
  (`ui/tabs/chat/permission.rs:29`, `chat/mod.rs:494-500`), so the digit is the label. Rebinding
  it would make the drawn list wrong. The same holds for a **choice field inside a form**
  (Priority's `m`/`l`/Space, Activation's Space, §2.3): the keys set the field's value, the field
  draws them, and they belong to the field the way characters belong to a `TextField`.

### 5.2 Action naming and shared letters

| Option | For | Against |
|---|---|---|
| N1 One name per letter (`settings.e`) | Trivial | Names the key, not the action; rebinding `e` makes no sense across the agents probe and the connection DSN |
| N2 One name per view and meaning (`settings.connection.edit`, `settings.kinds.edit`, ...) | Precise; no surprises | About 175 names. Changing "edit" everywhere means seven entries. Consistency across sections is lost the moment a user edits one. |
| **N3 One name per meaning**: shared verbs in shared contexts, which each view offering the verb uses and gives its own target; view-specific verbs in the view's context; a narrower context may override a shared verb for one view | About 100 names. One line rebinds "edit" in every section. lazygit's proven shape, and #3982 is solved by narrow-wins. | The meaning of a shared verb must actually be the same. That holds for `edit`, `clear`, `reload` and list navigation, not for `r`-as-probe (§2.3). Such cases are split by name. |

### 5.3 Where bindings live and who owns them

| Option | For | Against |
|---|---|---|
| **W1 Local file `<config_root>/keys.toml`**, per OS user and machine | Read synchronously before the first frame (§2.7); works offline and under `--offline`; no schema, migration or `.sqlx` change; the user syncs it with dotfiles; matches every tool surveyed | Not shared across boxes unless the user syncs it; first user-edited file (`R-STO-1` wording) |
| W2 Store: `app_setting` or a new user-scoped row | One edit reaches every box | Arrives only after `Online`, 0-10 s later or never. Unmirrored (§2.7), so every offline start silently reverts to the defaults, which is the state `R-STO-4` makes routine. Needs a new `SettingKind`, a rung or table, a migration and a live rebind. Writes are online-only. |
| W3 Both: defaults < store < file | Sync plus local override | All of W2's cost. The order is only well defined if the file is re-applied after every connect. Keys change under the user's fingers when the store answers. |
| W4 Per workspace | Different keys per workspace; lazygit (`.git/lazygit.yml`) and helix (`.helix/config.toml`) allow per-repo or workspace overrides | The keymap is built before a workspace is chosen (the switcher is the startup overlay, `app/mod.rs:90`), and keys would change on every switch. |

### 5.4 Validation at load, including `ctrl-c`

**On errors:**

| Option | For | Against |
|---|---|---|
| E1 Ignore the bad entries, keep the rest | Always starts | Silent: gitui #1491 and the whole class of "config does nothing" reports |
| E2 Print, wait for a key, then run on all defaults (helix, yazi) | Always starts, and the error is seen once | The user works on keys they did not choose; stdin interaction before the TUI; untestable in the harness |
| **E3 Refuse to start: print every error as `path:line: message`, exit non-zero, and offer `--default-keys` to ignore the file for one run** (zellij) | Loud, located and complete; nothing half-applied; offline-safe (no store involved) | A broken file blocks the default command until fixed or bypassed |

**On what is checked:**

| Check | Options |
|---|---|
| Unknown context, unknown action, unparseable chord | error (every option agrees) |
| Chord the legacy terminal encoding cannot deliver distinctly (`shift-<char>`, `ctrl-<Upper>`, `ctrl-i`, `ctrl-m`, `ctrl-[`) | ignore / **error with the spelling to use instead** |
| The same chord twice in one context | first wins (yazi) / **error** |
| The same chord on two actions reachable from the same view mode (a user `global.quit = "x"` against Agents' `x`) | ignore (every surveyed tool) / **error, unless both are defaults and the pair is on a reviewed allow-list** |
| A printable chord (a character with no CONTROL or ALT) bound to an action offered while a text field captures | runtime rule only (lazygit) / **error**. Named keys (`Tab`, `Up`, `F1`) and CONTROL or ALT chords stay allowed, which is what the widgets pass on today. |

**On `ctrl-c`:**

| Option | For | Against |
|---|---|---|
| C1 An ordinary action: unbindable and rebindable (gitui, lazygit, yazi) | Maximum freedom | Raw mode means nothing else stops htui (§2.6). A config that unbinds it, plus a modal overlay that swallows `q`, leaves a user with no way out but killing the terminal. MOD-52's guarantee becomes optional. |
| C2 Fixed in the table: `quit` always keeps `ctrl-c`, and other actions may not use it | Keeps MOD-52's table rows | Still depends on every view passing it on, and §2.6 defect 1 shows one does not |
| **C3 Fixed and checked first**: `App::on_key` quits on `ctrl-c` before any overlay or htui view; `quit` may gain chords and lose `q`; no action may bind `ctrl-c` | Holds by construction; fixes §2.6 defect 1; MOD-52's per-view guards stop carrying quit | `ctrl-c` can never mean "copy" or "cancel" in htui. No view uses it for either today. MOD-57's embedded editor pane (a child process on a PTY) needs `ctrl-c` forwarded to vim, nvim or emacs, so C3 covers htui's own views and that pane is the one declared exception (§6.4). |

### 5.5 Hints and the `?` help

| Option | For | Against |
|---|---|---|
| H1 Keep the `HINT_*` constants and interpolate the first key with `format!` | Small diff per site | Still one hand-written string per mode; the pairing (`j/k move`) and order are re-coded 67 times; drift stays possible |
| **H2 Each view mode declares a hint spec**: an ordered list of `(action or action pair, label)`. One renderer draws the first chord of each bound action and drops unbound ones. The `?` box lists every action of the active mode's context stack with all its chords. Prose names keys through `keys.label(action)`. | Hints cannot drift from bindings. One spelling (`label()`) and one separator. The Runs pane gains a real hint. The `?` box finally covers sections and overlays, once `help` is reachable there (§6.5). | About 110 snapshots change once, when spelling and separators are normalised |
| H3 Generate the hint line from every action the mode offers, with no spec | No per-view hint code | Loses the curated order and short labels; lines overflow 100 columns (the reason `library.rs:107-110` trimmed its hint) |

## 6. Verdict

1. **Scope: S3 with T3.** Every key outside text entry is a named action, resolved through one
   resolver. The following stay fixed:
   - printable characters, and editing and cursor keys inside `TextField`, `TextArea` and the
     chat composer;
   - a widget's own Enter (submit, or newline in a `TextArea`) and Esc (cancel);
   - the numbered permission answers, and the value keys of a choice field inside a form
     (Priority's `m`/`l`/Space/Left/Right, Activation's Space), which the field draws itself.

   Chords a view claims beside its widget become actions: `form.save` (`ctrl-s`),
   `form.external_editor` (`ctrl-e`), `form.next_field`/`form.prev_field` (`Tab`/`BackTab`) and
   the concepts chords. S1 was rejected because it leaves about 95% of the keys fixed and makes a
   rebound global key collide unseen with section letters. S2 was rejected because modes, not
   tabs, are the unit that owns keys.
2. **Naming: N3, one name per meaning.**
   - Shared verbs live in shared contexts: `list`, `pane`, `confirm`, `form` and `common` (§7.2).
     Every view that offers one uses it for its own target. `e` is `common.edit` in six Settings
     sections, including connection, where the target is the DSN; boxes splits it (below). `c` is
     `common.clear`. `r` is `common.reload`, and `j`/`k` are `list.down`/`list.up`.
   - Where one letter means two things, the names split. Agents' `r` is `settings.agents.probe`
     and boxes' `p` is `settings.boxes.probe`, because a probe spawns processes and a reload does
     not. Boxes' `e` and `t` are `settings.boxes.edit_quirks` and `settings.boxes.edit_tags`,
     because boxes has two edit targets. In the Runs pane `J`/`K` are `backlog.runs.next_step`/
     `prev_step`, not `pane.scroll_*`, because they move the step cursor there (§2.3).
   - A view-specific verb lives in the view's context (`backlog.runs.approve`).
   - A narrower context may override a shared verb for that view alone
     (`[settings.boxes] reload = "F5"`), and the narrowest wins.
   - MOD-67 changes no default. The two probe spellings stay as they are (§10).
3. **Ownership: W1, one local TOML file.**
   - The file is `<config_root>/keys.toml`, owned by the OS user on that machine. Defaults are
     compiled in, and the file lists only changes.
   - The store is not read or written for keys. It is unavailable when keys are needed, it is
     unmirrored offline, and it has no kind, rung or table for this (§2.7). A store layer (W3) is
     not built now. It can be added under the file later without changing the format (§10).
   - There are no per-workspace keys (W4).
   - The file is **not** a revival of ANA-10's `local_setting`. It is a read-only input to htui:
     htui never writes it, and it is not a store and is not mirrored.
   - `--demo` reads it too, since keys are a user preference, not data. Tests never do.
4. **Validation: E3, with every check marked in bold in §5.4, and C3 for `ctrl-c`.**
   - An invalid file stops htui before the terminal is touched. All errors are listed with
     `path:line`, and `--default-keys` bypasses the file for one run.
   - **`ctrl-c` always quits htui.** It is checked at the top of `App::on_key` before any overlay
     or htui view, it cannot be unbound, and no action may bind it. A user may add chords to
     `global.quit` and may unbind `q`. **One exception, declared now:** a pane that hosts a child
     process on a pseudo-terminal (MOD-57's in-pane editor) forwards `ctrl-c` to the child, since
     vim and nvim use it to cancel and emacs as a prefix. The check at the top of `App::on_key`
     skips only while such a pane holds focus, and that pane's leave action (a catalogue action,
     §10) is always bound, so the way out stays one keypress away. MOD-57 must not ship without
     that leave action.
   - `overlay.close` must keep at least one chord, because modal overlays swallow everything else.
   - An action reachable while a text field captures may not be bound to a printable chord (a
     character with no CONTROL or ALT). Named keys (`Tab`/`BackTab` for `form.next_field`, `Up`/
     `Down` in forms that move focus with them) and CONTROL or ALT chords are allowed. This is
     MOD-52's pass-through rule, made one checked rule instead of about 25 hand-kept guards. The
     compiled defaults pass it; a unit test asserts that.
5. **Hints and help: H2.**
   - Every `HINT_*` constant, inline hint literal and prose sentence that names a key is replaced
     by a hint spec or by `keys.label(action)`.
   - The `?` box lists the active mode's full context stack: overlay or view, then shared
     contexts, then global. It shows every chord of every bound action.
   - `global.help` must be reachable everywhere the box describes. It defaults to `["?", "f1"]`:
     `?` where it is not typed text, `f1` everywhere, including capturing fields, where the
     printable `?` is text. Overlay stacks include `global.help`, and `App::on_key` resolves it
     before the modal swallow, so the box opens over the workspace switcher, the startup screen.
     `f1` is an added default, like the `Down`/`Up` aliases in item 6.
   - The box is re-rendered from the live stack every frame, not captured when it opened, so a
     mode change underneath it updates it.
   - The status line renders the active stack's global layer: overlay-aware, and filtered to the
     chords that still reach the global table in a capturing mode. It stops advertising `q quit`
     where `q` is typed text.
   - One spelling (`KeyChord::label`) and one separator (` · `) are used throughout.
6. **The two routing defects found in §2.6 are fixed inside MOD-67**, because resolver dispatch
   removes them by construction:
   - modifier-blind browse arms, including the `ctrl-c` clear-DSN defect;
   - the Qdrant editor's pass-all.

   The missing `Down`/`Up` aliases in agents and hierarchy are added as defaults, since
   `list.down`/`list.up` default to `["j", "down"]`/`["k", "up"]` everywhere. That and `f1` for
   help (item 5) are the deliberate default changes besides `ctrl-c`. They add keys and remove
   none.

## 7. Design sketch

### 7.1 File format

`<config_root>/keys.toml`. It is optional, and a missing file means the defaults, silently. TOML
tables are contexts, keys are action names, and values are one chord or a list of chords. A list
replaces that action's default list; it does not add to it. `[]` unbinds.

```toml
# htui key bindings. List only what you change; `htui --print-keys` prints every action.
version = 1

[global]
quit = ["x"]                 # ctrl-c always quits as well and cannot be listed here
help = ["f1"]                # drop `?` (default ["?", "f1"])

[list]
down = ["j", "down", "ctrl-n"]
up   = ["k", "up", "ctrl-p"]

[common]
reload = "f5"                # every section's re-read

[settings.boxes]
reload = "r"                 # ...except here, where `r` stays

[backlog.runs]
reject = []                  # unbound: the hint drops it, the ? box omits it
cleanup = "ctrl-t"
```

**Chord grammar.** This is `KeyChord::parse`'s grammar (`keymap.rs:61-82`), made strict for the
file:
- Parts are trimmed, and modifiers are `ctrl`, `alt` and `shift`.
- A shifted letter is written as its uppercase character. `"shift-a"` is an error that suggests
  `"A"`, and `"ctrl-C"` is an error that suggests `"ctrl-c"`.
- `"shift-tab"` stays accepted as `BackTab`.
- Chords the legacy encoding cannot tell apart are errors that name the key they collide with:
  `ctrl-i` (`Tab`), `ctrl-m` (`Enter`) and `ctrl-[` (`Esc`).

The strict parser returns a reason. `parse` keeps its lenient behaviour for the test harness.

### 7.2 Action catalogue

There is one static table in `crates/htui/src/keys/catalogue.rs`, replacing `keymap.rs`. It is
the single source of truth for defaults, names and help:

```rust
pub struct ActionSpec {
    pub act: Act,                  // one enum, about 100 variants
    pub context: Ctx,              // Global, Overlay, List, Pane, Confirm, Form, Common, SettingsAgents, ...
    pub name: &'static str,        // "reload" -> written `[common] reload`
    pub defaults: &'static [&'static str], // spec strings, parsed by a unit test
    pub help: &'static str,        // "reload"; the ? box and the hint label default
    pub in_capture: bool,          // offered while a text field captures: CONTROL chords only
}
```

| Context (TOML table) | Actions (defaults) |
|---|---|
| `global` | `quit` (`q`; `ctrl-c` fixed), `next_tab` (`Tab`), `prev_tab` (`BackTab`), `select_tab_1`..`_9` (`1`..`9`), `help` (`?`, `f1`), `workspaces` (`w`), `find` (`ctrl-f`) |
| `overlay` | `close` (`Esc`; must stay bound) |
| `list` | `down`, `up`, `top`, `bottom`, `fold` |
| `pane` | `scroll_down` (`J`), `scroll_up` (`K`), `page_down`, `page_up`, `next_subtab`, `prev_subtab` |
| `confirm` | `yes` (`y`), `no` (`n`, `Esc`) |
| `form` | `next_field`, `prev_field`, `save` (`ctrl-s`), `external_editor` (`ctrl-e`) |
| `common` | `edit`, `new`, `delete`, `clear`, `reload`, `back`, `dismiss` |
| `settings` | `next_section`, `prev_section` |
| `settings.<section>`, `skills.<view>`, `backlog`, `backlog.<pane>`, `chat`, `requirements`, `concepts`, `switcher`, `migration` | view verbs (`settings.agents.probe`, `backlog.runs.approve`, `backlog.runs.next_step` (`J`), `concepts.reindex`, ...) |

Keys that §6.1 keeps fixed (characters, in-widget editing, a widget's Enter and Esc, numbered
answers, choice-field value keys) have no entry: they are not actions.

`htui --print-keys` renders this table as a complete, commented TOML file.

### 7.3 Scope model: context stacks

- Each view mode declares, statically, the contexts it offers, narrowest first. Settings >
  Connection in browse offers `settings.connection`, `settings`, `common`, `list`, then `global`.
  A capturing mode offers `form`, its view context, then `global` filtered to CONTROL chords.
- Every layer that routes keys by mode gains `fn key_layers(&self) -> &'static [Context]`, which
  returns its own contexts for the current mode, narrowest first: `Tab`, `Overlay`,
  `SettingsSection`, the Backlog `DetailTab` trait, the Skills views (`LibraryView`,
  `TemplatesView`, `AttachPane`) and the chat transcript. A parent composes its stack as the
  focused child's layers, then its own, then `global`. The Runs pane in browse is therefore
  `backlog.runs`, `pane`, `backlog`, `list`, `global`: the Backlog tab's `j`/`k`/`g`/`G`/`h`/`l`
  are its own layer, after the pane's. The validator walks the composed stacks, not the layers.
- The same stack drives three things:
  - dispatch: `ctx.keys().actions(stack, chord)` returns the **ordered candidates** for a chord,
    narrowest first. The view handles the first one it accepts and returns `Pass` to decline
    one, and the next candidate is tried. That keeps today's state-dependent keys: Backlog Enter
    folds a header, else goes to the detail pane, else reaches `backlog.runs.replay`'s miss
    message; connection's Enter acts only on the Rebuild row, and agents' `o`/`x` only while a
    flow runs. Views match on `Act`, not on `KeyCode`;
  - the `?` box;
  - the validator, which walks every declared stack. That is about 70 of them, and the count is
    checked by a unit test.
- `App::on_key` keeps its order (overlay, tab, global). The view layers call the resolver instead
  of matching `key.code`, and the `Keymap`/`Binding`/`KeyScope` types retire.
- `Action::Replay` and other payload-carrying actions stay emitted by views: an `Act` names the
  key, and the view maps it to the payload it holds. The Backlog Enter miss row becomes
  `backlog.runs.replay` resolving to the same error text, with the key named via `label`.

### 7.4 Load, merge and validation order

1. `lib.rs` resolves the file after the early-exit flags and before `connect::start`
   (`lib.rs:102`). The path is `--keys <PATH>`, or `<config_root>/keys.toml` found without
   creating anything. `identity::config_root()` creates the directory, so `htui-store` gains a
   non-creating sibling (`config_root_path()`); `crates/htui` does not take `dirs` directly.
   `--default-keys` skips the file.
2. Parse the TOML with spans kept (the `toml` crate's `Spanned`), so every later error has a
   line. `crates/htui` gains `serde` and `toml` as workspace dependencies.
3. Check the `version` key, and that every table and key name is in the catalogue.
4. Parse every chord strictly (§7.1).
5. Merge: the catalogue defaults, then the file's lists replacing per action and per context.
   A narrower context's entry for a shared verb creates an override for that view only. The
   user's binding wins (MOD-12 M3 R1 H1, maintainer 2026-10-08): when an entry shares a chord
   with an action of **the same context** that the file leaves at its default (no entry for it),
   that action loses the chord, and is unbound if it has no other. A state-guarded pair that
   shares the chord by default keeps sharing it; `overlay.close` never gives way. htui prints
   one notice per chord on stderr (`htui: PATH:LINE: [global] quit = "ctrl-q" takes "ctrl-q"
   from global.queue, which is now unbound`) and starts; `--print-keys` marks the action's line
   `(unbound by quit)`, or `(lost "j" to top)` when it keeps other chords. An entry that repeats
   an action's default is still an entry.
6. Refuse any `ctrl-c` in the file. Add the fixed `ctrl-c`. Require `overlay.close` to be
   non-empty.
7. For every composed stack, report:
   - two actions sharing a chord, unless both are defaults and the pair is on the catalogue's
     reviewed allow-list. The allow-list has two kinds of entry: **shadowing** (the narrower
     action always wins: `form.next_field` over `global.next_tab`, the chat permission digits
     over `global.select_tab_*`) and **state-guarded** (the view accepts at most one of the pair
     in any state, and declines the other so it falls through: `list.fold` and
     `backlog.runs.replay` on Enter). A pair a user creates between two of the file's entries
     is always an error, and so is a chord an entry shares with another context's default
     across a stack's layers (step 5 gives way only within one context);
   - an `in_capture` action bound to a printable chord (§6.4).
8. With any error: write every error to stderr and exit with status 2. Otherwise build the
   immutable `Keys` and pass it to `App::new`. There is no hot reload. Today `main.rs` maps every
   error from `run` to exit status 1 and reports it to Sentry/GlitchTip (`main.rs:39-43`), so the loader returns a
   typed `KeysError` that `main.rs` maps to status 2 and keeps out of error capture. A typo in a
   user's file is not a crash, and the report would carry their home path.

A unit test asserts that the compiled defaults pass steps 3-7 with no errors. Integration tests
load fixture files from a temporary root, following the `connect.rs:205-207` pattern. `testkit`
always uses the compiled defaults.

### 7.5 Error reporting

```
htui: /home/u/.config/htui/keys.toml has 3 errors:
  keys.toml:9: [global] quit = "ctrl-c": ctrl-c always quits and cannot be bound
  keys.toml:14: [common] edit = "shift-e": write a shifted letter as "E"
  keys.toml:21: [backlog.runs] retry = "a": "a" is already backlog.runs.approve (default) in the Runs pane
Fix the file, or run `htui --default-keys` to start with the default keys.
```

`htui --print-keys` prints the resolved keymap as TOML, marking user-changed lines. With an
invalid file it prints the errors instead and exits with status 2. It is therefore the check
command as well.

### 7.6 Generated hints and help

```rust
const BROWSE: HintSpec = &[
    Hint::One(Act::Edit, "edit DSN"), Hint::One(Act::Clear, "clear DSN"),
    Hint::One(Act::ConnectionRebuild, "rebuild cache"), Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
```

- `keys.hint(stack, BROWSE)` renders `e edit DSN · c clear DSN · R rebuild cache · r reload · j/k rows`
  with the defaults. It resolves each action through the stack, so an override in a narrower
  context shows correctly. It uses the first chord of each action and drops unbound ones.
- Labels come from `KeyChord::label`. Plain characters render as themselves.
- Prose uses `keys.label(stack, act)`, as in `format!("press {} or {}", ...)`.
- The `?` box renders one line per context of the active stack, with every chord of every bound
  action joined by `/`, under the context's heading. The literal "? closes this box" becomes the
  generated `help` entry.

## 8. Phasing

One implementing item, **MOD-67** (from ANA-26). Every milestone leaves the defaults working,
except the `ctrl-c` fix and the added `Down`/`Up` aliases. Every milestone keeps the spec-string
integration tests green unchanged.

| Milestone | Scope | Files (rough) |
|---|---|---|
| **M1 Catalogue and resolver** | `keys/` module: `Act`, `ActionSpec`, catalogue for `global`/`overlay`/shared contexts, strict chord parser, `Keys` resolver, `Stack`. `App::on_key` checks `ctrl-c` first, and the global/overlay layers use the resolver. The status line and `?` box are generated. The `keymap.rs` tests move. | ~8: `keys/{mod,chord,catalogue,stack,hint}.rs`, `app/state.rs`, `app/mod.rs`, `lib.rs`, `testkit.rs`. `Ctx::new(&Keymap)` has 22 call sites in 11 files, most of them test benches in files that M3-M5 own, so M1 keeps `Ctx::new(&Keymap)` unchanged: it fills in the compiled-default `Keys`, and only `App` replaces them through a new `Ctx::with_keys`. No M3-M5 file is touched early. |
| **M2 The file** | Loader with spans, validator (§7.4 steps 3-8), `--keys`, `--default-keys`, `--print-keys`, error report. `crates/htui` gains `serde` and `toml`. Integration tests over fixture files. Global and overlay keys are configurable end to end. | ~9: `keys/{load,validate}.rs`, `cli.rs`, `lib.rs`, `main.rs` (typed `KeysError` → status 2, kept out of error capture), `htui-store/src/identity.rs` (non-creating `config_root_path()`), `crates/htui/Cargo.toml`, `tests/keys_file.rs`, `Cargo.lock` |
| **M3 Settings and overlays** | Seven sections and the Settings tab dispatch through stacks; hint specs replace their 37 constants; §2.6 defects 1 and 2 fixed and pinned by tests; the three overlays converted. Snapshot re-baseline for these areas. | ~11 source files, ~46 snapshots |
| **M4 Skills and Requirements** | `skills/{mod,library,templates,attach}.rs`, `requirements/{mod,forms}.rs`; `form.*` actions; Skills `h`/`l` modifier blindness fixed. | ~6 source files, ~21 snapshots |
| **M5 Backlog and Chat** | `backlog/mod.rs`, `detail/{mod,body,documents,graph,notes,prompt,requirements,runs}.rs`, `chat/{mod,transcript}.rs`. The Runs pane gets a generated browse hint. The Enter miss row becomes an action. The Backlog modal ALT inconsistency is resolved by the resolver. | ~11 source files, ~38 snapshots |
| **M6 Close-out** | Prose sites that name keys go through `label`. `Keymap`/`Binding`/`KeyScope` are deleted. A unit test checks that every catalogue action is offered by some stack and every stack validates. A key reference page is generated from `--print-keys` into the docs. `Ctx::new` takes `&Keys`: its 22 call sites in 11 files change once, here. | ~15 |

M3 to M5 are independent of each other and could run in parallel, because M1 leaves the `Ctx`
constructor alone. They all touch snapshots, which couples "disjoint" tasks, so the snapshot re-baseline is verified on the real tree after
each merge.

## 9. Requirement changes

Proposed here and **approved by the maintainer on 2026-09-30**; applied to `docs/REQUIREMENTS.md`
in the close-out commit, as worded below.

**R-TUI-1, amended** (current text at `docs/REQUIREMENTS.md:312-315`). The first sentence gains a
pointer:

> **R-TUI-1 (must).** Keyboard driven, mouse optional; keys are configurable (R-TUI-10). Top bar:
> workspace or project, box, store state — distinguishing online, connecting, and offline since T
> — active run count. Tabs: Backlog, Chat (one per session), Skills, Requirements, Settings.
> Workspace switcher overlay. Queue overlay for auto mode with reorder and pause.

**R-TUI-10, new:**

> **R-TUI-10 (must).** Every key outside text entry is bound to a named action. The default
> bindings ship in `htui`. A user overrides them in a local file, `keys.toml` under the user's
> config directory, per OS user and per machine; the file lists only the actions it changes, may
> unbind one, and is never read from or written to the store, so the keys in force are the same
> offline. Printable characters, editing keys inside a text field, a field's Enter and Esc,
> numbered choices, and a choice field's value keys are fixed. `htui` refuses to start on an
> invalid file and names its path and line for every error, including a chord bound twice on one
> screen and a printable chord bound where a text field is typing; `htui --default-keys` starts
> with the defaults instead. `ctrl-c` always quits, from every htui screen and field, and cannot
> be unbound or bound to anything else; a pane that runs another program in a terminal forwards
> it to that program and always keeps a key that leaves the pane. Every hint line and the `?`
> help show the keys in force, and the help opens from every screen.

**R-STO-1, clarified** (`docs/REQUIREMENTS.md:140-143`). Add one sentence:

> Local files under the user's config directory that hold no secret and no domain data (the box
> identity, the read-only cache, the key bindings of R-TUI-10) are not a store.

## 10. Risks and open questions

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A broken `keys.toml` blocks the everyday command | Medium | Medium | All errors in one report with lines; `--default-keys`; `--print-keys` as the check |
| A cross-stack collision the allow-list misses leaves a key dead in one mode | Medium | Medium | The validator walks every declared stack; a unit test fails when a view mode has no stack |
| Stacks drift from what a view actually matches | Medium | Medium | Views match only `Act` values returned by their stack; M6's test rejects a stack action no view handles |
| The snapshot re-baseline hides a real regression | Medium | Low | Re-baseline per milestone and per area; the diff is reviewed as text-only (spelling, separators) |
| Legacy-encoding aliases differ per terminal (`ctrl-h`, `ctrl-j`) | Medium | Low | Start with the three certain aliases; the M1 plan checks crossterm's parser and extends the reject list with evidence |
| Users expect keys to follow them across boxes | Low | Low | Dotfile sync; a store layer can sit under the file later |
| About 100 actions make `--print-keys` long | High | Low | Grouped by context with help comments; users list only changes |

**Open for the maintainer or the MOD-67 plan:**

- The probe spelling (`r` in agents, `p` in boxes) and the `Down`/`Up` gaps are default choices.
  MOD-67 keeps `r`/`p` as they are. Harmonising them is a behaviour change the maintainer may
  want as its own item.
- Whether a read-only "Keys" section in Settings (`R-TUI-8`, `REQUIREMENTS.md:330-334`) should
  show the file path and the resolved table. The `?` box and `--print-keys` cover the need, so
  it is not planned.
- Whether a store layer for keys is ever wanted, once team use (`R-USR-3`) or multi-box sync
  makes it worth the migration. The order would be defaults < store < file.
- The exact `toml` API for spans (`Spanned` on a typed struct versus `toml::de` document spans)
  is a plan detail.
- MOD-57's in-pane editor "reserved chord to leave" (`HANDOFF.md:630-639`) should be an action
  in this catalogue from the start, not a new hard-coded key. It is also the one place `ctrl-c`
  does not quit (§6.4): MOD-57 forwards it to the child, and must keep its leave action bound,
  which the validator enforces like `overlay.close`.
- Kitty keyboard protocol (`PushKeyboardEnhancementFlags`) would make `ctrl-i`, `ctrl-m` and
  `shift-<char>` distinct. It is out of scope, and the reject list would shrink if it lands.

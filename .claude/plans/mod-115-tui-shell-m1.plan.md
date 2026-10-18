# Plan: MOD-115 M1 — widget kit + shell frame

**Source PRD**: `.claude/prds/mod-115-tui-shell.prd.md`
**Selected Milestone**: 1 — Widget kit + shell frame
**Complexity**: Large (new module, chrome change that touches every frame, wholesale snapshot rewrite of the chrome rows)

## Summary
Add a `ui/kit` module holding the shared parts (pane frame with title, sub-tab chips and bottom-border hints;
selection band; key chips; empty state; modal frame; status glyph + word; legend), replace the five-row chrome
(top bar, tab strip, hint row, status line) with the one-row header plus a body that tabs fill with kit panes, and
add the `F1` about panel. One real small screen (the waiting-list overlay and the quit-confirm modal frame) is
ported as the pilot so the kit API is proven on real state before it is frozen. Tabs themselves keep their
current bodies in M1; they are ported in M3/M4. Layouts, pane navigation, history and the leave dialog are M2.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| Chrome today is five fixed rows: top bar, tab strip, body, hint, status | true | `ui/layout.rs:24-40` (`chrome`), test at `layout.rs` asserts the five `Rect`s |
| One function draws the whole frame | true | `app/state.rs:873-941` `App::render`; also called by `testkit::render` |
| Per-tab hints come from `Tab::key_hint` into the hint row; status line carries errors and the global help line | true | `state.rs` render body (read) |
| ratatui is 0.30.2, single copy in the tree | true | `Cargo.toml:66`, comment at line 71 |
| Snapshots are insta, asserted in `src/` and `tests/` | true, count differs | 190 `assert_snapshot!` calls across 36 files (findings doc says 199 snapshot files; the two are not the same measure) |
| Tabs registered today: chat, backlog, requirements, skills, settings | partly verified | module list from the mapping agent; the registration order was not read |
| Sub-tab strips are drawn three separate ways (settings, backlog detail, and likely skills/requirements) | partly verified | `settings/mod.rs:480-495`, `backlog/detail/mod.rs:404-428`; the other two unconfirmed |
| Theme has role styles and a monochrome mode with a test that styles survive it | true | `ui/theme.rs:21-103`; test `active_main_tab_is_bold_and_underlined_without_colour` |
| Keymap is rebindable through `Keys`/`load_str`; `Act::NextTab`/`SelectTab1..9` exist | true | `keys/catalogue.rs:361-381` |
| Any existing UI-layout persistence | not found | no hit in the graph; absence not proven. Task 6 must confirm before choosing a home (PRD open question 1) |
| Snapshot impact of the chrome change | falsifiable only by running | Task 7 runs a full `cargo insta test`; grepping `.snap` undercounts (memory note) |

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `ui/layout.rs` | `//!` module doc stating what is shared; `#[must_use]` on pure geometry fns; struct of named `Rect`s |
| Rendering | `tabs/registry.rs:254-266` `render_strip` | `(frame, area, data, &Theme)`; styles from `Theme` roles, never raw colours |
| Rendering | `backlog/detail/compose.rs:585-661` | render returns the inner body `Rect` |
| Errors | `keys/load.rs:98` | fallible loaders return `Result<_, Vec<…Error>>`; render paths never panic |
| Tests | `top_bar.rs:161-245`, `registry.rs:280-299` | `TestBackend` read back as strings; sentence-named tests; one test per style claim proving it survives `Theme::monochrome` |
| Snapshots | `tests/shell.rs` | insta through `testkit` `Bench`/`Shell` |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui/src/ui/kit/mod.rs` | CREATE | module root, re-exports |
| `crates/htui/src/ui/kit/pane.rs` | CREATE | rounded pane: title, sub-tab chips, bottom-border hints, focus, scrollbar |
| `crates/htui/src/ui/kit/list.rs` | CREATE | selection band, gutter marker, ellipsis, groups, "hidden" note row |
| `crates/htui/src/ui/kit/chips.rs` | CREATE | key chips, legend |
| `crates/htui/src/ui/kit/status.rs` | CREATE | glyph + human word for every status (data table, one source) |
| `crates/htui/src/ui/kit/empty.rs` | CREATE | empty state with a next step |
| `crates/htui/src/ui/kit/modal.rs` | CREATE | modal frame over `layout::centered` |
| `crates/htui/src/ui/header.rs` | CREATE | one-row header; replaces `top_bar.rs` and the strip |
| `crates/htui/src/ui/about.rs` | CREATE | `F1` about panel |
| `crates/htui/src/ui/layout.rs` | UPDATE | `Chrome` becomes header, optional about panel, body |
| `crates/htui/src/ui/top_bar.rs` | DELETE | superseded by `header.rs` |
| `crates/htui/src/ui/tabs/registry.rs` | UPDATE | `render_strip` replaced by header tab chips; titles/accents stay here |
| `crates/htui/src/ui/theme.rs` | UPDATE | tab accents and kind colours as roles, with monochrome fallbacks |
| `crates/htui/src/app/state.rs` | UPDATE | `render` uses new chrome; flash message moves into the header; help-line moved |
| `crates/htui/src/keys/catalogue.rs` | UPDATE | `F1` about toggle action; hint source for the focused pane's border |
| `crates/htui/src/ui/overlay/waiting_list.rs`, `quit_confirm.rs` | UPDATE | pilot: modal frame and list on the kit |
| `crates/htui/src/testkit.rs` | UPDATE | if the render helper assumes the old chrome |
| `crates/htui/src/ui/tabs/*/…` | UPDATE (chrome-facing lines only) | drop the hint-row output; hints now come to the pane border |
| `crates/htui/tests/*.rs` snapshots | UPDATE | chrome rows change on every frame |

## Tasks
Tasks 1-4 touch disjoint new files and may run in parallel after the kit's types are named in Task 0;
Tasks 5-7 are serial (shared files). Independence is by the file lists above, to be intersected again at fan-out.

### Task 0: Kit API sketch (serial, tiny)
- **Action**: declare the public types and fn signatures of `kit` in `mod.rs` (empty bodies, `todo!`-free stubs
  returning the input area) so tasks 1-4 compile against one surface.
- **Validate**: `cargo check -p htui`

### Task 1: Pane + scrollbar (`kit/pane.rs`)
- **Action**: tests first (title, sub-tab chip placement from `area.right() - 1 - width`, hints trimmed to width so
  they never overwrite the corner, unfocused border faint, scrollbar stops at end, monochrome keeps focus visible).
- **Mirror**: `top_bar.rs` tests, `registry.rs:280-299`
- **Validate**: `cargo test -p htui --features testkit kit::pane -- --test-threads=1`

### Task 2: List parts (`kit/list.rs`, `kit/status.rs`)
- **Action**: selection band + `▌` gutter, ellipsis on long rows, group rows, hidden-note row, status table
  (`● done ◐ running ⚠ needs you ✗ failed ⊘ blocked ◌ queued ○ open ✓ closed`) with no raw internal names.
- **Validate**: `cargo test -p htui --features testkit kit::list kit::status -- --test-threads=1`

### Task 3: Chips, legend, empty state, modal (`kit/chips.rs`, `empty.rs`, `modal.rs`)
- **Action**: tests first; modal sits on `layout::centered`; empty state always carries a next-step line.
- **Validate**: `cargo test -p htui --features testkit kit:: -- --test-threads=1`

### Task 4: Header + about panel (`header.rs`, `about.rs`, theme roles)
- **Action**: header priority-drops its right side when narrow; tab chips clickable regions recorded; accents are
  `Theme` roles with monochrome fallback; about panel is two lines, `F1` toggles, default on at first launch.
  The history chip and background count render nothing in M1 (data arrives in M2 / MOD-116).
- **Validate**: `cargo test -p htui --features testkit header about -- --test-threads=1`

### Task 5: New chrome in the frame (serial)
- **Action**: `layout::Chrome`, `App::render`, delete `top_bar.rs`/strip; status text and errors become the
  header's flash; the global help line is reachable via `?`. Tabs stop emitting hint rows.
- **Validate**: `cargo build -p htui` then `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`

### Task 6: Pilot port + persistence decision (serial)
- **Action**: port `waiting_list` and `quit_confirm` onto the kit; confirm whether any UI-state persistence
  exists and record the finding in the plan's verified-claims table (answers PRD open question 1 before M2).
- **Validate**: pilot tests + review the two overlays' snapshots by eye.

### Task 7: Snapshot rewrite for the chrome rows (serial)
- **Action**: `cargo insta test --all-features`, review every diff (only chrome rows may differ; a body change is a
  defect), accept. Do not accept blindly: each tab's diff is read before the next.
- **Validate**: `cargo insta test --all-features --check`; `cargo clippy --workspace -- -D warnings` and
  `cargo clippy --workspace --all-features -- -D warnings` (featureless gate, see memory).

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
cargo insta test --all-features --check
```
Postgres-backed tests are not part of this milestone's gate; the host runs the real Postgres gate on the merged tree.

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| Removing the hint row and status line loses information users rely on (errors, global help) | Medium | errors become the header flash; `?` opens the help; both asserted by tests before the old rows go |
| Every snapshot's top rows change, hiding a real body regression in the diff | High | Task 7 reviews per tab; the body region must be byte-identical apart from its row offset |
| Pilot overlays are too small to prove the kit | Medium | Task 6 may add `quit_confirm`'s list rows; if the API bends, fix it before M3 (kit is frozen at M1 end) |
| `F1` or digit keys collide with existing catalogue bindings or `SHADOWING` | Low | `keys` validate tests run; add the action through the catalogue, never a hard-coded match |
| Stack-size or test-order flakiness in the full run | Medium | gate with `--test-threads=1`, `--no-fail-fast`; re-run serially before calling a regression |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] Kit API frozen and documented at the end of M1
- [ ] Body regions unchanged in snapshots apart from row offset

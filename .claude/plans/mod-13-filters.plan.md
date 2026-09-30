# Plan: MOD-13 milestone 1 — Backlog filters

**Source PRD**: `.claude/prds/mod-13-backlog-editing.prd.md`
**Selected Milestone**: 1 — Filters
**Complexity**: Medium

## Summary
The Backlog list can be narrowed by status, project, capability (required tags) and readiness,
and the list pane shows which filters are active. The store half already exists: `ItemFilter`
carries statuses, projects, tags and `ready` on all three backends. Milestone 1 adds the TUI
filter state and a filter form inside the Backlog tab. It also adds one worker-side composition,
so that "ready" means ANA-9 §7.4 *for this box*: `ready` in the store filter, plus required tags
covered by this box's `probed_tags ∪ declared_tags`.

## Design decisions
- **D1: filter state lives in `BacklogTab`, and the form is an inline, capturing panel in the list
  pane, not an overlay.** The overlay registry's doc anticipates "MOD-13's filter overlay", but an
  overlay `Factory` is `Fn() -> Box<dyn Overlay>`. It cannot be seeded with the current filter, and
  handing the result back to the tab would need a new `Action` routed by the shell. The tab already
  has a capture guard (`detail.captures_input()`), so the form reuses that pattern and adds no
  shell surface.
- **D2: the readiness filter reads this box inside the worker.** `StoreRequest::Items` gains
  `ready_here: bool`. When it is set, the worker sends `ItemFilter { ready: Some(true), .. }` to
  `backend.items` and keeps only the items whose `required_tags ⊆ probed_tags ∪ declared_tags` of
  `backend.box_info()`. `box_info()` returning `None` (an unregistered box) counts as no tags,
  the same as §7.4's `LEFT JOIN … COALESCE`. There are four reasons:
  - the view never holds a `BoxId` or tags, following the MOD-15 D5/D6 "worker fills identity"
    rule;
  - the result stays conjunctive with the other filters;
  - it answers on all three backends, the offline mirror included. `Backend::ready_items` refuses
    offline, and MOD-25 says an offline box still *browses*;
  - it keeps one request kind, so the shell's staleness index cannot mix a filtered reply with an
    unfiltered one.

  Rejected alternatives: a new `ReadyItems` request, which would be a second kind answering
  `StoreReply::Items`, and a `BoxId` in `ItemFilter`, which would change three backends' SQL.
- **D3: tags go through `declared_tags_from_text`**, the same canonicalisation Settings > Boxes
  uses: comma-split, trimmed, sorted, deduplicated, with uppercase or spaced tags refused by name.
  The capability filter is `ItemFilter.tags` ("required_tags contain all of these").
- **D4: a reveal (MOD-64) of an item the active filter hides clears the filter** and re-reads,
  then selects the item. This keeps the existing D251 "not in this backlog" error meaningful: it
  only fires when the item is not in the workspace at all.
- **D5: no filter set means no change on screen.** The list title and every existing snapshot stay
  byte-identical. An active filter appends a compact summary to the title, and an empty filtered
  result says "No items match the filter." instead of "No items in this workspace."
- **D6: the filter state is kept across a scope change, minus the projects the new scope lacks.**
  Status, tag and readiness choices are scope-independent. A project that is gone is dropped from
  the filter so it cannot silently empty the list.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Tab key handling and capture guard | `crates/htui/src/ui/tabs/backlog/mod.rs` (`on_key`) | Capture guard first, then CONTROL/ALT to the detail pane, then the letter match; unmatched keys fall to `detail.on_key` |
| Store request and serve arm | `crates/htui/src/store_worker.rs:115-121` (`Items`), `:1431` serve arm | Request carries only what the view knows; the worker fills identity (`Backend::box_info`) |
| Tag canonicalisation | `crates/htui-core/src/model/box_.rs:275` `declared_tags_from_text`; caller `ui/tabs/settings/boxes.rs:467` `submit` | Parse the text field, and put a refusal on the status line as `Action::Error` |
| Text field | `crate::ui::TextField` (used across the Settings sections) | Single-line field inside a capturing form |
| Unit tests on the tab | `mod.rs` tests (`platform()`, `Ctx::new`, `Emit`) | Build the tab over `MemStore::demo()` rows and assert on emitted `StoreRequest`s |
| Integration and snapshots | `crates/htui/tests/backlog.rs` (`backlog()`, `down`, `insta` snapshots at 100x30) | `Harness::demo()`, `harness.key(..)`, `drive_to_end`, `backlog__*.snap` |
| Readiness parity | `crates/htui-store/tests/pg_criteria.rs:3759-3890` | `ready_items` is already parity-checked Mem vs Pg; the new composition is pinned against `MemStore::ready_items` |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui/src/ui/tabs/backlog/filter.rs` | CREATE | `BacklogFilter` (statuses, projects, tags, ready), `to_request(scope)`, summary text, the capturing form (state, keys, render) |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | Hold the filter; `f` opens the form, `F` clears it; the capture guard includes the form; `wants_requests`/`reveal` use the filter (D4); scope change (D6) |
| `crates/htui/src/ui/tabs/backlog/list.rs` | UPDATE | `ListView` gains an optional filter summary; the title and empty-state text follow D5 |
| `crates/htui/src/store_worker.rs` | UPDATE | `Items { .., ready_here }`; the serve arm composes D2; unit tests on the composition; update the existing `Items` constructions in tests |
| `crates/htui/src/app/update.rs` | UPDATE | Two `StoreRequest::Items` constructions gain `ready_here: false` |
| `crates/htui/tests/backlog.rs` | UPDATE | Integration cases and new snapshots |
| `crates/htui/tests/snapshots/backlog__filter_*.snap` | CREATE | Form open, and a filtered list with its summary title |

The seven `StoreRequest::Items { .. }` construction sites (verified claims) get
`ready_here: false` unless they carry a filter.

## Tasks
### Task 1: worker composition (TDD)
- **Action**: Write the tests first, in `store_worker.rs` tests over `MemStore::demo()`:
  - `ready_here: true` with a default filter equals `MemStore::ready_items(scope, box)` for the
    demo box (same ids, same order);
  - it stays conjunctive with `project_ids` and `tags`;
  - `ready_here: false` behaves exactly as before.

  Then add the field and the serve arm.
- **Mirror**: The `try_serve` arms; the MOD-15 identity-from-`Backend` rule.
- **Validate**: `cargo test -p htui --all-features --lib store_worker`

### Task 2: `BacklogFilter` and the form (TDD)
- **Action**: Write the unit tests first:
  - `to_request` maps each field;
  - an empty filter maps to `ItemFilter::default()` with `ready_here: false`;
  - the summary text;
  - form keys: `j`/`k` move between rows, `space` toggles, `h`/`l` move within status and project
    options, the tags row edits a `TextField`, `Enter` applies, `Esc` cancels, `x` clears;
  - a refused tag list keeps the form open and emits `Action::Error`.

  Then implement `filter.rs`.
- **Mirror**: The Settings text-field forms; the `declared_tags_from_text` refusal path.
- **Validate**: `cargo test -p htui --all-features --lib backlog::filter`

### Task 3: wire into `BacklogTab` and the list (TDD)
- **Action**: Write the tests first:
  - `f` opens the form and it captures (`j` does not move the list cursor);
  - applying emits exactly one `StoreRequest::Items` carrying the filter;
  - `F` clears and re-reads;
  - `wants_requests` carries the active filter;
  - a reveal of a hidden item clears the filter (D4);
  - a scope change drops the projects that are gone (D6);
  - with no filter set, the list title is unchanged (D5).

  Then implement. Apply a filter by setting it and emitting `ctx.request(Items{..})`; `on_reply`
  and `reselect` already handle the new rows.
- **Mirror**: `mod.rs` unit tests (`platform()`, `CapturingProbe`).
- **Validate**: `cargo test -p htui --all-features --lib backlog`

### Task 4: integration and snapshots
- **Action**: Write these cases in `tests/backlog.rs`:
  - filter by status `done`, and the list shows only done items;
  - filter by one project in Platform;
  - filter by the tag `rust` (or whichever tag the demo carries);
  - readiness: the list equals the demo's `ready_items`;
  - the snapshot `backlog__filter_form`;
  - the snapshot `backlog__filtered_list`.

  Accept the new snapshots. Every existing `backlog__*.snap` stays untouched.
- **Mirror**: The existing `backlog()` harness and `insta::assert_snapshot!` usage.
- **Validate**: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`

Task order: **serial, 1 → 2 → 3 → 4.** The draft marked Tasks 1 and 2 as independent. The
fact-check struck that marking: `filter.rs` builds `StoreRequest::Items { .., ready_here }`, which is
Task 1's field, and it cannot compile without the `pub mod filter;` line in `mod.rs`, which is
Task 3's file. The file sets intersect in compile terms. Every later task needs the one before it.

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features -- --test-threads=1   # full gate before close of the milestone
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| Worker-side capability check drifts from §7.4's SQL (e.g. NULL box row, tag case) | M | Parity test against `MemStore::ready_items`, which `pg_criteria.rs` already pins against `PgStore` |
| `box_info()` fails offline or before registration, so readiness returns an error | M | The request answers `Failed` like any other read, and the status line shows it. No silent empty list. Pinned by a test |
| The form's letters collide with detail sub-tab letters | L | The form captures while open; `f`/`F` are unused by every detail sub-tab (checked below) |
| A snapshot changes although no filter is set | L | D5, plus the unchanged existing `backlog__*.snap` files as the check |
| The staleness index drops a filtered reply | L | One request kind (D2); a test that applying twice shows the second result |

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| `ItemFilter` has `statuses`, `project_ids`, `tags`, `ready` | ✓ | `crates/htui-core/src/model/item.rs:215-229` |
| All three backends honour `ItemFilter.ready` (store-side half of §7.4) | ✓ | `mem.rs:959` (`is_ready`), `pg/read.rs:122`, `cache/read.rs:308-316` (SQLite mirror) |
| `Backend::ready_items` refuses offline | ✓ | `crates/htui-store/src/backend.rs:540-545` → `orchestration_offline()`; this is why D2 does not call it |
| `Backend::box_info` answers on Memory, Online **and Offline** | ✓, amended | `backend.rs:252-258`. It returns **`Option<BoxInfo>`**, so `None` means an unregistered box. D2 treats `None` as "no tags", which matches §7.4's `LEFT JOIN box … COALESCE(.., '{}')` (`pg/read.rs:1871-1911`) |
| `BoxInfo` carries `probed_tags` and `declared_tags` | ✓ | `crates/htui-core/src/model/box_.rs:295-308` |
| Demo data makes readiness parity non-trivial | ✗ **struck** (blueprint E1) | The tagged items (`docker`, `gpu,vulkan`) are `awaiting_approval`/`in_progress`, so none passes `ready`; the capability half removes nothing in the demo. The tests mint an open `cuda`-tagged item (as `mem.rs:10540` does) |
| `ready_items` is parity-checked Mem vs Pg | ✓ | `crates/htui-store/tests/pg_criteria.rs:3759-3890` |
| Staleness is keyed by origin plus request name, and `Items` is one name | ✓ | `store_worker.rs:826-831` (`"items"`); `app/update.rs:93, 612-620` |
| `StoreRequest::Items { .. }` construction sites | ✓, amended | 7 sites: `app/update.rs:529, 1220`; `store_worker.rs:2592, 2864`; `backlog/mod.rs:219, 357, 545`. `update.rs:574, 1254` are `{ .. }` patterns and need no edit. There is no `testkit.rs` site, so that row was dropped from the file list |
| `f`/`F` are free on the Backlog list and in every detail sub-tab | ✓ | Every `KeyCode::Char` in `ui/tabs/backlog/**`: no `f`/`F`. The only `f` in the crate is **Ctrl**+F (concepts search, `app/mod.rs:87`), a CONTROL chord |
| `declared_tags_from_text` splits, trims, sorts, dedups and refuses by name | ✓ | `box_.rs:258-283` |
| `TextField` is exported from `crate::ui` | ✓ | `crates/htui/src/ui/mod.rs:15` |
| An overlay factory cannot be seeded with state (D1 rationale) | ✓ | `ui/overlay/registry.rs`: `type Factory = Box<dyn Fn() -> Box<dyn Overlay>>` |
| Tasks 1 and 2 are independent | ✗ struck | See the task order note. The shared compile surface is the `Items` field and the `mod.rs` module line |

Blueprint: `.claude/plans/mod-13-filters.blueprint.md` (errata E1–E7 amend this plan; the blueprint wins on detail).

## Acceptance
- [x] All tasks complete
- [x] Validation passes
- [x] Patterns mirrored, not reinvented
- [x] No existing snapshot changed
